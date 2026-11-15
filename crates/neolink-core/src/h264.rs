//! H.264 encoding for the DL3 transport, through VideoToolbox.
//!
//! Raw BGRA costs four bytes a pixel. The vendor gets around that with a GPU
//! encoder; this module is the same idea via the hardware H.264 encoder, which
//! needs no Metal shader port.
//!
//! What the *transport* can and cannot express is the important part, and it
//! was settled from the disassembly rather than guessed:
//!
//! * The 20-byte packet header (`0x1001270bc`, `0x1001271d4`) has no spare
//!   field for a format or codec. Every byte is tag, channel, big-endian
//!   length, tag again, fragment index, counter, four tile-rect bytes, a
//!   rectangle offset and the frame tag. So a compressed payload is not marked
//!   on the wire at all; the receiver must be in a matching mode out of band.
//! * The tag's low two bits are the vendor's `stream` nibble, carried over
//!   between packets of a burst (`bfxil w8, w9, #0, #2`) rather than derived
//!   from the payload, so they are not a codec selector either.
//!
//! Because of that the encoded path is opt-in ([`Codec::H264`]) and raw stays
//! the default. Each damage rectangle is encoded on its own and sent
//! intra-only (every frame a keyframe), which matches a sender that ships
//! rectangles rather than whole frames.
//!
//! VideoToolbox only accepts even coded dimensions. Damage rects are often odd
//! (a clipped edge, a single changed pixel), so [`Encoder`] pads one pixel and
//! repeats the edge instead of refusing the rect. The packet still carries the
//! true rect, so the receiver never reads the padding. This is the one place
//! where the coded size and the rect size differ and it is deliberate.

use anyhow::{bail, Result};
use std::ffi::c_void;
use std::ptr;

extern "C" {
    fn nl_enc_new(width: i32, height: i32, src_width: i32, src_height: i32) -> *mut c_void;
    fn nl_enc_frame(
        handle: *mut c_void,
        bgra: *const u8,
        stride: usize,
        out: *mut *mut u8,
        out_len: *mut usize,
        key: *mut i32,
    ) -> i32;
    fn nl_enc_free(handle: *mut c_void);
    fn nl_dec_new(width: i32, height: i32) -> *mut c_void;
    fn nl_dec_frame(handle: *mut c_void, annexb: *const u8, len: usize) -> i32;
    fn nl_dec_free(handle: *mut c_void);
    fn nl_dec_size(handle: *mut c_void, w: *mut i32, h: *mut i32);
    fn nl_buf_free(bytes: *mut c_void);
}

/// Which payload format goes on the wire.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Codec {
    /// Tight-packed BGRA. The default, because it is the format the header
    /// layout is proven against.
    #[default]
    Raw,
    /// H.264 Annex-B per damage rectangle.
    H264,
}

impl Codec {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "raw" | "bgra" => Some(Codec::Raw),
            "h264" | "avc" => Some(Codec::H264),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Codec::Raw => "raw",
            Codec::H264 => "h264",
        }
    }
}

/// One encoded rectangle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    pub bytes: Vec<u8>,
    /// True when the bitstream starts with SPS/PPS.
    pub keyframe: bool,
}

/// A live hardware encoder for one rectangle size.
///
/// VideoToolbox requires even coded dimensions, so an odd damage rect is
/// encoded one pixel wider or taller with its edge pixel repeated. [`size`]
/// returns the rectangle the caller asked for, not the padded coded size; the
/// coded size is what the receiver sees and it does not read the extra
/// column/row because the rect stays odd.
///
/// [`size`]: Encoder::size
pub struct Encoder {
    handle: *mut c_void,
    width: u16,
    height: u16,
    coded_width: u16,
    coded_height: u16,
    last_used: u64,
}

impl Encoder {
    /// Open an encoder for a rectangle size. Hardware is requested, but
    /// VideoToolbox may still fall back to software; either way the output is
    /// the same Annex-B stream.
    pub fn new(width: u16, height: u16) -> Result<Self> {
        if width == 0 || height == 0 {
            bail!("cannot encode a {width}x{height} rectangle");
        }
        let coded_width = width + (width & 1);
        let coded_height = height + (height & 1);
        let handle = unsafe {
            nl_enc_new(
                coded_width as i32,
                coded_height as i32,
                width as i32,
                height as i32,
            )
        };
        if handle.is_null() {
            bail!("VideoToolbox refused an H.264 encoder for {width}x{height}");
        }
        Ok(Self {
            handle,
            width,
            height,
            coded_width,
            coded_height,
            last_used: 0,
        })
    }

    /// The rectangle this encoder was opened for.
    pub fn size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    /// The padded size actually handed to VideoToolbox, which is never odd.
    pub fn coded_size(&self) -> (u16, u16) {
        (self.coded_width, self.coded_height)
    }

    /// Identity of the underlying VideoToolbox session, for tests that check a
    /// same-size rect reuses it instead of reopening.
    pub fn handle_id(&self) -> usize {
        self.handle as usize
    }

    /// Encode tightly packed BGRA, `width * 4` bytes per row.
    pub fn encode(&mut self, bgra: &[u8]) -> Result<Frame> {
        let stride = self.width as usize * 4;
        if bgra.len() < stride * self.height as usize {
            bail!(
                "expected {} bytes of BGRA, got {}",
                stride * self.height as usize,
                bgra.len()
            );
        }
        let mut out: *mut u8 = ptr::null_mut();
        let mut len = 0usize;

        let mut key = 0i32;
        let ok = unsafe {
            nl_enc_frame(
                self.handle,
                bgra.as_ptr(),
                stride,
                &mut out,
                &mut len,
                &mut key,
            )
        };
        if ok == 0 || out.is_null() || len == 0 {
            bail!("H.264 encode failed");
        }
        let bytes = unsafe { std::slice::from_raw_parts(out, len).to_vec() };
        unsafe { nl_buf_free(out.cast()) };
        if key == 0 {
            bail!("H.264 encoder produced a non-keyframe for a standalone rect");
        }
        Ok(Frame {
            bytes,
            keyframe: true,
        })
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe { nl_enc_free(self.handle) };
    }
}

/// A few encoders kept open, keyed by rectangle size.
///
/// A damage tracker hands back rects of a different size nearly every frame, and
/// a VideoToolbox session is fixed to one size, so a single encoder would be
/// torn down and rebuilt constantly. Sessions are also the expensive part of
/// encoding — far more than a frame — so the pool keeps the recently used sizes
/// warm and retires the least recently used one past [`EncoderPool::CAP`].
#[derive(Default)]
pub struct EncoderPool {
    open: Vec<Encoder>,
    clock: u64,
}

impl EncoderPool {
    /// Enough for the rect sizes a frame mixes in practice (a run plus a few
    /// partial rows) without pinning an unbounded number of GPU sessions.
    pub const CAP: usize = 6;

    /// The encoder for `width`x`height`, opened if it is not already pooled.
    pub fn get(&mut self, width: u16, height: u16) -> Result<&mut Encoder> {
        self.clock += 1;
        let stamp = self.clock;
        let at = match self.open.iter().position(|e| e.size() == (width, height)) {
            Some(at) => at,
            None => {
                if self.open.len() >= Self::CAP {
                    let (oldest, _) = self
                        .open
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, e)| e.last_used)
                        .expect("the pool is non-empty");
                    self.open.swap_remove(oldest);
                }
                self.open.push(Encoder::new(width, height)?);
                self.open.len() - 1
            }
        };
        let enc = &mut self.open[at];
        enc.last_used = stamp;
        Ok(enc)
    }

    /// Sizes currently holding an open session.
    pub fn sizes(&self) -> Vec<(u16, u16)> {
        self.open.iter().map(|e| e.size()).collect()
    }

    pub fn clear(&mut self) {
        self.open.clear();
    }
}

/// A decoder, used by the round-trip test (and handy for inspecting a capture).
pub struct Decoder {
    handle: *mut c_void,
}

impl Decoder {
    /// `width`/`height` is the rectangle that was encoded; the coded size the
    /// decoder delivers is that rounded up to even.
    pub fn new(width: u16, height: u16) -> Result<Self> {
        let coded_w = width + (width & 1);
        let coded_h = height + (height & 1);
        let handle = unsafe { nl_dec_new(coded_w as i32, coded_h as i32) };
        if handle.is_null() {
            bail!("could not allocate a decoder");
        }
        Ok(Self { handle })
    }

    /// Size of the last decoded picture, as the decoder delivered it.
    pub fn decoded_size(&mut self) -> (i32, i32) {
        let (mut w, mut h) = (0, 0);
        unsafe { nl_dec_size(self.handle, &mut w, &mut h) };
        (w, h)
    }

    /// Decode one Annex-B frame; true when it produces a picture of the
    /// expected size.
    pub fn decode(&mut self, annexb: &[u8]) -> bool {
        unsafe { nl_dec_frame(self.handle, annexb.as_ptr(), annexb.len()) != 0 }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe { nl_dec_free(self.handle) };
    }
}

/// Split an Annex-B stream into (nal type, bytes including the start code).
pub fn nal_units(stream: &[u8]) -> Vec<(u8, &[u8])> {
    let mut out = Vec::new();
    let mut starts = Vec::new();
    let mut at = 0usize;
    while at + 4 <= stream.len() {
        if stream[at..at + 4] == [0, 0, 0, 1] {
            starts.push(at);
            at += 4;
        } else {
            at += 1;
        }
    }
    for (i, &begin) in starts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(stream.len());
        if let Some(type_byte) = stream.get(begin + 4) {
            out.push((type_byte & 0x1f, &stream[begin..end]));
        }
    }
    out
}

/// A flat mid-grey BGRA rectangle, for tests and for padding.
pub fn solid(width: u16, height: u16, bgra: [u8; 4]) -> Vec<u8> {
    bgra.repeat(width as usize * height as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: usize, h: usize) -> Vec<u8> {
        let mut v = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                v.extend_from_slice(&[(x * 3) as u8, (y * 5) as u8, ((x + y) * 2) as u8, 0xff]);
            }
        }
        v
    }

    #[test]
    fn codec_parses_names() {
        assert_eq!(Codec::parse("raw"), Some(Codec::Raw));
        assert_eq!(Codec::parse("H264"), Some(Codec::H264));
        assert_eq!(Codec::parse("avc"), Some(Codec::H264));
        assert_eq!(Codec::parse("vp9"), None);
        assert_eq!(Codec::default(), Codec::Raw);
    }

    /// The one test that matters without a dock: encode a rectangle, decode it
    /// again, and check the bitstream is a self-contained keyframe.
    #[test]
    fn h264_round_trip_is_a_self_contained_keyframe() {
        let (w, h) = (64u16, 48u16);
        let pixels = gradient(w as usize, h as usize);
        let mut enc = Encoder::new(w, h).unwrap();
        let frame = enc.encode(&pixels).unwrap();

        let nals = nal_units(&frame.bytes);
        let types: Vec<u8> = nals.iter().map(|(t, _)| *t).collect();
        assert!(types.contains(&7), "no SPS in {types:?}");
        assert!(types.contains(&8), "no PPS in {types:?}");
        assert!(types.contains(&5), "no IDR slice in {types:?}");
        assert_eq!(types[0], 7, "SPS must lead: {types:?}");
        assert!(frame.keyframe);

        let mut dec = Decoder::new(w, h).unwrap();
        assert!(dec.decode(&frame.bytes), "decoder rejected the frame");
    }

    #[test]
    fn solid_rectangles_encode_and_decode() {
        let mut enc = Encoder::new(32, 32).unwrap();
        let frame = enc
            .encode(&solid(32, 32, [0x40, 0x40, 0x40, 0xff]))
            .unwrap();
        assert!(!frame.bytes.is_empty());
        let mut dec = Decoder::new(32, 32).unwrap();
        assert!(dec.decode(&frame.bytes));
    }

    #[test]
    fn encoded_rects_are_smaller_than_raw_pixels() {
        let (w, h) = (128u16, 96u16);
        let mut enc = Encoder::new(w, h).unwrap();
        let frame = enc.encode(&solid(w, h, [0x33, 0x66, 0x99, 0xff])).unwrap();
        // A flat rectangle must compress well below the 4 bytes/px of BGRA,
        // even counting the SPS/PPS every keyframe carries.
        assert!(
            frame.bytes.len() < w as usize * h as usize * 4,
            "{} vs {}",
            frame.bytes.len(),
            w as usize * h as usize * 4
        );
    }

    #[test]
    fn encoder_rejects_short_pixels_and_zero_size() {
        assert!(Encoder::new(0, 10).is_err());
        let mut enc = Encoder::new(16, 16).unwrap();
        assert!(enc.encode(&[0u8; 8]).is_err());
    }

    #[test]
    fn pool_reuses_a_size_and_retires_the_oldest() {
        let mut pool = EncoderPool::default();
        let id = pool.get(32, 32).unwrap().handle_id();
        assert_eq!(pool.get(32, 32).unwrap().handle_id(), id);
        assert_eq!(pool.sizes(), [(32, 32)]);

        // Fill to the cap with other sizes, remembering the first one pushed,
        // then touch 32x32 so the remembered one is the oldest by use.
        let mut stale = None;
        for i in 0..EncoderPool::CAP as u16 - 1 {
            let size = pool.get(8 + i, 8).unwrap().size();
            stale.get_or_insert(size);
        }
        assert_eq!(pool.sizes().len(), EncoderPool::CAP);
        let stale = stale.unwrap();
        pool.get(32, 32).unwrap();

        pool.get(200, 16).unwrap();
        assert_eq!(pool.sizes().len(), EncoderPool::CAP, "pool grew past cap");
        assert!(!pool.sizes().contains(&stale), "{stale:?} was not retired");
        assert_eq!(pool.get(32, 32).unwrap().handle_id(), id);
    }

    /// A rect clipped at a display edge is not a multiple of 16, and a single
    /// changed pixel can produce a 1x1 rect. VideoToolbox only accepts even
    /// coded dimensions, so these are padded by one pixel where needed.
    #[test]
    fn odd_and_tiny_rects_encode() {
        for (w, h) in [(1u16, 1u16), (3, 5), (17, 9), (1, 40), (40, 1), (17, 17)] {
            let mut enc = Encoder::new(w, h).unwrap_or_else(|e| panic!("{w}x{h}: {e}"));
            assert_eq!(enc.size(), (w, h), "reported size must stay the rect");
            let (cw, ch) = enc.coded_size();
            assert!(cw >= w && ch >= h && cw % 2 == 0 && ch % 2 == 0);

            let frame = enc
                .encode(&solid(w, h, [0x10, 0x20, 0x30, 0xff]))
                .unwrap_or_else(|e| panic!("{w}x{h}: {e}"));
            let mut dec = Decoder::new(w, h).unwrap();
            assert!(dec.decode(&frame.bytes), "{w}x{h} did not decode");
            assert_eq!(dec.decoded_size(), (cw as i32, ch as i32));
        }
    }

    #[test]
    fn pool_keys_on_the_rect_not_the_padded_size() {
        // 17x17 and 18x17 pad to different coded sizes only if the rect differs,
        // so pooling by rect keeps the logical size authoritative.
        let mut pool = EncoderPool::default();
        assert_eq!(pool.get(17, 16).unwrap().coded_size(), (18, 16));
        assert_eq!(pool.get(16, 17).unwrap().coded_size(), (16, 18));
        assert_eq!(pool.sizes(), [(17, 16), (16, 17)]);
    }

    #[test]
    fn pool_rejects_a_zero_size_without_leaving_a_hole() {
        let mut pool = EncoderPool::default();
        assert!(pool.get(0, 8).is_err());
        assert!(pool.sizes().is_empty());
    }

    #[test]
    fn nal_splitter_finds_every_unit() {
        let stream = [
            0, 0, 0, 1, 0x67, 1, 2, 0, 0, 0, 1, 0x68, 3, 0, 0, 0, 1, 0x65, 4,
        ];
        let units = nal_units(&stream);
        assert_eq!(units.len(), 3);
        assert_eq!(units[0].0, 7);
        assert_eq!(units[1].0, 8);
        assert_eq!(units[2].0, 5);
        assert_eq!(units[2].1, [0, 0, 0, 1, 0x65, 4]);
    }
}
