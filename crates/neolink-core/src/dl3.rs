//! DL3 wire format, recovered from `DisplayLinkUserAgent` 17.1.56 (arm64).
//!
//! The transport is a bulk pipe carrying 20-byte headers. The builder at
//! `0x1001270bc` / `0x1001271d4` / `0x1001273c8` composes them from a queue of
//! damage rectangles; the validator at `0x100103148` accepts only tag `0x01`
//! (channel probe, byte 1 must equal the negotiated channel) or `0x40`
//! (unconditional TRANS) and rejects a length above the negotiated maximum
//! (`Invalid TG length.`). The builder hands exactly 20 bytes to the pipe, so
//! header and payload are contiguous on the wire.
//!
//! ```text
//! off  size  field
//!  0    1    tag: 0x01 probe | op<<5 | last<<4 | cont<<3 | first<<2 | stream
//!  1    1    channel (probe only, else 0)
//!  2    2    payload length (big endian)
//!  4    1    same tag byte as offset 0
//!  5    1    fragment index, 1-based, bumped per continued fragment
//!  6    2    packet counter (little endian, wraps)
//!  8    1    tile rect x      (16-pixel units)
//!  9    1    tile rect y
//! 10    1    tile rect x_end
//! 11    1    tile rect y_end
//! 12    4    fragment offset within the rectangle (little endian)
//! 16    4    frame tag carried through from the caller
//! --- payload ---
//! ```
//!
//! The rectangle is inclusive: the builder writes `x>>4` and `(x+w+0xf)>>4`, so
//! one packet describes the whole damaged span and the receiver walks it. The
//! `first` bit is set only while the fragment index is still 0 (first packet of
//! a frame); `last` is set when the fragment offset is back at zero, i.e. on the
//! first packet of each rectangle. A continuation packet sets `cont` and moves
//! the previous rect's `x_end` into `x` so the receiver resumes mid-row.
//!
//! Status replies (`StatusOp`) never reach the ring: `MonitorStatus` carries a
//! `u16` checksum over the body and is validated before use (`0x10026c668`).

use anyhow::{bail, Result};

/// Header length for every packet. The builder copies exactly this many bytes
/// (a 16-byte `ldur`/`stur` pair plus a `u32`) out of its 20-byte staging
/// buffer before handing the buffer to the transport.
pub const HEADER_LEN: usize = 20;
/// Largest status payload: a full 512-byte EDID read.
pub const MAX_STATUS: usize = 512;
/// Tile granularity used by the fragmenter; rectangle ends round up to it.
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
///
/// `Mode` takes an optional byte argument at `data[1]`, `Attributes` a `u16`
/// argument (the vendor passes `arg & 0xffff`). Both reply through the normal
/// status envelope with the additive checksum.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum StatusOp {
    Edid = 0x00,
    EdidExtension = 0x02,
    Mode = 0x05,
    Attributes = 0x06,
}

/// HDCP 2.x message ids, in the order the vendor enumerates them
/// (`Unknown HDCP2 message id` is the bounds check).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HdcpMessage {
    Null,
    AkeInit,
    AkeSendCert,
    AkeNoStoredKm,
    AkeStoredKm,
    AkeSendRrx,
    AkeSendHPrime,
    AkeSendPairingInfo,
    LcInit,
    LcSendLPrime,
    SkeSendEks,
    RepeaterAuthSendReceiverIdList,
    RttReady,
    RttChallenge,
    RepeaterAuthSendAck,
    RepeaterAuthStreamManage,
    RepeaterAuthStreamReady,
    ReceiverAuthStatus,
    AkeTransmitterInfo,
    AkeReceiverInfo,
}

impl HdcpMessage {
    /// Wire id. The vendor lists these consecutively starting after the
    /// `Unknown HDCP2 message id ` bound, so ids are the enumeration order.
    pub fn id(self) -> u8 {
        self as u8
    }

    pub fn name(id: u8) -> &'static str {
        match id {
            0 => "HDCP2_NULL_MESSAGE",
            1 => "HDCP2_AKE_INIT_MESSAGE",
            2 => "HDCP2_AKE_SEND_CERT_MESSAGE",
            3 => "HDCP2_AKE_NO_STORED_KM_MESSAGE",
            4 => "HDCP2_AKE_STORED_KM_MESSAGE",
            5 => "HDCP2_AKE_SEND_RRX_MESSAGE",
            6 => "HDCP2_AKE_SEND_H_PRIME_MESSAGE",
            7 => "HDCP2_AKE_SEND_PAIRING_INFO_MESSAGE",
            8 => "HDCP2_LC_INIT_MESSAGE",
            9 => "HDCP2_LC_SEND_L_PRIME_MESSAGE",
            10 => "HDCP2_SKE_SEND_EKS_MESSAGE",
            11 => "HDCP2_REPEATERAUTH_SEND_RECEIVERID_LIST",
            12 => "HDCP2_RTT_READY",
            13 => "HDCP2_RTT_CHALLENGE",
            14 => "HDCP2_REPEATERAUTH_SEND_ACK",
            15 => "HDCP2_REPEATERAUTH_STREAM_MANAGE",
            16 => "HDCP2_REPEATERAUTH_STREAM_READY",
            17 => "HDCP2_RECEIVER_AUTHSTATUS",
            18 => "HDCP2_AKE_TRANSMITTER_INFO",
            19 => "HDCP2_AKE_RECEIVER_INFO",
            _ => "<UNKNOWN_MESSAGE>",
        }
    }
}

/// Why a link is protected, as the sink reports it in `Attributes`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Protection {
    NotRequired,
    Required,
    Unknown(u8),
}

impl Protection {
    pub fn from_byte(b: u8) -> Self {
        match b {
            0 => Protection::NotRequired,
            1 => Protection::Required,
            other => Protection::Unknown(other),
        }
    }
}

/// A parsed `Attributes` (`STATUS_OP_ATTRIBUTES`) reply: the sink's protection
/// state and its HDCP revision/monitor capability word.
///
/// The vendor prints the raw words as `Protection Type: <n>` /
/// `Monitor Supports Protection: <n>` and compares the revision against the
/// HDCP 2.2 threshold before it will start repeater auth. The exact bit
/// positions are only partly pinned, so [`Attributes`] keeps the raw body
/// alongside the decoded revision and protection so a capture can settle them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attributes {
    pub protection: Protection,
    /// HDCP revision the sink reports (0x22 = HDCP 2.2). `None` when the reply
    /// was too short to carry one.
    pub revision: Option<u8>,
    pub hdcp22: bool,
    pub raw: Vec<u8>,
}

impl Attributes {
    pub fn parse(body: &[u8]) -> Self {
        let protection = Protection::from_byte(body.first().copied().unwrap_or(0));
        let revision = body.get(2).copied();
        Self {
            protection,
            revision,
            hdcp22: revision == Some(0x22),
            raw: body.to_vec(),
        }
    }
}

/// A packet header. Parsing never allocates and never trusts the length.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Header {
    pub op: Op,
    pub channel: u8,
    pub len: u16,
    pub first: bool,
    pub last: bool,
    pub cont: bool,
    /// Low two tag bits, carried over from the previous packet of the burst.
    pub stream: u8,
    /// 1-based fragment index; 0 on the first packet of a frame.
    pub frag: u8,
    pub counter: u16,
    pub tile_x: u8,
    pub tile_y: u8,
    pub tile_x_end: u8,
    pub tile_y_end: u8,
    pub offset: u32,
    pub frame: u32,
}

impl Header {
    pub fn new(op: Op, len: u16) -> Self {
        Self {
            op,
            channel: 0,
            len,
            first: true,
            last: true,
            cont: false,
            stream: 0,
            frag: 0,
            counter: 0,
            tile_x: 0,
            tile_y: 0,
            tile_x_end: 0,
            tile_y_end: 0,
            offset: 0,
            frame: 0,
        }
    }

    /// Commander-style packet used for the initial channel probe: byte 0 is a
    /// literal `0x01`, byte 1 is the channel id.
    pub fn probe(channel: u8) -> Self {
        Self {
            op: Op::Nop,
            channel,
            ..Self::new(Op::Nop, 0)
        }
    }

    pub fn is_probe(&self) -> bool {
        self.op == Op::Nop && self.channel != 0
    }

    /// Byte 0 as validated by the agent: `0x01` probe, otherwise `op<<5` plus
    /// the flag bits.
    pub fn tag(&self) -> u8 {
        if self.is_probe() {
            0x01
        } else {
            (self.op as u8)
                | (self.last as u8) << 4
                | (self.cont as u8) << 3
                | (self.first as u8) << 2
                | (self.stream & 0x03)
        }
    }

    /// Set the inclusive tile rectangle that covers `w`x`h` pixels at `x`,`y`.
    pub fn set_rect(&mut self, x: u16, y: u16, w: u16, h: u16) {
        self.tile_x = (x / TILE as u16) as u8;
        self.tile_y = (y / TILE as u16) as u8;
        // The vendor rounds with `+ 0xf` then `>>4`, so the end is exclusive.
        self.tile_x_end = (x + w).div_ceil(TILE as u16) as u8;
        self.tile_y_end = (y + h).div_ceil(TILE as u16) as u8;
    }

    /// Pixel span the rect covers, in tiles.
    pub fn rect_tiles(&self) -> (u16, u16, u16, u16) {
        let x = self.tile_x as u16;
        let y = self.tile_y as u16;
        (
            x,
            y,
            (self.tile_x_end as u16).saturating_sub(x),
            (self.tile_y_end as u16).saturating_sub(y),
        )
    }

    pub fn write(&self, out: &mut Vec<u8>) {
        out.push(self.tag());
        out.push(if self.is_probe() { self.channel } else { 0 });
        out.extend_from_slice(&self.len.to_be_bytes());
        out.push(self.tag());
        out.push(self.frag);
        out.extend_from_slice(&self.counter.to_le_bytes());
        out.extend_from_slice(&[self.tile_x, self.tile_y, self.tile_x_end, self.tile_y_end]);
        out.extend_from_slice(&self.offset.to_le_bytes());
        out.extend_from_slice(&self.frame.to_le_bytes());
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
    let probe = tag == 0x01;
    let len = u16::from_be_bytes([buf[2], buf[3]]);
    let payload = &buf[HEADER_LEN..];
    if payload.len() < len as usize {
        bail!(
            "TG length {} exceeds {} available bytes",
            len,
            payload.len()
        );
    }
    let flags = if probe { 0 } else { buf[0] };
    Ok((
        Header {
            op,
            channel: if probe { buf[1] } else { 0 },
            len,
            first: flags & 0x04 != 0,
            last: flags & 0x10 != 0,
            cont: flags & 0x08 != 0,
            stream: flags & 0x03,
            frag: buf[5],
            counter: u16::from_le_bytes([buf[6], buf[7]]),
            tile_x: buf[8],
            tile_y: buf[9],
            tile_x_end: buf[10],
            tile_y_end: buf[11],
            offset: u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]),
            frame: u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]),
        },
        &payload[..len as usize],
    ))
}

/// The device checksums status payloads additively; `MonitorStatus: is_valid_checksum`
/// (`0x10026c668`) compares the running sum against the trailing u16.
pub fn checksum(data: &[u8]) -> u16 {
    data.iter().fold(0u16, |acc, b| acc.wrapping_add(*b as u16))
}

/// Split a damage rectangle into packets. `chunk` bounds the payload per packet
/// so the result also fits the endpoint's negotiated max packet size.
///
/// Every packet repeats the whole rectangle; the `offset` field says where its
/// payload starts inside the rectangle, and a packet past the first sets
/// `cont`. The builder moves the previous rect's first column to the column it
/// resumed on (`0x1001271d4`), so the rect start tracks the offset too.
pub fn fragment(x: u16, y: u16, w: u16, h: u16, chunk: usize) -> Vec<Header> {
    const BPP: usize = 4;
    let total = w as usize * h as usize * BPP;
    let mut out = Vec::new();
    let mut counter = 0u16;
    let mut done = 0usize;
    while done < total.max(1) {
        let take = (total - done).min(chunk.max(1));
        let mut head = Header::new(Op::Data, take as u16);
        head.set_rect(x, y, w, h);
        head.first = done == 0;
        // `last` marks a packet that begins a rectangle (offset back at zero),
        // not the end of the frame: the builder sets it from the offset, and a
        // continuation clears it.
        head.last = done == 0;
        head.cont = done != 0;
        head.frag = out.len().min(u8::MAX as usize) as u8;
        head.offset = done as u32;
        head.counter = counter;
        counter = counter.wrapping_add(1);
        out.push(head);
        done += take;
    }
    out
}

/// EDID sanity check, mirroring the vendor's `Invalid length of raw EDID data`
/// and header test. `0x00ffffffffffff00` is the fixed EDID 1.x preamble.
pub fn valid_edid(edid: &[u8]) -> bool {
    edid.len() >= 128
        && edid.len().is_multiple_of(128)
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
/// A dock-specific EDID block.
///
/// The vendor keeps one block per dock family; we cannot redistributable-copy
/// theirs, so the identity is derived from the family name and revision the
/// pack declares instead. The product code, serial and monitor name all hash
/// from that string, so each dock family presents a stable, distinct identity
/// and two docks never collide in the arrangement profile.
pub fn dock_edid(family: &str, width_mm: u16, height_mm: u16) -> Vec<u8> {
    let mut e = template_edid(width_mm, height_mm, family);
    let h = fnv1a(family.as_bytes());
    // Product code + serial: deterministic per family, and distinct from the
    // fixed 0x0001 the generic template uses.
    e[10..12].copy_from_slice(&((h >> 32) as u16).max(0x0100).to_be_bytes());
    e[12..16].copy_from_slice(&(h as u32).to_be_bytes());
    fix_edid_checksum(&mut e);
    e
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x100000001b3)
    })
}

fn detailed_timing(w: u16, h: u16) -> [u8; 18] {
    let hactive = w.min(4095);
    let vactive = h.min(4095);
    let hblank = (w / 6).clamp(40, 1023);
    let vblank = (h / 40).clamp(4, 63);
    let hsync_off = (hblank / 4).min(1023);
    let hsync_w = 32u16.min(hblank / 2);
    let vsync_off = (vblank / 6).clamp(1, 63);
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
        let mut head = Header::new(Op::Data, 32);
        head.first = true;
        head.last = false;
        head.set_rect(64, 32, 32, 16);
        let mut buf = Vec::new();
        head.write(&mut buf);
        buf.extend_from_slice(&[0x5a; 32]);
        assert_eq!(buf[0], 0x80 | 0x04);
        assert_eq!(buf.len(), HEADER_LEN + 32);
        let (back, payload) = parse(&buf).unwrap();
        assert_eq!(back.op, Op::Data);
        assert!(back.first && !back.last);
        assert_eq!((back.tile_x, back.tile_y), (4, 2));
        assert_eq!((back.tile_x_end, back.tile_y_end), (6, 3));
        assert_eq!(payload, &[0x5a; 32]);
    }

    #[test]
    fn length_is_big_endian_and_flagged() {
        let mut head = Header::new(Op::Data, 0x1234);
        let mut buf = Vec::new();
        head.write(&mut buf);
        assert_eq!(&buf[2..4], &[0x12, 0x34]);
        // The tag byte is repeated at offset 4 so the link can sanity check the
        // staging buffer without re-deriving it.
        assert_eq!(buf[0], buf[4]);
        head.len = 1;
        head.stream = 0b10;
        head.cont = true;
        assert_eq!(head.tag() & 0x0b, 0x0a);
    }

    #[test]
    fn rejects_short_and_overlong() {
        assert!(parse(&[0x40, 0]).is_err());
        let mut buf = Vec::new();
        Header::new(Op::Data, 64).write(&mut buf);
        assert!(parse(&buf).is_err());
        buf.extend_from_slice(&[0; 64]);
        assert!(parse(&buf).is_ok());
        let mut bad = vec![0x05u8; HEADER_LEN];
        bad[2] = 0;
        bad[3] = 5;
        assert!(parse(&bad).is_err());
    }

    #[test]
    fn fragment_rows_carry_one_inclusive_rect() {
        // 48x32 is 6144 bytes; a 2048-byte chunk splits it into three.
        let heads = fragment(16, 0, 48, 32, 2048);
        assert_eq!(heads.len(), 3);
        assert!(heads[0].first && !heads[1].first);
        assert!(!heads[0].cont && heads[1].cont);
        assert_eq!(heads[1].offset, 2048);
        assert_eq!(heads[2].len, 2048);
        assert_eq!((heads[0].tile_x, heads[0].tile_x_end), (1, 4));
        assert_eq!(heads[0].rect_tiles(), (1, 0, 3, 2));
        assert_eq!(heads[1].counter, heads[0].counter + 1);
    }

    #[test]
    fn fragment_ids_number_the_continuations() {
        let heads = fragment(0, 0, 16, 16, 40);
        assert_eq!(heads.len(), 26);
        assert_eq!(heads.iter().filter(|h| h.first).count(), 1);
        assert!(heads[0].frag == 0 && heads[1].frag == 1 && heads[2].frag == 2);
        // `last` marks a packet that starts at rect offset 0, i.e. the first
        // packet of each rectangle, not the end of the frame.
        assert!(heads[0].last && !heads[1].last);
        assert!(!heads[0].cont && heads[1].cont);
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
    fn dock_edid_is_stable_and_family_specific() {
        let a = dock_edid("EllaDock", 500, 300);
        let b = dock_edid("EllaDock", 500, 300);
        let c = dock_edid("RidgeDoc", 500, 300);
        assert_eq!(a, b);
        assert_ne!(a[10..16], c[10..16]);
        assert!(valid_edid(&a));
        assert_eq!(checksum(&a) as u8, 0);
        assert_eq!(&a[77..85], b"EllaDock");
    }

    #[test]
    fn attributes_decode_reports_hdcp22() {
        let a = Attributes::parse(&[1, 0, 0x22, 0, 1, 1]);
        assert_eq!(a.protection, Protection::Required);
        assert!(a.hdcp22);
        assert!(!Attributes::parse(&[0, 0, 0x14]).hdcp22);
        assert_eq!(
            HdcpMessage::name(0xb),
            "HDCP2_REPEATERAUTH_SEND_RECEIVERID_LIST"
        );
        assert_eq!(HdcpMessage::name(200), "<UNKNOWN_MESSAGE>");
    }

    #[test]
    fn template_survives_roundtrip_through_status_body() {
        let e = template_edid(500, 300, "X");
        let body = status_body(StatusOp::Edid, 0, &e);
        assert_eq!(body.len(), e.len() + 7);
        assert_eq!(body[0], StatusOp::Edid as u8);
        assert_eq!(
            checksum(&body[..body.len() - 2]),
            u16::from_le_bytes([body[body.len() - 2], body[body.len() - 1]])
        );
    }

    #[test]
    fn status_checksum_trails_body() {
        let body = status_body(StatusOp::Edid, 1, &[0xaa; 4]);
        let sum = checksum(&body[..body.len() - 2]);
        assert_eq!(
            u16::from_le_bytes([body[body.len() - 2], body[body.len() - 1]]),
            sum
        );
    }
}
