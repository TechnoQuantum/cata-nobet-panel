//! Persistent settings, roster and slot manifest.
//!
//! The JSON layout is compatible with the Python 1.x releases
//! (`%LOCALAPPDATA%\NobetPanel\nobet_listesi.json`), so existing users keep
//! their roster and settings after upgrading.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::duty::{parse_date, Record};

static SERVICE_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The Windows service always uses the shared machine-wide folder.
pub fn set_service_mode() {
    SERVICE_MODE.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Machine-wide data folder shared by the service and the window
/// (`C:\ProgramData\NobetPanel`), used once the installer has created it.
pub fn shared_dir() -> PathBuf {
    std::env::var_os("ProgramData").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\ProgramData")).join("NobetPanel")
}

pub fn data_dir() -> PathBuf {
    // NOBETPANEL_DATA_DIR lets testers run against a separate roster/settings file.
    if let Some(dir) = std::env::var_os("NOBETPANEL_DATA_DIR") {
        let dir = PathBuf::from(dir);
        let _ = fs::create_dir_all(&dir);
        return dir;
    }
    let shared = shared_dir();
    if SERVICE_MODE.load(std::sync::atomic::Ordering::SeqCst) || shared.join("nobet_listesi.json").exists() {
        let _ = fs::create_dir_all(&shared);
        return shared;
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("NobetPanel");
    let _ = fs::create_dir_all(&dir);
    dir
}

pub fn data_path() -> PathBuf {
    data_dir().join("nobet_listesi.json")
}

pub const ANIMATIONS: [(&str, u8); 6] = [
    ("Sabit", 0),
    ("Sola kaydır", 1),
    ("Sağa kaydır", 2),
    ("Yanıp sön", 5),
    ("Nefes efekti", 6),
    ("Kar efekti", 7),
];

pub fn animation_code(label: &str) -> u8 {
    ANIMATIONS.iter().find(|(l, _)| *l == label).map(|(_, c)| *c).unwrap_or(1)
}

/// How content is placed on the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendMethod {
    /// Upload into the chosen slot, then make that slot the panel's program
    /// (persists across power cycles). Recommended.
    SlotSelect,
    /// Upload to the live buffer (slot 0) only. Not kept after a power cycle.
    Live,
    /// Legacy 1.x behaviour: upload into the slot and again to the live buffer.
    SlotAndLive,
}

pub const SEND_METHODS: [(&str, SendMethod); 3] = [
    ("Slota kaydet ve seç (önerilen)", SendMethod::SlotSelect),
    ("Yalnızca canlı ekran (slot 0)", SendMethod::Live),
    ("Slota kaydet + canlı ekran (eski yöntem)", SendMethod::SlotAndLive),
];

/// How the E logo is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogoMode {
    /// Fixed blinking logo beside a ticker, uploaded as one compact GIF.
    FixedGif,
    /// Logo embedded at the start of firmware-scrolled text. Smallest transfer.
    InlineText,
}

pub const LOGO_MODES: [(&str, LogoMode); 2] = [
    ("Sabit logo + kayan yazı (GIF)", LogoMode::FixedGif),
    ("Yazıyla kayan logo (en küçük aktarım)", LogoMode::InlineText),
];

fn d_brightness() -> u8 { 70 }
fn d_animation() -> String { "Sola kaydır".into() }
fn d_speed() -> u8 { 70 }
fn d_text_color() -> String { "#ffffff".into() }
fn d_bg_color() -> String { "#071a35".into() }
fn d_font() -> String { "UNIFONT".into() }
fn d_true() -> bool { true }
fn d_slot() -> u8 { 1 }
fn d_method() -> SendMethod { SendMethod::SlotSelect }
fn d_logo_mode() -> LogoMode { LogoMode::FixedGif }
fn d_gif_budget() -> u32 { 40 }
fn d_width() -> u32 { 96 }
fn d_height() -> u32 { 16 }
fn d_scan_interval() -> u32 { 20 }
fn d_keepalive() -> u32 { 5 }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub device_name: String,
    #[serde(default)]
    pub device_address: String,
    #[serde(default = "d_brightness")]
    pub brightness: u8,
    #[serde(default = "d_animation")]
    pub animation: String,
    #[serde(default = "d_speed")]
    pub speed: u8,
    #[serde(default = "d_text_color")]
    pub text_color: String,
    #[serde(default = "d_bg_color")]
    pub background_color: String,
    #[serde(default = "d_font")]
    pub font: String,
    #[serde(default = "d_true")]
    pub auto_send: bool,
    #[serde(default)]
    pub last_auto_date: String,
    /// Fingerprint of what was last confirmed on the panel for `last_auto_date`;
    /// a different fingerprint (roster or display settings changed) triggers a re-send.
    #[serde(default)]
    pub last_auto_signature: String,
    #[serde(default)]
    pub own_pharmacy_name: String,
    #[serde(default)]
    pub own_duty_message_enabled: bool,
    #[serde(default)]
    pub roster_logo_enabled: bool,
    #[serde(default = "d_true")]
    pub save_to_slot: bool,
    #[serde(default = "d_slot")]
    pub save_slot: u8,
    #[serde(default)]
    pub orientation: u8,
    #[serde(default)]
    pub mirror_horizontal: bool,
    #[serde(default)]
    pub mirror_vertical: bool,
    #[serde(default)]
    pub free_mode: bool,
    #[serde(default)]
    pub free_text: String,
    #[serde(default)]
    pub startup_enabled: bool,
    // ---- New in 2.0 ----
    #[serde(default = "d_method")]
    pub send_method: SendMethod,
    #[serde(default = "d_logo_mode")]
    pub logo_mode: LogoMode,
    /// Upper bound for logo GIF size, in KiB. Larger animations are made coarser.
    #[serde(default = "d_gif_budget")]
    pub gif_budget_kb: u32,
    #[serde(default = "d_width")]
    pub panel_width: u32,
    #[serde(default = "d_height")]
    pub panel_height: u32,
    /// Re-send after the panel reappears (e.g. after a power cut).
    #[serde(default = "d_true")]
    pub resend_on_reappear: bool,
    #[serde(default = "d_scan_interval")]
    pub scan_interval_s: u32,
    /// Every N minutes re-select the saved program slot, so the panel leaves
    /// its default screen soon after an unnoticed reboot. 0 = off.
    #[serde(default = "d_keepalive")]
    pub keepalive_min: u32,
    /// Ask before every manual send, showing exactly what goes to the panel.
    #[serde(default = "d_true")]
    pub confirm_before_send: bool,
}

impl Default for Settings {
    fn default() -> Self {
        serde_json::from_str("{}").expect("defaults")
    }
}

impl Settings {
    pub fn effective_slot(&self) -> u8 {
        self.save_slot.clamp(1, 10)
    }
    pub fn animation_code(&self) -> u8 {
        animation_code(&self.animation)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SlotInfo {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub updated: String,
    #[serde(default)]
    pub device: String,
    /// True when the panel acknowledged the full transfer (2.0+).
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Serialize, Deserialize)]
struct RawRecord {
    #[serde(default)]
    date: String,
    #[serde(default)]
    pharmacy: String,
}

#[derive(Serialize, Deserialize, Default)]
struct RawFile {
    #[serde(default)]
    records: Vec<RawRecord>,
    #[serde(default)]
    settings: Option<serde_json::Value>,
    #[serde(default)]
    slots: BTreeMap<String, SlotInfo>,
}

#[derive(Clone, Debug, Default)]
pub struct Store {
    pub records: BTreeMap<NaiveDate, Record>,
    pub settings: Settings,
    pub slots: BTreeMap<String, SlotInfo>,
    /// The file content this copy was last synchronised with. The service and
    /// the window share one file; saving merges field by field against it so
    /// neither process overwrites what the other changed.
    base: serde_json::Value,
    disk_stamp: Option<std::time::SystemTime>,
}

/// Three-way merge: take our value where we changed it, otherwise theirs.
/// Settings are merged per key, records and slots as a whole.
fn merge3(base: &serde_json::Value, mine: &serde_json::Value, disk: &serde_json::Value, depth: u8) -> serde_json::Value {
    use serde_json::Value;
    match (mine, disk) {
        (Value::Object(m), Value::Object(d)) if depth < 2 => {
            let empty = serde_json::Map::new();
            let b = base.as_object().unwrap_or(&empty);
            let mut out = serde_json::Map::new();
            let keys: std::collections::BTreeSet<&String> = m.keys().chain(d.keys()).collect();
            for k in keys {
                let nested = depth == 0 && k == "settings";
                let (bv, mv, dv) = (b.get(k).unwrap_or(&Value::Null), m.get(k).unwrap_or(&Value::Null), d.get(k).unwrap_or(&Value::Null));
                let v = if nested {
                    merge3(bv, mv, dv, depth + 1)
                } else if mv != bv {
                    mv.clone()
                } else {
                    dv.clone()
                };
                if !v.is_null() || m.contains_key(k) {
                    out.insert(k.clone(), v);
                }
            }
            Value::Object(out)
        }
        _ => {
            if mine != base {
                mine.clone()
            } else {
                disk.clone()
            }
        }
    }
}

fn file_stamp(path: &std::path::Path) -> Option<std::time::SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl Store {
    /// Load the data file. On a parse error the broken file is kept as a
    /// `.corrupt-*.json` copy (1.x silently overwrote it on the next save).
    pub fn load() -> (Store, Option<String>) {
        let path = data_path();
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Store::default(), None),
            Err(e) => return (Store::default(), Some(format!("Kayıt dosyası okunamadı: {e}"))),
        };
        match Self::parse(&text) {
            Ok((mut store, skipped)) => {
                let warn = (skipped > 0).then(|| format!("{skipped} bozuk kayıt satırı atlandı."));
                store.base = store.to_value();
                store.disk_stamp = file_stamp(&path);
                (store, warn)
            }
            Err(e) => {
                let backup = path.with_file_name(format!(
                    "nobet_listesi.corrupt-{}.json",
                    chrono::Local::now().format("%Y%m%d-%H%M%S")
                ));
                let _ = fs::copy(&path, &backup);
                (
                    Store::default(),
                    Some(format!("Kayıt dosyası bozuk; boş listeyle açıldı. Yedek: {}\n{e}", backup.display())),
                )
            }
        }
    }

    fn parse(text: &str) -> Result<(Store, usize), String> {
        let raw: RawFile = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
        let mut store = Store::default();
        let mut skipped = 0;
        for r in raw.records {
            match parse_date(&r.date).and_then(|d| Record::new(d, &r.pharmacy)) {
                Ok(rec) => {
                    store.records.insert(rec.date, rec);
                }
                Err(_) => skipped += 1,
            }
        }
        if let Some(v) = raw.settings {
            store.settings = serde_json::from_value(v).unwrap_or_default();
        }
        store.slots = raw.slots;
        Ok((store, skipped))
    }

    fn to_value(&self) -> serde_json::Value {
        let raw = RawFile {
            records: self
                .records
                .values()
                .map(|r| RawRecord { date: r.date.format("%Y-%m-%d").to_string(), pharmacy: r.pharmacy.clone() })
                .collect(),
            settings: serde_json::to_value(&self.settings).ok(),
            slots: self.slots.clone(),
        };
        serde_json::to_value(&raw).unwrap_or_default()
    }

    /// The current file content, normalised; None if missing or unreadable.
    fn disk_value() -> Option<serde_json::Value> {
        let text = fs::read_to_string(data_path()).ok()?;
        Self::parse(&text).ok().map(|(s, _)| s.to_value())
    }

    fn adopt(&mut self, v: &serde_json::Value) {
        if let Ok((s, _)) = serde_json::to_string(v).map_err(|e| e.to_string()).and_then(|t| Self::parse(&t)) {
            self.records = s.records;
            self.settings = s.settings;
            self.slots = s.slots;
        }
    }

    /// Pick up changes another process (service or window) saved.
    /// Returns true if anything in memory changed.
    pub fn refresh_from_disk(&mut self) -> bool {
        let path = data_path();
        let stamp = file_stamp(&path);
        if stamp.is_none() || stamp == self.disk_stamp {
            return false;
        }
        let Some(disk) = Self::disk_value() else { return false };
        self.disk_stamp = stamp;
        let mine = self.to_value();
        let merged = merge3(&self.base, &mine, &disk, 0);
        self.base = disk;
        if merged != mine {
            self.adopt(&merged);
            return true;
        }
        false
    }

    pub fn save(&mut self) -> Result<(), String> {
        let _lock = crate::platform::NamedLock::acquire("NobetPanel.Store", std::time::Duration::from_secs(5));
        let mine = self.to_value();
        let merged = match Self::disk_value() {
            Some(disk) => merge3(&self.base, &mine, &disk, 0),
            None => mine.clone(),
        };
        let json = serde_json::to_string_pretty(&merged).map_err(|e| e.to_string())?;
        let path = data_path();
        let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&tmp, json).map_err(|e| format!("Kayıt yazılamadı: {e}"))?;
        fs::rename(&tmp, &path).map_err(|e| format!("Kayıt yazılamadı: {e}"))?;
        if merged != mine {
            self.adopt(&merged);
        }
        self.base = merged;
        self.disk_stamp = file_stamp(&path);
        Ok(())
    }

    pub fn record_for(&self, d: NaiveDate) -> Option<&Record> {
        self.records.get(&d)
    }

    /// Merge imported records. Returns (added, replaced-with-different-content).
    pub fn merge(&mut self, imported: Vec<Record>) -> (usize, usize) {
        let (mut added, mut replaced) = (0, 0);
        for r in imported {
            match self.records.insert(r.date, r.clone()) {
                None => added += 1,
                Some(old) if old.pharmacy != r.pharmacy => replaced += 1,
                Some(_) => {}
            }
        }
        (added, replaced)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_keeps_both_processes_changes() {
        use serde_json::json;
        let base = json!({"records": [1], "settings": {"brightness": 70, "last_auto_date": "a"}, "slots": {}});
        // The window changed brightness, the service recorded a send meanwhile.
        let mine = json!({"records": [1], "settings": {"brightness": 100, "last_auto_date": "a"}, "slots": {}});
        let disk = json!({"records": [1], "settings": {"brightness": 70, "last_auto_date": "b"}, "slots": {"1": "x"}});
        let m = merge3(&base, &mine, &disk, 0);
        assert_eq!(m, json!({"records": [1], "settings": {"brightness": 100, "last_auto_date": "b"}, "slots": {"1": "x"}}));
    }

    #[test]
    fn reads_python_1x_file() {
        let json = r##"{"records":[{"date":"2026-10-01","pharmacy":"A / B"},{"date":"bad","pharmacy":"x"}],
            "settings":{"device_name":"LED_BLE_1","animation":"Sabit","speed":40,"save_slot":3,"unknown_key":1},
            "slots":{"3":{"text":"hi","updated":"01.10.2026 10:00","device":"LED_BLE_1"}}}"##;
        let (s, skipped) = Store::parse(json).unwrap();
        assert_eq!(skipped, 1);
        assert_eq!(s.records.len(), 1);
        assert_eq!(s.settings.animation_code(), 0);
        assert_eq!(s.settings.speed, 40);
        assert_eq!(s.settings.effective_slot(), 3);
        assert_eq!(s.settings.send_method, SendMethod::SlotSelect);
        assert_eq!(s.slots["3"].text, "hi");
    }

    #[test]
    fn defaults() {
        let s = Settings::default();
        assert_eq!(s.brightness, 70);
        assert!(s.auto_send);
        assert_eq!(s.panel_width, 96);
    }
}
