//! Parser for the vendor's `.spkg` per-dock packs.
//!
//! Layout recovered from `DisplayLinkUserAgent` 17.1.56 (`0x10020a740`, error
//! strings "Invalid TLV Beginning" / "Invalid TLV Body" / "Invalid Length for
//! FRAGMENTED_SPKG TLV"):
//!
//! ```text
//! 0  "ELLA"
//! 4  u32 total file size (must equal the file's length)
//! 8  u16 offset of the TLV table
//! 10 u16 length of the TLV table
//! .. optional records: { u16 index, u16 len, body[len], sha224[28] } aligned to 16
//! .. TLV table: { u16 type, u16 len, body, pad-if-odd }
//! .. payload blob
//! ```
//!
//! A `PT` TLV carries a 5-byte preamble then the compressed payload stream,
//! which runs from there to the end of the file (zero-padding between the TLV
//! table and the stream is skipped). Records before and after the table are a
//! best-effort index of the same bytes: they are not needed to recover the
//! payload and are not in index order.
//!
//! The parser is strict about the bounds the vendor checks and lenient about
//! everything else, so a damaged pack errors clearly instead of half-reading.
//!
//! Note: this is a *container* parser. Turning the payload into an EDID or a
//! firmware image requires the per-dock format on top of it, which is separate
//! work; see `docs/RE-FINDINGS.md`.

use anyhow::{bail, Result};
use std::path::Path;

pub const MAGIC: &[u8; 4] = b"ELLA";
/// The vendor refuses files with a TLV table longer than this.
pub const MAX_TLV: usize = 0x200000;
/// Bytes between the `PT` TLV body's own length field and the payload. The
/// stream itself then starts with one or two framing bytes that decoders differ
/// on, so [`Pack::payload_candidates`] hands out every plausible split.
pub const PAYLOAD_PREAMBLE: usize = 2;
/// How many leading bytes a decoder may need to skip inside the stream.
pub const SKIP_GUESSES: usize = 4;

/// Metadata TLVs the agent recognises.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Field {
    /// `RD` — product/record name.
    Record,
    /// `OW` — owner string.
    Owner,
    /// `RE` — revision id, 8 or 9 bytes.
    Revision,
    /// `RL` — release level, exactly 1 byte.
    ReleaseLevel,
    /// `VE` — version triple.
    Version,
    /// `DA` — build date.
    Date,
    /// `TI` — build time.
    Time,
    /// `BD` — body/bundle index.
    Body,
    /// `PT` — payload.
    Payload,
    Unknown([u8; 2]),
}

impl Field {
    fn from_tag(tag: [u8; 2]) -> Self {
        match &tag {
            b"RD" => Field::Record,
            b"OW" => Field::Owner,
            b"RE" => Field::Revision,
            b"RL" => Field::ReleaseLevel,
            b"VE" => Field::Version,
            b"DA" => Field::Date,
            b"TI" => Field::Time,
            b"BD" => Field::Body,
            b"PT" => Field::Payload,
            _ => Field::Unknown(tag),
        }
    }

    pub fn tag(self) -> [u8; 2] {
        match self {
            Field::Record => *b"RD",
            Field::Owner => *b"OW",
            Field::Revision => *b"RE",
            Field::ReleaseLevel => *b"RL",
            Field::Version => *b"VE",
            Field::Date => *b"DA",
            Field::Time => *b"TI",
            Field::Body => *b"BD",
            Field::Payload => *b"PT",
            Field::Unknown(t) => t,
        }
    }
}

/// One fragment of the payload stream.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub index: u16,
    pub body: Vec<u8>,
    /// 28-byte trailer the vendor keeps per record (sha224-shaped).
    pub digest: [u8; 28],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pack {
    /// `PT` body past its 5-byte preamble, i.e. the start of the stream.
    pub head: Option<Vec<u8>>,
    /// Everything after the TLV table up to the end of file.
    pub tail: Vec<u8>,
    pub records: Vec<Record>,
    pub fields: Vec<(Field, Vec<u8>)>,
}

impl Pack {
    pub fn get(&self, field: Field) -> Option<&[u8]> {
        self.fields.iter().find(|(f, _)| *f == field).map(|(_, v)| v.as_slice())
    }

    pub fn text(&self, field: Field) -> Option<String> {
        let raw = self.get(field)?;
        let trimmed = raw.strip_suffix(&[0]).unwrap_or(raw);
        Some(String::from_utf8_lossy(trimmed).into_owned())
    }

    pub fn name(&self) -> Option<String> {
        self.text(Field::Record)
    }

    /// The raw compressed stream: `PT` body past its length field, then the
    /// region after the TLV table. Records are not consulted; they describe the
    /// same bytes.
    pub fn payload(&self) -> Vec<u8> {
        let mut out = self.head.clone().unwrap_or_default();
        out.extend_from_slice(&self.tail);
        out
    }

    /// `payload()` minus 0..[`SKIP_GUESSES`] leading bytes, in the order a
    /// decoder should try them. The vendor's own decoder is lenient about the
    /// first byte; `zlib` is not.
    pub fn payload_candidates(&self) -> Vec<Vec<u8>> {
        let p = self.payload();
        (0..SKIP_GUESSES)
            .filter(|n| *n < p.len())
            .map(|n| p[n..].to_vec())
            .collect()
    }

    /// Decode the payload with whichever candidate the decoder accepts.
    pub fn decode(&self) -> Result<Vec<u8>> {
        for c in self.payload_candidates() {
            if let Ok(out) = inflate(&c) {
                return Ok(out);
            }
        }
        bail!("payload did not inflate")
    }

    /// True when every record's digest matches its body. The trailer algorithm
    /// is unconfirmed, so a mismatch is reported rather than enforced.
    pub fn digests_ok(&self) -> bool {
        self.records.iter().all(|r| sha224_matches(&r.body, &r.digest))
    }
}

pub fn parse(bytes: &[u8]) -> Result<Pack> {
    if bytes.len() < 12 {
        bail!("file too small: {} bytes", bytes.len());
    }
    if &bytes[..4] != MAGIC {
        bail!("not an ELLA pack");
    }
    let total = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    if total != bytes.len() {
        bail!("size mismatch: header {total}, file {}", bytes.len());
    }
    if bytes.len() < 0xb {
        bail!("invalid TLV beginning");
    }
    let tlv_off = u16::from_le_bytes([bytes[8], bytes[9]]) as usize;
    let tlv_len = u16::from_le_bytes([bytes[10], bytes[11]]) as usize;
    if tlv_len > MAX_TLV {
        bail!("invalid length for FRAGMENTED_SPKG TLV: {tlv_len}");
    }
    let tlv_end = tlv_off.checked_add(tlv_len).filter(|e| *e <= bytes.len());
    let tlv_end = tlv_end.ok_or_else(|| anyhow::anyhow!("TLV table runs past end of file"))?;

    let mut pack = Pack { head: None, tail: Vec::new(), records: Vec::new(), fields: Vec::new() };
    let mut p = tlv_off;
    while p + 4 <= tlv_end {
        let tag = [bytes[p], bytes[p + 1]];
        let len = u16::from_le_bytes([bytes[p + 2], bytes[p + 3]]) as usize;
        let body_start = p + 4;
        let body_end = match body_start.checked_add(len).filter(|e| *e <= tlv_end) {
            Some(e) => e,
            None => bail!("invalid TLV body at {:#x}", p),
        };
        let body = bytes[body_start..body_end].to_vec();
        let field = Field::from_tag(tag);
        if field == Field::Payload {
            // The body's own u16 length precedes the stream.
            pack.head = Some(body.get(PAYLOAD_PREAMBLE..).unwrap_or(&[]).to_vec());
        } else {
            pack.fields.push((field, body));
        }
        p = body_end + (len & 1);
    }

    // Records live before the table and after it, on 16-byte boundaries. The
    // region between the table end and the first record is zero padding.
    let scan = |from: usize, to: usize, pack: &mut Pack| -> Result<()> {
        let mut q = from;
        while q + 4 + 28 <= to {
            let index = u16::from_le_bytes([bytes[q], bytes[q + 1]]);
            let len = u16::from_le_bytes([bytes[q + 2], bytes[q + 3]]) as usize;
            if len == 0 {
                q = align16(q + 16);
                continue;
            }
            let body_end = q + 4 + len;
            if body_end + 28 > to {
                break;
            }
            let mut digest = [0u8; 28];
            digest.copy_from_slice(&bytes[body_end..body_end + 28]);
            pack.records.push(Record { index, body: bytes[q + 4..body_end].to_vec(), digest });
            q = align16(body_end + 28);
        }
        Ok(())
    };
    scan(12, tlv_off, &mut pack)?;

    // The stream resumes on the next 256-byte boundary after the table.
    let mut tail_start = align256(tlv_end);
    while tail_start < bytes.len() && bytes[tail_start] == 0 {
        tail_start += 1;
    }
    pack.tail = bytes[tail_start.min(bytes.len())..].to_vec();
    Ok(pack)
}

/// Inflate a candidate stream (zlib, or raw deflate). Used by the dump tool and
/// by anything that wants to decode the payload once the head is in hand.
pub fn inflate(data: &[u8]) -> Result<Vec<u8>> {
    use flate2::read::ZlibDecoder;
    use std::io::Read;
    for raw in [false, true] {
        let mut out = Vec::new();
        let ok = if raw {
            flate2::read::DeflateDecoder::new(data).read_to_end(&mut out).is_ok()
        } else {
            ZlibDecoder::new(data).read_to_end(&mut out).is_ok()
        };
        if ok && !out.is_empty() {
            return Ok(out);
        }
    }
    // The vendor's encoder emits something zlib's strict reader rejects but
    // libcompression accepts (`COMPRESSION_ZLIB` is lenient about the head).
    apple_inflate(data).ok_or_else(|| anyhow::anyhow!("payload did not inflate"))
}

#[link(name = "compression")]
extern "C" {
    fn compression_decode_buffer(
        dst: *mut u8,
        dst_size: usize,
        src: *const u8,
        src_size: usize,
        scratch: *mut std::ffi::c_void,
        algorithm: i32,
    ) -> usize;
}

const COMPRESSION_ZLIB: i32 = 0x205;
/// Cap for one decoded pack; the largest vendor pack decodes well under this.
const MAX_INFLATED: usize = 8 << 20;

fn apple_inflate(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = vec![0u8; MAX_INFLATED];
    let n = unsafe {
        compression_decode_buffer(
            out.as_mut_ptr(),
            out.len(),
            data.as_ptr(),
            data.len(),
            std::ptr::null_mut(),
            COMPRESSION_ZLIB,
        )
    };
    if n == 0 || n >= MAX_INFLATED {
        return None;
    }
    out.truncate(n);
    Some(out)
}

fn align16(n: usize) -> usize {
    (n + 15) & !15
}

fn align256(n: usize) -> usize {
    (n + 255) & !255
}

/// Digest comparison for the record trailers. Unconfirmed algorithm; a
/// best-effort SHA-224 of the body is the closest match found.
fn sha224_matches(body: &[u8], want: &[u8; 28]) -> bool {
    let mut h = Sha224::new();
    h.update(body);
    h.finish() == *want
}

pub fn parse_file(path: &Path) -> Result<Pack> {
    let bytes = std::fs::read(path)?;
    parse(&bytes)
}

/// Minimal SHA-224, only so the trailer check needs no extra crate.
struct Sha224 {
    state: [u32; 8],
    buf: Vec<u8>,
    len: u64,
}

impl Sha224 {
    fn new() -> Self {
        Self {
            state: [
                0xc1059ed8, 0x367cd507, 0x3070dd17, 0xf70e5939, 0xffc00b31, 0x68581511,
                0x64f98fa7, 0xbefa4fa4,
            ],
            buf: Vec::new(),
            len: 0,
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.len = self.len.wrapping_add(data.len() as u64);
        if !self.buf.is_empty() {
            let take = (64 - self.buf.len()).min(data.len());
            self.buf.extend_from_slice(&data[..take]);
            data = &data[take..];
            if self.buf.len() == 64 {
                let block: [u8; 64] = self.buf[..].try_into().unwrap();
                self.compress(&block);
                self.buf.clear();
            }
        }
        while data.len() >= 64 {
            let block: [u8; 64] = data[..64].try_into().unwrap();
            self.compress(&block);
            data = &data[64..];
        }
        self.buf.extend_from_slice(data);
    }

    fn finish(mut self) -> [u8; 28] {
        let bits = self.len.wrapping_mul(8);
        self.buf.push(0x80);
        while self.buf.len() % 64 != 56 {
            self.buf.push(0);
        }
        self.buf.extend_from_slice(&bits.to_be_bytes());
        let blocks: Vec<[u8; 64]> = self
            .buf
            .chunks_exact(64)
            .map(|c| c.try_into().unwrap())
            .collect();
        for b in &blocks {
            self.compress(b);
        }
        let mut out = [0u8; 28];
        for (i, word) in self.state[..7].iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (s, v) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *s = s.wrapping_add(v);
        }
    }
}

/// Build a pack from fields, so a caller can round-trip a synthesised record.
pub fn build(fields: &[(Field, Vec<u8>)], records: &[Record]) -> Vec<u8> {
    let mut table = Vec::new();
    for (f, body) in fields {
        table.extend_from_slice(&f.tag());
        table.extend_from_slice(&(body.len() as u16).to_le_bytes());
        table.extend_from_slice(body);
        if body.len() % 2 == 1 {
            table.push(0);
        }
    }
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(table.len() as u16).to_le_bytes());
    for r in records {
        out.extend_from_slice(&r.index.to_le_bytes());
        out.extend_from_slice(&(r.body.len() as u16).to_le_bytes());
        out.extend_from_slice(&r.body);
        out.extend_from_slice(&r.digest);
        while out.len() % 16 != 0 {
            out.push(0);
        }
    }
    let tlv_off = out.len();
    out[8..10].copy_from_slice(&(tlv_off as u16).to_le_bytes());
    out.extend_from_slice(&table);
    let total = out.len() as u32;
    out[4..8].copy_from_slice(&total.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack() -> Vec<u8> {
        let fields = vec![
            (Field::Record, b"EllaDock".to_vec()),
            (Field::Version, vec![0x0c, 0x03, 0x25]),
            (Field::Payload, vec![4, 0, 0xde, 0xad, 0xbe, 0xef]),
        ];
        let records = vec![
            Record { index: 2, body: b"second".to_vec(), digest: [0; 28] },
            Record { index: 1, body: b"first".to_vec(), digest: [0; 28] },
        ];
        build(&fields, &records)
    }

    #[test]
    fn roundtrip_fields_and_order() {
        let bytes = pack();
        let p = parse(&bytes).unwrap();
        assert_eq!(p.name().unwrap(), "EllaDock");
        assert_eq!(p.get(Field::Version).unwrap(), &[0x0c, 0x03, 0x25]);
        assert_eq!(p.head.as_deref(), Some(&b"\xde\xad\xbe\xef"[..]));
        assert!(p.tail.is_empty());
        assert_eq!(p.records.len(), 2);
        assert_eq!(p.payload_candidates()[0], b"\xde\xad\xbe\xef");
    }

    #[test]
    fn rejects_bad_magic_and_size() {
        let mut bytes = pack();
        bytes[0] = b'X';
        assert!(parse(&bytes).is_err());
        let mut bytes = pack();
        bytes[4] = bytes[4].wrapping_add(1);
        assert!(parse(&bytes).is_err());
    }

    #[test]
    fn rejects_tlv_past_end() {
        let mut bytes = pack();
        let off = bytes.len() as u16 + 4;
        bytes[8..10].copy_from_slice(&off.to_le_bytes());
        assert!(parse(&bytes).is_err());
    }

    #[test]
    fn rejects_short_file() {
        assert!(parse(b"ELLA").is_err());
        assert!(parse(&[]).is_err());
    }

    #[test]
    fn sha224_known_vector() {
        let mut h = Sha224::new();
        h.update(b"abc");
        assert_eq!(
            h.finish().to_vec(),
            [
                0x23, 0x09, 0x7d, 0x22, 0x34, 0x05, 0xd8, 0x22, 0x86, 0x42, 0xa4, 0x77, 0xbd,
                0xa2, 0x55, 0xb3, 0x2a, 0xad, 0xbc, 0xe4, 0xbd, 0xa0, 0xb3, 0xf7, 0xe3, 0x6c,
                0x9d, 0xa7
            ]
        );
    }
}
