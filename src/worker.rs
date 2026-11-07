//! Background worker: owns all Bluetooth activity and runs jobs one at a time.
//!
//! 1.x silently dropped any button press while a 5-second background scan was
//! running, retried failed sends every 5 seconds, and reported success without
//! knowing whether the panel accepted the data. Here every job is queued, each
//! transfer waits for the panel's acknowledgement, and automatic retries back off.

use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use chrono::NaiveDate;

use crate::ble::{self, format_address, parse_address, Connection, Delivery, Found};
use crate::duty::today_duty_date;
use crate::font::BitmapFont;
use crate::protocol::{self, DEVICE_PREFIX};
use crate::render::{self, PanelContent, Source};
use crate::store::{SendMethod, Settings, SlotInfo, Store};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Ok,
    Warn,
    Error,
}

#[derive(Clone, Debug)]
pub enum DeviceCommand {
    PowerOn,
    PowerOff,
    SyncTime,
    Brightness(u8),
    Orientation(u8),
    ShowSlot(u8),
    DeleteSlot(u8),
    ClearAll,
    Info,
    Raw(Vec<u8>),
}

pub enum Job {
    Scan,
    Send { source: Source, label: String, automatic: bool },
    Command(DeviceCommand),
    Diagnose,
    /// Settings changed: re-evaluate automatic mode now.
    Wake,
    Shutdown,
}

/// Who runs automatic sending in this process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoMode {
    /// The window: automatic sending unless the background service is running.
    Window,
    /// The Windows service (no window).
    Service,
}

/// Serialises Bluetooth use between the service and the window.
const BLE_LOCK: &str = "NobetPanel.Bluetooth";

#[derive(Clone, Debug)]
pub enum PanelState {
    Disabled,
    /// The background service handles automatic sending.
    Service,
    Searching,
    Online { name: String, rssi: i16 },
    Offline { detail: String },
}

#[derive(Clone, Debug)]
pub enum Event {
    Log(Level, String),
    Busy(Option<String>),
    Progress(f32),
    Panel(PanelState),
    Devices(Vec<Found>),
    Done { ok: bool, title: String, detail: String },
    StoreChanged,
}

pub type Emit = Arc<dyn Fn(Event) + Send + Sync>;

pub struct Worker {
    tx: Sender<Job>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Worker {
    pub fn spawn(store: Arc<Mutex<Store>>, emit: Emit, mode: AutoMode) -> Worker {
        let (tx, rx) = channel::<Job>();
        let handle = thread::Builder::new()
            .name("ble-worker".into())
            .spawn(move || {
                ble::init_thread();
                let mut st = State { store, emit, online: None, misses: 0, failures: 0, backoff_until: None, no_record_logged: None, last_keepalive: None, keepalive_failed: false };
                let mut next_check = Instant::now() + Duration::from_secs(2);
                // Both processes watch the shared data file for the other's changes.
                let poll = Duration::from_secs(2);
                loop {
                    let timeout = next_check.saturating_duration_since(Instant::now()).min(poll);
                    match rx.recv_timeout(timeout) {
                        Ok(Job::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
                        Ok(Job::Wake) => next_check = Instant::now(),
                        Ok(job) => {
                            st.refresh_store();
                            st.run(job);
                            // Give the panel a moment before the next automatic scan.
                            next_check = next_check.max(Instant::now() + Duration::from_secs(3));
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            let changed = st.refresh_store();
                            // The service reacts to settings saved by the window right away.
                            if Instant::now() < next_check && !(changed && mode == AutoMode::Service) {
                                continue;
                            }
                            st.auto_tick(mode);
                            let interval = st.settings().scan_interval_s.clamp(5, 600) as u64;
                            next_check = Instant::now() + Duration::from_secs(interval);
                        }
                    }
                }
            })
            .expect("worker thread");
        Worker { tx, handle: Some(handle) }
    }

    pub fn submit(&self, job: Job) {
        let _ = self.tx.send(job);
    }

    pub fn shutdown(mut self) {
        let _ = self.tx.send(Job::Shutdown);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Run a single send on the current thread (for `--send-today`).
pub fn send_blocking(store: Arc<Mutex<Store>>, source: Source, label: &str) -> Result<String, String> {
    ble::init_thread();
    let outcome: Arc<Mutex<Option<(bool, String)>>> = Arc::default();
    let sink = outcome.clone();
    let emit: Emit = Arc::new(move |e| {
        if let Event::Done { ok, detail, .. } = e {
            *sink.lock().unwrap() = Some((ok, detail));
        }
    });
    let mut st = State { store, emit, online: None, misses: 0, failures: 0, backoff_until: None, no_record_logged: None, last_keepalive: None, keepalive_failed: false };
    let lock = crate::platform::NamedLock::acquire(BLE_LOCK, Duration::from_secs(90));
    if !lock.held() {
        return Err("Arka plan servisi panelle bağlantıda; 90 saniye beklendi.".into());
    }
    let r = st.job_send(source, label, false);
    let detail = outcome.lock().unwrap().take().map(|(_, d)| d).unwrap_or_default();
    r.map(|_| detail.clone()).map_err(|e| if detail.is_empty() { e } else { detail })
}

/// Run one device command on the current thread (for `--command`).
pub fn command_blocking(store: Arc<Mutex<Store>>, cmd: DeviceCommand) -> Result<String, String> {
    ble::init_thread();
    let outcome: Arc<Mutex<Option<(bool, String)>>> = Arc::default();
    let sink = outcome.clone();
    let emit: Emit = Arc::new(move |e| {
        if let Event::Done { ok, detail, .. } = e {
            *sink.lock().unwrap() = Some((ok, detail));
        }
    });
    let lock = crate::platform::NamedLock::acquire(BLE_LOCK, Duration::from_secs(90));
    if !lock.held() {
        return Err("Arka plan servisi panelle bağlantıda; 90 saniye beklendi.".into());
    }
    let mut st = State { store, emit, online: None, misses: 0, failures: 0, backoff_until: None, no_record_logged: None, last_keepalive: None, keepalive_failed: false };
    st.job_command(cmd);
    let result = outcome.lock().unwrap().take();
    match result {
        Some((true, d)) => Ok(d),
        Some((false, d)) => Err(d),
        None => Err("Sonuç yok".into()),
    }
}

struct State {
    store: Arc<Mutex<Store>>,
    emit: Emit,
    online: Option<bool>,
    misses: u32,
    failures: u32,
    backoff_until: Option<Instant>,
    no_record_logged: Option<NaiveDate>,
    last_keepalive: Option<Instant>,
    keepalive_failed: bool,
}

struct Target {
    address: u64,
    name: String,
}

impl State {
    fn settings(&self) -> Settings {
        self.store.lock().unwrap().settings.clone()
    }

    fn log(&self, level: Level, text: impl Into<String>) {
        let text = text.into();
        let tag = match level {
            Level::Info => "INFO",
            Level::Ok => "OK",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        };
        crate::logfile::write(tag, &text);
        (self.emit)(Event::Log(level, text));
    }

    fn trace(&self) -> impl Fn(String) + '_ {
        move |t: String| {
            crate::logfile::write("BLE", &t);
            if !t.starts_with('←') {
                (self.emit)(Event::Log(Level::Info, t));
            }
        }
    }

    fn update_store(&self, f: impl FnOnce(&mut Store)) {
        let mut s = self.store.lock().unwrap();
        f(&mut s);
        if let Err(e) = s.save() {
            drop(s);
            self.log(Level::Error, e);
        }
        (self.emit)(Event::StoreChanged);
    }

    fn matches(s: &Settings, f: &Found) -> bool {
        let known = parse_address(&s.device_address);
        if !s.device_name.is_empty() {
            f.name == s.device_name || Some(f.address) == known
        } else {
            known.map(|k| k == f.address).unwrap_or(f.name.starts_with(DEVICE_PREFIX))
        }
    }

    fn remember_device(&self, f: &Found) {
        let s = self.settings();
        let addr = format_address(f.address);
        if s.device_address != addr || (s.device_name.is_empty() && !f.name.is_empty()) {
            let name = f.name.clone();
            self.update_store(move |st| {
                st.settings.device_address = addr;
                if st.settings.device_name.is_empty() {
                    st.settings.device_name = name;
                }
            });
        }
    }

    fn find_target(&self, s: &Settings) -> Result<Target, String> {
        (self.emit)(Event::Busy(Some("Panel aranıyor…".into())));
        let known = parse_address(&s.device_address);
        let found = ble::scan(Duration::from_secs(8), known, &|f| Self::matches(s, f))?;
        let hit = found.iter().find(|f| Self::matches(s, f)).cloned().or_else(|| {
            // A different LED_BLE_ panel is only used when none was chosen before.
            (s.device_name.is_empty() && known.is_none()).then(|| found.first().cloned()).flatten()
        });
        match hit {
            Some(f) => {
                self.remember_device(&f);
                Ok(Target { address: f.address, name: if f.name.is_empty() { s.device_name.clone() } else { f.name } })
            }
            None if !found.is_empty() => Err(format!(
                "Seçili panel ({}) bulunamadı. Görülen paneller: {}",
                if s.device_name.is_empty() { s.device_address.clone() } else { s.device_name.clone() },
                found.iter().map(|f| f.name.clone()).collect::<Vec<_>>().join(", ")
            )),
            None => Err(format!(
                "{DEVICE_PREFIX} ile başlayan panel bulunamadı. Panel açık mı? Telefon uygulaması panele bağlıysa kapatın (panel aynı anda tek bağlantı kabul eder)."
            )),
        }
    }

    /// Connect, preferring the remembered address; re-scan once if that fails.
    fn connect(&self, s: &Settings) -> Result<(Connection, Target), String> {
        let trace = self.trace();
        if let Some(addr) = parse_address(&s.device_address) {
            (self.emit)(Event::Busy(Some("Panele bağlanılıyor…".into())));
            match Connection::open(addr, &trace) {
                Ok(c) => {
                    let name = if c.name.is_empty() { s.device_name.clone() } else { c.name.clone() };
                    return Ok((c, Target { address: addr, name }));
                }
                Err(e) => self.log(Level::Warn, format!("Kayıtlı adresle bağlantı kurulamadı: {e}. Panel yeniden aranıyor.")),
            }
        }
        let t = self.find_target(s)?;
        (self.emit)(Event::Busy(Some(format!("{} paneline bağlanılıyor…", t.name))));
        let c = Connection::open(t.address, &trace)?;
        Ok((c, t))
    }

    fn exec(&self, conn: &Connection, plan: &protocol::Plan, show_progress: bool) -> Result<Delivery, String> {
        let trace = self.trace();
        let emit = self.emit.clone();
        let mut last = 0.0f32;
        let mut progress = |sent: usize, total: usize| {
            if show_progress && total > 0 {
                let p = sent as f32 / total as f32;
                if p - last > 0.02 || sent == total {
                    last = p;
                    emit(Event::Progress(p));
                }
            }
        };
        conn.execute(plan, &mut progress, &trace)
    }

    /// Reload changes the other process saved; true if anything changed.
    fn refresh_store(&self) -> bool {
        let changed = self.store.lock().unwrap().refresh_from_disk();
        if changed {
            (self.emit)(Event::StoreChanged);
        }
        changed
    }

    fn run(&mut self, job: Job) {
        let needs_ble = !matches!(job, Job::Wake | Job::Shutdown);
        let lock = if needs_ble {
            let l = crate::platform::NamedLock::acquire(BLE_LOCK, Duration::from_millis(200));
            if l.held() {
                l
            } else {
                (self.emit)(Event::Busy(Some("Arka plan servisi panelle konuşuyor, bekleniyor…".into())));
                crate::platform::NamedLock::acquire(BLE_LOCK, Duration::from_secs(90))
            }
        } else {
            crate::platform::NamedLock::acquire(BLE_LOCK, Duration::ZERO)
        };
        if needs_ble && !lock.held() {
            (self.emit)(Event::Done {
                ok: false,
                title: "Panel meşgul".into(),
                detail: "Arka plan servisi 90 saniyedir panelle bağlantıda; biraz sonra tekrar deneyin.".into(),
            });
            (self.emit)(Event::Busy(None));
            return;
        }
        match job {
            Job::Scan => self.job_scan(),
            Job::Send { source, label, automatic } => {
                let _ = self.job_send(source, &label, automatic);
            }
            Job::Command(c) => self.job_command(c),
            Job::Diagnose => self.job_diagnose(),
            Job::Wake | Job::Shutdown => {}
        }
        (self.emit)(Event::Busy(None));
    }

    fn job_scan(&mut self) {
        (self.emit)(Event::Busy(Some("Bluetooth panelleri taranıyor (8 sn)…".into())));
        let s = self.settings();
        match ble::scan(Duration::from_secs(8), parse_address(&s.device_address), &|_| false) {
            Ok(list) if list.is_empty() => {
                (self.emit)(Event::Devices(list));
                (self.emit)(Event::Done {
                    ok: false,
                    title: "Panel bulunamadı".into(),
                    detail: "Panel açık ve yakında olmalı; telefon uygulaması panele bağlıysa kapatın.".into(),
                });
            }
            Ok(list) => {
                self.log(Level::Ok, format!("{} panel bulundu.", list.len()));
                if s.device_name.is_empty() && s.device_address.is_empty() {
                    self.remember_device(&list[0]);
                }
                (self.emit)(Event::Devices(list));
            }
            Err(e) => (self.emit)(Event::Done { ok: false, title: "Tarama başarısız".into(), detail: e }),
        }
    }

    fn build_content(&self, source: &Source, s: &Settings) -> Result<PanelContent, String> {
        let (font, warn) = BitmapFont::load(&s.font);
        if let Some(w) = warn {
            self.log(Level::Warn, w);
        }
        render::build(source, s, &font, today_duty_date())
    }

    fn job_send(&mut self, source: Source, label: &str, automatic: bool) -> Result<(), String> {
        (self.emit)(Event::Busy(Some(label.to_string())));
        let result = (|| -> Result<String, String> {
            let mut s = self.settings();
            let mut content = self.build_content(&source, &s)?;
            for w in &content.warnings {
                self.log(Level::Warn, w.clone());
            }
            let mut last_err = String::new();
            for attempt in 1..=3 {
                if attempt > 1 {
                    self.log(Level::Warn, format!("{attempt}. deneme…"));
                    thread::sleep(Duration::from_millis(1500 * attempt as u64));
                    s = self.settings();
                }
                match self.try_send(&mut content, &source, &mut s) {
                    Ok(summary) => return Ok(summary),
                    Err(e) => {
                        self.log(Level::Warn, format!("Gönderim denemesi {attempt}/3 başarısız: {e}"));
                        last_err = e;
                    }
                }
            }
            Err(last_err)
        })();
        match result {
            Ok(summary) => {
                self.failures = 0;
                self.backoff_until = None;
                self.last_keepalive = Some(Instant::now());
                self.log(Level::Ok, format!("{label} — {summary}"));
                (self.emit)(Event::Done { ok: true, title: "Panel içeriği onayladı".into(), detail: summary });
                Ok(())
            }
            Err(e) => {
                let msg = if automatic { format!("Otomatik gönderim başarısız: {e}") } else { e.clone() };
                self.log(Level::Error, msg.clone());
                (self.emit)(Event::Done { ok: false, title: "Gönderim tamamlanamadı".into(), detail: msg });
                Err(e)
            }
        }
    }

    fn try_send(&self, content: &mut PanelContent, source: &Source, s: &mut Settings) -> Result<String, String> {
        let (conn, target) = self.connect(s)?;
        (self.emit)(Event::Busy(Some(format!("{} paneline gönderiliyor…", target.name))));
        let trace = self.trace();
        match conn.device_info(&trace) {
            Ok(info) if (info.width, info.height) != (s.panel_width, s.panel_height) => {
                self.log(Level::Info, format!("Panel boyutu {}×{} olarak algılandı.", info.width, info.height));
                let (w, h) = (info.width, info.height);
                self.update_store(move |st| {
                    st.settings.panel_width = w;
                    st.settings.panel_height = h;
                });
                s.panel_width = w;
                s.panel_height = h;
                *content = self.build_content(source, s)?;
            }
            Ok(_) => {}
            Err(e) => self.log(Level::Warn, format!("{e} Kayıtlı panel boyutu kullanılıyor.")),
        }
        for plan in [protocol::brightness(s.brightness), protocol::orientation(s.orientation)] {
            if let Err(e) = self.exec(&conn, &plan, false) {
                self.log(Level::Warn, format!("{} ayarı onaylanmadı: {e}", plan.name));
            }
        }
        let slot = s.effective_slot();
        let mut steps = Vec::new();
        let upload = |target_slot: u8| -> Result<Delivery, String> {
            let plan = content.payload.plan(target_slot)?;
            match self.exec(&conn, &plan, true) {
                Err(e) if e.contains(ble::EARLY_COMPLETE) && target_slot > 0 => {
                    // Clearing the slot makes the panel accept uploads again.
                    self.log(Level::Warn, format!("{e}. Slot {target_slot} silinip yeniden yükleniyor."));
                    self.exec(&conn, &protocol::delete_slot(target_slot), false)?;
                    thread::sleep(Duration::from_millis(300));
                    self.exec(&conn, &plan, true)
                }
                r => r,
            }
        };
        let first = match s.send_method {
            SendMethod::Live => upload(0)?,
            SendMethod::SlotSelect | SendMethod::SlotAndLive => upload(slot)?,
        };
        steps.push(format!(
            "{} pencere onaylandı, {:.1} KB, {:.1} sn",
            first.windows_acked,
            first.bytes as f64 / 1024.0,
            first.elapsed.as_secs_f32()
        ));
        match s.send_method {
            SendMethod::SlotSelect => {
                self.exec(&conn, &protocol::show_slot(slot), false)
                    .map_err(|e| format!("İçerik slot {slot}'e kaydedildi ama slot seçilemedi: {e}"))?;
                steps.push(format!("slot {slot} kalıcı program olarak seçildi"));
            }
            SendMethod::SlotAndLive => {
                upload(0).map_err(|e| format!("Slot {slot} kaydedildi ama canlı ekrana gönderilemedi: {e}"))?;
                steps.push(format!("slot {slot} + canlı ekran"));
            }
            SendMethod::Live => steps.push("canlı ekran (kalıcı değil)".into()),
        }
        drop(conn);
        let now = chrono::Local::now().format("%d.%m.%Y %H:%M").to_string();
        let text = content.text.clone();
        let device = target.name.clone();
        let duty = today_duty_date();
        let record_date = match source {
            Source::Record(r) => Some(r.date),
            _ => None,
        };
        let uses_slot = s.send_method != SendMethod::Live;
        let signature = content.signature(s.brightness, s.orientation);
        self.update_store(move |st| {
            if uses_slot {
                st.slots.insert(slot.to_string(), SlotInfo { text, updated: now, device, confirmed: true });
            }
            if record_date == Some(duty) {
                st.settings.last_auto_date = duty.format("%Y-%m-%d").to_string();
                st.settings.last_auto_signature = signature;
            }
        });
        Ok(format!("{} · {}", content.kind, steps.join(" · ")))
    }

    fn job_command(&mut self, cmd: DeviceCommand) {
        let label = match &cmd {
            DeviceCommand::PowerOn => "Panel açılıyor".to_string(),
            DeviceCommand::PowerOff => "Panel kapatılıyor".into(),
            DeviceCommand::SyncTime => "Saat eşitleniyor".into(),
            DeviceCommand::Brightness(b) => format!("Parlaklık %{b}"),
            DeviceCommand::Orientation(o) => format!("Ekran yönü {o}"),
            DeviceCommand::ShowSlot(n) => format!("Slot {n} seçiliyor"),
            DeviceCommand::DeleteSlot(n) => format!("Slot {n} siliniyor"),
            DeviceCommand::ClearAll => "Panel belleği temizleniyor".into(),
            DeviceCommand::Info => "Panel bilgisi okunuyor".into(),
            DeviceCommand::Raw(_) => "Ham komut gönderiliyor".into(),
        };
        (self.emit)(Event::Busy(Some(format!("{label}…"))));
        let s = self.settings();
        let result = (|| -> Result<String, String> {
            let (conn, target) = self.connect(&s)?;
            let trace = self.trace();
            let now = chrono::Local::now();
            use chrono::Timelike;
            let plan = match &cmd {
                DeviceCommand::PowerOn => protocol::power(true),
                DeviceCommand::PowerOff => protocol::power(false),
                DeviceCommand::SyncTime => protocol::set_time(now.hour() as u8, now.minute() as u8, now.second() as u8),
                DeviceCommand::Brightness(b) => protocol::brightness(*b),
                DeviceCommand::Orientation(o) => protocol::orientation(*o),
                DeviceCommand::ShowSlot(n) => protocol::show_slot(*n),
                DeviceCommand::DeleteSlot(n) => protocol::delete_slot(*n),
                DeviceCommand::ClearAll => protocol::clear_all(),
                DeviceCommand::Info => {
                    let info = conn.device_info(&trace)?;
                    return Ok(format!(
                        "{} · {}×{} piksel · tip {} · MTU {} · paket {} bayt",
                        target.name, info.width, info.height, info.device_type, conn.mtu, conn.chunk
                    ));
                }
                DeviceCommand::Raw(bytes) => {
                    let replies = conn.write_raw(bytes, &trace)?;
                    return Ok(if replies.is_empty() {
                        "Gönderildi; panel yanıt vermedi.".into()
                    } else {
                        format!("Yanıt: {}", replies.iter().map(|r| protocol::hex(r)).collect::<Vec<_>>().join(" | "))
                    });
                }
            };
            self.exec(&conn, &plan, false)?;
            Ok(format!("{label} — panel onayladı ({})", target.name))
        })();
        match result {
            Ok(msg) => {
                match cmd {
                    DeviceCommand::DeleteSlot(n) => self.update_store(move |st| {
                        st.slots.remove(&n.to_string());
                    }),
                    DeviceCommand::ClearAll => self.update_store(|st| st.slots.clear()),
                    _ => {}
                }
                self.log(Level::Ok, msg.clone());
                (self.emit)(Event::Done { ok: true, title: "Tamamlandı".into(), detail: msg });
            }
            Err(e) => {
                self.log(Level::Error, format!("{label}: {e}"));
                (self.emit)(Event::Done { ok: false, title: format!("{label} başarısız"), detail: e });
            }
        }
    }

    fn job_diagnose(&mut self) {
        let mut report = Vec::new();
        (self.emit)(Event::Busy(Some("Bağlantı testi: tarama…".into())));
        let s = self.settings();
        match ble::scan(Duration::from_secs(6), parse_address(&s.device_address), &|_| false) {
            Ok(list) => {
                report.push(format!("Tarama: {} panel ({})", list.len(), list.iter().map(|f| format!("{} {} dBm", f.name, f.rssi)).collect::<Vec<_>>().join(", ")));
                if let Some(f) = list.iter().find(|f| Self::matches(&s, f)) {
                    if f.rssi < -85 {
                        report.push(format!("Sinyal zayıf ({} dBm): paneli bilgisayara yaklaştırın veya USB Bluetooth adaptörü kullanın.", f.rssi));
                    }
                }
            }
            Err(e) => report.push(format!("Tarama hatası: {e}")),
        }
        (self.emit)(Event::Busy(Some("Bağlantı testi: bağlanılıyor…".into())));
        let t0 = Instant::now();
        match self.connect(&s) {
            Ok((conn, target)) => {
                report.push(format!("Bağlantı: {} · {:.1} sn · MTU {} · paket {} bayt", target.name, t0.elapsed().as_secs_f32(), conn.mtu, conn.chunk));
                let trace = self.trace();
                match conn.device_info(&trace) {
                    Ok(i) => report.push(format!("Cihaz: {}×{} · tip {}", i.width, i.height, i.device_type)),
                    Err(e) => report.push(format!("Cihaz bilgisi: {e}")),
                }
                let t1 = Instant::now();
                match self.exec(&conn, &protocol::brightness(s.brightness), false) {
                    Ok(_) => report.push(format!("Komut onayı: {} ms", t1.elapsed().as_millis())),
                    Err(e) => report.push(format!("Komut onayı alınamadı: {e}")),
                }
            }
            Err(e) => report.push(format!("Bağlantı kurulamadı: {e}")),
        }
        let text = report.join("\n");
        self.log(Level::Info, format!("Bağlantı testi:\n{text}"));
        (self.emit)(Event::Done { ok: true, title: "Bağlantı testi".into(), detail: text });
    }

    /// Re-select the saved program slot. The panel boots to its default
    /// screen after a power blip or firmware reset that can be too short for
    /// the scans to notice; this brings the duty list back within minutes.
    fn keepalive(&mut self, s: &Settings) {
        if s.keepalive_min == 0 || s.send_method != SendMethod::SlotSelect {
            return;
        }
        let due = self.last_keepalive.map(|t| t.elapsed() >= Duration::from_secs(s.keepalive_min as u64 * 60)).unwrap_or(true);
        if !due {
            return;
        }
        self.last_keepalive = Some(Instant::now());
        let slot = s.effective_slot();
        let result = self.connect(s).and_then(|(conn, _)| self.exec(&conn, &protocol::show_slot(slot), false));
        match result {
            Ok(_) => {
                if self.keepalive_failed {
                    self.log(Level::Ok, format!("Panel programı (slot {slot}) yeniden seçildi."));
                }
                self.keepalive_failed = false;
            }
            Err(e) => {
                if !self.keepalive_failed {
                    self.log(Level::Warn, format!("Panel programı (slot {slot}) yeniden seçilemedi: {e}"));
                }
                self.keepalive_failed = true;
            }
        }
        (self.emit)(Event::Busy(None));
    }

    fn auto_tick(&mut self, mode: AutoMode) {
        let s = self.settings();
        if !s.auto_send {
            (self.emit)(Event::Panel(PanelState::Disabled));
            return;
        }
        if mode == AutoMode::Window && crate::platform::service_running() {
            (self.emit)(Event::Panel(PanelState::Service));
            return;
        }
        let lock = crate::platform::NamedLock::acquire(BLE_LOCK, Duration::from_secs(30));
        if !lock.held() {
            return;
        }
        if let Some(until) = self.backoff_until {
            if Instant::now() < until {
                return;
            }
        }
        (self.emit)(Event::Panel(PanelState::Searching));
        let found = match ble::scan(Duration::from_secs(5), parse_address(&s.device_address), &|f| Self::matches(&s, f)) {
            Ok(f) => f,
            Err(e) => {
                (self.emit)(Event::Panel(PanelState::Offline { detail: e }));
                return;
            }
        };
        let hit = found.into_iter().find(|f| Self::matches(&s, f));
        let Some(dev) = hit else {
            self.misses += 1;
            if self.misses >= 2 && self.online != Some(false) {
                if self.online == Some(true) {
                    self.log(Level::Warn, "Panel art arda iki taramada görülmedi; yeniden görünmesi bekleniyor.");
                }
                self.online = Some(false);
            }
            (self.emit)(Event::Panel(PanelState::Offline {
                detail: "Panel görünmüyor (kapalı, menzil dışında veya telefona bağlı).".into(),
            }));
            return;
        };
        self.misses = 0;
        let reappeared = self.online == Some(false);
        self.online = Some(true);
        self.remember_device(&dev);
        (self.emit)(Event::Panel(PanelState::Online { name: dev.name.clone(), rssi: dev.rssi }));

        let duty = today_duty_date();
        let duty_iso = duty.format("%Y-%m-%d").to_string();
        let rec = self.store.lock().unwrap().record_for(duty).cloned();
        let Some(rec) = rec else {
            if self.no_record_logged != Some(duty) {
                self.no_record_logged = Some(duty);
                self.log(Level::Warn, format!("{} için nöbet kaydı yok; otomatik gönderim yapılmadı.", crate::duty::date_label(duty)));
            }
            return;
        };
        let changed = || -> bool {
            // Older data files have no fingerprint yet: treat as up to date.
            if s.last_auto_signature.is_empty() {
                return false;
            }
            match self.build_content(&Source::Record(rec.clone()), &s) {
                Ok(c) => c.signature(s.brightness, s.orientation) != s.last_auto_signature,
                Err(_) => false,
            }
        };
        let reason = if s.last_auto_date != duty_iso {
            Some("yeni nöbet günü")
        } else if changed() {
            Some("kayıt veya görünüm ayarları değişti")
        } else if reappeared && s.resend_on_reappear {
            Some("panel yeniden göründü")
        } else {
            None
        };
        if reason.is_none() {
            self.keepalive(&s);
        }
        if let Some(reason) = reason {
            self.log(Level::Info, format!("Otomatik gönderim ({reason})."));
            if self.job_send(Source::Record(rec), "Bugünün nöbeti otomatik gönderiliyor…", true).is_err() {
                self.failures += 1;
                let wait = (30u64 << (self.failures - 1).min(5)).min(600);
                self.backoff_until = Some(Instant::now() + Duration::from_secs(wait));
                self.log(Level::Warn, format!("Sonraki otomatik deneme {wait} sn sonra."));
                if reappeared {
                    // Keep treating it as a fresh appearance until a send succeeds.
                    self.online = Some(false);
                }
            }
            (self.emit)(Event::Busy(None));
        }
    }
}
