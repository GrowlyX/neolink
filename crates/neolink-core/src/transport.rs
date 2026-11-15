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

use crate::dl3::{self, Header, Op, StatusOp};
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

fn describe(info: &DeviceInfo) -> DeviceDesc {
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
    pub desc: DeviceDesc,
    max_payload: usize,
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
            desc,
            max_payload,
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
                Header::new(Op::Reset, self.seq, 0).write(&mut reset);
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
        Header::new(Op::Trans, self.seq, body.len() as u16).write(&mut packet);
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

    pub fn write_edid(&mut self, edid: &[u8]) -> Result<()> {
        if !dl3::valid_edid(edid) {
            bail!("invalid length of raw EDID data");
        }
        self.status(StatusOp::Edid, 0, edid).map(|_| ())
    }

    /// A full frame: tile the damage rectangle and ship each fragment.
    pub fn frame(
        &mut self,
        x: u16,
        y: u16,
        w: u16,
        h: u16,
        pixel: impl Fn(usize) -> &'static [u8],
    ) -> Result<usize> {
        let heads = dl3::fragment(&mut self.seq, x, y, w, h, self.max_payload);
        let mut bytes_sent = 0;
        for (i, head) in heads.iter().enumerate() {
            let payload = pixel(i);
            let len = payload.len().min(head.len as usize);
            let mut packet = Vec::with_capacity(dl3::HEADER_LEN + len);
            let mut h = *head;
            h.len = len as u16;
            h.write(&mut packet);
            packet.extend_from_slice(&payload[..len]);
            self.write(&packet)?;
            bytes_sent += packet.len();
        }
        Ok(bytes_sent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_id_matches_synaptics() {
        assert_eq!(VENDOR_ID, 0x17e9);
    }
}
