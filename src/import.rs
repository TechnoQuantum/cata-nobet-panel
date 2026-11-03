//! Roster import (pasted text / CSV / monthly Excel / ZIP of Excel files) and CSV export.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::Path;

use chrono::{Datelike, Duration, NaiveDate};

use crate::duty::{normalize_tr, parse_date, Record};

pub struct ImportResult {
    pub records: Vec<Record>,
    pub errors: Vec<String>,
}

/// Lines like `YYYY-MM-DD | Name 1 | Name 2`; tab, `|`, `;` or `,` separated.
pub fn parse_bulk(text: &str) -> ImportResult {
    let mut records = Vec::new();
    let mut errors = Vec::new();
    for (idx, raw_line) in text.trim_start_matches('\u{feff}').lines().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let delim = ['\t', '|', ';', ','].into_iter().find(|d| line.contains(*d));
        let Some(delim) = delim else {
            errors.push(format!("{line_no}. satır: tarih ve eczane adı bulunamadı"));
            continue;
        };
        let parts: Vec<String> = line.split(delim).map(|p| p.trim().trim_matches('"').trim().to_string()).collect();
        let date = match parse_date(&parts[0]) {
            Ok(d) => d,
            Err(e) => {
                // A header row ("Tarih;Eczane adı…") is skipped silently.
                let lower = normalize_tr(&parts[0]);
                if !(lower.contains("TARIH") || lower.contains("DATE")) {
                    errors.push(format!("{line_no}. satır: {e}"));
                }
                continue;
            }
        };
        let names: Vec<&str> = parts[1..].iter().map(|s| s.as_str()).filter(|s| !s.is_empty()).collect();
        match Record::new(date, &names.join(" / ")) {
            Ok(r) => records.push(r),
            Err(e) => errors.push(format!("{line_no}. satır: {e}")),
        }
    }
    ImportResult { records, errors }
}

pub fn export_csv(records: &[Record]) -> String {
    let mut out = String::from("\u{feff}Tarih;Nöbetçi eczaneler\r\n");
    for r in records {
        let name = r.pharmacy.replace('"', "\"\"");
        let name = if name.contains(';') { format!("\"{name}\"") } else { name };
        out.push_str(&format!("{};{}\r\n", r.date.format("%Y-%m-%d"), name));
    }
    out
}

const MONTHS: [(&str, u32); 12] = [
    ("OCAK", 1), ("SUBAT", 2), ("MART", 3), ("NISAN", 4), ("MAYIS", 5), ("HAZIRAN", 6),
    ("TEMMUZ", 7), ("AGUSTOS", 8), ("EYLUL", 9), ("EKIM", 10), ("KASIM", 11), ("ARALIK", 12),
];

fn find_month(text: &str) -> Option<u32> {
    let n = normalize_tr(text);
    MONTHS.iter().find(|(m, _)| n.contains(m)).map(|(_, v)| *v)
}

fn find_year(text: &str) -> Option<i32> {
    let b = text.as_bytes();
    (0..b.len().saturating_sub(3)).find_map(|i| {
        let s = &text.get(i..i + 4)?;
        let boundary_ok = (i == 0 || !b[i - 1].is_ascii_digit()) && (i + 4 == b.len() || !b[i + 4].is_ascii_digit());
        (s.starts_with("20") && s.bytes().all(|c| c.is_ascii_digit()) && boundary_ok).then(|| s.parse().ok()).flatten()
    })
}

fn col_of(cell_ref: &str) -> String {
    cell_ref.chars().take_while(|c| c.is_ascii_alphabetic()).collect::<String>().to_ascii_uppercase()
}

struct Sheet {
    rows: Vec<HashMap<String, String>>,
    b2: String,
}

fn zip_text(archive: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Option<String>, String> {
    let mut file = match archive.by_name(name) {
        Ok(f) => f,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let mut s = String::new();
    file.read_to_string(&mut s).map_err(|e| e.to_string())?;
    Ok(Some(s))
}

fn read_first_sheet(content: &[u8]) -> Result<Sheet, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(content)).map_err(|e| format!("Excel dosyası açılamadı: {e}"))?;
    let workbook = zip_text(&mut archive, "xl/workbook.xml")?.ok_or("xl/workbook.xml bulunamadı")?;
    let rels = zip_text(&mut archive, "xl/_rels/workbook.xml.rels")?.ok_or("Çalışma kitabı ilişkileri bulunamadı")?;
    let wb = roxmltree::Document::parse(&workbook).map_err(|e| e.to_string())?;
    let sheet = wb.descendants().find(|n| n.has_tag_name("sheet")).ok_or("Excel dosyasında çalışma sayfası bulunamadı.")?;
    let rel_id = sheet
        .attributes()
        .find(|a| a.name() == "id")
        .map(|a| a.value().to_string())
        .ok_or("Çalışma sayfası kimliği okunamadı")?;
    let rd = roxmltree::Document::parse(&rels).map_err(|e| e.to_string())?;
    let target = rd
        .descendants()
        .find(|n| n.has_tag_name("Relationship") && n.attribute("Id") == Some(rel_id.as_str()))
        .and_then(|n| n.attribute("Target"))
        .ok_or("Excel çalışma sayfası bağlantısı okunamadı.")?;
    let sheet_path = if let Some(stripped) = target.strip_prefix('/') { stripped.to_string() } else { format!("xl/{target}") };
    let sheet_path = sheet_path.replace("xl/xl/", "xl/");

    let shared: Vec<String> = match zip_text(&mut archive, "xl/sharedStrings.xml")? {
        Some(xml) => {
            let doc = roxmltree::Document::parse(&xml).map_err(|e| e.to_string())?;
            doc.root_element()
                .children()
                .filter(|n| n.has_tag_name("si"))
                .map(|si| {
                    si.descendants()
                        .filter(|n| n.has_tag_name("t") && !n.ancestors().any(|a| a.has_tag_name("rPh")))
                        .filter_map(|t| t.text())
                        .collect()
                })
                .collect()
        }
        None => Vec::new(),
    };
    let xml = zip_text(&mut archive, &sheet_path)?.ok_or_else(|| format!("{sheet_path} bulunamadı"))?;
    let doc = roxmltree::Document::parse(&xml).map_err(|e| e.to_string())?;
    let cell_value = |c: roxmltree::Node| -> String {
        let t = c.attribute("t").unwrap_or("");
        if t == "inlineStr" {
            return c.descendants().filter(|n| n.has_tag_name("t")).filter_map(|n| n.text()).collect();
        }
        let v = c.children().find(|n| n.has_tag_name("v")).and_then(|n| n.text()).unwrap_or("");
        if t == "s" {
            v.trim().parse::<usize>().ok().and_then(|i| shared.get(i).cloned()).unwrap_or_default()
        } else {
            v.to_string()
        }
    };
    let mut rows = Vec::new();
    let mut b2 = String::new();
    for row in doc.descendants().filter(|n| n.has_tag_name("row")) {
        let mut values = HashMap::new();
        for c in row.children().filter(|n| n.has_tag_name("c")) {
            let r = c.attribute("r").unwrap_or("");
            let v = cell_value(c).trim().to_string();
            if r.eq_ignore_ascii_case("B2") {
                b2 = v.clone();
            }
            values.insert(col_of(r), v);
        }
        rows.push(values);
    }
    Ok(Sheet { rows, b2 })
}

fn excel_serial_date(v: &str) -> Option<NaiveDate> {
    let n: f64 = v.trim().parse().ok()?;
    if !(30000.0..80000.0).contains(&n) {
        return None;
    }
    NaiveDate::from_ymd_opt(1899, 12, 30).map(|base| base + Duration::days(n.floor() as i64))
}

/// Monthly layout: title "2026 KASIM" in B2 (or month/year in the file name),
/// column A "1 - Pazartesi" (optionally "(ARALIK 2026)" for spill-over days),
/// pharmacy names in B and C, note in D. Full dates / Excel dates in A also work.
pub fn parse_xlsx(filename: &str, content: &[u8]) -> Result<Vec<(NaiveDate, Vec<String>)>, String> {
    let sheet = read_first_sheet(content)?;
    let stem = Path::new(filename).file_stem().and_then(|s| s.to_str()).unwrap_or(filename);
    let year = find_year(&sheet.b2).or_else(|| find_year(stem)).unwrap_or_else(|| chrono::Local::now().year());
    let month = find_month(&sheet.b2).or_else(|| find_month(stem));

    let mut out = Vec::new();
    for values in &sheet.rows {
        let a = values.get("A").map(String::as_str).unwrap_or("").trim();
        if a.is_empty() {
            continue;
        }
        let date = if let Some(d) = excel_serial_date(a).or_else(|| parse_date(a).ok()) {
            Some(d)
        } else {
            // "12 - Perşembe" style day number.
            let digits: String = a.chars().take_while(|c| c.is_ascii_digit()).collect();
            let rest = a[digits.len()..].trim_start();
            if digits.is_empty() || !rest.starts_with('-') {
                None
            } else if let Some(month) = month {
                let (mut y, mut m) = (year, month);
                if let (Some(open), Some(close)) = (a.find('('), a.rfind(')')) {
                    if open < close {
                        let note = &a[open + 1..close];
                        if let Some(em) = find_month(note) {
                            m = em;
                            y = find_year(note).unwrap_or(if em < month { year + 1 } else { year });
                        }
                    }
                }
                digits.parse().ok().and_then(|d| NaiveDate::from_ymd_opt(y, m, d))
            } else {
                return Err(format!("Ay başlığı anlaşılamadı: {filename} (B2 hücresine örn. \"2026 KASIM\" yazın)"));
            }
        };
        let Some(date) = date else { continue };
        let names: Vec<String> = ["B", "C"]
            .iter()
            .filter_map(|c| values.get(*c))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !names.is_empty() {
            out.push((date, names));
        }
    }
    Ok(out)
}

/// Import a `.xlsx` file or a `.zip` that contains `.xlsx` files.
pub fn import_monthly_file(path: &Path) -> Result<ImportResult, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Dosya açılamadı: {e}"))?;
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    if name.to_ascii_lowercase().ends_with(".zip") {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes.as_slice())).map_err(|e| format!("ZIP açılamadı: {e}"))?;
        for i in 0..archive.len() {
            let mut f = archive.by_index(i).map_err(|e| e.to_string())?;
            let fname = f.name().to_string();
            let base = fname.rsplit('/').next().unwrap_or(&fname).to_string();
            if fname.to_ascii_lowercase().ends_with(".xlsx") && !fname.starts_with("__MACOSX/") && !base.starts_with("~$") {
                let mut buf = Vec::new();
                f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
                files.push((base, buf));
            }
        }
        if files.is_empty() {
            return Err("ZIP içinde .xlsx dosyası bulunamadı.".into());
        }
    } else {
        files.push((name, bytes));
    }
    let mut records = Vec::new();
    let mut errors = Vec::new();
    for (fname, content) in files {
        match parse_xlsx(&fname, &content) {
            Ok(rows) => {
                for (d, names) in rows {
                    match Record::new(d, &names.join(" / ")) {
                        Ok(r) => records.push(r),
                        Err(e) => errors.push(format!("{fname} {d}: {e}")),
                    }
                }
            }
            Err(e) => errors.push(format!("{fname}: {e}")),
        }
    }
    if records.is_empty() && errors.is_empty() {
        errors.push("A sütununda gün ve B/C sütunlarında eczane adı olan kayıt bulunamadı.".into());
    }
    Ok(ImportResult { records, errors })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bulk_formats_and_roundtrip() {
        let text = "Tarih;Eczane adı A;Eczane adı B\n2026-10-01;A;B\n02.10.2026 | C\n# yorum\n2026-10-03\nxx;Y\n";
        let r = parse_bulk(text);
        assert_eq!(r.records.len(), 2);
        assert_eq!(r.records[0].pharmacy, "A / B");
        assert_eq!(r.errors.len(), 2, "{:?}", r.errors);
        let csv = export_csv(&r.records);
        let again = parse_bulk(&csv);
        assert_eq!(again.records, r.records);
        assert!(again.errors.is_empty());
    }

    #[test]
    fn sample_roster_imports() {
        let text = include_str!("../examples/anonymized_roster.csv");
        let r = parse_bulk(text);
        assert!(r.records.len() > 20);
        assert!(r.errors.is_empty(), "{:?}", r.errors);
    }

    #[test]
    fn year_and_month_detection() {
        assert_eq!(find_year("nobet_2027_ocak"), Some(2027));
        assert_eq!(find_year("120265"), None);
        assert_eq!(find_month("2026 Şubat"), Some(2));
        assert_eq!(excel_serial_date("46326"), NaiveDate::from_ymd_opt(2026, 10, 31));
    }
}
