//! Windows BLE transport (WinRT), with acknowledged delivery.
//!
//! Differences from the 1.x Python stack that matter for reliability:
//! * Connects straight to the known address (no 6–8 s scan before every action).
//! * Write chunks are sized to the negotiated ATT MTU instead of a fixed 244
//!   bytes, so Windows never falls back to "long writes" the firmware rejects.
//! * One notification subscription for the whole session (1.x re-subscribed
//!   for every device-info request); stale ACKs are drained before each window.
//! * Every GATT object is closed on drop so Windows really disconnects and the
//!   panel starts advertising again.

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::{Duration, Instant};

use windows::core::{RuntimeType, GUID};
use windows::Devices::Bluetooth::Advertisement::{
    BluetoothLEAdvertisementReceivedEventArgs, BluetoothLEAdvertisementWatcher, BluetoothLEAdvertisementWatcherStatus,
    BluetoothLEScanningMode,
};
use windows::Devices::Bluetooth::GenericAttributeProfile::{
    GattCharacteristic, GattClientCharacteristicConfigurationDescriptorValue, GattCommunicationStatus, GattDeviceService,
    GattSession, GattValueChangedEventArgs, GattWriteOption,
};
use windows::Devices::Bluetooth::{BluetoothCacheMode, BluetoothLEDevice};
use windows::Foundation::{AsyncStatus, EventRegistrationToken, IAsyncOperation, TypedEventHandler};
use windows::Storage::Streams::{DataReader, DataWriter};

use crate::protocol::{self, parse_notify, AckPolicy, DeviceInfo, Notify, Plan, DEVICE_PREFIX};

pub fn init_thread() {
    unsafe {
        let _ = windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED);
    }
}

fn wait<T: RuntimeType + 'static>(op: windows::core::Result<IAsyncOperation<T>>, timeout: Duration, what: &str) -> Result<T, String> {
    let op = op.map_err(|e| format!("{what}: {}", e.message()))?;
    let start = Instant::now();
    loop {
        match op.Status() {
            Ok(AsyncStatus::Completed) | Ok(AsyncStatus::Error) => {
                return op.GetResults().map_err(|e| format!("{what}: {}", e.message()));
            }
            Ok(AsyncStatus::Canceled) => return Err(format!("{what}: iptal edildi")),
            Ok(_) => {}
            Err(e) => return Err(format!("{what}: {}", e.message())),
        }
        if start.elapsed() > timeout {
            let _ = op.Cancel();
            return Err(format!("{what}: {} sn içinde yanıt yok", timeout.as_secs()));
        }
        sleep(Duration::from_millis(10));
    }
}

pub fn format_address(a: u64) -> String {
    (0..6).rev().map(|i| format!("{:02X}", (a >> (i * 8)) & 0xff)).collect::<Vec<_>>().join(":")
}

pub fn parse_address(s: &str) -> Option<u64> {
    let hex: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    (hex.len() == 12).then(|| u64::from_str_radix(&hex, 16).ok()).flatten()
}

#[derive(Clone, Debug)]
pub struct Found {
    pub name: String,
    pub address: u64,
    pub rssi: i16,
}

/// Active scan for LED panels. Stops early when `stop_when` matches.
pub fn scan(duration: Duration, known: Option<u64>, stop_when: &dyn Fn(&Found) -> bool) -> Result<Vec<Found>, String> {
    let seen: Arc<Mutex<HashMap<u64, Found>>> = Arc::default();
    let watcher = BluetoothLEAdvertisementWatcher::new().map_err(|e| e.message().to_string())?;
    watcher.SetScanningMode(BluetoothLEScanningMode::Active).map_err(|e| e.message().to_string())?;
    let sink = seen.clone();
    let handler = TypedEventHandler::<BluetoothLEAdvertisementWatcher, BluetoothLEAdvertisementReceivedEventArgs>::new(
        move |_, args| {
            if let Some(a) = args.as_ref() {
                let address = a.BluetoothAddress()?;
                let name = a.Advertisement()?.LocalName().map(|n| n.to_string()).unwrap_or_default();
                let rssi = a.RawSignalStrengthInDBm().unwrap_or(-127);
                let mut map = sink.lock().unwrap();
                let entry = map.entry(address).or_insert(Found { name: String::new(), address, rssi });
                if !name.is_empty() {
                    entry.name = name;
                }
                entry.rssi = rssi;
            }
            Ok(())
        },
    );
    let token = watcher.Received(&handler).map_err(|e| e.message().to_string())?;
    watcher.Start().map_err(|e| format!("Bluetooth taraması başlatılamadı: {}", e.message()))?;
    let start = Instant::now();
    let mut result: Vec<Found>;
    loop {
        sleep(Duration::from_millis(100));
        if watcher.Status().ok() == Some(BluetoothLEAdvertisementWatcherStatus::Aborted) {
            let _ = watcher.RemoveReceived(token);
            return Err("Bluetooth kapalı veya kullanılamıyor. Windows Bluetooth ayarını kontrol edin.".into());
        }
        result = seen
            .lock()
            .unwrap()
            .values()
            .filter(|f| f.name.starts_with(DEVICE_PREFIX) || Some(f.address) == known)
            .cloned()
            .collect();
        if result.iter().any(|f| stop_when(f)) || start.elapsed() >= duration {
            break;
        }
    }
    let _ = watcher.Stop();
    let _ = watcher.RemoveReceived(token);
    result.sort_by(|a, b| b.rssi.cmp(&a.rssi));
    Ok(result)
}

#[derive(Debug, Default, Clone)]
pub struct Delivery {
    pub windows: usize,
    pub windows_acked: usize,
    pub complete: bool,
    pub bytes: usize,
    pub elapsed: Duration,
}

pub struct Connection {
    device: BluetoothLEDevice,
    session: Option<GattSession>,
    services: Vec<GattDeviceService>,
    write: GattCharacteristic,
    notify: GattCharacteristic,
    token: Option<EventRegistrationToken>,
    rx: Receiver<Vec<u8>>,
    pub chunk: usize,
    pub mtu: u16,
    pub name: String,
}

fn status_text(s: GattCommunicationStatus) -> &'static str {
    match s {
        GattCommunicationStatus::Unreachable => "panel erişilemez (menzil dışında veya başka cihaza bağlı)",
        GattCommunicationStatus::ProtocolError => "protokol hatası",
        GattCommunicationStatus::AccessDenied => "erişim reddedildi",
        _ => "bilinmeyen hata",
    }
}

impl Connection {
    pub fn open(address: u64, log: &dyn Fn(String)) -> Result<Connection, String> {
        let device = wait(BluetoothLEDevice::FromBluetoothAddressAsync(address), Duration::from_secs(10), "Panel açılamadı")
            .map_err(|e| format!("{e} ({})", format_address(address)))?;
        let name = device.Name().map(|n| n.to_string()).unwrap_or_default();
        let session = device
            .BluetoothDeviceId()
            .ok()
            .and_then(|id| wait(GattSession::FromDeviceIdAsync(&id), Duration::from_secs(5), "GATT oturumu").ok());
        if let Some(s) = &session {
            let _ = s.SetMaintainConnection(true);
        }
        let mut services_result = None;
        for (attempt, mode) in [BluetoothCacheMode::Uncached, BluetoothCacheMode::Uncached, BluetoothCacheMode::Cached].into_iter().enumerate() {
            match wait(device.GetGattServicesWithCacheModeAsync(mode), Duration::from_secs(10), "Panel servisleri") {
                Ok(r) if r.Status().ok() == Some(GattCommunicationStatus::Success) => {
                    services_result = Some(r);
                    break;
                }
                Ok(r) => log(format!("Servis keşfi {}. deneme: {}", attempt + 1, status_text(r.Status().unwrap_or(GattCommunicationStatus::Unreachable)))),
                Err(e) => log(format!("Servis keşfi {}. deneme: {e}", attempt + 1)),
            }
            sleep(Duration::from_millis(600));
        }
        let services_result = services_result.ok_or_else(|| {
            "Panele bağlanılamadı. Panel açık mı, telefon uygulaması bağlı değil mi? Paneli bilgisayara yaklaştırın.".to_string()
        })?;
        let services: Vec<GattDeviceService> = services_result.Services().map_err(|e| e.message().to_string())?.into_iter().collect();
        let write_uuid = GUID::from_u128(protocol::WRITE_UUID);
        let notify_uuid = GUID::from_u128(protocol::NOTIFY_UUID);
        let (mut write, mut notify) = (None, None);
        for svc in &services {
            let Ok(chars) = wait(svc.GetCharacteristicsWithCacheModeAsync(BluetoothCacheMode::Uncached), Duration::from_secs(8), "Özellikler") else {
                continue;
            };
            if let Ok(list) = chars.Characteristics() {
                for ch in list {
                    match ch.Uuid() {
                        Ok(u) if u == write_uuid => write = Some(ch),
                        Ok(u) if u == notify_uuid => notify = Some(ch),
                        _ => {}
                    }
                }
            }
        }
        let close_all = |services: &[GattDeviceService]| {
            for s in services {
                let _ = s.Close();
            }
        };
        let (Some(write), Some(notify)) = (write, notify) else {
            close_all(&services);
            let _ = device.Close();
            return Err("Bu cihaz iPIXEL panel servislerini (FA02/FA03) sunmuyor.".into());
        };
        let (tx, rx) = channel::<Vec<u8>>();
        let tx = Mutex::new(tx);
        let handler = TypedEventHandler::<GattCharacteristic, GattValueChangedEventArgs>::new(move |_, args| {
            if let Some(a) = args.as_ref() {
                let buf = a.CharacteristicValue()?;
                let len = buf.Length()? as usize;
                let mut bytes = vec![0u8; len];
                DataReader::FromBuffer(&buf)?.ReadBytes(&mut bytes)?;
                let _ = tx.lock().unwrap().send(bytes);
            }
            Ok(())
        });
        let token = notify.ValueChanged(&handler).ok();
        let cccd = wait(
            notify.WriteClientCharacteristicConfigurationDescriptorAsync(GattClientCharacteristicConfigurationDescriptorValue::Notify),
            Duration::from_secs(6),
            "Bildirim aboneliği",
        );
        match cccd {
            Ok(GattCommunicationStatus::Success) => {}
            Ok(s) => log(format!("Uyarı: bildirim aboneliği başarısız ({}); onaylar alınamayabilir.", status_text(s))),
            Err(e) => log(format!("Uyarı: {e}; onaylar alınamayabilir.")),
        }
        // Windows negotiates the MTU shortly after connecting.
        let mut mtu = 23u16;
        let t0 = Instant::now();
        while let Some(s) = &session {
            mtu = s.MaxPduSize().unwrap_or(23);
            if mtu > 23 || t0.elapsed() > Duration::from_millis(1500) {
                break;
            }
            sleep(Duration::from_millis(100));
        }
        let chunk = (mtu as usize).saturating_sub(3).clamp(20, 244);
        log(format!("Bağlandı: {} · MTU {mtu} · paket {chunk} bayt", if name.is_empty() { format_address(address) } else { name.clone() }));
        Ok(Connection { device, session, services, write, notify, token, rx, chunk, mtu, name })
    }

    fn drain(&self) {
        while let Ok(stale) = self.rx.try_recv() {
            let _ = stale;
        }
    }

    fn write_chunk(&self, data: &[u8]) -> Result<(), String> {
        let writer = DataWriter::new().map_err(|e| e.message().to_string())?;
        writer.WriteBytes(data).map_err(|e| e.message().to_string())?;
        let buffer = writer.DetachBuffer().map_err(|e| e.message().to_string())?;
        let mut last = String::new();
        for attempt in 0..3 {
            match wait(self.write.WriteValueWithResultAndOptionAsync(&buffer, GattWriteOption::WriteWithResponse), Duration::from_secs(6), "Yazma") {
                Ok(r) => match r.Status() {
                    Ok(GattCommunicationStatus::Success) => return Ok(()),
                    Ok(s) => last = format!("Yazma başarısız: {}", status_text(s)),
                    Err(e) => last = e.message().to_string(),
                },
                Err(e) => last = e,
            }
            sleep(Duration::from_millis(150 * (attempt + 1)));
        }
        Err(last)
    }

    fn next_notify(&self, deadline: Instant, log: &dyn Fn(String)) -> Option<Vec<u8>> {
        let now = Instant::now();
        if now >= deadline {
            return None;
        }
        match self.rx.recv_timeout(deadline - now) {
            Ok(b) => {
                log(format!("← {}", protocol::hex(&b)));
                Some(b)
            }
            Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => None,
        }
    }

    /// Send a plan and wait for the panel's acknowledgements.
    pub fn execute(&self, plan: &Plan, progress: &mut dyn FnMut(usize, usize), log: &dyn Fn(String)) -> Result<Delivery, String> {
        let start = Instant::now();
        let total = plan.total_bytes();
        let mut sent = 0usize;
        let mut d = Delivery { windows: plan.windows.len(), bytes: total, ..Default::default() };
        let mut odd_status: Option<u8> = None;
        for (i, window) in plan.windows.iter().enumerate() {
            self.drain();
            for chunk in window.chunks(self.chunk) {
                self.write_chunk(chunk).map_err(|e| format!("{} aktarımı {}. pencerede kesildi: {e}", plan.name, i + 1))?;
                sent += chunk.len();
                progress(sent, total);
            }
            let deadline = Instant::now() + Duration::from_secs(if plan.ack == AckPolicy::PerWindow { 4 } else { 10 });
            loop {
                let Some(frame) = self.next_notify(deadline, log) else {
                    let detail = odd_status.map(|c| format!(" (panel durum kodu {c} bildirdi)")).unwrap_or_default();
                    return Err(format!("Panel {}. pencereyi onaylamadı{detail}. Veri panele ulaşmamış olabilir.", i + 1));
                };
                if let Some(op) = plan.opcode {
                    if frame.len() >= 5 && frame[0] == 0x05 && frame[2..4] != op {
                        log(format!("Önceki komuta ait gecikmiş onay yok sayıldı: {}", protocol::hex(&frame)));
                        continue;
                    }
                }
                match parse_notify(&frame) {
                    Notify::WindowAck => {
                        d.windows_acked += 1;
                        break;
                    }
                    Notify::Complete => {
                        d.windows_acked += 1;
                        d.complete = true;
                        break;
                    }
                    Notify::Status(c) => odd_status = Some(c),
                    Notify::Other => {}
                }
            }
        }
        if plan.ack == AckPolicy::PerWindowAndFinal && !d.complete {
            let deadline = Instant::now() + Duration::from_secs(12);
            while !d.complete {
                match self.next_notify(deadline, log).map(|f| parse_notify(&f)) {
                    Some(Notify::Complete) => d.complete = true,
                    Some(Notify::Status(c)) => odd_status = Some(c),
                    Some(_) => {}
                    None => {
                        let detail = odd_status.map(|c| format!(" Son durum kodu: {c}.")).unwrap_or_default();
                        return Err(format!("Tüm veri gönderildi ama panel aktarımın tamamlandığını onaylamadı.{detail}"));
                    }
                }
            }
        }
        if plan.ack == AckPolicy::PerWindow {
            d.complete = d.windows_acked == d.windows;
        }
        d.elapsed = start.elapsed();
        Ok(d)
    }

    pub fn device_info(&self, log: &dyn Fn(String)) -> Result<DeviceInfo, String> {
        let now = chrono::Local::now();
        use chrono::Timelike;
        let plan = protocol::device_info_request(now.hour() as u8, now.minute() as u8, now.second() as u8);
        self.drain();
        self.write_chunk(&plan.windows[0])?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while let Some(frame) = self.next_notify(deadline, log) {
            if let Some(info) = protocol::parse_device_info(&frame) {
                return Ok(info);
            }
        }
        Err("Panel cihaz bilgisini göndermedi.".into())
    }

    pub fn write_raw(&self, bytes: &[u8], log: &dyn Fn(String)) -> Result<Vec<Vec<u8>>, String> {
        self.drain();
        for chunk in bytes.chunks(self.chunk) {
            self.write_chunk(chunk)?;
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut out = Vec::new();
        while let Some(f) = self.next_notify(deadline, log) {
            out.push(f);
        }
        Ok(out)
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if let Some(t) = self.token.take() {
            let _ = self.notify.RemoveValueChanged(t);
        }
        let _ = wait(
            self.notify.WriteClientCharacteristicConfigurationDescriptorAsync(GattClientCharacteristicConfigurationDescriptorValue::None),
            Duration::from_millis(1500),
            "abonelik kapat",
        );
        if let Some(s) = self.session.take() {
            let _ = s.SetMaintainConnection(false);
            let _ = s.Close();
        }
        for s in self.services.drain(..) {
            let _ = s.Close();
        }
        let _ = self.device.Close();
    }
}
