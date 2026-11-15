//! DL3 wire format, recovered from `DisplayLinkUserAgent` 17.1.56 (arm64).
//!
//! The transport is a bulk pipe carrying 12-byte base headers. The builder at
//! `0x1001273c8` composes them from a mode/fragment table; the validator at
//! `0x100103148` rejects anything whose first byte is neither `0x01` (channel
//! tagged) nor `0x40` (unconditional TRANS) or whose length exceeds the
//! negotiated maximum. Every packet is therefore a fragment of a tile update.
//!
//! ```text
//! off  size  field
//!  0    1    opcode<<5 | flags
//!  1    1    fragment index
//!  2    2    payload length (little endian)
//!  4    1    flags: last<<4 | first<<2 | reserved
//!  5    1    sequence
//!  6    2    reserved
//!  8    2    tile x
//! 10    2    tile y
//! --- payload ---
//! ```
//!
//! Tile coordinates are in 16-pixel units. The vendor also emits a 20-byte
//! "frame section" (`header[0..16]` plus a trailing u32 that is zero on the last
//! fragment of a frame) which is what the receiver echoes back.

use anyhow::{bail, Result};

/// Header length for every packet. The builder hard-codes 12, then appends the
/// 8-byte frame section only for the debug echo.
pub const HEADER_LEN: usize = 12;
/// Largest status payload: a full 512-byte EDID read.
pub const MAX_STATUS: usize = 512;
/// Tile granularity used by the fragmenter.
pub const TILE: usize = 16;

/// Bulk message opcodes (top three bits of byte 0).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Op {
    Nop = 0x00,
    Reset = 0x20,
    Trans = 0x40,
    Map = 0x60,
    Data = 0x80,
}

impl Op {
    pub fn from_byte(b: u8) -> Option<Self> {
        Some(match b & 0xe0 {
            0x00 => Op::Nop,
            0x20 => Op::Reset,
            0x40 => Op::Trans,
            0x60 => Op::Map,
            0x80 => Op::Data,
            _ => return None,
        })
    }
}

/// Control requests handled by `monitor_status` (`0x10028e42c`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum StatusOp {
    Edid = 0x00,
    EdidExtension = 0x02,
    Mode = 0x05,
    Attributes = 0x06,
}

/// A parsed packet header. Parsing never allocates and never trusts the length.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Header {
    pub op: Op,
    pub channel: u8,
    pub len: u16,
    pub first: bool,
    pub last: bool,
    pub seq: u8,
    pub tile_x: u16,
    pub tile_y: u16,
}

impl Header {
    pub fn new(op: Op, seq: u8, len: u16) -> Self {
        Self { op, channel: 0, len, first: true, last: true, seq, tile_x: 0, tile_y: 0 }
    }

    /// Commander-style packet used for the initial channel probe: byte 0 is a
    /// literal `0x01`, byte 1 is the channel id.
    pub fn probe(channel: u8) -> Self {
        Self {
            op: Op::Nop,
            channel,
            len: 0,
            first: true,
            last: true,
            seq: 0,
            tile_x: 0,
            tile_y: 0,
        }
    }

    pub fn flags(&self) -> u8 {
        (self.last as u8) << 4 | (self.first as u8) << 2
    }

    /// Byte 0 as validated by the agent: `0x01` probe, otherwise `op<<5`.
    pub fn tag(&self) -> u8 {
        if self.op == Op::Nop && self.channel != 0 {
            0x01
        } else {
            (self.op as u8) | self.flags()
        }
    }

    pub fn write(&self, out: &mut Vec<u8>) {
        out.push(self.tag());
        out.push(if self.tag() == 0x01 { self.channel } else { 0 });
        out.extend_from_slice(&self.len.to_le_bytes());
        out.push(if self.tag() == 0x01 { 0 } else { self.flags() });
        out.push(self.seq);
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&self.tile_x.to_le_bytes());
        out.extend_from_slice(&self.tile_y.to_le_bytes());
    }
}

pub fn parse(buf: &[u8]) -> Result<(Header, &[u8])> {
    if buf.len() < HEADER_LEN {
        bail!("packet shorter than {HEADER_LEN} bytes");
    }
    let tag = buf[0];
    let op = match tag {
        0x01 => Op::Nop,
        other => Op::from_byte(other).ok_or_else(|| anyhow::anyhow!("invalid TG header"))?,
    };
    let channel = if tag == 0x01 { buf[1] } else { 0 };
    let len = u16::from_le_bytes([buf[2], buf[3]]);
    let payload = &buf[HEADER_LEN..];
    if payload.len() < len as usize {
        bail!("TG length {} exceeds {} available bytes", len, payload.len());
    }
    let flags = if tag == 0x01 { 0 } else { buf[4] };
    Ok((
        Header {
            op,
            channel,
            len,
            first: flags & 0x04 != 0,
            last: flags & 0x10 != 0,
            seq: buf[5],
            tile_x: u16::from_le_bytes([buf[8], buf[9]]),
            tile_y: u16::from_le_bytes([buf[10], buf[11]]),
        },
        &payload[..len as usize],
    ))
}

/// The device checksums status payloads additively; `MonitorStatus: is_valid_checksum`
/// (`0x10026c668`) compares the running sum against the trailing u16.
pub fn checksum(data: &[u8]) -> u16 {
    data.iter().fold(0u16, |acc, b| acc.wrapping_add(*b as u16))
}

/// Split a frame into tile fragments. `chunk` bounds the payload per packet so
/// the result also fits the endpoint's negotiated max packet size.
pub fn fragment(seq: &mut u8, x: u16, y: u16, w: u16, h: u16, chunk: usize) -> Vec<Header> {
    let mut out = Vec::new();
    let mut frag_index = 0u8;
    let mut ty = y;
    while ty < y.saturating_add(h) {
        let mut tx = x;
        while tx < x.saturating_add(w) {
            *seq = seq.wrapping_add(1);
            let mut head = Header::new(Op::Data, *seq, 0);
            head.first = frag_index == 0;
            head.tile_x = tx / TILE as u16;
            head.tile_y = ty / TILE as u16;
            head.len = chunk.min(MAX_STATUS) as u16;
            frag_index = frag_index.wrapping_add(1);
            out.push(head);
            tx = tx.saturating_add(TILE as u16);
        }
        ty = ty.saturating_add(TILE as u16);
    }
    if let Some(last) = out.last_mut() {
        last.last = true;
    }
    let n = out.len();
    for (i, h) in out.iter_mut().enumerate() {
        h.first = i == 0;
        h.last = i + 1 == n;
    }
    out
}

/// EDID sanity check, mirroring the vendor's `Invalid length of raw EDID data`
/// and header test. `0x00ffffffffffff00` is the fixed EDID 1.x preamble.
pub fn valid_edid(edid: &[u8]) -> bool {
    edid.len() >= 128
        && edid.len() % 128 == 0
        && edid.starts_with(&[0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00])
}

/// Set the standard 1-byte EDID checksum on a 128-byte block so it sums to zero.
pub fn fix_edid_checksum(block: &mut [u8]) {
    if block.len() < 128 {
        return;
    }
    block[127] = 0;
    block[127] = 0u8.wrapping_sub(block[..127].iter().fold(0u8, |a, b| a.wrapping_add(*b)));
}

/// A status payload (EDID read reply, mode list, attributes) with its checksum.
pub fn status_body(op: StatusOp, arg: u16, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 7);
    out.push(op as u8);
    out.extend_from_slice(&arg.to_le_bytes());
    out.extend_from_slice(&(data.len() as u16).to_le_bytes());
    out.extend_from_slice(data);
    let sum = checksum(&out);
    out.extend_from_slice(&sum.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_roundtrip() {
        let mut buf = Vec::new();
        Header::probe(7).write(&mut buf);
        assert_eq!(buf.len(), HEADER_LEN);
        assert_eq!(buf[0], 0x01);
        assert_eq!(buf[1], 7);
        let (head, payload) = parse(&buf).unwrap();
        assert_eq!(head.op, Op::Nop);
        assert_eq!(head.channel, 7);
        assert!(payload.is_empty());
    }

    #[test]
    fn data_header_flags() {
        let mut head = Header::new(Op::Data, 9, 32);
        head.first = true;
        head.last = false;
        head.tile_x = 4;
        head.tile_y = 2;
        let mut buf = Vec::new();
        head.write(&mut buf);
        buf.extend_from_slice(&[0x5a; 32]);
        assert_eq!(buf[0], 0x80 | 0x04);
        assert_eq!(&buf[..HEADER_LEN].len(), &HEADER_LEN);
        let (back, _) = parse(&buf).unwrap();
        assert_eq!(back.op, Op::Data);
        assert!(back.first && !back.last);
        assert_eq!((back.tile_x, back.tile_y), (4, 2));
        assert_eq!(back.seq, 9);
    }

    #[test]
    fn rejects_short_and_overlong() {
        assert!(parse(&[0x40, 0]).is_err());
        let mut buf = Vec::new();
        Header::new(Op::Data, 0, 64).write(&mut buf);
        assert!(parse(&buf).is_err());
        buf.extend_from_slice(&[0; 64]);
        assert!(parse(&buf).is_ok());
        let mut bad = vec![0x05u8; HEADER_LEN];
        bad[2] = 0;
        assert!(parse(&bad).is_err());
    }

    #[test]
    fn fragment_marks_first_and_last() {
        let mut seq = 0;
        let heads = fragment(&mut seq, 0, 0, 48, 32, 1024);
        assert_eq!(heads.len(), 6);
        assert!(heads[0].first && !heads[0].last);
        assert!(heads.last().unwrap().last && !heads.last().unwrap().first);
        assert_eq!(heads[1].seq, heads[0].seq + 1);
        assert_eq!(heads[3].tile_y, 1);
    }

    #[test]
    fn edid_rules() {
        let mut edid = [0u8; 128];
        edid[..8].copy_from_slice(&[0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00]);
        assert!(valid_edid(&edid));
        assert!(!valid_edid(&edid[..64]));
        fix_edid_checksum(&mut edid);
        assert_eq!(checksum(&edid) as u8, 0);
    }

    #[test]
    fn template_edid_is_valid_and_describes_the_size() {
        let e = template_edid(600, 340, "Neolink Panel");
        assert!(valid_edid(&e));
        assert_eq!(checksum(&e) as u8, 0);
        assert_eq!((e[21], e[22]), (255u8, 255u8), "cm fields saturate");
        assert_eq!((e[8], e[9]), (0x39, 0x6f), "NEO");
        assert_eq!(&e[72..90], &{
            let mut d = [0u8; 18];
            d[3] = 0xfc;
            d[5..18].copy_from_slice(b"Neolink Panel");
            d[17] = 0x0a;
            d
        });
        assert_eq!(e[126], 0, "no extensions");
    }

    #[test]
    fn template_survives_roundtrip_through_status_body() {
        let e = template_edid(500, 300, "X");
        let body = status_body(StatusOp::Edid, 0, &e);
        assert_eq!(body.len(), e.len() + 7);
        assert_eq!(body[0], StatusOp::Edid as u8);
        assert_eq!(checksum(&body[..body.len() - 2]), u16::from_le_bytes([body[body.len() - 2], body[body.len() - 1]]));
    }

    #[test]
    fn status_checksum_trails_body() {
        let body = status_body(StatusOp::Edid, 1, &[0xaa; 4]);
        let sum = checksum(&body[..body.len() - 2]);
        assert_eq!(u16::from_le_bytes([body[body.len() - 2], body[body.len() - 1]]), sum);
    }
}

/// A synthesised 128-byte EDID 1.4 base block for a display of `width`×`height` mm.
///
/// Used when a dock answers no EDID (or the user overrides one): the descriptor is only
/// geometry and a range limit, so nothing vendor-specific is baked in and the block is ours to
/// ship. The checksum is set, so [`valid_edid`] accepts it.
pub fn template_edid(width_mm: u16, height_mm: u16, name: &str) -> Vec<u8> {
    let mut e = vec![0u8; 128];
    e[..8].copy_from_slice(&[0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00]);
    // Manufacturer id "NEO", little-endian 5-bit packing; product + serial are fixed so a given
    // dock always presents the same identity.
    // "NEO" packed 5 bits per letter, most significant letter first.
    e[8] = 0x39;
    e[9] = 0x6f;
    e[10] = 0x01;
    e[11] = 0x00;
    e[12] = 0x01;
    e[13] = 0x00;
    e[14] = 0x00;
    e[16] = 35; // week
    e[17] = 30; // 2020
    e[18] = 1; // EDID 1.4
    e[19] = 4;
    e[20] = 0xa5; // digital input, 8 bpc, DisplayPort
    e[21] = (width_mm.min(u16::from(u8::MAX))) as u8;
    e[22] = (height_mm.min(u16::from(u8::MAX))) as u8;
    e[23] = 120; // gamma 2.2
    e[24] = 0x0a; // RGB, preferred timing, sRGB
    e[25] = 0xee; // RGB 4:4:4, sRGB default
    e[26] = 0x91;
    e[27] = 0xa3;
    e[28] = 0x57;
    e[29] = 0x4c;
    e[30] = 0x9e;
    e[31] = 0x25;
    e[32] = 0x10;
    e[33] = 0x50;
    e[34] = 0x54; // established timings: 640x480@60, 800x600@60, 1024x768@60
    // Standard timings, all 16:9 and 16:10 at 60 Hz, then unused.
    let pairs: [(u16, u16); 8] = [
        (1280, 720),
        (1280, 800),
        (1366, 768),
        (1440, 900),
        (1600, 900),
        (1680, 1050),
        (1920, 1080),
        (1920, 1200),
    ];
    for (i, (w, h)) in pairs.iter().enumerate() {
        let at = 38 + i * 2;
        e[at] = (w / 8).saturating_sub(31) as u8;
        e[at + 1] = ((((*h as f32 / *w as f32) * 4.0 - 1.0) * 16.0) as u8 & 0xf0) | 0x01;
    }
    // Descriptor 1: range limits (48–75 Hz, 300 MHz).
    e[54..72].copy_from_slice(&[
        0x00, 0x00, 0x00, 0xfd, 0x00, 0x30, 0x4b, 0x12, 0x3c, 0x11, 0x00, 0x0a, 0x20, 0x20, 0x20,
        0x20, 0x20, 0x20,
    ]);
    // Descriptor 2: monitor name.
    let mut d = [0u8; 18];
    d[3] = 0xfc;
    d[5..].fill(0x20);
    for (i, b) in name.as_bytes().iter().take(13).enumerate() {
        d[5 + i] = *b;
    }
    d[17] = 0x0a;
    e[72..90].copy_from_slice(&d);
    // Descriptor 3: a second, preferred-ish detailed timing for 1920x1080@60.
    e[90..108].copy_from_slice(&detailed_timing(1920, 1080));
    e[108..126].fill(0x20);
    e[126] = 0;
    e[127] = 0;
    fix_edid_checksum(&mut e);
    e
}

/// One 18-byte detailed timing descriptor. Blanking follows the CEA-861 1080p60 numbers scaled
/// to the pixel clock so any caller size stays arithmetically consistent.
fn detailed_timing(w: u16, h: u16) -> [u8; 18] {
    let hactive = w.min(4095);
    let vactive = h.min(4095);
    let hblank = (w / 6).max(40).min(1023);
    let vblank = (h / 40).max(4).min(63);
    let hsync_off = (hblank / 4).min(1023);
    let hsync_w = 32u16.min(hblank / 2);
    let vsync_off = (vblank / 6).max(1).min(63);
    let vsync_w = 3u16.min(vblank / 2).max(1);
    let htotal = hactive + hblank;
    let vtotal = vactive + vblank;
    // Pixel clock in 10 kHz units, derived from the totals so it stays consistent with the
    // blanking written below (1920x1080@60 lands on 148.7 MHz).
    let clock = (htotal as u32 * vtotal as u32 * 60 / 10_000).min(65_535) as u16;
    let mut d = [0u8; 18];
    d[0..2].copy_from_slice(&clock.to_le_bytes());
    d[2] = (hactive & 0xff) as u8;
    d[3] = (hblank & 0xff) as u8;
    d[4] = ((hactive >> 8) as u8 & 0x0f) << 4 | ((hblank >> 8) as u8 & 0x0f);
    d[5] = (vactive & 0xff) as u8;
    d[6] = (vblank & 0xff) as u8;
    d[7] = ((vactive >> 8) as u8 & 0x0f) << 4 | ((vblank >> 8) as u8 & 0x0f);
    d[8] = (hsync_off & 0xff) as u8;
    d[9] = (hsync_w & 0xff) as u8;
    d[10] = (((vsync_off & 0x0f) as u8) << 4) | (vsync_w & 0x0f) as u8;
    d[11] = (((hsync_off >> 8) as u8 & 0x03) << 6)
        | (((hsync_w >> 8) as u8 & 0x03) << 4)
        | (((vsync_off >> 4) as u8 & 0x03) << 2)
        | ((vsync_w >> 4) as u8 & 0x03);
    d[12] = (w / 10).min(255) as u8;
    d[13] = (h / 10).min(255) as u8;
    d[14] = 0;
    d[15] = 0;
    d[16] = 0;
    d[17] = 0x1e;
    d
}
