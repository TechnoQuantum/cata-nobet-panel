//! iPIXEL BLE command encoding (wire-compatible with pypixelcolor 0.5.0).

pub const WRITE_UUID: u128 = 0x0000fa02_0000_1000_8000_00805f9b34fb;
pub const NOTIFY_UUID: u128 = 0x0000fa03_0000_1000_8000_00805f9b34fb;
pub const DEVICE_PREFIX: &str = "LED_BLE_";

/// Max data bytes per transport window (the firmware's 12 KiB receive buffer).
const WINDOW_SIZE: usize = 12 * 1024;
/// Longest text the firmware accepts: the item count is one byte.
pub const MAX_TEXT_ITEMS: usize = 255;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AckPolicy {
    /// Wait for one acknowledgement after the (single) window.
    PerWindow,
    /// Data transfer: ACK after each window and a final "complete" ACK (code 3).
    PerWindowAndFinal,
    /// The command returns a response frame instead of an ACK.
    Response,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub name: String,
    pub windows: Vec<Vec<u8>>,
    pub ack: AckPolicy,
    /// Command opcode the panel echoes in its ACK (`05 00 <op0> <op1> <status>`),
    /// used to ignore late ACKs that belong to an earlier command.
    pub opcode: Option<[u8; 2]>,
}

impl Plan {
    fn single(name: &str, data: Vec<u8>) -> Plan {
        let opcode = Some([data[2], data[3]]);
        Plan { name: name.into(), windows: vec![data], ack: AckPolicy::PerWindow, opcode }
    }
    pub fn total_bytes(&self) -> usize {
        self.windows.iter().map(Vec::len).sum()
    }
}

pub fn brightness(level: u8) -> Plan {
    Plan::single("parlaklık", vec![5, 0, 4, 0x80, level.clamp(1, 100)])
}

pub fn orientation(value: u8) -> Plan {
    Plan::single("yön", vec![5, 0, 6, 0x80, value.min(3)])
}

pub fn power(on: bool) -> Plan {
    Plan::single(if on { "aç" } else { "kapat" }, vec![5, 0, 7, 1, on as u8])
}

/// Make `slot` the panel's program list (persists across power cycles).
pub fn show_slot(slot: u8) -> Plan {
    Plan::single("slot seç", vec![7, 0, 8, 0x80, 1, 0, slot])
}

pub fn delete_slot(slot: u8) -> Plan {
    Plan::single("slot sil", vec![7, 0, 2, 1, 1, 0, slot])
}

pub fn clear_all() -> Plan {
    Plan::single("belleği temizle", vec![4, 0, 3, 0x80])
}

pub fn set_time(h: u8, m: u8, s: u8) -> Plan {
    Plan::single("saat eşitle", vec![8, 0, 1, 0x80, h, m, s, 0])
}

pub fn device_info_request(h: u8, m: u8, s: u8) -> Plan {
    Plan { name: "cihaz bilgisi".into(), windows: vec![vec![8, 0, 1, 0x80, h, m, s, 0]], ack: AckPolicy::Response, opcode: None }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub device_type: u8,
    pub width: u32,
    pub height: u32,
    pub password_flag: u8,
}

pub fn parse_device_info(resp: &[u8]) -> Option<DeviceInfo> {
    if resp.len() < 5 {
        return None;
    }
    let device_type = resp[4];
    let led_type = match device_type {
        128 => 0, 129 => 2, 130 => 4, 131 => 3, 132 => 1, 133 => 5, 134 => 6, 135 => 7, 136 => 8, 137 => 9,
        138 => 10, 139 => 11, 140 => 12, 141 => 13, 142 => 14, 143 => 15, 144 => 16, 145 => 17, 146 => 18, 147 => 19,
        _ => return None,
    };
    let (width, height) = match led_type {
        0 => (64, 64), 1 => (96, 16), 2 => (32, 32), 3 => (64, 16), 4 => (32, 16), 5 => (64, 20),
        6 => (128, 32), 7 => (144, 16), 8 => (192, 16), 9 => (48, 24), 10 => (64, 32), 11 => (96, 32),
        12 => (128, 32), 13 => (96, 32), 14 => (160, 32), 15 => (192, 32), 16 => (256, 32), 17 => (320, 32),
        18 => (384, 32), _ => (448, 32),
    };
    Some(DeviceInfo { device_type, width, height, password_flag: resp.get(10).copied().unwrap_or(255) })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notify {
    /// Window received (codes 0/1).
    WindowAck,
    /// Whole transfer received (code 3).
    Complete,
    /// Some other status code; logged and reported if no success follows.
    Status(u8),
    Other,
}

pub fn parse_notify(data: &[u8]) -> Notify {
    if data.len() >= 5 && data[0] == 0x05 {
        match data[4] {
            0 | 1 => Notify::WindowAck,
            3 => Notify::Complete,
            c => Notify::Status(c),
        }
    } else {
        Notify::Other
    }
}

fn data_plan(name: &str, type_bytes: [u8; 2], tail0: u8, payload: &[u8], slot: u8) -> Plan {
    let crc = crc32fast::hash(payload);
    let size = payload.len() as u32;
    let windows = payload
        .chunks(WINDOW_SIZE)
        .enumerate()
        .map(|(i, chunk)| {
            let mut frame = Vec::with_capacity(chunk.len() + 15);
            frame.extend_from_slice(&type_bytes);
            frame.push(if i == 0 { 0x00 } else { 0x02 });
            frame.extend_from_slice(&size.to_le_bytes());
            frame.extend_from_slice(&crc.to_le_bytes());
            frame.push(tail0);
            frame.push(slot);
            frame.extend_from_slice(chunk);
            let mut out = ((frame.len() + 2) as u16).to_le_bytes().to_vec();
            out.extend(frame);
            out
        })
        .collect();
    Plan { name: name.into(), windows, ack: AckPolicy::PerWindowAndFinal, opcode: None }
}

pub fn image(bytes: &[u8], is_gif: bool, slot: u8) -> Plan {
    if is_gif {
        data_plan("GIF", [0x03, 0x00], 0x02, bytes, slot)
    } else {
        data_plan("görsel", [0x02, 0x00], 0x00, bytes, slot)
    }
}

/// One item of a firmware-rendered text: a glyph bitmap or an inline JPEG.
#[derive(Clone, Debug)]
pub enum TextItem {
    /// 16 rows; pixel x=0 at bit 15. `wide` = 16 px cell, otherwise 8 px.
    Glyph { rows: [u16; 16], wide: bool, color: [u8; 3] },
    Jpeg(Vec<u8>),
}

fn encode_item(item: &TextItem, out: &mut Vec<u8>) {
    match item {
        TextItem::Glyph { rows, wide, color } => {
            out.push(*wide as u8);
            out.extend_from_slice(color);
            for r in rows {
                out.push(((r >> 8) as u8).reverse_bits());
                if *wide {
                    out.push((*r as u8).reverse_bits());
                }
            }
        }
        TextItem::Jpeg(bytes) => {
            out.push(0x08);
            out.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
            out.push(0x00);
            out.extend_from_slice(bytes);
        }
    }
}

pub struct TextOptions {
    pub animation: u8,
    pub speed: u8,
    pub color: [u8; 3],
    pub bg: [u8; 3],
}

pub fn text(items: &[TextItem], opts: &TextOptions, slot: u8) -> Result<Plan, String> {
    if items.is_empty() {
        return Err("Gönderilecek metin boş.".into());
    }
    if items.len() > MAX_TEXT_ITEMS {
        return Err(format!(
            "Metin {} karakter; panel en fazla {MAX_TEXT_ITEMS} karakter kabul ediyor. Eczane adlarını kısaltın.",
            items.len()
        ));
    }
    let mut payload = vec![items.len() as u8, 0x00, 0x01, 0x01, opts.animation, opts.speed.min(100), 0];
    payload.extend_from_slice(&opts.color);
    payload.push(0x01);
    payload.extend_from_slice(&opts.bg);
    for item in items {
        encode_item(item, &mut payload);
    }
    Ok(data_plan("metin", [0x00, 0x01], 0x00, &payload, slot))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::BitmapFont;

    /// Bytes produced by pypixelcolor 0.5.0:
    /// send_text("Nöbet İ-1", animation=1, speed=70, color="ffffff", bg_color="071a35", save_slot=1, char_height=16)
    const PYPIXELCOLOR_REFERENCE: &str = "d100000100c20000001ff26033000109000101014600ffffff01071a3500ffffff000000004246464a4a5252626242000000ffffff0000242400003c4242424242423c000000ffffff0000000202023a4642424242463a000000ffffff0000000000003c42427e0202423c000000ffffff000000000808083e080808080830000000ffffff0000000000000000000000000000000000ffffff101000007c10101010101010107c000000ffffff0000000000000000003c00000000000000ffffff000000001018141010101010107c0000";

    #[test]
    fn text_matches_pypixelcolor() {
        let font = BitmapFont::unifont();
        let items = font.text_items("Nöbet İ-1", [0xff, 0xff, 0xff], false);
        let plan = text(&items, &TextOptions { animation: 1, speed: 70, color: [0xff; 3], bg: [0x07, 0x1a, 0x35] }, 1).unwrap();
        assert_eq!(plan.windows.len(), 1);
        let got: String = plan.windows[0].iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(got, PYPIXELCOLOR_REFERENCE);
    }

    #[test]
    fn large_payload_is_windowed() {
        let data = vec![7u8; 30_000];
        let plan = image(&data, true, 2);
        assert_eq!(plan.windows.len(), 3);
        assert_eq!(plan.windows[0][2..5], [0x03, 0x00, 0x00]);
        assert_eq!(plan.windows[1][4], 0x02);
        assert_eq!(plan.windows[0][13..15], [0x02, 0x02]);
        assert_eq!(plan.total_bytes(), 30_000 + 3 * 15);
    }

    #[test]
    fn too_long_text_rejected() {
        let font = BitmapFont::unifont();
        let items = font.text_items(&"x".repeat(256), [255; 3], false);
        assert!(text(&items, &TextOptions { animation: 1, speed: 1, color: [0; 3], bg: [0; 3] }, 0).is_err());
    }

    #[test]
    fn notify_parsing() {
        assert_eq!(parse_notify(&[5, 0, 1, 0, 1]), Notify::WindowAck);
        assert_eq!(parse_notify(&[5, 0, 1, 0, 3]), Notify::Complete);
        assert_eq!(parse_notify(&[5, 0, 1, 0, 2]), Notify::Status(2));
        // Real CT-4568 replies (2026-11-03): brightness ACK and device info.
        assert_eq!(brightness(70).opcode, Some([0x04, 0x80]));
        assert_eq!(parse_notify(&[0x05, 0x00, 0x04, 0x80, 0x01]), Notify::WindowAck);
        let info = parse_device_info(&[0x0b, 0x00, 0x01, 0x80, 0x84, 0x39, 0x0b, 0x00, 0x00, 0x01, 0x00]).unwrap();
        assert_eq!((info.width, info.height, info.device_type), (96, 16, 132));
        assert_eq!(parse_device_info(&[0, 0, 0, 0, 132, 0]).unwrap().width, 96);
    }
}
