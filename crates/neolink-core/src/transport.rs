//! USB transport for DisplayLink DL3 adapters, built on `nusb`.
//!
//! The vendor agent takes the device through `IOUSBLib` (see `IOUSBDevice` /
//! `HANDLE_IOUSBLIB_ERROR` in the 17.1.56 slice): match on vendor id `0x17e9`,
//! open the device, claim the vendor interface, reset the receiver with a
//! `RESET` packet, wait for the echoed resets, then stream `TRANS`/`MAP`/`DATA`
//! tiles over the bulk-out pipe and service status packets on bulk-in.
//!
//! Only one process may own an interface, so [`Transport::open`] fails with a
//! clear message while the vendor agent holds the dock.

use crate::diff::{Damage, DirtyTracker};
use crate::dl3::{self, Header, Op, StatusOp};
use crate::h264::{Codec, EncoderPool};
use anyhow::{anyhow, bail, Context, Result};
use nusb::transfer::{Bulk, In, Out};
use nusb::{DeviceInfo, MaybeFuture};
use std::time::Duration;

/// Synaptics/DisplayLink USB vendor id.
pub const VENDOR_ID: u16 = 0x17e9;
/// Interface class used by the vendor stack for DL3 bulk traffic.
pub const VENDOR_CLASS: u8 = 0xff;
const TIMEOUT: Duration = Duration::from_millis(500);
const RESET_ATTEMPTS: usize = 50;

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceDesc {
    pub vendor_id: u16,
    pub product_id: u16,
    pub serial: Option<String>,
    pub bus_id: String,
}

pub fn describe(info: &DeviceInfo) -> DeviceDesc {
    DeviceDesc {
        vendor_id: info.vendor_id(),
        product_id: info.product_id(),
        serial: info.serial_number().map(str::to_owned),
        bus_id: format!("{}:{}", info.bus_id(), info.device_address()),
    }
}

/// Every attached DisplayLink adapter. Empty without a dock attached.
pub fn devices() -> Result<Vec<DeviceDesc>> {
    Ok(nusb::list_devices()
        .wait()
        .context("listing USB devices")?
        .filter(|d| d.vendor_id() == VENDOR_ID)
        .map(|d| describe(&d))
        .collect())
}

pub fn first() -> Result<DeviceDesc> {
    devices()?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no DisplayLink adapter attached"))
}

/// Bulk endpoint addresses discovered from the vendor interface descriptor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Endpoints {
    pub out: u8,
    pub in_: u8,
}

/// Pick the vendor-specific interface and its bulk pair. Falls back to the
/// first endpoint of the right direction when a device omits the descriptor
/// class (some hubs report class `0` before the first `SET_CONFIGURATION`).
pub fn find_endpoints(
    config: &nusb::descriptors::ConfigurationDescriptor<'_>,
) -> Option<Endpoints> {
    let mut fallback = None;
    for iface in config.interfaces() {
        let mut out = None;
        let mut in_ = None;
        for alt in iface.alt_settings() {
            for ep in alt.endpoints() {
                if ep.transfer_type() != nusb::descriptors::TransferType::Bulk {
                    continue;
                }
                if ep.address() & 0x80 == 0 {
                    out.get_or_insert(ep.address());
                } else {
                    in_.get_or_insert(ep.address());
                }
            }
        }
        let Some((out, in_)) = out.zip(in_) else {
            continue;
        };
        let pair = Endpoints { out, in_ };
        if iface.first_alt_setting().class() == VENDOR_CLASS {
            return Some(pair);
        }
        fallback = fallback.or(Some(pair));
    }
    fallback
}

/// An open DL3 link.
pub struct Transport {
    iface: nusb::Interface,
    eps: Endpoints,
    seq: u8,
    channel: u8,
    counter: u16,
    frame_no: u32,
    pub desc: DeviceDesc,
    pub max_payload: usize,
    codec: Codec,
    /// Encoders kept warm per rectangle size: a VideoToolbox session is fixed to
    /// one size and damage rects vary frame to frame.
    encoders: EncoderPool,
}

impl std::fmt::Debug for Transport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transport")
            .field("desc", &self.desc)
            .field("eps", &self.eps)
            .finish()
    }
}

impl Transport {
    pub fn open() -> Result<Self> {
        Self::open_matching(|_| true)
    }

    pub fn open_matching(pick: impl Fn(&DeviceDesc) -> bool) -> Result<Self> {
        let info = nusb::list_devices()
            .wait()
            .context("listing USB devices")?
            .filter(|d| d.vendor_id() == VENDOR_ID)
            .find(|d| pick(&describe(d)))
            .ok_or_else(|| anyhow!("no matching DisplayLink adapter attached"))?;
        let desc = describe(&info);
        if crate::agent::running() {
            bail!(
                "{} is held by the DisplayLink agent; quit it first (neolinkctl agent stop)",
                desc.bus_id
            );
        }
        let dev = info.open().wait().context("opening USB device")?;
        let config = dev.active_configuration().map_err(|e| anyhow!("{e}"))?;
        let iface_num = config
            .interfaces()
            .find(|i| i.first_alt_setting().class() == VENDOR_CLASS)
            .map(|i| i.interface_number())
            .unwrap_or(0);
        let eps = find_endpoints(&config).ok_or_else(|| anyhow!("no bulk endpoints"))?;
        let iface = dev
            .claim_interface(iface_num)
            .wait()
            .context("claiming vendor interface")?;
        let max_payload = iface
            .endpoint::<Bulk, Out>(eps.out)
            .map(|e| e.max_packet_size())
            .unwrap_or(1024)
            .clamp(16, 16384);
        let mut transport = Self {
            iface,
            eps,
            seq: 0,
            channel: 1,
            counter: 0,
            frame_no: 0,
            desc,
            max_payload,
            codec: Codec::Raw,
            encoders: EncoderPool::default(),
        };
        transport.handshake()?;
        Ok(transport)
    }

    /// Take the link out of reset. The receiver echoes every reset; a dock that
    /// stops echoing has stalled (vendor: "Failed to reset receiver - no echoed
    /// DL3 reset packets received").
    fn handshake(&mut self) -> Result<()> {
        // The channel probe (`0x01`) is the one tag the validator always accepts; the following
        // RESET is what actually parks the receiver, and the dock echoes it. Both are retried
        // because the very first writes can land while the pipe is still draining.
        let mut probe = Vec::new();
        Header::probe(self.channel).write(&mut probe);
        for attempt in 0..RESET_ATTEMPTS {
            self.write(&probe)?;
            if let Ok((head, _)) = self.read_packet() {
                if head.tag() == 0x01 || head.len > 0 {
                    self.channel = head.channel.max(1);
                    break;
                }
            }
            if attempt == RESET_ATTEMPTS / 2 {
                let mut reset = Vec::new();
                self.seq = self.seq.wrapping_add(1);
                Header::new(Op::Reset, 0).write(&mut reset);
                self.write(&reset)?;
            }
        }
        if self.channel == 1 && self.seq == 0 {
            // Never saw any echo at all.
            bail!("no echoed DL3 reset packets received")
        }
        Ok(())
    }

    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        let ep = self.iface.endpoint::<Bulk, Out>(self.eps.out)?;
        let mut writer = ep.writer(bytes.len().max(64));
        use std::io::Write;
        writer.set_write_timeout(TIMEOUT);
        writer.write_all(bytes).context("bulk write")?;
        writer.flush().context("bulk flush")?;
        Ok(())
    }

    /// Read one whole packet: fixed header, then exactly the declared payload.
    fn read_packet(&mut self) -> Result<(dl3::Header, Vec<u8>)> {
        let mut reader = self
            .iface
            .endpoint::<Bulk, In>(self.eps.in_)?
            .reader(dl3::MAX_STATUS + dl3::HEADER_LEN);
        use std::io::Read;
        reader.set_read_timeout(TIMEOUT);
        let mut head = [0u8; dl3::HEADER_LEN];
        reader.read_exact(&mut head).context("bulk read header")?;
        let (parsed, _) = dl3::parse(&head)?;
        let mut payload = vec![0u8; parsed.len as usize];
        if !payload.is_empty() {
            reader
                .read_exact(&mut payload)
                .context("bulk read payload")?;
        }
        Ok((parsed, payload))
    }

    /// Read one logical message, stitching its fragments back together. A reply may arrive in
    /// several packets, each flagged `first`/`last`; the assembled body is bounded so a wedged
    /// dock cannot grow it without limit.
    fn read_message(&mut self) -> Result<Vec<u8>> {
        let mut body = Vec::new();
        let mut frags = 0;
        loop {
            let (head, payload) = self.read_packet()?;
            if frags == 0 && !head.first && body.is_empty() && payload.is_empty() {
                // Stray keep-alive; keep waiting.
                continue;
            }
            if frags == 0 && !head.first {
                bail!("reply did not start with a first fragment");
            }
            body.extend_from_slice(&payload);
            if body.len() > dl3::MAX_STATUS * 16 {
                bail!("reply exceeded {} bytes", dl3::MAX_STATUS * 16);
            }
            frags += 1;
            if head.last {
                return Ok(body);
            }
            if frags > 512 {
                bail!("reply fragment runaway");
            }
        }
    }

    /// Issue a status/control request and return the validated payload.
    pub fn status(&mut self, op: StatusOp, arg: u16, data: &[u8]) -> Result<Vec<u8>> {
        let body = dl3::status_body(op, arg, data);
        let mut packet = Vec::with_capacity(dl3::HEADER_LEN + body.len());
        self.seq = self.seq.wrapping_add(1);
        Header::new(Op::Trans, body.len() as u16).write(&mut packet);
        packet.extend_from_slice(&body);
        self.write(&packet)?;
        let payload = self.read_message()?;
        if payload.len() < 2 {
            bail!("status reply too short");
        }
        let (body, sum) = payload.split_at(payload.len() - 2);
        if dl3::checksum(body) != u16::from_le_bytes([sum[0], sum[1]]) {
            bail!("status checksum mismatch");
        }
        Ok(body.to_vec())
    }

    /// Full EDID read, falling back to a synthesised block when the dock answers
    /// `STATUS_OP_EDID not supported on this device` (some units have no pass-through).
    pub fn edid_or_template(&mut self, width_mm: u16, height_mm: u16, name: &str) -> Vec<u8> {
        self.edid()
            .unwrap_or_else(|_| dl3::template_edid(width_mm, height_mm, name))
    }

    /// Full EDID read. Reads the base block first so the extension count can be
    /// honoured; `STATUS_OP_EDID` rejects devices without EDID pass-through.
    pub fn edid(&mut self) -> Result<Vec<u8>> {
        let head = self.status(StatusOp::Edid, 0, &[0; 128])?;
        let mut edid = head;
        let ext = edid.get(126).copied().unwrap_or(0) as usize;
        for i in 1..=ext {
            let blk = self.status(StatusOp::Edid, i as u16, &[0; 128])?;
            edid.extend_from_slice(&blk);
        }
        if !dl3::valid_edid(&edid) {
            bail!("invalid length of raw EDID data");
        }
        Ok(edid)
    }

    /// HDCP/attributes status. `MonitorStatus` is what carries protection
    /// state, the receiver's HDCP revision and its capability flags; the agent
    /// validates the body's trailing checksum before trusting any of it, which
    /// [`Transport::status`] already does for us.
    pub fn attributes(&mut self) -> Result<dl3::Attributes> {
        let body = self.status(StatusOp::Attributes, 0, &[0; 16])?;
        Ok(dl3::Attributes::parse(&body))
    }

    /// True when the sink negotiated HDCP 2.2. The vendor logs
    /// `repeater auth received but rx is not HDCP2.2` instead of proceeding.
    pub fn hdcp22(&mut self) -> Result<bool> {
        Ok(self.attributes()?.hdcp22)
    }

    /// Select the payload format. Raw is the default; H.264 is opt-in because
    /// the packet header carries no format flag (see [`crate::h264`]).
    pub fn set_codec(&mut self, codec: Codec) {
        if self.codec != codec {
            self.codec = codec;
            self.encoders.clear();
        }
    }

    pub fn codec(&self) -> Codec {
        self.codec
    }

    /// Message name for an HDCP 2.x id, so a receiver log can be read.
    pub fn hdcp_name(id: u8) -> &'static str {
        dl3::HdcpMessage::name(id)
    }

    pub fn write_edid(&mut self, edid: &[u8]) -> Result<()> {
        if !dl3::valid_edid(edid) {
            bail!("invalid length of raw EDID data");
        }
        self.status(StatusOp::Edid, 0, edid).map(|_| ())
    }

    /// Ship a damage rectangle as DL3 packets.
    ///
    /// `pixels` is BGRA, four bytes per pixel, laid out with `stride` bytes per
    /// row; the rectangle is copied out tightly before it is cut, so `stride`
    /// padding never reaches the wire. The header carries the inclusive tile
    /// rectangle, and `offset` says where each packet's payload starts inside
    /// it; a packet past the first sets `cont` and its rect starts on the tile
    /// it resumes at, matching the vendor builder (`0x1001271d4`). `first` is
    /// set on the first packet of the frame only and `last` on packets that
    /// begin a rectangle, exactly as the builder derives them. Returns the
    /// bytes written.
    pub fn frame(
        &mut self,
        x: u16,
        y: u16,
        w: u16,
        h: u16,
        stride: usize,
        pixels: &[u8],
    ) -> Result<usize> {
        let bytes = self.rect(x, y, w, h, stride, pixels)?;
        self.frame_no = self.frame_no.wrapping_add(1);
        Ok(bytes)
    }

    /// Ship only the tiles that changed since the previous call, and return
    /// what was sent.
    ///
    /// The first call (or a call after a resolution change) ships the frame
    /// whole. A frame identical to the previous one sends nothing at all, so
    /// the receiver keeps showing what it already has. Otherwise each damage
    /// rect from [`DirtyTracker`] goes out as its own burst of DL3 packets.
    ///
    /// This is the bandwidth lever the vendor gets from its h264 encoder
    /// without the encoder: a static desktop costs zero bytes instead of a
    /// full BGRA frame, and a moving window costs its tiles.
    pub fn frame_dirty(
        &mut self,
        tracker: &mut DirtyTracker,
        stride: usize,
        pixels: &[u8],
    ) -> Result<(Damage, usize)> {
        let damage = tracker.diff(pixels, stride);
        let mut bytes = 0usize;
        for rect in &damage.rects {
            bytes += self.rect(
                rect.x as u16,
                rect.y as u16,
                rect.w as u16,
                rect.h as u16,
                stride,
                pixels,
            )?;
        }
        // An unchanged frame ships nothing, so the id stays with the last frame
        // the dock actually received instead of skipping over a gap.
        if !damage.is_empty() {
            self.frame_no = self.frame_no.wrapping_add(1);
        }
        Ok((damage, bytes))
    }

    /// Pack one pixel rect tightly and ship it. Every packet of the burst
    /// shares this frame id; `frame` and `frame_dirty` bump it once per frame.
    fn rect(
        &mut self,
        x: u16,
        y: u16,
        w: u16,
        h: u16,
        stride: usize,
        pixels: &[u8],
    ) -> Result<usize> {
        if w == 0 || h == 0 {
            return Ok(0);
        }
        let payload = rect_payload(self.codec, &mut self.encoders, (x, y, w, h), stride, pixels)?;
        let mut heads = dl3::fragment_len(x, y, w, h, payload.len(), self.max_payload.max(1));
        let mut bytes_sent = 0;
        let mut offset = 0usize;
        for head in heads.iter_mut() {
            let take = head.len as usize;
            head.counter = self.counter;
            head.frame = self.frame_no;
            let mut packet = Vec::with_capacity(dl3::HEADER_LEN + take);
            head.write(&mut packet);
            packet.extend_from_slice(&payload[offset..offset + take]);
            self.write(&packet)?;
            bytes_sent += packet.len();
            self.counter = self.counter.wrapping_add(1);
            offset += take;
        }
        Ok(bytes_sent)
    }
}

/// Copy a rect out of a padded BGRA frame and return the bytes that go on the
/// wire: the tight BGRA itself for [`Codec::Raw`], or one intra keyframe of
/// H.264 for [`Codec::H264`]. Split out from [`Transport::rect`] so the payload
/// shape can be tested without a dock attached.
fn rect_payload(
    codec: Codec,
    encoders: &mut EncoderPool,
    rect: (u16, u16, u16, u16),
    stride: usize,
    pixels: &[u8],
) -> Result<Vec<u8>> {
    const BPP: usize = 4;
    let (x, y, w, h) = rect;
    let (wf, hf) = (w as usize, h as usize);
    let mut packed = Vec::with_capacity(wf * hf * BPP);
    for row in 0..hf {
        let at = (y as usize + row) * stride + x as usize * BPP;
        if at + wf * BPP > pixels.len() {
            bail!("frame rect runs past the {stride}-byte buffer");
        }
        packed.extend_from_slice(&pixels[at..at + wf * BPP]);
    }
    match codec {
        Codec::Raw => Ok(packed),
        Codec::H264 => Ok(encoders.get(w, h)?.encode(&packed)?.bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_id_matches_synaptics() {
        assert_eq!(VENDOR_ID, 0x17e9);
    }

    /// Replays what `frame` would push for a pixel buffer, so geometry and
    /// stride handling are checked without a dock. Returns (offset, len).
    fn tiles(w: u16, h: u16, cap: usize) -> Vec<(usize, usize)> {
        let total = w as usize * h as usize * 4;
        let mut out = Vec::new();
        let mut at = 0;
        while at < total {
            let take = (total - at).min(cap);
            out.push((at, take));
            at += take;
        }
        out
    }

    #[test]
    fn frame_tiles_cover_the_rectangle_once() {
        // 40x24 with a 4096-byte cap ships one packet per tile row; the whole
        // span must be covered exactly once.
        let t = tiles(40, 24, 4096);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0], (0, 40 * 24 * 4));
    }

    /// Padded BGRA: 6 px of stride for a 4 px rect, so a leak of row padding
    /// would show up as the wrong payload length.
    fn padded(w: u16, h: u16, stride: usize) -> Vec<u8> {
        let mut v = vec![0u8; stride * h as usize];
        for y in 0..h as usize {
            for x in 0..w as usize {
                let at = y * stride + x * 4;
                v[at..at + 4].copy_from_slice(&[x as u8, y as u8, 7, 0xff]);
            }
        }
        v
    }

    #[test]
    fn raw_payload_is_the_tight_rows() {
        let (w, h, stride) = (4u16, 3u16, 24);
        let frame = padded(w, h, stride);
        let mut enc = EncoderPool::default();
        let out = rect_payload(Codec::Raw, &mut enc, (0, 0, w, h), stride, &frame).unwrap();
        assert_eq!(out.len(), w as usize * h as usize * 4);
        assert_eq!(out[..4], [0, 0, 7, 0xff]);
        assert_eq!(out[16..20], [0, 1, 7, 0xff]);
        assert!(enc.sizes().is_empty());
    }

    /// The compressed path must put a bitstream on the wire, not BGRA: the
    /// payload has to be a self-contained Annex-B keyframe and cannot be
    /// `w * h * 4` bytes for a flat rectangle.
    #[test]
    fn h264_payload_replaces_bgra_with_a_keyframe() {
        let (w, h, stride) = (64u16, 48u16, 64 * 4);
        let frame = vec![0x40u8; stride * h as usize];
        let mut enc = EncoderPool::default();
        let out = rect_payload(Codec::H264, &mut enc, (0, 0, w, h), stride, &frame).unwrap();
        assert_ne!(out.len(), w as usize * h as usize * 4);
        assert!(out.starts_with(&[0, 0, 0, 1]), "not Annex-B");
        assert!(
            crate::h264::nal_units(&out).iter().any(|(t, _)| *t == 5),
            "no IDR slice"
        );
        // The session is pooled by size: the same rect reuses it, and a
        // different rect adds a second size rather than dropping the first.
        assert_eq!(enc.sizes(), [(w, h)]);
        let first = enc.get(w, h).unwrap().handle_id();
        rect_payload(Codec::H264, &mut enc, (0, 0, w, h), stride, &frame).unwrap();
        assert_eq!(enc.get(w, h).unwrap().handle_id(), first);
        let small = padded(32, 32, stride);
        rect_payload(Codec::H264, &mut enc, (0, 0, 32, 32), stride, &small).unwrap();
        assert!(enc.sizes().contains(&(32, 32)));
        assert_eq!(enc.get(w, h).unwrap().handle_id(), first);
    }

    /// A one-pixel damage rect is the smallest thing the tracker can emit and
    /// still has to survive encode + fragmentation.
    #[test]
    fn a_single_pixel_h264_rect_ships() {
        let (w, h, stride) = (8u16, 3u16, 32);
        let frame = padded(w, h, stride);
        let mut enc = EncoderPool::default();
        let out = rect_payload(Codec::H264, &mut enc, (1, 1, 1, 1), stride, &frame).unwrap();
        let heads = dl3::fragment_len(1, 1, 1, 1, out.len(), 512);
        assert_eq!(
            heads.iter().map(|h| h.len as usize).sum::<usize>(),
            out.len()
        );
        assert!(heads[0].first);
        assert!(out.starts_with(&[0, 0, 0, 1]));
    }

    #[test]
    fn h264_payload_splits_by_bitstream_length() {
        let (w, h, stride) = (96u16, 64u16, 96 * 4);
        let frame = vec![0x22u8; stride * h as usize];
        let mut enc = EncoderPool::default();
        let out = rect_payload(Codec::H264, &mut enc, (0, 0, w, h), stride, &frame).unwrap();
        let heads = dl3::fragment_len(0, 0, w, h, out.len(), 128);
        let total: usize = heads.iter().map(|h| h.len as usize).sum();
        assert_eq!(total, out.len());
        assert!(heads.len() > 1);
    }

    #[test]
    fn frame_splits_oversized_tiles() {
        let t = tiles(16, 16, 100);
        assert_eq!(t.len(), 11);
        assert!(t.iter().all(|(_, n)| *n <= 100));
        assert_eq!(t.iter().map(|(_, n)| n).sum::<usize>(), 16 * 16 * 4);
        assert_eq!(t[1].0, 100);
    }
}
