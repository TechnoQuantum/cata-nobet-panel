//! 16-pixel glyphs for the panel: embedded GNU Unifont bitmaps (identical to
//! what pypixelcolor sent) or a user-selected TTF/OTF rasterised the same way.

use std::collections::HashMap;

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};

use crate::protocol::TextItem;

#[path = "glyphs_data.rs"]
mod glyphs_data;

pub const GLYPH_HEIGHT: usize = 16;

#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    pub rows: [u16; 16],
    pub wide: bool,
}

impl Glyph {
    pub fn width(&self) -> usize {
        if self.wide { 16 } else { 8 }
    }
    pub fn pixel(&self, x: usize, y: usize) -> bool {
        x < 16 && y < 16 && (self.rows[y] >> (15 - x)) & 1 == 1
    }
}

pub struct BitmapFont {
    glyphs: HashMap<char, Glyph>,
    pub name: String,
}

/// A monochrome strip of rendered text (row-major, `width` × 16).
pub struct Strip {
    pub width: usize,
    pub pixels: Vec<bool>,
}

impl Strip {
    pub fn get(&self, x: i64, y: i64) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.width && (y as usize) < GLYPH_HEIGHT && self.pixels[y as usize * self.width + x as usize]
    }
}

impl BitmapFont {
    pub fn unifont() -> BitmapFont {
        let glyphs = glyphs_data::GLYPHS.iter().map(|(c, wide, rows)| (*c, Glyph { rows: *rows, wide: *wide })).collect();
        BitmapFont { glyphs, name: "UNIFONT".into() }
    }

    /// Rasterise a TTF/OTF at a 16 px em size with a 50 % coverage threshold,
    /// like Pillow + pypixelcolor do. Glyphs not in the font fall back to Unifont.
    pub fn from_file(path: &str) -> Result<BitmapFont, String> {
        let data = std::fs::read(path).map_err(|e| format!("Yazı tipi okunamadı: {e}"))?;
        let font = FontVec::try_from_vec(data).map_err(|_| "Yazı tipi dosyası geçersiz.".to_string())?;
        let upem = font.units_per_em().unwrap_or(1000.0);
        let scale = PxScale::from(16.0 * font.height_unscaled() / upem);
        let scaled = font.as_scaled(scale);
        let ascent = scaled.ascent();
        let mut base = BitmapFont::unifont();
        let chars: Vec<char> = base.glyphs.keys().copied().collect();
        for ch in chars {
            let id = font.glyph_id(ch);
            if id.0 == 0 && ch != ' ' {
                continue;
            }
            let g = id.with_scale_and_position(scale, ab_glyph::point(0.0, ascent));
            let mut rows = [0u16; 16];
            let mut ink_width = 0.0f32;
            if let Some(outline) = font.outline_glyph(g) {
                let b = outline.px_bounds();
                ink_width = b.max.x - b.min.x.min(0.0);
                outline.draw(|x, y, cov| {
                    let px = x as i32 + b.min.x.floor() as i32;
                    let py = y as i32 + b.min.y.floor() as i32;
                    if cov > 0.5 && (0..16).contains(&px) && (0..16).contains(&py) {
                        rows[py as usize] |= 1 << (15 - px);
                    }
                });
            }
            let wide = ink_width > 10.0;
            if !wide {
                for r in rows.iter_mut() {
                    *r &= 0xff00;
                }
            }
            base.glyphs.insert(ch, Glyph { rows, wide });
        }
        base.name = std::path::Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path).to_string();
        Ok(base)
    }

    pub fn load(setting: &str) -> (BitmapFont, Option<String>) {
        let s = setting.trim();
        if s.is_empty() || s.eq_ignore_ascii_case("UNIFONT") || s.eq_ignore_ascii_case("DEFAULT") {
            return (BitmapFont::unifont(), None);
        }
        match BitmapFont::from_file(s) {
            Ok(f) => (f, None),
            Err(e) => (BitmapFont::unifont(), Some(format!("{e} Unifont kullanılıyor."))),
        }
    }

    pub fn glyph(&self, c: char) -> Glyph {
        self.glyphs.get(&c).or_else(|| self.glyphs.get(&'?')).copied().unwrap()
    }

    pub fn missing_chars(&self, text: &str) -> Vec<char> {
        let mut out: Vec<char> = text.chars().filter(|c| !self.glyphs.contains_key(c)).collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    pub fn text_items(&self, text: &str, color: [u8; 3], reverse: bool) -> Vec<TextItem> {
        let mut items: Vec<TextItem> = text
            .chars()
            .map(|c| {
                let g = self.glyph(c);
                TextItem::Glyph { rows: g.rows, wide: g.wide, color }
            })
            .collect();
        if reverse {
            items.reverse();
        }
        items
    }

    pub fn render_strip(&self, text: &str) -> Strip {
        let glyphs: Vec<Glyph> = text.chars().map(|c| self.glyph(c)).collect();
        let width = glyphs.iter().map(Glyph::width).sum::<usize>().max(1);
        let mut pixels = vec![false; width * GLYPH_HEIGHT];
        let mut x0 = 0;
        for g in glyphs {
            for y in 0..GLYPH_HEIGHT {
                for x in 0..g.width() {
                    if g.pixel(x, y) {
                        pixels[y * width + x0 + x] = true;
                    }
                }
            }
            x0 += g.width();
        }
        Strip { width, pixels }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turkish_coverage() {
        let f = BitmapFont::unifont();
        assert!(f.missing_chars("ÇĞİÖŞÜçğıöşü NÖBETÇİ ECZANELER: 18:00 - 08:30 ₺").is_empty());
        assert_eq!(f.missing_chars("a😀"), vec!['😀']);
        assert_eq!(f.render_strip("Ab").width, 16);
    }
}
