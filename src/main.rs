#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ble;
mod duty;
mod font;
mod import;
mod logfile;
mod platform;
mod protocol;
mod render;
#[cfg(windows)]
mod service;
mod store;
mod worker;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{Datelike, NaiveDate};
use slint::{CloseRequestResponse, ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, Timer, TimerMode, VecModel};

use crate::duty::{date_label, now_local, parse_date, shift_is_running, shift_start, shift_window_label, today_duty_date, Record, MONTHS_TITLE};
use crate::font::BitmapFont;
use crate::render::{color_hex, parse_hex_color, Frame, PreviewModel, Source};
use crate::store::{LogoMode, SendMethod, Store, ANIMATIONS, LOGO_MODES, SEND_METHODS};
use crate::worker::{AutoMode, DeviceCommand, Event, Job, Level, PanelState, Worker};

slint::include_modules!();

const VERSION: &str = env!("CARGO_PKG_VERSION");
const SWATCHES: [&str; 12] = [
    "#ffffff", "#ffd400", "#ff8a00", "#ff2d2d", "#00e676", "#00b0ff", "#2962ff", "#d500f9", "#071a35", "#000000", "#0b3d2e", "#4a0b0b",
];

enum Pending {
    Send { source: Source, label: String },
    SaveRecord(Record),
    DeleteRecord(NaiveDate),
    Import(Vec<Record>),
    ShowSlot(u8),
    DeleteSlot(u8),
    ClearAll,
    PowerOff,
    Raw(Vec<u8>),
    Close,
}

struct Ctx {
    ui: slint::Weak<AppWindow>,
    store: Arc<Mutex<Store>>,
    worker: RefCell<Option<Worker>>,
    preview: RefCell<Option<PreviewModel>>,
    preview_t0: Cell<Instant>,
    pending: RefCell<Option<Pending>>,
    selected: Cell<NaiveDate>,
    view_month: Cell<NaiveDate>,
    duty_day: Cell<NaiveDate>,
    devices: RefCell<Vec<ble::Found>>,
    logs: Rc<VecModel<LogRow>>,
    panel_state: RefCell<String>,
    allow_close: Cell<bool>,
    save_timer: Timer,
    banner_timer: Timer,
    resend_timer: Timer,
}

thread_local! {
    static CTX: RefCell<Option<Rc<Ctx>>> = const { RefCell::new(None) };
}

fn ctx() -> Option<Rc<Ctx>> {
    CTX.with(|c| c.borrow().clone())
}

fn ss(s: impl AsRef<str>) -> SharedString {
    SharedString::from(s.as_ref())
}

fn month_start(d: NaiveDate) -> NaiveDate {
    d.with_day(1).unwrap()
}

fn add_months(d: NaiveDate, n: i32) -> NaiveDate {
    let idx = d.year() * 12 + d.month0() as i32 + n;
    NaiveDate::from_ymd_opt(idx.div_euclid(12), idx.rem_euclid(12) as u32 + 1, 1).unwrap()
}

fn to_slint_color(hex: &str) -> slint::Color {
    let c = parse_hex_color(hex).unwrap_or([0, 0, 0]);
    slint::Color::from_rgb_u8(c[0], c[1], c[2])
}

/// Upscale a panel frame into an LED-matrix-looking image.
fn led_image(f: &Frame) -> Image {
    const S: usize = 7;
    let (w, h) = (f.width * S, f.height * S);
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(w as u32, h as u32);
    let px = buf.make_mut_slice();
    for p in px.iter_mut() {
        *p = Rgba8Pixel { r: 5, g: 8, b: 13, a: 255 };
    }
    for y in 0..f.height {
        for x in 0..f.width {
            let c = f.pixels[y * f.width + x];
            let off = c[0] < 6 && c[1] < 6 && c[2] < 6;
            let (r, g, b) = if off { (24, 28, 36) } else { (c[0], c[1], c[2]) };
            for dy in 0..S - 1 {
                for dx in 0..S - 1 {
                    let corner = (dx == 0 || dx == S - 2) && (dy == 0 || dy == S - 2);
                    if !corner {
                        px[(y * S + dy) * w + x * S + dx] = Rgba8Pixel { r, g, b, a: 255 };
                    }
                }
            }
        }
    }
    Image::from_rgba8(buf)
}

fn decode_text(bytes: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.trim_start_matches('\u{feff}').to_string();
    }
    // Excel on Turkish Windows saves CSV as Windows-1254.
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', '\u{8e}', '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜',
        '™', 'š', '›', 'œ', '\u{9d}', '\u{9e}', 'Ÿ',
    ];
    bytes
        .iter()
        .map(|&b| match b {
            0x00..=0x7f => b as char,
            0x80..=0x9f => HIGH[(b - 0x80) as usize],
            0xd0 => 'Ğ',
            0xdd => 'İ',
            0xde => 'Ş',
            0xf0 => 'ğ',
            0xfd => 'ı',
            0xfe => 'ş',
            _ => char::from_u32(b as u32).unwrap_or('?'),
        })
        .collect()
}

impl Ctx {
    fn ui(&self) -> AppWindow {
        self.ui.upgrade().expect("window")
    }

    fn submit(&self, job: Job) {
        if let Some(w) = self.worker.borrow().as_ref() {
            w.submit(job);
        }
    }

    fn save_soon(&self) {
        let store = self.store.clone();
        self.save_timer.start(TimerMode::SingleShot, Duration::from_millis(400), move || {
            if let Err(e) = store.lock().unwrap().save() {
                if let Some(c) = ctx() {
                    c.banner(false, "Ayarlar kaydedilemedi", &e);
                }
            }
        });
    }

    fn save_now(&self) {
        if let Err(e) = self.store.lock().unwrap().save() {
            self.banner(false, "Kayıt yazılamadı", &e);
        }
    }

    fn banner(&self, ok: bool, title: &str, text: &str) {
        let ui = self.ui();
        ui.set_banner_ok(ok);
        ui.set_banner_title(ss(title));
        ui.set_banner_text(ss(text));
        ui.set_banner_visible(true);
        if ok {
            let weak = self.ui.clone();
            self.banner_timer.start(TimerMode::SingleShot, Duration::from_secs(15), move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_banner_visible(false);
                }
            });
        } else {
            self.banner_timer.stop();
        }
    }

    fn log(&self, level: Level, text: &str) {
        let lvl = match level {
            Level::Info => 0,
            Level::Ok => 1,
            Level::Warn => 2,
            Level::Error => 3,
        };
        self.logs.insert(0, LogRow { time: ss(chrono::Local::now().format("%H:%M:%S").to_string()), text: ss(text), level: lvl });
        while self.logs.row_count() > 500 {
            self.logs.remove(self.logs.row_count() - 1);
        }
    }

    fn confirm(&self, pending: Pending, title: &str, text: &str, ok: &str, danger: bool) {
        let ui = self.ui();
        ui.set_confirm_title(ss(title));
        ui.set_confirm_text(ss(text));
        ui.set_confirm_ok(ss(ok));
        ui.set_confirm_alt(ss(""));
        ui.set_confirm_danger(danger);
        ui.set_confirm_preview(false);
        ui.set_confirm_details(ss(""));
        ui.set_confirm_warnings(ss(""));
        ui.set_confirm_show_dont_ask(false);
        ui.set_confirm_dont_ask(false);
        *self.pending.borrow_mut() = Some(pending);
        ui.set_confirm_visible(true);
    }

    // ---------------- settings <-> UI ----------------

    fn sync_ui_from_settings(&self) {
        let ui = self.ui();
        let s = self.store.lock().unwrap().settings.clone();
        ui.set_text_color(ss(&s.text_color));
        ui.set_bg_color(ss(&s.background_color));
        ui.set_text_color_value(to_slint_color(&s.text_color));
        ui.set_bg_color_value(to_slint_color(&s.background_color));
        ui.set_animation_index(ANIMATIONS.iter().position(|(l, _)| *l == s.animation).unwrap_or(1) as i32);
        ui.set_speed(s.speed as f32);
        ui.set_brightness(s.brightness as f32);
        ui.set_font_label(ss(if s.font.is_empty() || s.font.eq_ignore_ascii_case("UNIFONT") {
            "Unifont (varsayılan)".to_string()
        } else {
            s.font.clone()
        }));
        ui.set_logo_enabled(s.roster_logo_enabled);
        ui.set_logo_mode_index(LOGO_MODES.iter().position(|(_, m)| *m == s.logo_mode).unwrap_or(0) as i32);
        ui.set_own_name(ss(&s.own_pharmacy_name));
        ui.set_own_enabled(s.own_duty_message_enabled);
        ui.set_mirror_h(s.mirror_horizontal);
        ui.set_mirror_v(s.mirror_vertical);
        ui.set_orientation_index(s.orientation.min(3) as i32);
        ui.set_send_method_index(SEND_METHODS.iter().position(|(_, m)| *m == s.send_method).unwrap_or(0) as i32);
        ui.set_save_slot(s.effective_slot() as i32);
        ui.set_auto_send(s.auto_send);
        ui.set_resend_reappear(s.resend_on_reappear);
        ui.set_scan_interval(s.scan_interval_s.clamp(5, 600) as i32);
        ui.set_confirm_send(s.confirm_before_send);
        ui.set_gif_budget(s.gif_budget_kb.clamp(8, 200) as i32);
        ui.set_free_text(ss(&s.free_text));
        self.refresh_panel_info();
    }

    fn read_display_settings(&self) {
        let ui = self.ui();
        {
            let mut st = self.store.lock().unwrap();
            let s = &mut st.settings;
            s.animation = ANIMATIONS.get(ui.get_animation_index().max(0) as usize).map(|(l, _)| l.to_string()).unwrap_or_else(|| "Sola kaydır".into());
            s.speed = ui.get_speed().round().clamp(1.0, 100.0) as u8;
            s.brightness = ui.get_brightness().round().clamp(1.0, 100.0) as u8;
            s.roster_logo_enabled = ui.get_logo_enabled();
            s.logo_mode = LOGO_MODES.get(ui.get_logo_mode_index().max(0) as usize).map(|(_, m)| *m).unwrap_or(LogoMode::FixedGif);
            s.own_pharmacy_name = ui.get_own_name().trim().to_string();
            s.own_duty_message_enabled = ui.get_own_enabled();
            s.mirror_horizontal = ui.get_mirror_h();
            s.mirror_vertical = ui.get_mirror_v();
            s.orientation = ui.get_orientation_index().clamp(0, 3) as u8;
        }
        self.save_soon();
        self.refresh_today();
        self.refresh_preview();
        self.resend_soon();
    }

    /// After the user stops changing display settings for a moment, wake the
    /// worker: automatic mode re-sends today's duty if its content changed.
    fn resend_soon(&self) {
        let auto = self.store.lock().unwrap().settings.auto_send;
        if !auto {
            return;
        }
        self.resend_timer.start(TimerMode::SingleShot, Duration::from_millis(2500), || {
            if let Some(c) = ctx() {
                c.save_now();
                c.submit(Job::Wake);
            }
        });
    }

    fn read_panel_settings(&self) {
        let ui = self.ui();
        {
            let mut st = self.store.lock().unwrap();
            let s = &mut st.settings;
            s.send_method = SEND_METHODS.get(ui.get_send_method_index().max(0) as usize).map(|(_, m)| *m).unwrap_or(SendMethod::SlotSelect);
            s.save_slot = ui.get_save_slot().clamp(1, 10) as u8;
            s.save_to_slot = s.send_method != SendMethod::Live;
            s.auto_send = ui.get_auto_send();
            s.resend_on_reappear = ui.get_resend_reappear();
            s.scan_interval_s = ui.get_scan_interval().clamp(5, 600) as u32;
            s.confirm_before_send = ui.get_confirm_send();
            s.gif_budget_kb = ui.get_gif_budget().clamp(8, 200) as u32;
        }
        self.save_soon();
        self.submit(Job::Wake);
        self.refresh_slots();
        self.refresh_auto_summary();
        self.refresh_preview();
    }

    // ---------------- views ----------------

    fn refresh_panel_info(&self) {
        let ui = self.ui();
        let s = self.store.lock().unwrap().settings.clone();
        ui.set_device_name(ss(&s.device_name));
        ui.set_device_address(ss(&s.device_address));
        ui.set_panel_size(ss(format!("{} × {} piksel", s.panel_width, s.panel_height)));
        self.refresh_devices();
    }

    fn refresh_devices(&self) {
        let s = self.store.lock().unwrap().settings.clone();
        let rows: Vec<DeviceRow> = self
            .devices
            .borrow()
            .iter()
            .map(|d| {
                let addr = ble::format_address(d.address);
                DeviceRow {
                    name: ss(if d.name.is_empty() { "(adsız)" } else { &d.name }),
                    selected: addr == s.device_address || (!d.name.is_empty() && d.name == s.device_name),
                    address: ss(addr),
                    rssi: ss(format!("{} dBm", d.rssi)),
                }
            })
            .collect();
        self.ui().set_devices(ModelRc::new(VecModel::from(rows)));
    }

    fn refresh_slots(&self) {
        let st = self.store.lock().unwrap();
        let rows: Vec<SlotRow> = (1..=10)
            .map(|n| match st.slots.get(&n.to_string()) {
                Some(i) => SlotRow { slot: n, content: ss(&i.text), updated: ss(&i.updated), confirmed: i.confirmed },
                None => SlotRow { slot: n, content: ss("Bilinmiyor / bu uygulamayla kaydedilmedi"), updated: ss(""), confirmed: false },
            })
            .collect();
        drop(st);
        self.ui().set_slots(ModelRc::new(VecModel::from(rows)));
    }

    fn refresh_auto_summary(&self) {
        let s = self.store.lock().unwrap().settings.clone();
        let state = self.panel_state.borrow().clone();
        let last = if s.last_auto_date.is_empty() {
            "Bugünün nöbeti henüz panele gönderilmedi.".to_string()
        } else {
            match parse_date(&s.last_auto_date) {
                Ok(d) if d == today_duty_date() => format!("Bugünün nöbeti panele gönderildi ({}).", date_label(d)),
                Ok(d) => format!("Son gönderilen nöbet: {}.", date_label(d)),
                Err(_) => String::new(),
            }
        };
        let summary = if s.auto_send { format!("{state} {last}") } else { format!("Kapalı. {last}") };
        self.ui().set_auto_summary(ss(summary.trim()));
    }

    fn refresh_today(&self) {
        let ui = self.ui();
        let now = now_local();
        let duty = today_duty_date();
        let st = self.store.lock().unwrap();
        let rec = st.record_for(duty).cloned();
        let s = st.settings.clone();
        drop(st);
        ui.set_today_title(ss(format!("{} · {}", date_label(duty), duty::weekday_long(duty))));
        ui.set_today_window(ss(shift_window_label(duty)));
        ui.set_today_has_record(rec.is_some());
        ui.set_today_names(ss(rec.as_ref().map(|r| r.names().join("\n")).unwrap_or_default()));
        let (status, kind) = if shift_is_running(duty, now) {
            ("Nöbet sürüyor".to_string(), 1)
        } else {
            (format!("{} başlıyor", shift_start(duty).format("%H:%M")), 4)
        };
        ui.set_today_status(ss(status));
        ui.set_today_status_kind(kind);
        ui.set_own_duty(
            rec.as_ref().map(|r| s.own_duty_message_enabled && duty::record_mentions(r, &s.own_pharmacy_name)).unwrap_or(false),
        );
    }

    fn refresh_roster(&self) {
        let ui = self.ui();
        let st = self.store.lock().unwrap();
        let month = self.view_month.get();
        let selected = self.selected.get();
        let duty = today_duty_date();
        ui.set_month_title(ss(format!("{} {}", MONTHS_TITLE[month.month0() as usize], month.year())));
        let first_cell = month - chrono::Duration::days(month.weekday().num_days_from_monday() as i64);
        let days: Vec<CalDay> = (0..42)
            .map(|i| {
                let d = first_cell + chrono::Duration::days(i);
                let rec = st.record_for(d);
                CalDay {
                    day: ss(d.day().to_string()),
                    iso: ss(d.format("%Y-%m-%d").to_string()),
                    names: ss(rec.map(|r| r.names().join("\n")).unwrap_or_default()),
                    in_month: d.month() == month.month(),
                    today: d == duty,
                    selected: d == selected,
                    has_record: rec.is_some(),
                    weekend: d.weekday().num_days_from_monday() >= 5,
                }
            })
            .collect();
        ui.set_cal_days(ModelRc::new(VecModel::from(days)));
        let rows: Vec<RecordRow> = st
            .records
            .values()
            .map(|r| RecordRow {
                iso: ss(r.date.format("%Y-%m-%d").to_string()),
                date: ss(r.date.format("%d.%m.%Y").to_string()),
                weekday: ss(duty::weekday_long(r.date)),
                names: ss(&r.pharmacy),
                today: r.date == duty,
                selected: r.date == selected,
            })
            .collect();
        ui.set_record_count(ss(format!("{} kayıt", rows.len())));
        ui.set_records(ModelRc::new(VecModel::from(rows)));
        let sel_label = format!("Seçili gün ({} {})", selected.day(), MONTHS_TITLE[selected.month0() as usize]);
        ui.set_preview_modes(ModelRc::new(VecModel::from(vec![ss("Bugünün nöbeti"), ss(sel_label), ss("Serbest metin")])));
    }

    fn select_date(&self, d: NaiveDate) {
        self.selected.set(d);
        if month_start(d) != self.view_month.get() {
            self.view_month.set(month_start(d));
        }
        let rec = self.store.lock().unwrap().record_for(d).cloned();
        let ui = self.ui();
        ui.set_form_date(ss(d.format("%d.%m.%Y").to_string()));
        ui.set_form_names(ss(rec.map(|r| r.pharmacy).unwrap_or_default()));
        self.refresh_roster();
        if ui.get_preview_mode() == 1 {
            self.refresh_preview();
        }
    }

    fn current_source(&self) -> Result<(Source, String), String> {
        let ui = self.ui();
        let st = self.store.lock().unwrap();
        match ui.get_preview_mode() {
            0 => {
                let d = today_duty_date();
                st.record_for(d)
                    .cloned()
                    .map(|r| (Source::Record(r), "Bugünün nöbeti panele gönderiliyor…".to_string()))
                    .ok_or_else(|| format!("{} için nöbet kaydı yok. Nöbet listesi sayfasından ekleyin.", date_label(d)))
            }
            1 => {
                let d = self.selected.get();
                st.record_for(d)
                    .cloned()
                    .map(|r| (Source::Record(r), format!("{} nöbeti panele gönderiliyor…", date_label(d))))
                    .ok_or_else(|| format!("{} için kayıt yok. Takvimden kayıtlı bir gün seçin.", date_label(d)))
            }
            _ => {
                let t = ui.get_free_text().trim().to_string();
                if t.is_empty() {
                    Err("Önce serbest metni yazın.".into())
                } else {
                    Ok((Source::FreeText(t), "Serbest metin panele gönderiliyor…".into()))
                }
            }
        }
    }

    fn build(&self, source: &Source) -> Result<render::PanelContent, String> {
        let s = self.store.lock().unwrap().settings.clone();
        let (font, warn) = BitmapFont::load(&s.font);
        let mut c = render::build(source, &s, &font, today_duty_date())?;
        if let Some(w) = warn {
            c.warnings.insert(0, w);
        }
        Ok(c)
    }

    fn show_content(&self, c: &render::PanelContent) {
        let ui = self.ui();
        ui.set_preview_error(ss(""));
        ui.set_preview_text(ss(&c.text));
        ui.set_preview_caption(ss(&c.kind));
        ui.set_preview_warnings(ss(c.warnings.join("\n")));
        *self.preview.borrow_mut() = Some(c.preview.clone());
        self.preview_t0.set(Instant::now());
        self.tick();
    }

    fn refresh_preview(&self) {
        let ui = self.ui();
        match self.current_source().and_then(|(src, _)| self.build(&src)) {
            Ok(c) => self.show_content(&c),
            Err(e) => {
                ui.set_preview_error(ss(&e));
                ui.set_preview_text(ss(""));
                ui.set_preview_caption(ss(""));
                ui.set_preview_warnings(ss(""));
                *self.preview.borrow_mut() = None;
                let s = self.store.lock().unwrap().settings.clone();
                let blank = Frame { width: s.panel_width as usize, height: s.panel_height as usize, pixels: vec![[0, 0, 0]; (s.panel_width * s.panel_height) as usize], delay_ms: 0 };
                ui.set_preview(led_image(&blank));
            }
        }
    }

    fn tick(&self) {
        let p = self.preview.borrow();
        if let Some(model) = p.as_ref() {
            let t = self.preview_t0.get().elapsed().as_millis() as u64;
            self.ui().set_preview(led_image(&model.render(t)));
        }
    }

    // ---------------- sending ----------------

    fn request_send(&self, source: Source, label: String) {
        let content = match self.build(&source) {
            Ok(c) => c,
            Err(e) => {
                self.banner(false, "Gönderilemiyor", &e);
                return;
            }
        };
        let s = self.store.lock().unwrap().settings.clone();
        if !s.confirm_before_send {
            self.submit(Job::Send { source, label, automatic: false });
            return;
        }
        self.show_content(&content);
        let target = if s.device_name.is_empty() { "ilk bulunan LED_BLE_ paneli".to_string() } else { s.device_name.clone() };
        let method = match s.send_method {
            SendMethod::SlotSelect => format!("slot {} kaydedilip kalıcı program yapılır", s.effective_slot()),
            SendMethod::Live => "yalnızca canlı ekran (elektrik kesilince kaybolur)".to_string(),
            SendMethod::SlotAndLive => format!("slot {} + canlı ekran (iki aktarım)", s.effective_slot()),
        };
        let details = format!(
            "Panel: {target}\nTür: {}\nYöntem: {method}\nAktarım: {:.1} KB · parlaklık %{} · yön {} · ekran {}×{}",
            content.kind, content.bytes as f64 / 1024.0, s.brightness, s.orientation, s.panel_width, s.panel_height
        );
        self.confirm(Pending::Send { source, label }, "Panele gönderilsin mi?", &content.text, "Gönder", false);
        let ui = self.ui();
        ui.set_confirm_preview(true);
        ui.set_confirm_details(ss(details));
        ui.set_confirm_warnings(ss(content.warnings.join("\n")));
        ui.set_confirm_show_dont_ask(true);
    }

    fn accept_pending(&self) {
        let ui = self.ui();
        ui.set_confirm_visible(false);
        let Some(p) = self.pending.borrow_mut().take() else { return };
        match p {
            Pending::Send { source, label } => {
                if ui.get_confirm_dont_ask() {
                    self.store.lock().unwrap().settings.confirm_before_send = false;
                    ui.set_confirm_send(false);
                    self.save_now();
                }
                self.submit(Job::Send { source, label, automatic: false });
            }
            Pending::SaveRecord(r) => {
                let date = r.date;
                self.store.lock().unwrap().records.insert(r.date, r);
                self.save_now();
                self.after_roster_change();
                self.select_date(date);
                self.banner(true, "Kaydedildi", &date_label(date));
            }
            Pending::DeleteRecord(d) => {
                self.store.lock().unwrap().records.remove(&d);
                self.save_now();
                self.after_roster_change();
                self.select_date(d);
                self.banner(true, "Kayıt silindi", &date_label(d));
            }
            Pending::Import(records) => {
                let n = records.len();
                let (added, replaced) = self.store.lock().unwrap().merge(records);
                self.save_now();
                self.after_roster_change();
                self.banner(true, "İçe aktarıldı", &format!("{n} kayıt işlendi: {added} yeni, {replaced} değişti."));
            }
            Pending::ShowSlot(n) => self.submit(Job::Command(DeviceCommand::ShowSlot(n))),
            Pending::DeleteSlot(n) => self.submit(Job::Command(DeviceCommand::DeleteSlot(n))),
            Pending::ClearAll => self.submit(Job::Command(DeviceCommand::ClearAll)),
            Pending::PowerOff => self.submit(Job::Command(DeviceCommand::PowerOff)),
            Pending::Raw(b) => self.submit(Job::Command(DeviceCommand::Raw(b))),
            Pending::Close => {
                self.allow_close.set(true);
                let _ = slint::quit_event_loop();
            }
        }
    }

    fn after_roster_change(&self) {
        self.refresh_roster();
        self.refresh_today();
        self.refresh_preview();
        // Today's record may have changed: let automatic mode re-send it.
        self.submit(Job::Wake);
    }

    fn confirm_import(&self, result: import::ImportResult, source: &str) {
        if result.records.is_empty() {
            let detail = if result.errors.is_empty() { "Kayıt bulunamadı.".to_string() } else { result.errors.iter().take(8).cloned().collect::<Vec<_>>().join("\n") };
            self.banner(false, "İçe alınacak kayıt yok", &detail);
            return;
        }
        let st = self.store.lock().unwrap();
        let mut new = 0;
        let mut changed = Vec::new();
        for r in &result.records {
            match st.record_for(r.date) {
                None => new += 1,
                Some(old) if old.pharmacy != r.pharmacy => changed.push(format!("{}: {} → {}", r.date.format("%d.%m.%Y"), old.pharmacy, r.pharmacy)),
                Some(_) => {}
            }
        }
        drop(st);
        let first = result.records.iter().map(|r| r.date).min().unwrap();
        let last = result.records.iter().map(|r| r.date).max().unwrap();
        let text = format!(
            "{source}: {} kayıt ({} – {}). {new} yeni gün eklenecek, {} mevcut kayıt değişecek.",
            result.records.len(),
            first.format("%d.%m.%Y"),
            last.format("%d.%m.%Y"),
            changed.len()
        );
        let mut details = Vec::new();
        if !changed.is_empty() {
            details.push(format!("Değişecek kayıtlar:\n{}", changed.iter().take(10).cloned().collect::<Vec<_>>().join("\n")));
            if changed.len() > 10 {
                details.push(format!("… ve {} kayıt daha", changed.len() - 10));
            }
        }
        let warnings = if result.errors.is_empty() {
            String::new()
        } else {
            format!("Atlanan satırlar ({}):\n{}", result.errors.len(), result.errors.iter().take(6).cloned().collect::<Vec<_>>().join("\n"))
        };
        self.confirm(Pending::Import(result.records), "Nöbet listesi içe aktarılsın mı?", &text, "İçe aktar", !changed.is_empty());
        let ui = self.ui();
        ui.set_confirm_details(ss(details.join("\n")));
        ui.set_confirm_warnings(ss(warnings));
    }

    // ---------------- worker events ----------------

    fn on_event(&self, e: Event) {
        let ui = self.ui();
        match e {
            Event::Log(level, text) => self.log(level, &text),
            Event::Busy(Some(t)) => {
                ui.set_busy(true);
                ui.set_busy_text(ss(t));
            }
            Event::Busy(None) => {
                ui.set_busy(false);
                ui.set_progress(-1.0);
            }
            Event::Progress(p) => ui.set_progress(p),
            Event::Panel(state) => {
                let (text, kind) = match &state {
                    PanelState::Disabled => ("Otomatik izleme kapalı".to_string(), 0),
                    PanelState::Service => ("Arka plan servisi çalışıyor".to_string(), 1),
                    PanelState::Searching => (ui.get_panel_text().to_string(), ui.get_panel_kind()),
                    PanelState::Online { name, rssi } => (format!("{name} hazır · {rssi} dBm"), 1),
                    PanelState::Offline { .. } => ("Panel görünmüyor".to_string(), 2),
                };
                let summary = match &state {
                    PanelState::Disabled => String::new(),
                    PanelState::Service => "Otomatik gönderimi Windows servisi yapıyor; bu pencere kapalıyken de çalışır.".into(),
                    PanelState::Searching => self.panel_state.borrow().clone(),
                    PanelState::Online { name, .. } => format!("Panel {name} menzilde."),
                    PanelState::Offline { detail } => detail.clone(),
                };
                ui.set_panel_text(ss(text));
                ui.set_panel_kind(kind);
                *self.panel_state.borrow_mut() = summary;
                self.refresh_auto_summary();
            }
            Event::Devices(list) => {
                *self.devices.borrow_mut() = list;
                self.refresh_panel_info();
            }
            Event::Done { ok, title, detail } => {
                if ok && title.starts_with("Panel içeriği") {
                    ui.set_last_send(ss(format!("{} · {}", chrono::Local::now().format("%d.%m.%Y %H:%M"), detail)));
                    ui.set_panel_text(ss("Gönderim onaylandı"));
                    ui.set_panel_kind(1);
                }
                self.banner(ok, &title, &detail);
            }
            Event::StoreChanged => {
                self.refresh_panel_info();
                self.refresh_slots();
                self.refresh_auto_summary();
                self.refresh_roster();
                self.refresh_today();
            }
        }
    }
}

fn headless_send(quiet: bool) -> i32 {
    let (store, warn) = Store::load();
    if let Some(w) = warn {
        logfile::write("WARN", &w);
    }
    let duty = today_duty_date();
    let result_path = store::data_dir().join("last_send.json");
    let rec = store.record_for(duty).cloned();
    let outcome = match rec {
        None => Err(format!("{} için nöbet kaydı bulunamadı.", date_label(duty))),
        Some(r) => worker::send_blocking(Arc::new(Mutex::new(store)), Source::Record(r), "Bugünün nöbeti (komut satırı)"),
    };
    let now = chrono::Local::now().to_rfc3339();
    let (code, json) = match &outcome {
        Ok(d) => (0, serde_json::json!({"result": "sent", "date": duty.to_string(), "sent_at": now, "detail": d})),
        Err(e) => (1, serde_json::json!({"result": "error", "date": duty.to_string(), "sent_at": now, "error": e})),
    };
    let _ = std::fs::write(&result_path, serde_json::to_string_pretty(&json).unwrap_or_default());
    if !quiet {
        match outcome {
            Ok(d) => platform::message_box("Nöbetçi Eczane Paneli", &format!("Bugünün nöbeti panele gönderildi ve panel onayladı.\n\n{d}"), false),
            Err(e) => platform::message_box("Nöbetçi Eczane Paneli", &format!("Bugünün nöbeti gönderilemedi.\n\n{e}"), true),
        }
    }
    code
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    // The service runs in session 0 and must never show a message box.
    #[cfg(windows)]
    if has("--service") {
        service::run();
        return;
    }
    if !platform::single_instance() {
        if !has("--minimized") {
            platform::message_box(
                "Nöbetçi Eczane Paneli",
                "Uygulama zaten çalışıyor (görev çubuğuna bakın). İki kopya aynı panele aynı anda bağlanmaya çalışırdı.",
                false,
            );
        }
        return;
    }
    if has("--send-today") {
        std::process::exit(headless_send(has("--quiet")));
    }
    platform::refresh_startup_path();
    if let Err(e) = run_ui(has("--minimized")) {
        platform::message_box("Nöbetçi Eczane Paneli", &format!("Uygulama başlatılamadı: {e}"), true);
    }
}

fn run_ui(minimized: bool) -> Result<(), slint::PlatformError> {
    let (mut store, load_warning) = Store::load();
    if minimized {
        store.settings.auto_send = true;
    }
    store.settings.startup_enabled = platform::startup_enabled();
    let store = Arc::new(Mutex::new(store));
    let ui = AppWindow::new()?;
    ui.set_version(ss(VERSION));
    ui.set_log_path(ss(logfile::path().display().to_string()));
    ui.set_animation_names(ModelRc::new(VecModel::from(ANIMATIONS.iter().map(|(l, _)| ss(l)).collect::<Vec<_>>())));
    ui.set_logo_mode_names(ModelRc::new(VecModel::from(LOGO_MODES.iter().map(|(l, _)| ss(l)).collect::<Vec<_>>())));
    ui.set_send_method_names(ModelRc::new(VecModel::from(SEND_METHODS.iter().map(|(l, _)| ss(l)).collect::<Vec<_>>())));
    ui.set_swatches(ModelRc::new(VecModel::from(
        SWATCHES.iter().map(|h| SwatchItem { hex: ss(h), c: to_slint_color(h) }).collect::<Vec<_>>(),
    )));
    ui.set_startup(platform::startup_enabled());

    let logs = Rc::new(VecModel::<LogRow>::default());
    ui.set_logs(ModelRc::from(logs.clone()));
    let duty = today_duty_date();
    let c = Rc::new(Ctx {
        ui: ui.as_weak(),
        store: store.clone(),
        worker: RefCell::new(None),
        preview: RefCell::new(None),
        preview_t0: Cell::new(Instant::now()),
        pending: RefCell::new(None),
        selected: Cell::new(duty),
        view_month: Cell::new(month_start(duty)),
        duty_day: Cell::new(duty),
        devices: RefCell::new(Vec::new()),
        logs,
        panel_state: RefCell::new("Panel aranıyor…".into()),
        allow_close: Cell::new(false),
        save_timer: Timer::default(),
        resend_timer: Timer::default(),
        banner_timer: Timer::default(),
    });
    CTX.with(|x| *x.borrow_mut() = Some(c.clone()));

    // Worker → UI bridge.
    let weak = ui.as_weak();
    let emit: worker::Emit = Arc::new(move |e: Event| {
        let _ = weak.upgrade_in_event_loop(move |_| {
            if let Some(c) = ctx() {
                c.on_event(e);
            }
        });
    });
    *c.worker.borrow_mut() = Some(Worker::spawn(store.clone(), emit, AutoMode::Window));

    c.sync_ui_from_settings();
    c.select_date(duty);
    c.refresh_today();
    c.refresh_slots();
    c.refresh_auto_summary();
    c.refresh_preview();
    c.log(Level::Info, &format!("Nöbetçi Eczane Paneli {VERSION} başladı. Veri: {}", store::data_path().display()));
    if let Some(w) = load_warning {
        c.banner(false, "Kayıt dosyası", &w);
        c.log(Level::Warn, &w);
    }

    macro_rules! on {
        ($setter:ident, || $body:expr) => {
            ui.$setter(move || {
                if let Some(c) = ctx() {
                    let c: &Ctx = &c;
                    ($body)(c)
                }
            });
        };
        ($setter:ident, |$($arg:ident),*| $body:expr) => {
            ui.$setter(move |$($arg),*| {
                if let Some(c) = ctx() {
                    let c: &Ctx = &c;
                    let _ = &c;
                    $body(c)
                }
            });
        };
    }

    // ---- shell / dashboard ----
    on!(on_banner_close, || |c: &Ctx| c.ui().set_banner_visible(false));
    on!(on_preview_mode_changed, |_i| |c: &Ctx| c.refresh_preview());
    on!(on_free_text_edited, |t| |c: &Ctx| {
        c.store.lock().unwrap().settings.free_text = t.to_string();
        c.save_soon();
        c.refresh_preview();
    });
    on!(on_send_current, || |c: &Ctx| match c.current_source() {
        Ok((src, label)) => c.request_send(src, label),
        Err(e) => c.banner(false, "Gönderilemiyor", &e),
    });
    on!(on_send_image, || |c: &Ctx| {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Panele gönderilecek görsel veya GIF")
            .add_filter("Görsel ve animasyon", &["png", "jpg", "jpeg", "bmp", "webp", "gif"])
            .pick_file()
        {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("görsel").to_string();
            c.request_send(Source::Image(path), format!("{name} panele gönderiliyor…"));
        }
    });

    // ---- roster ----
    on!(on_month_prev, || |c: &Ctx| {
        c.view_month.set(add_months(c.view_month.get(), -1));
        c.refresh_roster();
    });
    on!(on_month_next, || |c: &Ctx| {
        c.view_month.set(add_months(c.view_month.get(), 1));
        c.refresh_roster();
    });
    on!(on_month_today, || |c: &Ctx| c.select_date(today_duty_date()));
    on!(on_select_date, |iso| |c: &Ctx| {
        if let Ok(d) = parse_date(&iso) {
            c.select_date(d);
        }
    });
    on!(on_form_date_edited, |t| |c: &Ctx| {
        if let Ok(d) = parse_date(&t) {
            c.selected.set(d);
            c.view_month.set(month_start(d));
            c.refresh_roster();
        }
    });
    on!(on_save_record, || |c: &Ctx| {
        let ui = c.ui();
        let rec = parse_date(&ui.get_form_date()).and_then(|d| Record::new(d, &ui.get_form_names()));
        match rec {
            Err(e) => c.banner(false, "Kaydedilemedi", &e),
            Ok(r) => {
                let old = c.store.lock().unwrap().record_for(r.date).cloned();
                match old {
                    Some(o) if o.pharmacy == r.pharmacy => c.banner(true, "Değişiklik yok", &date_label(r.date)),
                    Some(o) => {
                        let text = format!("{} kaydı değiştirilsin mi?\n\nEski: {}\nYeni: {}", date_label(r.date), o.pharmacy, r.pharmacy);
                        c.confirm(Pending::SaveRecord(r), "Kayıt değiştirilsin mi?", &text, "Değiştir", false);
                    }
                    None => {
                        let date = r.date;
                        c.store.lock().unwrap().records.insert(r.date, r);
                        c.save_now();
                        c.after_roster_change();
                        c.select_date(date);
                        c.banner(true, "Kaydedildi", &date_label(date));
                    }
                }
            }
        }
    });
    on!(on_delete_record, || |c: &Ctx| {
        let d = parse_date(&c.ui().get_form_date()).unwrap_or(c.selected.get());
        let rec = c.store.lock().unwrap().record_for(d).cloned();
        match rec {
            None => c.banner(false, "Silinecek kayıt yok", &format!("{} için kayıt bulunmuyor.", date_label(d))),
            Some(r) => c.confirm(
                Pending::DeleteRecord(d),
                "Kayıt silinsin mi?",
                &format!("{} · {}\n\nBu işlem geri alınamaz.", date_label(d), r.pharmacy),
                "Sil",
                true,
            ),
        }
    });
    on!(on_import_bulk, || |c: &Ctx| {
        let text = c.ui().get_bulk_text().to_string();
        if text.trim().is_empty() {
            c.banner(false, "Metin boş", "Önce listeyi metin alanına yapıştırın.");
        } else {
            c.confirm_import(import::parse_bulk(&text), "Yapıştırılan liste");
        }
    });
    on!(on_import_csv, || |c: &Ctx| {
        if let Some(p) = rfd::FileDialog::new().set_title("Nöbet listesi CSV").add_filter("CSV / metin", &["csv", "txt"]).pick_file() {
            match std::fs::read(&p) {
                Ok(b) => c.confirm_import(import::parse_bulk(&decode_text(&b)), &p.file_name().and_then(|n| n.to_str()).unwrap_or("CSV").to_string()),
                Err(e) => c.banner(false, "Dosya açılamadı", &e.to_string()),
            }
        }
    });
    on!(on_import_excel, || |c: &Ctx| {
        if let Some(p) = rfd::FileDialog::new().set_title("Aylık nöbet Excel dosyası veya ZIP").add_filter("Excel veya ZIP", &["xlsx", "zip"]).pick_file() {
            match import::import_monthly_file(&p) {
                Ok(r) => c.confirm_import(r, &p.file_name().and_then(|n| n.to_str()).unwrap_or("Excel").to_string()),
                Err(e) => c.banner(false, "Aylık liste okunamadı", &e),
            }
        }
    });
    on!(on_export_csv, || |c: &Ctx| {
        let records: Vec<Record> = c.store.lock().unwrap().records.values().cloned().collect();
        if records.is_empty() {
            c.banner(false, "Dışa aktarılacak kayıt yok", "");
            return;
        }
        if let Some(p) = rfd::FileDialog::new().set_title("Nöbet listesini kaydet").set_file_name("nobet_listesi.csv").add_filter("CSV", &["csv"]).save_file() {
            match std::fs::write(&p, import::export_csv(&records)) {
                Ok(()) => c.banner(true, "CSV kaydedildi", &p.display().to_string()),
                Err(e) => c.banner(false, "CSV yazılamadı", &e.to_string()),
            }
        }
    });

    // ---- display ----
    on!(on_display_changed, || |c: &Ctx| c.read_display_settings());
    on!(on_set_text_color, |hex| |c: &Ctx| match parse_hex_color(&hex) {
        Some(rgb) => {
            c.store.lock().unwrap().settings.text_color = color_hex(rgb);
            c.save_soon();
            c.sync_ui_from_settings();
            c.refresh_preview();
            c.resend_soon();
        }
        None => c.banner(false, "Geçersiz renk", "Renk #RRGGBB biçiminde olmalı, örn. #ffd400."),
    });
    on!(on_set_bg_color, |hex| |c: &Ctx| match parse_hex_color(&hex) {
        Some(rgb) => {
            c.store.lock().unwrap().settings.background_color = color_hex(rgb);
            c.save_soon();
            c.sync_ui_from_settings();
            c.refresh_preview();
            c.resend_soon();
        }
        None => c.banner(false, "Geçersiz renk", "Renk #RRGGBB biçiminde olmalı, örn. #071a35."),
    });
    on!(on_choose_font, || |c: &Ctx| {
        if let Some(p) = rfd::FileDialog::new().set_title("Yazı tipi").add_filter("Yazı tipi", &["ttf", "otf"]).pick_file() {
            let path = p.display().to_string();
            match BitmapFont::from_file(&path) {
                Ok(_) => {
                    c.store.lock().unwrap().settings.font = path;
                    c.save_soon();
                    c.sync_ui_from_settings();
                    c.refresh_preview();
                    c.resend_soon();
                }
                Err(e) => c.banner(false, "Yazı tipi kullanılamıyor", &e),
            }
        }
    });
    on!(on_reset_font, || |c: &Ctx| {
        c.store.lock().unwrap().settings.font = "UNIFONT".into();
        c.save_soon();
        c.sync_ui_from_settings();
        c.refresh_preview();
        c.resend_soon();
    });
    on!(on_apply_orientation, || |c: &Ctx| {
        c.read_display_settings();
        let o = c.store.lock().unwrap().settings.orientation;
        c.submit(Job::Command(DeviceCommand::Orientation(o)));
    });
    on!(on_apply_brightness, || |c: &Ctx| {
        c.read_display_settings();
        let b = c.store.lock().unwrap().settings.brightness;
        c.submit(Job::Command(DeviceCommand::Brightness(b)));
    });

    // ---- panel ----
    on!(on_scan, || |c: &Ctx| c.submit(Job::Scan));
    on!(on_select_device, |i| |c: &Ctx| {
        let d = c.devices.borrow().get(i as usize).cloned();
        if let Some(d) = d {
            {
                let mut st = c.store.lock().unwrap();
                st.settings.device_address = ble::format_address(d.address);
                st.settings.device_name = d.name.clone();
            }
            c.save_now();
            c.refresh_panel_info();
            c.submit(Job::Wake);
            c.banner(true, "Panel seçildi", &format!("{} ({})", d.name, ble::format_address(d.address)));
        }
    });
    on!(on_forget_device, || |c: &Ctx| {
        {
            let mut st = c.store.lock().unwrap();
            st.settings.device_address.clear();
            st.settings.device_name.clear();
        }
        c.save_now();
        c.refresh_panel_info();
    });
    on!(on_diagnose, || |c: &Ctx| c.submit(Job::Diagnose));
    on!(on_device_info, || |c: &Ctx| c.submit(Job::Command(DeviceCommand::Info)));
    on!(on_power_on, || |c: &Ctx| c.submit(Job::Command(DeviceCommand::PowerOn)));
    on!(on_power_off, || |c: &Ctx| c.confirm(
        Pending::PowerOff,
        "Panel kapatılsın mı?",
        "Panel ekranı kapanır; tekrar açmak için “Aç” düğmesini veya panelin kendi düğmesini kullanın.",
        "Kapat",
        true
    ));
    on!(on_sync_time, || |c: &Ctx| c.submit(Job::Command(DeviceCommand::SyncTime)));
    on!(on_panel_settings_changed, || |c: &Ctx| c.read_panel_settings());
    on!(on_toggle_startup, |enabled| |c: &Ctx| {
        match platform::set_startup(enabled) {
            Ok(()) => {
                {
                    let mut st = c.store.lock().unwrap();
                    st.settings.startup_enabled = enabled;
                    if enabled {
                        st.settings.auto_send = true;
                    }
                }
                c.save_now();
                c.sync_ui_from_settings();
                c.refresh_auto_summary();
                c.banner(true, if enabled { "Windows açılışında başlatma etkin" } else { "Windows açılışında başlatma kapatıldı" }, "");
            }
            Err(e) => {
                c.ui().set_startup(!enabled);
                c.banner(false, "Başlangıç ayarı kaydedilemedi", &e);
            }
        }
    });
    on!(on_show_slot, |n| |c: &Ctx| {
        let known = c.store.lock().unwrap().slots.contains_key(&n.to_string());
        if known {
            c.submit(Job::Command(DeviceCommand::ShowSlot(n as u8)));
        } else {
            c.confirm(
                Pending::ShowSlot(n as u8),
                &format!("Slot {n} gösterilsin mi?"),
                "Bu slotun içeriği bilinmiyor. Slot boşsa panel diğer slotlar arasında dolaşabilir veya genel ekranı gösterebilir.",
                "Göster",
                false,
            );
        }
    });
    on!(on_delete_slot, |n| |c: &Ctx| c.confirm(
        Pending::DeleteSlot(n as u8),
        &format!("Slot {n} silinsin mi?"),
        "Paneldeki bu slotun içeriği silinir. Bu işlem geri alınamaz.",
        "Sil",
        true
    ));
    on!(on_clear_memory, || |c: &Ctx| c.confirm(
        Pending::ClearAll,
        "Tüm panel belleği temizlensin mi?",
        "Paneldeki TÜM kayıtlı içerikler ve bazı cihaz ayarları silinir. Panel varsayılan “LED” ekranına döner. Bu işlem geri alınamaz.",
        "Belleği temizle",
        true
    ));
    on!(on_send_raw, || |c: &Ctx| {
        let text = c.ui().get_raw_hex().to_string();
        let hex: String = text.chars().filter(|ch| ch.is_ascii_hexdigit()).collect();
        if hex.is_empty() || hex.len() % 2 != 0 {
            c.banner(false, "Geçersiz hex", "Baytları çift haneli onaltılık sayı olarak yazın, örn. 05 00 04 80 46.");
            return;
        }
        let bytes: Vec<u8> = (0..hex.len()).step_by(2).filter_map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok()).collect();
        c.confirm(
            Pending::Raw(bytes.clone()),
            "Ham komut gönderilsin mi?",
            &format!("{} bayt: {}\n\nYanlış komutlar paneli beklenmedik bir duruma sokabilir.", bytes.len(), protocol::hex(&bytes)),
            "Gönder",
            true,
        );
    });

    // ---- log ----
    on!(on_open_log, || |_c: &Ctx| {
        let _ = std::process::Command::new("notepad.exe").arg(logfile::path()).spawn();
    });
    on!(on_open_data_folder, || |_c: &Ctx| platform::open_in_shell(&store::data_dir()));

    // ---- dialogs ----
    on!(on_confirm_accept, || |c: &Ctx| c.accept_pending());
    on!(on_confirm_cancel, || |c: &Ctx| {
        c.pending.borrow_mut().take();
        c.ui().set_confirm_visible(false);
    });
    on!(on_confirm_alternative, || |c: &Ctx| {
        let ui = c.ui();
        ui.set_confirm_visible(false);
        if matches!(c.pending.borrow_mut().take(), Some(Pending::Close)) {
            ui.window().set_minimized(true);
        }
    });

    // Closing with automatic mode on would silently stop daily sends.
    ui.window().on_close_requested(|| {
        let Some(c) = ctx() else { return CloseRequestResponse::HideWindow };
        if c.allow_close.get() || !c.store.lock().unwrap().settings.auto_send || platform::service_running() {
            let _ = slint::quit_event_loop();
            return CloseRequestResponse::HideWindow;
        }
        c.confirm(
            Pending::Close,
            "Uygulama kapatılsın mı?",
            "Otomatik gönderim yalnızca uygulama açıkken çalışır. Kapatırsanız nöbet günü değişince panel güncellenmez.",
            "Kapat",
            false,
        );
        c.ui().set_confirm_alt(ss("Simge durumuna küçült"));
        CloseRequestResponse::KeepWindowShown
    });

    // Preview animation.
    let preview_timer = Timer::default();
    preview_timer.start(TimerMode::Repeated, Duration::from_millis(40), || {
        if let Some(c) = ctx() {
            c.tick();
        }
    });
    // Clock: refresh "today" when the duty day or status changes.
    let clock_timer = Timer::default();
    clock_timer.start(TimerMode::Repeated, Duration::from_secs(30), || {
        if let Some(c) = ctx() {
            c.refresh_today();
            let d = today_duty_date();
            if d != c.duty_day.get() {
                c.duty_day.set(d);
                c.refresh_roster();
                c.refresh_preview();
                c.refresh_auto_summary();
            }
        }
    });

    ui.show()?;
    if minimized {
        ui.window().set_minimized(true);
    }
    slint::run_event_loop_until_quit()?;
    let _ = ui.hide();

    // Flush settings and stop the worker cleanly (closes any BLE connection).
    let _ = store.lock().unwrap().save();
    if let Some(c) = ctx() {
        if let Some(w) = c.worker.borrow_mut().take() {
            w.shutdown();
        }
    }
    CTX.with(|x| x.borrow_mut().take());
    Ok(())
}
