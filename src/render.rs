//! Builds what goes to the panel (native text, logo GIF or image) together
//! with a pixel-accurate preview of it.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;

use crate::duty::{format_display, own_duty_message, Record};
use crate::font::{BitmapFont, GLYPH_HEIGHT};
use crate::protocol::{self, Plan, TextItem, TextOptions};
use crate::store::{LogoMode, Settings};

pub type Rgb = [u8; 3];

const LOGO_RED: Rgb = [225, 0, 28];
const WHITE: Rgb = [255, 255, 255];
const FRAME_ON: Rgb = [230, 20, 35];
const FRAME_OFF: Rgb = [90, 10, 20];

pub fn parse_hex_color(value: &str) -> Option<Rgb> {
    let v = value.trim().trim_start_matches('#');
    if v.len() != 6 || !v.is_ascii() {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&v[i..i + 2], 16).ok();
    Some([p(0)?, p(2)?, p(4)?])
}

pub fn color_hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<Rgb>,
    pub delay_ms: u32,
}

impl Frame {
    fn new(width: usize, height: usize, bg: Rgb, delay_ms: u32) -> Frame {
        Frame { width, height, pixels: vec![bg; width * height], delay_ms }
    }
    fn set(&mut self, x: i64, y: i64, c: Rgb) {
        if x >= 0 && y >= 0 && (x as usize) < self.width && (y as usize) < self.height {
            self.pixels[y as usize * self.width + x as usize] = c;
        }
    }
    fn fill_rect(&mut self, x0: i64, y0: i64, x1: i64, y1: i64, c: Rgb) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.set(x, y, c);
            }
        }
    }
    fn outline_rect(&mut self, x0: i64, y0: i64, x1: i64, y1: i64, c: Rgb) {
        for x in x0..=x1 {
            self.set(x, y0, c);
            self.set(x, y1, c);
        }
        for y in y0..=y1 {
            self.set(x0, y, c);
            self.set(x1, y, c);
        }
    }
    fn mirror(&mut self, horizontal: bool, vertical: bool) {
        if !horizontal && !vertical {
            return;
        }
        let src = self.pixels.clone();
        for y in 0..self.height {
            for x in 0..self.width {
                let sx = if horizontal { self.width - 1 - x } else { x };
                let sy = if vertical { self.height - 1 - y } else { y };
                self.pixels[y * self.width + x] = src[sy * self.width + sx];
            }
        }
    }
}

/// The 16×16 pharmacy "E" mark: blinking red frame, white rim, red E.
fn draw_logo(f: &mut Frame, x0: i64, y0: i64, frame_on: bool) {
    f.outline_rect(x0 + 1, y0 + 1, x0 + 14, y0 + 14, if frame_on { FRAME_ON } else { FRAME_OFF });
    f.outline_rect(x0 + 2, y0 + 2, x0 + 13, y0 + 13, WHITE);
    // 8×8 white tile with one dark pixel of margin on every side (rows 3 and
    // 12, columns 3 and 12), so the mark sits centred inside the white rim.
    f.fill_rect(x0 + 4, y0 + 4, x0 + 11, y0 + 11, WHITE);
    // Red E: two-pixel spine, 1-px top/bottom bars, 2-px shorter middle bar.
    for (a, b, c, d) in [(5, 5, 10, 5), (5, 5, 6, 10), (5, 7, 9, 8), (5, 10, 10, 10)] {
        f.fill_rect(x0 + a, y0 + b, x0 + c, y0 + d, LOGO_RED);
    }
}

/// A coloured strip of content (`None` = background), 16 rows high.
#[derive(Clone, Debug)]
pub struct ColorStrip {
    pub width: usize,
    pub pixels: Vec<Option<Rgb>>,
}

impl ColorStrip {
    fn from_text(font: &BitmapFont, text: &str, fg: Rgb) -> ColorStrip {
        let s = font.render_strip(text);
        ColorStrip { width: s.width, pixels: s.pixels.iter().map(|p| p.then_some(fg)).collect() }
    }
    fn prepend_logo(&mut self) {
        let mut logo = Frame::new(16, 16, [0, 0, 0], 0);
        draw_logo(&mut logo, 0, 0, true);
        let gap = 8;
        let new_w = self.width + 16 + gap;
        let mut px = vec![None; new_w * GLYPH_HEIGHT];
        for y in 0..GLYPH_HEIGHT {
            for x in 0..16 {
                let c = logo.pixels[y * 16 + x];
                if c != [0, 0, 0] {
                    px[y * new_w + x] = Some(c);
                }
            }
            for x in 0..self.width {
                px[y * new_w + 16 + gap + x] = self.pixels[y * self.width + x];
            }
        }
        self.width = new_w;
        self.pixels = px;
    }
    fn get(&self, x: i64, y: i64) -> Option<Rgb> {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= GLYPH_HEIGHT {
            return None;
        }
        self.pixels[y as usize * self.width + x as usize]
    }
}

/// How the UI preview animates.
#[derive(Clone, Debug)]
pub enum PreviewModel {
    /// Exact frames (GIF / image) – exactly what the panel plays.
    Frames(Vec<Frame>),
    /// Firmware-rendered text: our glyphs, firmware-like motion (approximate timing).
    Firmware { strip: ColorStrip, animation: u8, speed: u8, bg: Rgb, width: usize, height: usize, mirror_h: bool, mirror_v: bool },
}

fn firmware_px_per_s(speed: u8) -> f64 {
    12.0 + speed as f64 * 0.9
}

impl PreviewModel {
    pub fn render(&self, t_ms: u64) -> Frame {
        match self {
            PreviewModel::Frames(frames) => {
                let total: u64 = frames.iter().map(|f| f.delay_ms.max(20) as u64).sum::<u64>().max(1);
                let mut t = t_ms % total;
                for f in frames {
                    let d = f.delay_ms.max(20) as u64;
                    if t < d {
                        return f.clone();
                    }
                    t -= d;
                }
                frames.last().cloned().unwrap_or_else(|| Frame::new(96, 16, [0, 0, 0], 100))
            }
            PreviewModel::Firmware { strip, animation, speed, bg, width, height, mirror_h, mirror_v } => {
                let (w, h) = (*width as i64, *height as i64);
                let mut f = Frame::new(*width, *height, *bg, 0);
                let y0 = (h - GLYPH_HEIGHT as i64) / 2;
                let sw = strip.width as i64;
                let travel = (t_ms as f64 / 1000.0 * firmware_px_per_s(*speed)) as i64;
                let centered = if sw <= w { (w - sw) / 2 } else { 0 };
                let mut visible = true;
                let mut dim = 1.0f64;
                let x0 = match animation {
                    1 => w - travel % (sw + w),
                    2 => -sw + travel % (sw + w),
                    5 => {
                        visible = (t_ms / 500) % 2 == 0;
                        centered
                    }
                    6 => {
                        dim = 0.25 + 0.75 * (0.5 + 0.5 * (t_ms as f64 / 600.0).sin());
                        centered
                    }
                    _ => centered,
                };
                if visible {
                    for y in 0..GLYPH_HEIGHT as i64 {
                        for x in 0..w {
                            if let Some(c) = strip.get(x - x0, y) {
                                let c = [(c[0] as f64 * dim) as u8, (c[1] as f64 * dim) as u8, (c[2] as f64 * dim) as u8];
                                f.set(x, y + y0, c);
                            }
                        }
                    }
                }
                if *animation == 7 {
                    // Snow: a few falling pixels.
                    for i in 0..10u64 {
                        let x = ((i * 37 + 11) % w as u64) as i64;
                        let y = ((t_ms / 90 + i * 7) % h as u64) as i64;
                        f.set(x, y, [200, 220, 255]);
                    }
                }
                f.mirror(*mirror_h, *mirror_v);
                f
            }
        }
    }
}

pub enum Payload {
    Text(Vec<TextItem>, TextOptions),
    Gif(Vec<u8>),
    Png(Vec<u8>),
}

impl Payload {
    pub fn plan(&self, slot: u8) -> Result<Plan, String> {
        match self {
            Payload::Text(items, opts) => protocol::text(items, opts, slot),
            Payload::Gif(b) => Ok(protocol::image(b, true, slot)),
            Payload::Png(b) => Ok(protocol::image(b, false, slot)),
        }
    }
}

pub struct PanelContent {
    /// The text that will scroll on the panel (or the image file name).
    pub text: String,
    /// Human-readable description of the transfer.
    pub kind: String,
    pub payload: Payload,
    pub preview: PreviewModel,
    pub warnings: Vec<String>,
    pub bytes: usize,
}

impl PanelContent {
    /// Fingerprint of the exact bytes this content puts on the panel.
    pub fn signature(&self, brightness: u8, orientation: u8) -> String {
        let mut h = crc32fast::Hasher::new();
        if let Ok(plan) = self.payload.plan(0) {
            for w in &plan.windows {
                h.update(w);
            }
        }
        h.update(&[brightness, orientation]);
        format!("{:08x}", h.finalize())
    }
}

#[derive(Clone, Debug)]
pub enum Source {
    Record(Record),
    FreeText(String),
    Image(PathBuf),
}

pub struct RenderSettings {
    pub width: usize,
    pub height: usize,
    pub fg: Rgb,
    pub bg: Rgb,
    pub animation: u8,
    pub speed: u8,
    pub mirror_h: bool,
    pub mirror_v: bool,
    pub logo: bool,
    pub logo_mode: LogoMode,
    pub gif_budget: usize,
}

impl RenderSettings {
    pub fn from_settings(s: &Settings) -> RenderSettings {
        RenderSettings {
            width: s.panel_width.clamp(16, 512) as usize,
            height: s.panel_height.clamp(16, 64) as usize,
            fg: parse_hex_color(&s.text_color).unwrap_or(WHITE),
            bg: parse_hex_color(&s.background_color).unwrap_or([7, 26, 53]),
            animation: s.animation_code(),
            speed: s.speed.min(100),
            mirror_h: s.mirror_horizontal,
            mirror_v: s.mirror_vertical,
            logo: s.roster_logo_enabled,
            logo_mode: s.logo_mode,
            gif_budget: (s.gif_budget_kb.clamp(8, 200) as usize) * 1024,
        }
    }
}

pub fn build(source: &Source, s: &Settings, font: &BitmapFont, duty_today: NaiveDate) -> Result<PanelContent, String> {
    let rs = RenderSettings::from_settings(s);
    match source {
        Source::Record(rec) => {
            let special = own_duty_message(rec, &s.own_pharmacy_name, s.own_duty_message_enabled, duty_today);
            let logo = special.is_some() || rs.logo;
            let text = special.unwrap_or_else(|| format_display(rec));
            build_text(&text, &rs, font, logo)
        }
        Source::FreeText(t) => {
            let t = t.trim();
            if t.is_empty() {
                return Err("Önce serbest metni yazın.".into());
            }
            build_text(t, &rs, font, false)
        }
        Source::Image(path) => build_image(path, &rs),
    }
}

fn text_options(rs: &RenderSettings, animation: u8) -> TextOptions {
    TextOptions { animation, speed: rs.speed, color: rs.fg, bg: rs.bg }
}

pub fn build_text(text: &str, rs: &RenderSettings, font: &BitmapFont, logo: bool) -> Result<PanelContent, String> {
    let mut warnings = Vec::new();
    let missing = font.missing_chars(text);
    if !missing.is_empty() {
        warnings.push(format!("Yazı tipinde olmayan karakterler '?' olarak görünecek: {}", missing.iter().collect::<String>()));
    }
    let mirrored = rs.mirror_h || rs.mirror_v;
    if logo && rs.logo_mode == LogoMode::FixedGif && rs.height >= 16 {
        let (frames, gif, note) = logo_ticker_gif(text, rs, font)?;
        if let Some(n) = note {
            warnings.push(n);
        }
        let bytes = gif.len();
        return Ok(PanelContent {
            text: text.to_string(),
            kind: format!("E logolu kayan yazı · GIF {} kare, {:.1} KB", frames.len(), bytes as f64 / 1024.0),
            payload: Payload::Gif(gif),
            preview: PreviewModel::Frames(frames),
            warnings,
            bytes,
        });
    }
    if mirrored {
        // The firmware cannot mirror text, so render it ourselves as a GIF.
        let (frames, gif) = plain_ticker_gif(text, rs, font)?;
        let bytes = gif.len();
        return Ok(PanelContent {
            text: text.to_string(),
            kind: format!("Aynalanmış kayan yazı · GIF {} kare, {:.1} KB", frames.len(), bytes as f64 / 1024.0),
            payload: Payload::Gif(gif),
            preview: PreviewModel::Frames(frames),
            warnings,
            bytes,
        });
    }
    let reverse = rs.animation == 2;
    let mut items = font.text_items(text, rs.fg, reverse);
    let mut strip = ColorStrip::from_text(font, text, rs.fg);
    let mut kind = "Panelin kendi kayan yazısı".to_string();
    if logo {
        let jpeg = logo_jpeg()?;
        let space = font.text_items(" ", rs.fg, false);
        let mut with_logo = vec![TextItem::Jpeg(jpeg)];
        with_logo.extend(space);
        if reverse {
            items.extend(with_logo.into_iter().rev());
        } else {
            with_logo.extend(items);
            items = with_logo;
        }
        strip.prepend_logo();
        kind = "Logolu kayan yazı (deneysel: satır içi logo)".into();
    }
    let plan = protocol::text(&items, &text_options(rs, rs.animation), 0)?;
    let bytes = plan.total_bytes();
    Ok(PanelContent {
        text: text.to_string(),
        kind: format!("{kind} · {} karakter, {:.1} KB", items.len(), bytes as f64 / 1024.0),
        payload: Payload::Text(items, text_options(rs, rs.animation)),
        preview: PreviewModel::Firmware {
            strip,
            animation: rs.animation,
            speed: rs.speed,
            bg: rs.bg,
            width: rs.width,
            height: rs.height,
            mirror_h: false,
            mirror_v: false,
        },
        warnings,
        bytes,
    })
}

fn logo_jpeg() -> Result<Vec<u8>, String> {
    let mut f = Frame::new(16, 16, [0, 0, 0], 0);
    draw_logo(&mut f, 0, 0, true);
    let raw: Vec<u8> = f.pixels.iter().flatten().copied().collect();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 95)
        .encode(&raw, 16, 16, image::ExtendedColorType::Rgb8)
        .map_err(|e| e.to_string())?;
    // The official app sends JPEGs without the JFIF APP0 segment.
    if out.len() > 4 && out[2..4] == [0xff, 0xe0] {
        if let Some(p) = out.windows(2).position(|w| w == [0xff, 0xdb]) {
            let mut stripped = vec![0xff, 0xd8];
            stripped.extend_from_slice(&out[p..]);
            out = stripped;
        }
    }
    Ok(out)
}

fn gif_px_per_s(speed: u8) -> f64 {
    15.0 + speed as f64 * 0.6
}

/// Choose the smoothest scroll step whose GIF fits the size budget.
fn ticker_frames<F>(cycle_base: usize, rs: &RenderSettings, palette: &[Rgb], mut draw: F) -> Result<(Vec<Frame>, Vec<u8>, Option<String>), String>
where
    F: FnMut(i64, u32, usize) -> Frame,
{
    let pxps = gif_px_per_s(rs.speed);
    let mut last = None;
    for step in [1usize, 2, 3, 4, 5, 6, 8, 10, 12] {
        let count = cycle_base.div_ceil(step).clamp(2, 400);
        let cycle = count * step;
        let delay_cs = ((step as f64 * 100.0 / pxps).round() as u32).clamp(2, 50);
        let mut frames = Vec::with_capacity(count);
        for i in 0..count {
            let mut f = draw((i * step) as i64, i as u32 * delay_cs * 10, cycle);
            f.delay_ms = delay_cs * 10;
            f.mirror(rs.mirror_h, rs.mirror_v);
            frames.push(f);
        }
        let gif = encode_gif(&frames, palette)?;
        if gif.len() <= rs.gif_budget && count < 400 {
            return Ok((frames, gif, (step > 4).then(|| format!("GIF boyut sınırı için kayma adımı {step} piksel seçildi."))));
        }
        last = Some((frames, gif, step));
    }
    let (frames, gif, step) = last.unwrap();
    let note = format!(
        "Metin çok uzun: GIF {:.0} KB (sınır {} KB, adım {step} px). Aktarım uzun sürebilir; eczane adlarını kısaltmayı düşünün.",
        gif.len() as f64 / 1024.0,
        rs.gif_budget / 1024
    );
    Ok((frames, gif, Some(note)))
}

fn logo_ticker_gif(text: &str, rs: &RenderSettings, font: &BitmapFont) -> Result<(Vec<Frame>, Vec<u8>, Option<String>), String> {
    let strip = font.render_strip(text);
    let logo_w = 16i64;
    let text_left = logo_w + 2;
    let viewport = rs.width as i64 - text_left;
    let y0 = (rs.height as i64 - GLYPH_HEIGHT as i64) / 2;
    let ly = (rs.height as i64 - 16) / 2;
    let palette = [rs.bg, rs.fg, LOGO_RED, WHITE, FRAME_ON, FRAME_OFF];
    ticker_frames(strip.width + 32, rs, &palette, |offset, t_ms, cycle| {
        let mut f = Frame::new(rs.width, rs.height, rs.bg, 0);
        draw_logo(&mut f, 0, ly, (t_ms / 600) % 2 == 0);
        for copy in [-offset, cycle as i64 - offset] {
            for y in 0..GLYPH_HEIGHT as i64 {
                for x in 0..viewport {
                    if strip.get(x - copy, y) {
                        f.set(text_left + x, y + y0, rs.fg);
                    }
                }
            }
        }
        f
    })
}

fn plain_ticker_gif(text: &str, rs: &RenderSettings, font: &BitmapFont) -> Result<(Vec<Frame>, Vec<u8>), String> {
    let strip = font.render_strip(text);
    let w = rs.width as i64;
    let y0 = (rs.height as i64 - GLYPH_HEIGHT as i64) / 2;
    let palette = [rs.bg, rs.fg];
    if rs.animation == 0 || strip.width as i64 <= w && rs.animation != 1 && rs.animation != 2 {
        let mut f = Frame::new(rs.width, rs.height, rs.bg, 1000);
        let x0 = if (strip.width as i64) < w { (w - strip.width as i64) / 2 } else { 0 };
        for y in 0..GLYPH_HEIGHT as i64 {
            for x in 0..w {
                if strip.get(x - x0, y) {
                    f.set(x, y + y0, rs.fg);
                }
            }
        }
        f.mirror(rs.mirror_h, rs.mirror_v);
        let frames = vec![f];
        let gif = encode_gif(&frames, &palette)?;
        return Ok((frames, gif));
    }
    let rightwards = rs.animation == 2;
    let (frames, gif, _) = ticker_frames(strip.width + rs.width, rs, &palette, |offset, _, cycle| {
        let mut f = Frame::new(rs.width, rs.height, rs.bg, 0);
        let x0 = if rightwards { -(strip.width as i64) + offset } else { w - offset };
        let _ = cycle;
        for y in 0..GLYPH_HEIGHT as i64 {
            for x in 0..w {
                if strip.get(x - x0, y) {
                    f.set(x, y + y0, rs.fg);
                }
            }
        }
        f
    })?;
    Ok((frames, gif))
}

/// Encode frames with one small global palette (no per-frame tables).
pub fn encode_gif(frames: &[Frame], palette: &[Rgb]) -> Result<Vec<u8>, String> {
    let mut pal: Vec<Rgb> = Vec::new();
    for c in palette {
        if !pal.contains(c) {
            pal.push(*c);
        }
    }
    let first = frames.first().ok_or("Boş animasyon")?;
    for f in frames {
        for p in &f.pixels {
            if !pal.contains(p) {
                if pal.len() >= 256 {
                    return Err("Animasyonda 256'dan fazla renk var.".into());
                }
                pal.push(*p);
            }
        }
    }
    let flat: Vec<u8> = pal.iter().flatten().copied().collect();
    let mut out = Vec::new();
    {
        let mut enc = gif::Encoder::new(&mut out, first.width as u16, first.height as u16, &flat).map_err(|e| e.to_string())?;
        enc.set_repeat(gif::Repeat::Infinite).map_err(|e| e.to_string())?;
        for f in frames {
            let idx: Vec<u8> = f.pixels.iter().map(|p| pal.iter().position(|c| c == p).unwrap_or(0) as u8).collect();
            let frame = gif::Frame {
                width: f.width as u16,
                height: f.height as u16,
                delay: (f.delay_ms / 10).max(2) as u16,
                dispose: gif::DisposalMethod::Background,
                buffer: Cow::Owned(idx),
                ..Default::default()
            };
            enc.write_frame(&frame).map_err(|e| e.to_string())?;
        }
    }
    Ok(out)
}

fn fit_rgba(img: &image::RgbaImage, w: u32, h: u32) -> image::RgbImage {
    let (iw, ih) = img.dimensions();
    let scale = (w as f64 / iw as f64).min(h as f64 / ih as f64);
    let nw = ((iw as f64 * scale).round() as u32).clamp(1, w);
    let nh = ((ih as f64 * scale).round() as u32).clamp(1, h);
    let resized = image::imageops::resize(img, nw, nh, image::imageops::FilterType::Lanczos3);
    let mut out = image::RgbImage::new(w, h);
    let (ox, oy) = ((w - nw) / 2, (h - nh) / 2);
    for (x, y, p) in resized.enumerate_pixels() {
        let a = p[3] as u32;
        out.put_pixel(x + ox, y + oy, image::Rgb([(p[0] as u32 * a / 255) as u8, (p[1] as u32 * a / 255) as u8, (p[2] as u32 * a / 255) as u8]));
    }
    out
}

fn rgb_frame(img: &image::RgbImage, delay_ms: u32) -> Frame {
    Frame {
        width: img.width() as usize,
        height: img.height() as usize,
        pixels: img.pixels().map(|p| p.0).collect(),
        delay_ms,
    }
}

pub fn build_image(path: &Path, rs: &RenderSettings) -> Result<PanelContent, String> {
    use image::AnimationDecoder;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("görsel").to_string();
    let bytes = std::fs::read(path).map_err(|e| format!("Dosya açılamadı: {e}"))?;
    let (w, h) = (rs.width as u32, rs.height as u32);
    let is_gif = bytes.starts_with(b"GIF8");
    let mut frames: Vec<Frame> = Vec::new();
    if is_gif {
        let dec = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(&bytes)).map_err(|e| format!("GIF okunamadı: {e}"))?;
        for fr in dec.into_frames() {
            let fr = fr.map_err(|e| format!("GIF karesi okunamadı: {e}"))?;
            let (n, d) = fr.delay().numer_denom_ms();
            let delay = if d == 0 { 100 } else { (n / d).max(20) };
            let mut f = rgb_frame(&fit_rgba(fr.buffer(), w, h), delay);
            f.mirror(rs.mirror_h, rs.mirror_v);
            frames.push(f);
            if frames.len() > 300 {
                return Err("GIF 300 kareden uzun; panel için çok büyük.".into());
            }
        }
    } else {
        let img = image::load_from_memory(&bytes).map_err(|e| format!("Görsel okunamadı: {e}"))?.to_rgba8();
        let mut f = rgb_frame(&fit_rgba(&img, w, h), 1000);
        f.mirror(rs.mirror_h, rs.mirror_v);
        frames.push(f);
    }
    let mut warnings = Vec::new();
    let (payload, size, kind) = if frames.len() > 1 {
        let gif = encode_gif_quantized(&frames)?;
        let n = gif.len();
        (Payload::Gif(gif), n, format!("Animasyon · {} kare, {:.1} KB", frames.len(), n as f64 / 1024.0))
    } else {
        let f = &frames[0];
        let raw: Vec<u8> = f.pixels.iter().flatten().copied().collect();
        let mut png_bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut png_bytes, w, h);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_compression(png::Compression::Best);
            let mut wr = enc.write_header().map_err(|e| e.to_string())?;
            wr.write_image_data(&raw).map_err(|e| e.to_string())?;
        }
        let n = png_bytes.len();
        (Payload::Png(png_bytes), n, format!("Sabit görsel · PNG {:.1} KB", n as f64 / 1024.0))
    };
    if size > 64 * 1024 {
        warnings.push(format!("Dosya büyük ({:.0} KB); aktarım uzun sürebilir veya panel reddedebilir.", size as f64 / 1024.0));
    }
    Ok(PanelContent { text: name, kind, payload, preview: PreviewModel::Frames(frames), warnings, bytes: size })
}

fn encode_gif_quantized(frames: &[Frame]) -> Result<Vec<u8>, String> {
    let first = &frames[0];
    let mut out = Vec::new();
    {
        let mut enc = gif::Encoder::new(&mut out, first.width as u16, first.height as u16, &[]).map_err(|e| e.to_string())?;
        enc.set_repeat(gif::Repeat::Infinite).map_err(|e| e.to_string())?;
        for f in frames {
            let mut rgba: Vec<u8> = f.pixels.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect();
            let mut gf = gif::Frame::from_rgba_speed(f.width as u16, f.height as u16, &mut rgba, 10);
            gf.delay = (f.delay_ms / 10).max(2) as u16;
            gf.dispose = gif::DisposalMethod::Background;
            enc.write_frame(&gf).map_err(|e| e.to_string())?;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duty::Record;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn logo_gif_is_compact() {
        let mut s = Settings::default();
        s.roster_logo_enabled = true;
        let rec = Record::new(day(2026, 11, 2), "Merkez / Yıldız / Şifa").unwrap();
        let c = build(&Source::Record(rec), &s, &BitmapFont::unifont(), day(2026, 11, 2)).unwrap();
        assert!(matches!(c.payload, Payload::Gif(_)));
        assert!(c.bytes <= 40 * 1024, "GIF too large: {}", c.bytes);
        let frame = c.preview.render(0);
        assert_eq!((frame.width, frame.height), (96, 16));
        // The logo's red E is present at (5,5).
        assert_eq!(frame.pixels[5 * 96 + 5], LOGO_RED);
        // Decodes as a valid looping GIF.
        if let Payload::Gif(b) = &c.payload {
            let mut opts = gif::DecodeOptions::new();
            opts.set_color_output(gif::ColorOutput::Indexed);
            let mut d = opts.read_info(std::io::Cursor::new(b)).unwrap();
            let mut n = 0;
            while d.read_next_frame().unwrap().is_some() {
                n += 1;
            }
            assert!(n > 10);
        }
    }

    #[test]
    fn logo_is_vertically_centred() {
        let mut f = Frame { width: 16, height: 16, pixels: vec![[0, 0, 0]; 256], delay_ms: 0 };
        draw_logo(&mut f, 0, 0, true);
        let rows: Vec<usize> = (0..16).filter(|&y| (0..16).any(|x| f.pixels[y * 16 + x] != [0, 0, 0])).collect();
        assert_eq!((rows[0], *rows.last().unwrap()), (1, 14));
        // Same dark margin above and below the white tile.
        assert_eq!(f.pixels[3 * 16 + 7], [0, 0, 0]);
        assert_eq!(f.pixels[12 * 16 + 7], [0, 0, 0]);
        assert_eq!(f.pixels[4 * 16 + 7], WHITE);
        assert_eq!(f.pixels[11 * 16 + 7], WHITE);
    }

    #[test]
    fn logo_build_is_fast_and_signature_tracks_settings() {
        let mut s = Settings::default();
        s.roster_logo_enabled = true;
        let rec = Record::new(day(2026, 11, 3), "ÇİĞDEM / YAŞAM").unwrap();
        let t = std::time::Instant::now();
        let c = build(&Source::Record(rec.clone()), &s, &BitmapFont::unifont(), day(2026, 11, 3)).unwrap();
        eprintln!("logo build: {:?}, {} bytes", t.elapsed(), c.bytes);
        let a = c.signature(s.brightness, s.orientation);
        assert_eq!(a, build(&Source::Record(rec.clone()), &s, &BitmapFont::unifont(), day(2026, 11, 3)).unwrap().signature(s.brightness, s.orientation));
        s.roster_logo_enabled = false;
        let b = build(&Source::Record(rec), &s, &BitmapFont::unifont(), day(2026, 11, 3)).unwrap();
        assert_ne!(a, b.signature(s.brightness, s.orientation));
    }

    #[test]
    fn own_duty_uses_logo_and_text() {
        let mut s = Settings::default();
        s.own_pharmacy_name = "Şifa".into();
        s.own_duty_message_enabled = true;
        let rec = Record::new(day(2026, 11, 2), "Merkez / Şifa Eczanesi").unwrap();
        let c = build(&Source::Record(rec.clone()), &s, &BitmapFont::unifont(), day(2026, 11, 2)).unwrap();
        assert!(c.text.contains("BUGÜN NÖBETÇİYİZ: ŞİFA"));
        // Another day: regular native text, no logo.
        let c2 = build(&Source::Record(rec), &s, &BitmapFont::unifont(), day(2026, 11, 3)).unwrap();
        assert!(matches!(c2.payload, Payload::Text(..)));
    }

    #[test]
    fn inline_logo_and_plain_text() {
        let mut s = Settings::default();
        s.roster_logo_enabled = true;
        s.logo_mode = LogoMode::InlineText;
        let rec = Record::new(day(2026, 11, 2), "Merkez").unwrap();
        let c = build(&Source::Record(rec), &s, &BitmapFont::unifont(), day(2026, 11, 2)).unwrap();
        match &c.payload {
            Payload::Text(items, _) => assert!(matches!(items[0], TextItem::Jpeg(_))),
            _ => panic!("expected text"),
        }
        assert!(c.payload.plan(1).is_ok());
    }

    #[test]
    fn mirrored_text_becomes_gif() {
        let mut s = Settings::default();
        s.mirror_horizontal = true;
        let c = build(&Source::FreeText("Merhaba".into()), &s, &BitmapFont::unifont(), day(2026, 11, 2)).unwrap();
        assert!(matches!(c.payload, Payload::Gif(_)));
    }

    #[test]
    fn colors() {
        assert_eq!(parse_hex_color("#071a35"), Some([7, 26, 53]));
        assert_eq!(parse_hex_color("zz"), None);
        assert_eq!(color_hex([255, 0, 16]), "#ff0010");
    }
}
