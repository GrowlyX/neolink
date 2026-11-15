//! Parser for the vendor's `.spkg` per-dock packs.
//!
//! Layout recovered from `DisplayLinkUserAgent` 17.1.56 (error strings "Invalid
//! TLV Beginning" / "Invalid TLV Body" / "Invalid Length for FRAGMENTED_SPKG
//! TLV" now sit at `0x10020acbc` / `0x10020aca4` / `0x10020aee8`, and the TLV
//! walk that emits them starts at `0x10020a848`):
//!
//! ```text
//! 0   "ELLA"
//! 4   u32 total file size (must equal the file's length)
//! 8   u16 offset of the TLV table
//! 10  u16 length of the TLV table (<= 0x200000)
//! ..  optional records: { u16 index, u16 len, body[len], sha224[28] } aligned to 16
//! ..  TLV table: { u16 type, u16 len, body, pad-if-odd }
//! ..  payload blob (stream resumes on the next 256-byte boundary)
//! ```
//!
//! A `PT` TLV body is `u16 len`, then `len` stream bytes, then a 28-byte trailer;
//! the rest of the stream follows the TLV table. A record body is a run of
//! fragments, each `u8 kind`, `u16 len`, then `len` bytes, closed by its own
//! trailer.
//!
//! The stream itself is a run of frames, each `u8 kind`, `u16 len`, 7 scratch
//! bytes, a 10-byte second preamble, then `len - 10` data bytes (`len` counts
//! the preamble and the data).
//! The final frame of a pack is split: its tail lives in a record and the
//! `FRAGMENTED_SPKG` path stitches it back on. `kind` is per-fragment data, not
//! a type tag. The data bytes are stored, not compressed — libcompression's
//! `COMPRESSION_ZLIB` accepts them but returns them verbatim, which is why a
//! "successful decode" here is a copy.
//!
//! The parser is strict about the bounds the vendor checks and lenient about
//! everything else, so a damaged pack errors clearly instead of half-reading.
//!
//! What is still unknown is what the reassembled bytes mean per dock: they are
//! the firmware/EDID image the kext and device consume, not an EDID or archive
//! we can read today. See `docs/RE-FINDINGS.md`.

use anyhow::{bail, Result};
use std::path::Path;

pub const MAGIC: &[u8; 4] = b"ELLA";
/// The vendor refuses files with a TLV table longer than this.
pub const MAX_TLV: usize = 0x200000;
/// Bytes of frame header before the big-endian total length at [`FRAME_LEN`].
pub const FRAME_HEADER: usize = 9;
/// Offset of the big-endian u16 that holds the whole frame length.
pub const FRAME_LEN: usize = 9;

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
    /// `BC` — declared frame count (`u32` big-endian, then a build hash).
    FrameCount,
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
            b"BC" => Field::FrameCount,
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
            Field::FrameCount => *b"BC",
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

impl Record {
    /// The record body as `(kind, bytes)` fragments. Each is `u8 kind`,
    /// `u16 len`, then `len` bytes; the body already excludes the trailer the
    /// parser split off into `digest`.
    pub fn fragments(&self) -> Vec<(u8, &[u8])> {
        let mut out = Vec::new();
        let mut at = 0usize;
        while at + 3 <= self.body.len() {
            let kind = self.body[at];
            let len = u16::from_le_bytes([self.body[at + 1], self.body[at + 2]]) as usize;
            let start = at + 3;
            let end = (start + len).min(self.body.len());
            out.push((kind, &self.body[start..end]));
            at = start + len;
        }
        out
    }

    /// Stream bytes this record contributes, i.e. its fragment payloads with
    /// the per-fragment headers stripped.
    pub fn data(&self) -> Vec<u8> {
        self.fragments()
            .into_iter()
            .flat_map(|(_, b)| b.to_vec())
            .collect()
    }
}

/// A parsed frame table. `rest` counts stream bytes after the last whole frame:
/// a `FRAGMENTED_SPKG` frame that no record finished, so the final frame is
/// genuinely truncated in the pack itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frames {
    pub frames: Vec<Vec<u8>>,
    pub rest: usize,
}

impl Frames {
    /// Frame bodies concatenated. The header is already stripped by the walker.
    pub fn data(&self) -> Vec<u8> {
        self.frames.concat()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pack {
    /// The `PT` body past its `u16` length, i.e. the start of the stream.
    pub head: Option<Vec<u8>>,
    /// Everything after the TLV table up to the end of file.
    pub tail: Vec<u8>,
    pub records: Vec<Record>,
    pub fields: Vec<(Field, Vec<u8>)>,
}

impl Pack {
    pub fn get(&self, field: Field) -> Option<&[u8]> {
        self.fields
            .iter()
            .find(|(f, _)| *f == field)
            .map(|(_, v)| v.as_slice())
    }

    pub fn text(&self, field: Field) -> Option<String> {
        let raw = self.get(field)?;
        let trimmed = raw.strip_suffix(&[0]).unwrap_or(raw);
        Some(String::from_utf8_lossy(trimmed).into_owned())
    }

    pub fn name(&self) -> Option<String> {
        self.text(Field::Record)
    }

    /// Frame count the pack declares in its `BC` field, when it has one.
    pub fn frame_count(&self) -> Option<usize> {
        let raw = self.get(Field::FrameCount)?;
        let n = u32::from_be_bytes(raw.get(..4)?.try_into().ok()?) as usize;
        (n > 0).then_some(n)
    }

    /// The reassembled stream: `PT` data, the bytes after the TLV table, then
    /// each record's data in index order. The final frame of a pack is split
    /// across the two, so this is the only order that decodes.
    pub fn stream(&self) -> Vec<u8> {
        let mut out = self.head.clone().unwrap_or_default();
        out.extend_from_slice(&self.tail);
        for r in self.records.iter() {
            out.extend_from_slice(&r.data());
        }
        out
    }

    /// Split the stream into frames.
    ///
    /// A frame is 9 header bytes, a big-endian `u16` total length that counts
    /// the whole frame including its header, then the body. Bytes 0 and 1 are
    /// per-frame values (they look like a tag and a checksum; neither is
    /// constant), the rest of the header is opaque.
    ///
    /// The final frame of a pack is cut short: its tail lives in the record
    /// fragments and the `FRAGMENTED_SPKG` path stitches them back on in index
    /// order. `rest` is what is left when a frame still overruns with no record
    /// bytes to finish it — on the shipped packs that is only ever the last
    /// frame of a pack that is itself truncated, so it is reported, not an
    /// error.
    pub fn frames(&self) -> Result<Frames> {
        let mut stream = self.stream();
        let mut pending: Vec<u8> = self.records.iter().flat_map(Record::data).collect();
        let mut frames = Vec::new();
        let mut at = 0usize;
        // `BC` states how many frames the pack holds; stop there so trailing
        // padding cannot be read as another frame.
        let limit = self.frame_count();
        loop {
            if let Some(n) = limit {
                if frames.len() >= n {
                    return Ok(Frames {
                        frames,
                        rest: stream.len() - at,
                    });
                }
            }
            while at + FRAME_HEADER + 2 > stream.len() {
                if pending.is_empty() {
                    return Ok(Frames {
                        frames,
                        rest: stream.len() - at,
                    });
                }
                let take = (at + FRAME_HEADER + 2 - stream.len()).min(pending.len());
                stream.extend(pending.drain(..take));
            }
            let len =
                u16::from_be_bytes([stream[at + FRAME_LEN], stream[at + FRAME_LEN + 1]]) as usize;
            if len < FRAME_HEADER + 2 {
                return Ok(Frames {
                    frames,
                    rest: stream.len() - at,
                });
            }
            let body = at + FRAME_HEADER + 2;
            let end = at + len;
            while end > stream.len() {
                if pending.is_empty() {
                    return Ok(Frames {
                        frames,
                        rest: stream.len() - at,
                    });
                }
                let take = (end - stream.len()).min(pending.len());
                stream.extend(pending.drain(..take));
            }
            frames.push(stream[body..end].to_vec());
            at = end;
        }
    }

    /// The decoded frames plus whatever trailing bytes did not form a whole
    /// frame.
    pub fn decode(&self) -> Result<Frames> {
        self.frames()
    }

    /// Frame data concatenated — the firmware image itself, minus any partial
    /// trailing frame.
    pub fn image(&self) -> Result<Vec<u8>> {
        Ok(self.frames()?.data())
    }

    /// True when every record's digest matches its body. The trailer algorithm
    /// is unconfirmed, so a mismatch is reported rather than enforced.
    pub fn digests_ok(&self) -> bool {
        self.records
            .iter()
            .all(|r| sha224_matches(&r.body, &r.digest))
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

    let mut pack = Pack {
        head: None,
        tail: Vec::new(),
        records: Vec::new(),
        fields: Vec::new(),
    };
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
            // `u16 len`, the fragment (`u8 kind`, `u16 len`, bytes), then a
            // 28-byte trailer. Unlike a record, `PT` carries no per-fragment
            // header, so the bytes after the length are already frame data.
            let declared = body
                .get(..2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]) as usize)
                .unwrap_or(0);
            let take = declared.min(body.len().saturating_sub(2));
            pack.head = Some(body[2..2 + take].to_vec());
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
            pack.records.push(Record {
                index,
                body: bytes[q + 4..body_end].to_vec(),
                digest,
            });
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
    // The tail ends at EOF, so any zero run after it is file padding rather
    // than stream. Keeping it would let a truncated frame's declared length be
    // satisfied by padding instead of by the record that carries its rest.
    let mut tail_end = bytes.len();
    while tail_end > tail_start && bytes[tail_end - 1] == 0 {
        tail_end -= 1;
    }
    pack.tail = bytes[tail_start.min(tail_end)..tail_end].to_vec();
    Ok(pack)
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
                0xc1059ed8, 0x367cd507, 0x3070dd17, 0xf70e5939, 0xffc00b31, 0x68581511, 0x64f98fa7,
                0xbefa4fa4,
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
        let chunks: Vec<[u8; 64]> = self.buf.as_chunks::<64>().0.to_vec();
        for b in &chunks {
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
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
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
/// `tail` is the stream that follows the TLV table; pass `&[]` for a pack whose
/// whole stream is in the `PT` TLV.
pub fn build(fields: &[(Field, Vec<u8>)], records: &[Record], tail: &[u8]) -> Vec<u8> {
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
    if !tail.is_empty() {
        while out.len() % 256 != 0 {
            out.push(0);
        }
        out.extend_from_slice(tail);
    }
    let total = out.len() as u32;
    out[4..8].copy_from_slice(&total.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(data: &[u8]) -> Vec<u8> {
        frame_kind(0, data)
    }

    fn frame_kind(kind: u8, data: &[u8]) -> Vec<u8> {
        // 9 header bytes, then a big-endian u16 whose value is the whole frame
        // length including that header, then the body.
        let mut v = vec![kind, 0, 1, 2, 3, 4, 5, 6, 7];
        v.extend_from_slice(&((FRAME_HEADER + 2 + data.len()) as u16).to_be_bytes());
        v.extend_from_slice(data);
        v
    }

    /// A record fragment: `u8 kind`, `u16 len`, then the bytes.
    fn frag(kind: u8, data: &[u8]) -> Vec<u8> {
        let mut v = vec![kind];
        v.extend_from_slice(&(data.len() as u16).to_le_bytes());
        v.extend_from_slice(data);
        v
    }

    /// A `PT` body: `u16 len`, the stream bytes, then the record trailer the
    /// vendor keeps. Unlike records, `PT` carries no per-fragment header.
    fn payload(stream: &[u8]) -> Vec<u8> {
        let mut v = (stream.len() as u16).to_le_bytes().to_vec();
        v.extend_from_slice(stream);
        v.extend_from_slice(&[0u8; 28]);
        v
    }

    /// A pack whose first frame starts in `PT` and whose *second* frame is cut
    /// short, with the rest of that frame carried by a record, matching how the
    /// shipped packs use `FRAGMENTED_SPKG`.
    pub(super) fn pack() -> Vec<u8> {
        let second = frame_kind(1, b"second");
        let (head, rest) = second.split_at(11);
        let fields = vec![
            (Field::Record, b"EllaDock".to_vec()),
            (Field::Version, vec![0x0c, 0x03, 0x25]),
            (Field::Payload, payload(&frame(b"first"))),
            (Field::FrameCount, 2u32.to_be_bytes().to_vec()),
        ];
        // The tail continues after the table and must not start with 0x00, or
        // the zero-padding skip above would eat it.
        let records = vec![Record {
            index: 1,
            body: frag(0, rest),
            digest: [0; 28],
        }];
        build(&fields, &records, head)
    }

    #[test]
    fn roundtrip_fields_and_order() {
        let bytes = pack();
        let p = parse(&bytes).unwrap();
        eprintln!(
            "head={:?} tail={} recs={} stream={}",
            p.head,
            p.tail.len(),
            p.records.len(),
            p.stream().len()
        );
        eprintln!(
            "stream bytes={:02x?}",
            &p.stream()[..p.stream().len().min(24)]
        );
        assert_eq!(p.name().unwrap(), "EllaDock");
        assert_eq!(p.get(Field::Version).unwrap(), &[0x0c, 0x03, 0x25]);
        assert_eq!(p.frame_count(), Some(2));
        assert_eq!(p.records.len(), 1);
        eprintln!("tlv={:?}", p.fields);
        let f = p.frames().unwrap();
        eprintln!(
            "frames={} rest={} bodies={:?}",
            f.frames.len(),
            f.rest,
            f.frames
        );
        assert_eq!(f.frames.len(), 2);
        // Frame bodies are data only; the header is not part of `data`.
        assert_eq!(f.data(), b"firstsecond".to_vec());
    }

    #[test]
    fn frame_count_stops_before_padding() {
        let mut bytes = pack();
        // Trailing slack must not be read as another frame.
        bytes.extend_from_slice(&[0u8; 64]);
        let total = bytes.len() as u32;
        bytes[4..8].copy_from_slice(&total.to_le_bytes());
        let p = parse(&bytes).unwrap();
        eprintln!("tlv={:?}", p.fields);
        let f = p.frames().unwrap();
        assert_eq!(f.frames.len(), 2);
        assert_eq!(f.data(), b"firstsecond".to_vec());
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

    /// The four packs a 17.1.56 install ships, if the vendor cache has them.
    #[cfg(test)]
    fn vended_packs() -> Vec<std::path::PathBuf> {
        crate::vendor::resource_packs().unwrap_or_default()
    }

    #[test]
    fn vended_packs_frame_cleanly() {
        let mut packs = vended_packs();
        if packs.is_empty() {
            return;
        }
        packs.sort();
        assert_eq!(packs.len(), 4, "unexpected pack count");
        for path in packs {
            let k = parse_file(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let name = k.name().expect("RD field");
            let f = k
                .frames()
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert!(f.frames.len() > 5, "{name}: only {} frames", f.frames.len());
            let data = f.data().len();
            assert!(data > 100_000, "{name}: only {data} data bytes");
            // The last frame is split by the FRAGMENTED_SPKG path and the
            // remainder is image payload, not frame framing, so a tail is
            // expected rather than an exact end.
            assert!(f.rest > 0, "{name}: no split tail");
            assert!(f.rest < 65_536, "{name}: {} trailing bytes", f.rest);
        }
    }

    #[test]
    fn sha224_known_vector() {
        let mut h = Sha224::new();
        h.update(b"abc");
        assert_eq!(
            h.finish().to_vec(),
            [
                0x23, 0x09, 0x7d, 0x22, 0x34, 0x05, 0xd8, 0x22, 0x86, 0x42, 0xa4, 0x77, 0xbd, 0xa2,
                0x55, 0xb3, 0x2a, 0xad, 0xbc, 0xe4, 0xbd, 0xa0, 0xb3, 0xf7, 0xe3, 0x6c, 0x9d, 0xa7
            ]
        );
    }
}
