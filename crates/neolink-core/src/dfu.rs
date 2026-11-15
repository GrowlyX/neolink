//! Dock firmware update, recovered from `DisplayLinkUserAgent` 17.1.56 (arm64).
//!
//! The vendor does **not** invent a protocol here: it drives the dock through
//! the standard USB DFU 1.1 class requests on the DFU interface (interface 1),
//! and everything that is DisplayLink-specific sits *above* that in the
//! firmware container. The request bytes are taken verbatim from the driver
//! (`0x100205cc0` status, `0x100208f14` download, `0x100208cd0` detach):
//!
//! ```text
//! DFU_DETACH    bmRequestType 0x21 bRequest 0 wValue 100 wIndex 1
//! DFU_DNLOAD    bmRequestType 0x21 bRequest 1 wValue <block> wIndex 1 data
//! DFU_GETSTATUS bmRequestType 0xa1 bRequest 3 wValue 0 wIndex 1 6 bytes
//! ```
//!
//! `GETSTATUS` is polled: byte 5 is `bState`, `4` (`dfuDNBUSY`) means poll
//! again, `2` (`dfuIDLE`) and `5` (`dfuMANIFEST_SYNC`) mean done. Bytes 1..4
//! are the little-endian `bwPollTimeout`; the vendor scales it by 16960 before
//! sleeping, and [`Status::poll_delay`] keeps that scaling.
//!
//! Two descriptors gate a flash, and both are validated exactly as the vendor
//! validates them:
//!
//! * a 16-byte firmware descriptor (`0x100205488`): byte 0 `0x10`, byte 1
//!   `0x40`, bytes 2..16 printable ASCII. Anything else is rejected with
//!   "Descriptor too short" / "too long" / "corrupt".
//! * a 9-byte DFU functional descriptor (`bLength` 9, `bDescriptorType` 0x21)
//!   whose `wTransferSize` at offset 5 is the block size, default 4096 and
//!   capped at `0xffff`; a failing transfer is retried with a smaller block
//!   ("Reducing block size to: N"), and the ladder restarts from block 0.
//!
//! The last download is a zero-length `DFU_DNLOAD` (`wValue` 0), which is what
//! actually commits the image ("Firmware sent").
//!
//! The image itself is an ELLA pack (see [`crate::spkg`]) whose `BD` TLV bodies
//! are the firmware. The vendor's masked variant (`0x10020dd00`) XORs the file
//! with a fixed 22-byte key before the same walk, so [`Firmware::parse`] accepts
//! either form.
//!
//! Everything here is recovered, not measured: no dock was attached when it was
//! written. See `docs/RE-FINDINGS.md`.

use anyhow::{bail, Context, Result};
use nusb::MaybeFuture;

use crate::spkg;
use std::time::Duration;

/// DFU interface number (`DfuInterface` in the vendor's interface table).
pub const DFU_INTERFACE: u16 = 1;
/// Block size the vendor starts from when the descriptor does not say.
pub const DEFAULT_BLOCK: usize = 4096;
/// Smallest block the vendor will retry down to.
pub const MIN_BLOCK: usize = 64;
/// Largest block the descriptor's `wTransferSize` is allowed to name.
pub const MAX_BLOCK: usize = 0xffff;
/// `bwPollTimeout` scaling the vendor applies before sleeping.
pub const POLL_SCALE_NS: u64 = 16_960;
/// `GET_DESCRIPTOR` request code.
const GET_DESCRIPTOR: u8 = 6;
/// The class request the dock's firmware descriptor answers.
const DFU_DESCRIPTOR: u8 = 0x10;
/// Per-request timeout the vendor passes to every DFU control transfer.
pub const TIMEOUT: Duration = Duration::from_millis(1000);

/// `bState` values that end a poll loop.
const STATE_BUSY: u8 = 4;
const STATE_IDLE: u8 = 2;
const STATE_MANIFEST: u8 = 5;

/// The masking key a masked `.fullrom` is XORed with.
///
/// The vendor stores it obfuscated: a 4-byte seed (`0x10020de54`) XORed over a
/// 17-byte body (`0x10020de6c`), where a zero seed byte is replaced by `0x5e`
/// (`0x1000948cc`, the same substitute the modifier object at `0x1003e4a38`
/// keeps at `+0x40`). Decoded that reads `FirmwareModif\x1cer` — the vendor's
/// own string, with a stray `0x1c`, and it is repeated over the file.
const MASK: [u8; 17] = [
    0x46, 0x69, 0x72, 0x6d, 0x77, 0x61, 0x72, 0x65, b'M', b'o', b'd', b'i', b'f', 0x1c, b'e', b'r',
    0x00,
];

/// Substitute the vendor uses for a zero key byte.
const MASK_ZERO: u8 = 0x5e;

/// `GETSTATUS` reply, decoded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Status {
    pub poll: u32,
    pub state: u8,
}

impl Status {
    /// The reply the vendor requires to be exactly this long.
    pub const LEN: usize = 6;

    pub fn parse(buf: &[u8]) -> Result<Self> {
        if buf.len() != Self::LEN {
            bail!("DFU status is {} bytes, expected {}", buf.len(), Self::LEN);
        }
        Ok(Self {
            poll: u32::from_le_bytes([buf[1], buf[2], buf[3], 0]),
            state: buf[5],
        })
    }

    /// True while the dock is still working through the last block.
    pub fn busy(&self) -> bool {
        self.state == STATE_BUSY
    }

    /// True when the dock reports a finished (or finishable) state.
    pub fn done(&self) -> bool {
        self.state == STATE_IDLE || self.state == STATE_MANIFEST
    }

    /// How long the dock asked us to wait, in the vendor's scaling. Capped so a
    /// bogus `bwPollTimeout` cannot stall an update.
    pub fn poll_delay(&self) -> Duration {
        let ns = (self.poll as u64).saturating_mul(POLL_SCALE_NS);
        Duration::from_millis((ns / 1_000_000).clamp(1, 1000))
    }
}

/// The dock's 16-byte firmware descriptor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Descriptor {
    raw: [u8; 16],
}

impl Descriptor {
    pub const LEN: usize = 16;

    /// Validate a descriptor exactly as the vendor does (`0x100205488`).
    ///
    /// The length check comes first there, so a short buffer reports "too
    /// short" and anything else "too long"; the caller kept the length, so both
    /// collapse into one error here except for the length itself.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < Self::LEN {
            bail!("Missing descriptor");
        }
        if bytes.len() > Self::LEN {
            bail!("Descriptor too long");
        }
        let mut raw = [0u8; Self::LEN];
        raw.copy_from_slice(bytes);
        if raw[0] != 0x10 || raw[1] != 0x40 {
            bail!("Descriptor corrupt");
        }
        if !raw[8..].iter().all(|b| (0x20..=0x7e).contains(b)) {
            bail!("Descriptor corrupt");
        }
        Ok(Self { raw })
    }

    pub fn raw(&self) -> &[u8; Self::LEN] {
        &self.raw
    }

    /// The 24-bit customer/board id, packed the way `0x10020a654` composes it:
    /// `u16` at bytes 2..4 little-endian, then byte 4 in the high half.
    pub fn customer_board(&self) -> u32 {
        u32::from(self.raw[2]) | u32::from(self.raw[3]) << 8 | u32::from(self.raw[4]) << 16
    }

    /// The firmware id the vendor prints as `FirmwareType` / `firmwareId`
    /// (bytes 5..13, big-endian).
    pub fn firmware_id(&self) -> u64 {
        let mut v = 0u64;
        for b in &self.raw[5..13] {
            v = (v << 8) | u64::from(*b);
        }
        v
    }
}

/// The DFU functional descriptor: `wTransferSize` is the block size.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Functional {
    pub block: usize,
}

impl Functional {
    pub const LEN: usize = 9;

    /// Read `wTransferSize` out of the descriptor, defaulting to
    /// [`DEFAULT_BLOCK`] when the dock does not name a usable one ("No block
    /// size could be read" in the vendor is this same fallback).
    pub fn parse(bytes: &[u8]) -> Self {
        let block = bytes
            .get(5..7)
            .map(|s| usize::from(u16::from_le_bytes([s[0], s[1]])))
            .unwrap_or(0)
            .clamp(MIN_BLOCK, MAX_BLOCK);
        Self {
            block: if bytes.len() <= 5 {
                DEFAULT_BLOCK
            } else {
                block
            },
        }
    }
}

/// First block size to try: the descriptor's, capped so one transfer cannot
/// outgrow the vendor's own 4096-byte default stack buffer.
pub fn initial_block(functional: Option<&Functional>) -> usize {
    match functional {
        Some(f) => f.block.min(DEFAULT_BLOCK).max(MIN_BLOCK),
        None => DEFAULT_BLOCK,
    }
}

/// One firmware image: the container file itself, validated and patched.
///
/// The vendor does not strip the container before flashing — `0x10020de1c`
/// loads the file, unmasks it if needed, checks its ELLA header and length
/// against the file size (`0x10020a390`), walks the TLV table for the `BD`
/// entry, clears the single "candidate" byte inside it, and then hands the
/// *whole buffer* to the DFU writer. The dock receives the container and
/// splits it itself.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Firmware {
    /// The exact bytes to push, container and all.
    pub body: Vec<u8>,
}

impl Firmware {
    /// The only tag this step cares about; the rest are carried through.
    const BODY: u16 = u16::from_le_bytes(*b"BD");
    const END: u16 = 0x0001;

    /// Undo the vendor's masking. The key repeats over the file.
    pub fn unmask(bytes: &[u8]) -> Vec<u8> {
        bytes
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let key = MASK[i % MASK.len()];
                let key = if key == 0 { MASK_ZERO } else { key };
                b ^ key
            })
            .collect()
    }

    /// Parse a `.fullrom`/`.spkg` in either form.
    ///
    /// A masked file is recognised by its header not being the magic, which is
    /// exactly what the vendor does: it unmasks, then validates, and reports
    /// "Bad magic value" only when the unmasked copy still misses it.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let owned;
        let bytes = if bytes.starts_with(spkg::MAGIC) {
            (bytes, None)
        } else {
            owned = Self::unmask(bytes);
            (owned.as_slice(), Some(()))
        };
        let (bytes, _) = bytes;
        Self::validate(bytes)?;
        let mut body = bytes.to_vec();
        Self::clear_candidate(&mut body)?;
        Ok(Self { body })
    }

    fn validate(bytes: &[u8]) -> Result<()> {
        if bytes.len() <= 0xb {
            bail!("File too small: {} bytes", bytes.len());
        }
        if bytes.len() % 4 != 0 {
            bail!("Firmware size {} is not a multiple of 4", bytes.len());
        }
        if !bytes.starts_with(spkg::MAGIC) {
            bail!("Bad magic value");
        }
        let declared = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
        if declared != bytes.len() {
            bail!(
                "Internal length ({declared}) different to file size ({})",
                bytes.len()
            );
        }
        Ok(())
    }

    /// Zero the `BD` candidate byte: the vendor requires that TLV to be exactly
    /// one byte, and refuses the file ("wrong length of TLV") otherwise. A file
    /// with no `BD` at all is accepted unchanged.
    fn clear_candidate(bytes: &mut [u8]) -> Result<()> {
        let at = u16::from_le_bytes([bytes[8], bytes[9]]) as usize;
        let len = u16::from_le_bytes([bytes[10], bytes[11]]) as usize;
        if len == 0 {
            bail!("File too small for TLV offset={at}, length={len}");
        }
        let end = at
            .checked_add(len)
            .filter(|e| *e <= bytes.len())
            .ok_or_else(|| anyhow::anyhow!("File too small for TLV offset={at}, length={len}"))?;
        let mut cur = at;
        while cur + 4 <= end {
            let tag = u16::from_le_bytes([bytes[cur], bytes[cur + 1]]);
            let l = u16::from_le_bytes([bytes[cur + 2], bytes[cur + 3]]) as usize;
            let start = cur + 4;
            let stop = start
                .checked_add(l)
                .filter(|s| *s <= end)
                .ok_or_else(|| anyhow::anyhow!("Invalid TLV Body"))?;
            if tag == Self::END {
                break;
            }
            if tag == Self::BODY {
                if l != 1 {
                    bail!("Invalid firmware: wrong length of TLV");
                }
                bytes[start] = 0;
            }
            cur = stop + (l & 1);
        }
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.body.len()
    }

    pub fn is_empty(&self) -> bool {
        self.body.is_empty()
    }
}

/// An opened device in DFU mode.
///
/// This deliberately does not reuse [`crate::transport::Transport`]: that one
/// claims the vendor interface and runs a DL3 handshake, and a dock sitting in
/// DFU answers neither.
pub struct Device {
    iface: nusb::Interface,
    pub desc: crate::transport::DeviceDesc,
}

impl Device {
    /// Claim the DFU interface of a matching dock.
    pub fn open_matching(pick: impl Fn(&crate::transport::DeviceDesc) -> bool) -> Result<Self> {
        let info = nusb::list_devices()
            .wait()
            .context("listing USB devices")?
            .filter(|d| d.vendor_id() == crate::transport::VENDOR_ID)
            .find(|d| pick(&crate::transport::describe(d)))
            .ok_or_else(|| anyhow::anyhow!("no matching DisplayLink adapter attached"))?;
        let desc = crate::transport::describe(&info);
        if crate::agent::running() {
            bail!(
                "{} is held by the DisplayLink agent; quit it first (neolinkctl agent stop)",
                desc.bus_id
            );
        }
        let dev = info.open().wait().context("opening USB device")?;
        let iface = dev
            .claim_interface(DFU_INTERFACE as u8)
            .wait()
            .context("claiming DFU interface")?;
        Ok(Self { iface, desc })
    }

    /// Read the 16-byte firmware descriptor the dock exposes.
    pub fn descriptor(&self) -> Result<Descriptor> {
        let raw = self
            .iface
            .control_in(
                nusb::transfer::ControlIn {
                    control_type: nusb::transfer::ControlType::Class,
                    recipient: nusb::transfer::Recipient::Interface,
                    request: DFU_DESCRIPTOR,
                    value: 0,
                    index: DFU_INTERFACE,
                    length: Descriptor::LEN as u16,
                },
                TIMEOUT,
            )
            .wait()
            .context("reading DFU descriptor")?;
        Descriptor::parse(&raw)
    }

    /// Read the functional descriptor, whose `wTransferSize` is the block size.
    /// A dock that does not answer is not fatal; the default is used instead
    /// ("No block size could be read" in the vendor).
    pub fn functional(&self) -> Option<Functional> {
        let raw = self
            .iface
            .control_in(
                nusb::transfer::ControlIn {
                    control_type: nusb::transfer::ControlType::Standard,
                    recipient: nusb::transfer::Recipient::Interface,
                    request: GET_DESCRIPTOR,
                    value: 0x2100,
                    index: DFU_INTERFACE,
                    length: Functional::LEN as u16,
                },
                TIMEOUT,
            )
            .wait()
            .ok()?;
        Some(Functional::parse(&raw))
    }

    pub fn session(&self, functional: Option<&Functional>) -> Session<'_> {
        Session::new(&self.iface, functional)
    }
}

/// Block plan for one size/size pair: `(block, offset, len)`.
///
/// The doubles as the retry ladder driver: every block is `block` bytes except
/// the last, which is short, and the count matches what the dock expects in
/// `wValue`.
pub fn plan(total: usize, block: usize) -> impl Iterator<Item = (u16, usize, usize)> {
    let block = block.clamp(1, MAX_BLOCK);
    let mut at = 0usize;
    let mut n: u16 = 0;
    std::iter::from_fn(move || {
        if at >= total {
            return None;
        }
        let take = (total - at).min(block);
        let item = (n, at, take);
        at += take;
        n = n.wrapping_add(1);
        Some(item)
    })
}

/// Why an update stopped, so a caller can tell a stall from a bad image.
#[derive(Debug)]
pub enum FlashError {
    /// The dock rejected a block at the smallest size we are willing to try.
    Stall {
        block: usize,
        offset: usize,
    },
    Other(anyhow::Error),
}

impl From<anyhow::Error> for FlashError {
    fn from(e: anyhow::Error) -> Self {
        FlashError::Other(e)
    }
}

impl std::fmt::Display for FlashError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FlashError::Stall { block, offset } => {
                write!(f, "dock stalled at block size {block} (offset {offset})")
            }
            FlashError::Other(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for FlashError {}

/// Progress callback: `(sent, total)`.
pub type Progress<'a> = &'a mut dyn FnMut(usize, usize);

/// A DFU session over an open interface.
pub struct Session<'a> {
    iface: &'a nusb::Interface,
    block: usize,
    progress: Option<Progress<'a>>,
}

impl<'a> Session<'a> {
    pub fn new(iface: &'a nusb::Interface, functional: Option<&Functional>) -> Self {
        Self {
            iface,
            block: initial_block(functional),
            progress: None,
        }
    }

    pub fn with_progress(mut self, progress: Progress<'a>) -> Self {
        self.progress = Some(progress);
        self
    }

    fn class_out(&self, request: u8, value: u16, data: &[u8]) -> Result<()> {
        self.iface
            .control_out(
                nusb::transfer::ControlOut {
                    control_type: nusb::transfer::ControlType::Class,
                    recipient: nusb::transfer::Recipient::Interface,
                    request,
                    value,
                    index: DFU_INTERFACE,
                    data,
                },
                TIMEOUT,
            )
            .wait()
            .context("DFU control out")?;
        Ok(())
    }

    /// Read `bState` until the dock stops reporting busy.
    fn poll(&self) -> Result<Status> {
        for _ in 0..256 {
            let reply = self
                .iface
                .control_in(
                    nusb::transfer::ControlIn {
                        control_type: nusb::transfer::ControlType::Class,
                        recipient: nusb::transfer::Recipient::Interface,
                        request: 3,
                        value: 0,
                        index: DFU_INTERFACE,
                        length: Status::LEN as u16,
                    },
                    TIMEOUT,
                )
                .wait()
                .context("DFU_GETSTATUS")?;
            let status = Status::parse(&reply)?;
            if !status.busy() {
                if status.done() {
                    return Ok(status);
                }
                bail!("DFU state 0x{:02x}", status.state);
            }
            std::thread::sleep(status.poll_delay());
        }
        bail!("DFU_GETSTATUS kept reporting busy")
    }

    /// Ask the dock to leave application mode and enter DFU.
    pub fn detach(&self) -> Result<()> {
        self.class_out(0, 100, &[])
    }

    /// Push the whole image.
    ///
    /// On a failing block the size is halved and the image restarts from block
    /// 0 at the new size, matching the vendor's ladder; below [`MIN_BLOCK`] the
    /// update gives up.
    pub fn flash(&mut self, fw: &Firmware) -> std::result::Result<usize, FlashError> {
        loop {
            match self.run(fw) {
                Ok(sent) => return Ok(sent),
                Err(FlashError::Stall { block, .. }) if block > MIN_BLOCK => {
                    self.block = (block / 2).max(MIN_BLOCK);
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn run(&mut self, fw: &Firmware) -> std::result::Result<usize, FlashError> {
        let mut sent = 0usize;
        for (index, at, len) in plan(fw.len(), self.block) {
            let chunk = &fw.body[at..at + len];
            self.class_out(1, index, chunk)
                .map_err(|_| FlashError::Stall {
                    block: self.block,
                    offset: at,
                })?;
            self.poll()?;
            sent += len;
            if let Some(cb) = self.progress.as_mut() {
                cb(sent, fw.len());
            }
        }
        // Zero-length DNLOAD commits the image; the dock then manifests.
        self.class_out(1, 0, &[])?;
        self.poll()?;
        Ok(sent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor() -> Vec<u8> {
        let mut d = vec![0x10, 0x40, 0x02, 0x00, 0x07];
        d.extend_from_slice(b"ABCDEFGHIJK");
        assert_eq!(d.len(), 16);
        d
    }

    #[test]
    fn descriptor_roundtrip() {
        let d = Descriptor::parse(&descriptor()).unwrap();
        assert_eq!(d.customer_board(), 0x0007_0002);
        assert_eq!(d.firmware_id(), 0x4142_4344_4546_4748);
        assert_eq!(d.raw()[0], 0x10);
    }

    #[test]
    fn descriptor_rejects_short_long_and_corrupt() {
        assert!(Descriptor::parse(&[0x10; 15])
            .unwrap_err()
            .to_string()
            .contains("Missing"));
        let mut d = descriptor();
        d.push(b'Z');
        assert!(Descriptor::parse(&d)
            .unwrap_err()
            .to_string()
            .contains("too long"));
        let mut d = descriptor();
        d[1] = 0x41;
        assert_eq!(
            Descriptor::parse(&d).unwrap_err().to_string(),
            "Descriptor corrupt"
        );
        let mut d = descriptor();
        d[9] = 0x01;
        assert_eq!(
            Descriptor::parse(&d).unwrap_err().to_string(),
            "Descriptor corrupt"
        );
    }

    #[test]
    fn functional_descriptor_clamps() {
        let mut f = [0u8; 9];
        f[5..7].copy_from_slice(&8192u16.to_le_bytes());
        assert_eq!(Functional::parse(&f).block, 8192);
        // initial_block never exceeds the vendor's 4096 stack buffer.
        assert_eq!(initial_block(Some(&Functional::parse(&f))), DEFAULT_BLOCK);
        f[5..7].copy_from_slice(&4u16.to_le_bytes());
        assert_eq!(Functional::parse(&f).block, MIN_BLOCK);
        assert_eq!(Functional::parse(&[0u8; 4]).block, DEFAULT_BLOCK);
    }

    #[test]
    fn status_rejects_bad_length_and_reads_state() {
        assert!(Status::parse(&[0u8; 5]).is_err());
        let s = Status::parse(&[0, 0x10, 0, 0, 0, STATE_BUSY]).unwrap();
        assert!(s.busy() && !s.done());
        assert_eq!(s.poll, 0x10);
        // 0x10 * 16960 ns = 271360 ns -> 0 ms, clamped up to 1 ms.
        assert_eq!(s.poll_delay(), Duration::from_millis(1));
        assert!(Status::parse(&[0, 0, 0, 0, 0, STATE_IDLE]).unwrap().done());
    }

    #[test]
    fn mask_is_the_vendors_obfuscated_key() {
        // The seed/body pair as the vendor lays it out before deobfuscation,
        // decoded exactly like `0x1000948cc`: body ^ seed[i % 4], 0 -> 0x5e.
        let seed = [0x99u8, 0xc3, 0xfc, 0xc6];
        let body = [
            0xdfu8, 0xaa, 0x8e, 0xab, 0xee, 0xa2, 0x8e, 0xa3, 0xd4, 0xac, 0x98, 0xaf, 0xff, 0xdf,
            0x99, 0xb4, 0x99,
        ];
        let decoded: Vec<u8> = body
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let key = seed[i % 4];
                let key = if key == 0 { MASK_ZERO } else { key };
                b ^ key
            })
            .collect();
        assert_eq!(decoded, MASK.to_vec());
        assert_eq!(&MASK[..11], b"FirmwareMod");
        assert_eq!(&MASK[14..], b"er\0");
    }

    #[test]
    fn mask_roundtrips() {
        let plain = b"ELLA-and-more-bytes-than-the-key".to_vec();
        let masked = Firmware::unmask(&plain);
        assert_ne!(masked, plain);
        assert_eq!(Firmware::unmask(&masked), plain);
    }

    fn pack(tlvs: &[(u16, &[u8])]) -> Vec<u8> {
        let mut table = Vec::new();
        for (tag, body) in tlvs {
            table.extend_from_slice(&tag.to_le_bytes());
            table.extend_from_slice(&(body.len() as u16).to_le_bytes());
            table.extend_from_slice(body);
            if body.len() & 1 == 1 {
                table.push(0);
            }
        }
        let mut out = vec![0u8; 16];
        out[..4].copy_from_slice(b"ELLA");
        out[8..10].copy_from_slice(&16u16.to_le_bytes());
        out[10..12].copy_from_slice(&(table.len() as u16).to_le_bytes());
        out.extend_from_slice(&table);
        // The vendor requires the file to be a multiple of four and to match
        // the header length, so pad the same way a real pack is padded.
        while out.len() % 4 != 0 {
            out.push(0);
        }
        let total = out.len() as u32;
        out[4..8].copy_from_slice(&total.to_le_bytes());
        out
    }

    #[test]
    fn firmware_sends_the_whole_container_with_the_candidate_cleared() {
        let bytes = pack(&[
            (u16::from_le_bytes(*b"RD"), b"r"),
            (Firmware::BODY, &[1]),
            (Firmware::END, b""),
        ]);
        let fw = Firmware::parse(&bytes).unwrap();
        // Everything is shipped; only the one candidate byte changed.
        assert_eq!(fw.body.len(), bytes.len());
        let at = u16::from_le_bytes([bytes[8], bytes[9]]) as usize;
        // RD is 4 + 1 + 1 pad, so the BD payload sits at at + 6 + 4.
        assert_eq!(fw.body[at + 10], 0);
        assert_eq!(bytes[at + 10], 1);
        let mut want = bytes.clone();
        want[at + 10] = 0;
        assert_eq!(fw.body, want);
    }

    #[test]
    fn firmware_requires_multiple_of_four() {
        let mut bytes = pack(&[(Firmware::BODY, &[1])]);
        bytes.push(b'x');
        assert!(Firmware::parse(&bytes)
            .unwrap_err()
            .to_string()
            .contains("multiple of 4"));
    }

    #[test]
    fn firmware_rejects_length_mismatch() {
        let mut bytes = pack(&[(Firmware::BODY, &[1])]);
        bytes[4] = bytes[4].wrapping_add(4);
        assert!(Firmware::parse(&bytes)
            .unwrap_err()
            .to_string()
            .contains("different to file size"));
    }

    #[test]
    fn firmware_rejects_bad_magic_and_short_file() {
        assert_eq!(
            Firmware::parse(b"not firmware at all!!!!!")
                .unwrap_err()
                .to_string(),
            "Bad magic value"
        );
        assert!(Firmware::parse(b"ELLA")
            .unwrap_err()
            .to_string()
            .contains("too small"));
    }

    #[test]
    fn firmware_rejects_wrong_candidate_length() {
        let bytes = pack(&[(Firmware::BODY, &[1, 2])]);
        assert!(Firmware::parse(&bytes)
            .unwrap_err()
            .to_string()
            .contains("wrong length of TLV"));
    }

    #[test]
    fn firmware_accepts_a_pack_without_a_body_tag() {
        let bytes = pack(&[(u16::from_le_bytes(*b"RD"), b"r")]);
        let fw = Firmware::parse(&bytes).unwrap();
        assert_eq!(fw.body, bytes);
    }

    #[test]
    fn firmware_accepts_masked_pack() {
        let plain = pack(&[(Firmware::BODY, &[1])]);
        let masked = Firmware::unmask(&plain);
        assert!(!masked.starts_with(b"ELLA"));
        let a = Firmware::parse(&masked).unwrap();
        let b = Firmware::parse(&plain).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.body.len(), plain.len());
    }

    #[test]
    fn plan_covers_once_and_ends_short() {
        let p: Vec<_> = plan(10, 4).collect();
        assert_eq!(p.len(), 3);
        assert_eq!(p[0], (0, 0, 4));
        assert_eq!(p[2], (2, 8, 2));
        assert_eq!(p.iter().map(|(_, o, l)| o + l).max(), Some(10));
    }
}
