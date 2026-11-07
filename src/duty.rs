//! Duty-roster domain logic: dates, shift windows and the text shown on the panel.

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use unicode_normalization::UnicodeNormalization;

/// Regular shifts start at 18:00; Sunday's continuous shift starts at 08:30.
pub const SHIFT_END: (u32, u32) = (8, 30);
pub const REGULAR_START: (u32, u32) = (18, 0);

const MONTHS_UPPER: [&str; 12] = [
    "OCAK", "ŞUBAT", "MART", "NİSAN", "MAYIS", "HAZİRAN", "TEMMUZ", "AĞUSTOS", "EYLÜL", "EKİM", "KASIM", "ARALIK",
];
pub const MONTHS_TITLE: [&str; 12] = [
    "Ocak", "Şubat", "Mart", "Nisan", "Mayıs", "Haziran", "Temmuz", "Ağustos", "Eylül", "Ekim", "Kasım", "Aralık",
];
pub const WEEKDAYS_LONG: [&str; 7] = ["Pazartesi", "Salı", "Çarşamba", "Perşembe", "Cuma", "Cumartesi", "Pazar"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub date: NaiveDate,
    /// One or more pharmacy names separated by " / ".
    pub pharmacy: String,
}

impl Record {
    pub fn new(date: NaiveDate, pharmacy: &str) -> Result<Self, String> {
        let names = split_pharmacies(pharmacy);
        if names.is_empty() {
            return Err("Eczane adı zorunludur.".into());
        }
        Ok(Self { date, pharmacy: names.join(" / ") })
    }

    pub fn names(&self) -> Vec<String> {
        split_pharmacies(&self.pharmacy)
    }
}

/// Split a "/"-separated list of pharmacy names, trimming blanks.
pub fn split_pharmacies(value: &str) -> Vec<String> {
    value
        .split('/')
        .map(|n| n.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|n| !n.is_empty())
        .collect()
}

pub fn parse_date(value: &str) -> Result<NaiveDate, String> {
    let v = value.trim();
    for fmt in ["%Y-%m-%d", "%d.%m.%Y", "%d/%m/%Y", "%d-%m-%Y", "%Y.%m.%d", "%Y/%m/%d"] {
        if let Ok(d) = NaiveDate::parse_from_str(v, fmt) {
            if (2000..=2100).contains(&d.year()) {
                return Ok(d);
            }
        }
    }
    Err(format!("Tarih okunamadı: {v}"))
}

fn shift_end_time() -> NaiveTime {
    NaiveTime::from_hms_opt(SHIFT_END.0, SHIFT_END.1, 0).unwrap()
}

/// The roster date whose shift is running now, or today's upcoming shift during
/// the daytime gap. Shifts end at 08:30, so before 08:30 the previous day's shift
/// (including Sunday's 24-hour shift on Monday morning) is still active.
pub fn active_duty_date(now: NaiveDateTime) -> NaiveDate {
    if now.time() < shift_end_time() {
        now.date() - Duration::days(1)
    } else {
        now.date()
    }
}

pub fn now_local() -> NaiveDateTime {
    chrono::Local::now().naive_local()
}

pub fn today_duty_date() -> NaiveDate {
    active_duty_date(now_local())
}

pub fn shift_start(d: NaiveDate) -> NaiveDateTime {
    let (h, m) = if d.weekday() == Weekday::Sun { SHIFT_END } else { REGULAR_START };
    d.and_hms_opt(h, m, 0).unwrap()
}

pub fn shift_end(d: NaiveDate) -> NaiveDateTime {
    (d + Duration::days(1)).and_time(shift_end_time())
}

/// Is the shift for `d` running at `now` (as opposed to upcoming / finished)?
pub fn shift_is_running(d: NaiveDate, now: NaiveDateTime) -> bool {
    now >= shift_start(d) && now < shift_end(d)
}

pub fn shift_window_label(d: NaiveDate) -> String {
    let start = shift_start(d);
    let end = shift_end(d);
    format!(
        "{} {} {} - {} {} {}",
        start.day(),
        MONTHS_UPPER[start.month0() as usize],
        start.format("%H:%M"),
        end.day(),
        MONTHS_UPPER[end.month0() as usize],
        end.format("%H:%M")
    )
}

pub fn date_label(d: NaiveDate) -> String {
    format!("{} {} {}", d.day(), MONTHS_TITLE[d.month0() as usize], d.year())
}

pub fn weekday_long(d: NaiveDate) -> &'static str {
    WEEKDAYS_LONG[d.weekday().num_days_from_monday() as usize]
}

/// Turkish-aware uppercase (i → İ, ı → I).
pub fn tr_upper(value: &str) -> String {
    value
        .chars()
        .flat_map(|c| match c {
            'i' => vec!['İ'],
            'ı' => vec!['I'],
            other => other.to_uppercase().collect(),
        })
        .collect()
}

/// Uppercase, Turkish dotted/dotless I folded to I, and diacritics removed.
pub fn normalize_tr(value: &str) -> String {
    let folded: String = value
        .chars()
        .map(|c| match c {
            'İ' | 'ı' | 'i' => 'I',
            other => other,
        })
        .collect();
    folded
        .to_uppercase()
        .nfd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .collect()
}

fn pharmacy_key(value: &str) -> String {
    let n = normalize_tr(value);
    let n = n.replace("ECZANESI", "").replace("ECZANE", "");
    n.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Regular roster ticker text.
pub fn format_display(rec: &Record) -> String {
    let names: Vec<String> = rec
        .names()
        .into_iter()
        .map(|n| if normalize_tr(&n).contains("ECZANE") { n } else { format!("{n} ECZANESİ") })
        .collect();
    format!("{} - NÖBETÇİ ECZANELER: {}", shift_window_label(rec.date), names.join("  -  "))
}

/// Does our own pharmacy appear in this record?
pub fn record_mentions(rec: &Record, own_name: &str) -> bool {
    let own = pharmacy_key(own_name);
    !own.is_empty() && rec.names().iter().any(|n| pharmacy_key(n) == own)
}

/// "BUGÜN NÖBETÇİYİZ" text: only for the currently active duty date, so the
/// preview and the actual send always agree.
pub fn own_duty_message(rec: &Record, own_name: &str, enabled: bool, duty_today: NaiveDate) -> Option<String> {
    if !enabled || rec.date != duty_today || !record_mentions(rec, own_name) {
        return None;
    }
    Some(format!("{} - BUGÜN NÖBETÇİYİZ: {}", shift_window_label(rec.date), tr_upper(own_name.trim())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(y: i32, m: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, mi, 0).unwrap()
    }
    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn duty_date_switches_at_0830() {
        // 2026-11-02 is a Monday.
        assert_eq!(active_duty_date(dt(2026, 11, 2, 8, 29)), day(2026, 11, 1));
        assert_eq!(active_duty_date(dt(2026, 11, 2, 8, 30)), day(2026, 11, 2));
        assert_eq!(active_duty_date(dt(2026, 11, 2, 23, 59)), day(2026, 11, 2));
        assert_eq!(active_duty_date(dt(2026, 11, 3, 0, 0)), day(2026, 11, 2));
    }

    #[test]
    fn shift_windows() {
        assert_eq!(shift_window_label(day(2026, 11, 2)), "2 KASIM 18:00 - 3 KASIM 08:30");
        // Sunday 2026-11-01 runs 08:30 → Monday 08:30.
        assert_eq!(shift_window_label(day(2026, 11, 1)), "1 KASIM 08:30 - 2 KASIM 08:30");
        assert_eq!(shift_window_label(day(2026, 12, 31)), "31 ARALIK 18:00 - 1 OCAK 08:30");
        assert!(shift_is_running(day(2026, 11, 1), dt(2026, 11, 1, 9, 0)));
        assert!(!shift_is_running(day(2026, 11, 2), dt(2026, 11, 2, 12, 0)));
        assert!(shift_is_running(day(2026, 11, 2), dt(2026, 11, 3, 8, 0)));
    }

    #[test]
    fn display_text() {
        let r = Record::new(day(2026, 11, 2), "Merkez / Yıldız Eczanesi").unwrap();
        assert_eq!(
            format_display(&r),
            "2 KASIM 18:00 - 3 KASIM 08:30 - NÖBETÇİ ECZANELER: Merkez ECZANESİ  -  Yıldız Eczanesi"
        );
    }

    #[test]
    fn own_duty_matches_consistently() {
        let r = Record::new(day(2026, 11, 2), "Merkez/ŞİFA ECZANESİ").unwrap();
        assert_eq!(r.pharmacy, "Merkez / ŞİFA ECZANESİ");
        assert!(record_mentions(&r, "şifa"));
        assert!(record_mentions(&r, "Şifa Eczanesi"));
        assert!(!record_mentions(&r, "Şif"));
        let msg = own_duty_message(&r, "şifa", true, day(2026, 11, 2)).unwrap();
        assert!(msg.ends_with("BUGÜN NÖBETÇİYİZ: ŞİFA"));
        assert!(own_duty_message(&r, "şifa", true, day(2026, 11, 3)).is_none());
        assert!(own_duty_message(&r, "şifa", false, day(2026, 11, 2)).is_none());
    }

    #[test]
    fn parses_dates() {
        assert_eq!(parse_date("2026-11-03").unwrap(), day(2026, 11, 3));
        assert_eq!(parse_date("03.11.2026").unwrap(), day(2026, 11, 3));
        assert_eq!(parse_date(" 3/11/2026 ").unwrap(), day(2026, 11, 3));
        assert!(parse_date("33.11.2026").is_err());
        assert!(Record::new(day(2026, 1, 1), " / ").is_err());
    }
}
