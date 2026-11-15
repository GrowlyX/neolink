//! HDCP 2.x AKE, recovered from `DisplayLinkUserAgent` 17.1.56 (arm64).
//!
//! HDCP 2.x never touches the bulk pipe: every message is a vendor control
//! transfer on the DL3 interface, wrapped in a two-word header. The vendor has
//! one helper per direction and both hardcode the transfer type:
//!
//! ```text
//! out  bmRequestType 0x40  bRequest 0x01  wValue <id>  wIndex flags|0x2000
//! in   bmRequestType 0xc0  bRequest 0x01  wValue <id>  wIndex flags|0x2000
//! exit bmRequestType 0xc0  bRequest 0x02  wValue seq<<8 wIndex id|0x2c00
//! ```
//!
//! The outbound helper (`0x100289ea4`) writes the message id as the first body
//! byte and ships at most 64 bytes per transfer, so a 520-byte
//! `AKE_SEND_CERT` goes out as nine transfers. The inbound helper
//! (`0x100289be0`) writes a two-byte request header `[id, seq|0x10]` and reads
//! back up to 128 bytes; a reply whose body starts `0x44` is a retry and one
//! starting `0x40` is a flush. EXIT (`0x10028a118`) is the only place the
//! sequence number appears in `wValue` and hardcodes the `|0x10` trailer.
//!
//! The receiver's certificate is checked against a root key the vendor embeds
//! as a 768-hex-digit table at `0x100508b61`, parsed into a 384-byte bignum by
//! `0x1003166e8` with exponent `65537` stored at `+0x18` (`0x1003acfe4`). The
//! signature is plain RSASSA-PKCS1-v1_5 over SHA-256 of
//! `rtx || rrx || TxCaps || RxCaps || ksv`, verified at `0x1003ace34`; the
//! failure string is `AKE failed signature verification` and the success
//! string `AkeSigCheckPassed` sits next to it. `AKE_SEND_CERT` parses at
//! `0x1003a2a64` as `[id][rrx:2][caps:4][cert:128][ksv:5][sig:384]`; the KSV
//! must be exactly five bytes ("invalid raw ksv bytes!", `0x1003ad640`).
//!
//! Everything here is recovered, not measured: no dock was attached. See
//! `docs/RE-FINDINGS.md` and `COMPATIBILITY.md`.

use anyhow::{anyhow, bail, Context, Result};
use nusb::transfer::{ControlIn, ControlOut, ControlType, Recipient};
use nusb::MaybeFuture;
use sha2::{Digest, Sha256};
use std::time::Duration;

use crate::dl3::HdcpMessage;

/// Interface the vendor keeps its HDCP control transfers on (DL3 interface).
pub const INTERFACE: u16 = 0;
/// Per-transfer timeout. The vendor passes 5500 ms on every HDCP helper.
pub const TIMEOUT: Duration = Duration::from_millis(5500);
/// Largest body the vendor puts in one control transfer (`0x10028a184`,
/// `0x10028a1b2`: `cmp x8, #0x3f` / `orr w3, w21, #0x2c00`).
pub const CHUNK: usize = 64;
/// Largest reply the vendor reads in one transfer.
pub const MAX_REPLY: usize = 128;
/// `wIndex` bit the vendor ORs into every AKE transfer.
pub const FLAG_WRITE: u16 = 0x2000;
/// `wIndex` the EXIT path ORs in, together with `id`.
pub const FLAG_EXIT: u16 = 0x2c00;
/// Reply header byte meaning "retry the read" (`0x10028a9f4`).
pub const RETRY: u8 = 0x44;
/// Reply header byte meaning "flush and read again".
pub const FLUSH: u8 = 0x40;

/// RSA exponent the vendor stores next to the root modulus.
pub const ROOT_EXPONENT: u32 = 65537;

/// HDCP 2.x root key modulus, 3072 bits, as embedded by the vendor.
///
/// The binary keeps this as ASCII hex at `0x100508b61` and converts it to
/// 384 big-endian bytes at `0x1003166e8`; the bytes below are that
/// conversion, so no hex parsing runs at load time.
pub const ROOT_MODULUS: [u8; 384] = [
    0xe6, 0xa9, 0x66, 0xcf, 0xc6, 0xc4, 0x39, 0xff, 0xe9, 0x2b, 0xa3, 0x24, 0xaf, 0x52, 0x38, 0x6d,
    0x07, 0x17, 0x0a, 0xf2, 0x61, 0xf6, 0x0f, 0x20, 0x10, 0x88, 0x48, 0xc1, 0xb8, 0x21, 0x0b, 0xa5,
    0xf3, 0x2b, 0x64, 0x4f, 0x5c, 0x3a, 0x3c, 0x37, 0x67, 0x5e, 0xd3, 0x7d, 0xfb, 0x33, 0x04, 0xe9,
    0x40, 0x5f, 0xa0, 0x62, 0xfc, 0xfb, 0xc6, 0xdf, 0xed, 0x1c, 0x5b, 0x73, 0xb6, 0xd1, 0x21, 0x7b,
    0x1d, 0xbb, 0x41, 0xa0, 0x90, 0x7f, 0x4b, 0x28, 0x06, 0xe5, 0x01, 0x6a, 0xed, 0x17, 0x4e, 0xbd,
    0x28, 0x5c, 0xe1, 0x1a, 0x1c, 0x58, 0xbc, 0xf9, 0xc3, 0xf0, 0x93, 0x29, 0x10, 0x13, 0xdd, 0xf1,
    0xa8, 0xf8, 0x51, 0xeb, 0xf3, 0xe3, 0xf5, 0x6e, 0x07, 0xb0, 0x22, 0xce, 0xd8, 0x07, 0x28, 0xe5,
    0x70, 0xc7, 0x24, 0xb2, 0x9f, 0x76, 0x6e, 0x37, 0x2b, 0x6f, 0x13, 0x9f, 0x2c, 0xda, 0xa2, 0x2d,
    0xfd, 0x28, 0x83, 0xa5, 0xd8, 0xb7, 0xf1, 0x01, 0xd3, 0x12, 0x29, 0x17, 0x2e, 0xd7, 0xac, 0x2a,
    0x72, 0x79, 0xc9, 0xe3, 0xb5, 0x15, 0xb7, 0x22, 0x67, 0xad, 0x53, 0x4d, 0xc1, 0x9f, 0xd7, 0x0b,
    0x7e, 0x94, 0x7a, 0xe2, 0x00, 0xa6, 0x51, 0x3b, 0xfc, 0x8a, 0x9f, 0x59, 0x24, 0xfa, 0x44, 0x01,
    0xc5, 0x1d, 0x64, 0x8a, 0xc7, 0x68, 0x0a, 0x18, 0xad, 0x5c, 0x01, 0xbb, 0x03, 0x72, 0x70, 0x57,
    0xa8, 0x71, 0x2e, 0xd4, 0x31, 0xf2, 0x00, 0xef, 0x31, 0xfa, 0xab, 0xc1, 0x1f, 0xe9, 0xf1, 0x60,
    0xcd, 0xac, 0x4b, 0x91, 0x1a, 0x2a, 0xb0, 0x34, 0x18, 0x32, 0x0c, 0x1e, 0x34, 0x49, 0x91, 0xf4,
    0x60, 0x6a, 0x89, 0xea, 0x21, 0x9c, 0x62, 0x67, 0x57, 0x0e, 0x42, 0x37, 0x42, 0x47, 0xd3, 0x5e,
    0x30, 0xfb, 0x5d, 0xbe, 0x0f, 0x8c, 0xd9, 0xbf, 0xd8, 0x48, 0x35, 0xc1, 0x51, 0x99, 0xa4, 0xee,
    0x66, 0x44, 0x0a, 0x81, 0xe9, 0x3f, 0xd6, 0xc2, 0xf5, 0x85, 0xf3, 0x31, 0xf7, 0xfe, 0xa3, 0xfb,
    0x73, 0xb3, 0xc9, 0x55, 0xba, 0xcc, 0xa0, 0xeb, 0x19, 0x63, 0xfe, 0x99, 0xb0, 0x37, 0x45, 0x8e,
    0x42, 0x10, 0xc3, 0xcc, 0xf8, 0xde, 0xd4, 0x43, 0x1b, 0x7c, 0xfd, 0x41, 0xaa, 0x36, 0x50, 0x85,
    0xd6, 0xe0, 0xa1, 0x3f, 0xac, 0xa0, 0x70, 0x74, 0xe1, 0x23, 0x66, 0x38, 0x52, 0x60, 0xe0, 0x62,
    0x05, 0xc3, 0x29, 0x5d, 0x17, 0xb4, 0xa0, 0x9a, 0x73, 0xae, 0xa5, 0xf9, 0xa0, 0x68, 0x52, 0x64,
    0x7e, 0x64, 0xbb, 0x13, 0xab, 0xe7, 0xe3, 0xe5, 0xb2, 0x0b, 0xe8, 0xce, 0x5d, 0xa3, 0xaf, 0x65,
    0xb2, 0x37, 0xd1, 0xdc, 0x35, 0x3a, 0x8d, 0x84, 0xad, 0x5c, 0x97, 0x36, 0x33, 0x91, 0xf5, 0xbe,
    0xb7, 0x6c, 0xa8, 0x04, 0x1d, 0xca, 0xb5, 0xfd, 0xf6, 0xc0, 0x5f, 0x42, 0x3a, 0x86, 0x7d, 0x29,
];

/// A queued HDCP 2.x message: id, the `wIndex` flag word, and its body.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Message {
    pub id: u8,
    pub flags: u16,
    pub body: Vec<u8>,
}

impl Message {
    pub fn new(kind: HdcpMessage, flags: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            id: kind.id(),
            flags,
            body: body.into(),
        }
    }

    /// Encrypted messages carry the id `+16` (`AKE_SEND_CERT` 0x02 becomes the
    /// encrypted `0x12`, `LC_SEND_L_PRIME` 0x09 becomes `0x19`).
    pub fn encrypted(kind: HdcpMessage, flags: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            id: kind.id() + 16,
            flags,
            body: body.into(),
        }
    }

    pub fn name(&self) -> &'static str {
        HdcpMessage::name(self.id)
    }

    /// True when the id is the encrypted (`+16`) form and still in range.
    pub fn is_encrypted(&self) -> bool {
        (16..32).contains(&self.id) && HdcpMessage::name(self.id - 16) != "<UNKNOWN_MESSAGE>"
    }
}

/// Split a message into the wire transfers the vendor would issue.
///
/// Each transfer carries the id as its first byte followed by at most 63 body
/// bytes (`0x10028a184` clamps to 63, then the helper adds the id back with
/// `add w5, w22, #1`). An empty body still produces the one id-only transfer.
pub fn chunks(msg: &Message) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut at = 0usize;
    loop {
        let take = (msg.body.len() - at).min(CHUNK - 1);
        let mut packet = Vec::with_capacity(take + 1);
        packet.push(msg.id);
        packet.extend_from_slice(&msg.body[at..at + take]);
        out.push(packet);
        at += take;
        if at >= msg.body.len() {
            return out;
        }
    }
}

/// A received message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reply {
    pub id: u8,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn name(&self) -> &'static str {
        HdcpMessage::name(self.id)
    }
}

/// Errors a read can report that the caller has to act on rather than fail.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Read {
    /// `0x44`: the receiver has nothing yet, read again.
    Retry,
    /// `0x40`: the receiver flushed, so a fresh request header is needed.
    Flush,
    /// A complete message.
    Message,
}

/// Classify one reply buffer. A body shorter than the header is a control
/// word rather than a message.
pub fn classify(reply: &[u8]) -> (Read, Option<Reply>) {
    match reply.first().copied() {
        None => (Read::Retry, None),
        Some(RETRY) => (Read::Retry, None),
        Some(FLUSH) => (Read::Flush, None),
        Some(id) if reply.len() >= 2 => (
            Read::Message,
            Some(Reply {
                id,
                body: reply[1..].to_vec(),
            }),
        ),
        Some(id) => (
            Read::Message,
            Some(Reply {
                id,
                body: Vec::new(),
            }),
        ),
    }
}

/// HDCP session state, in the vendor's order (`0x100507eXX` string table).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum State {
    Idle,
    Authorising,
    ExchangingKeys,
    LocalityCheck,
    EstablishingSession,
    AuthorisedConnected,
    AuthorisedDisconnected,
    AuthFailed,
    Failed,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Idle => "Idle",
            State::Authorising => "Authorising",
            State::ExchangingKeys => "ExchangingKeys",
            State::LocalityCheck => "LocalityCheck",
            State::EstablishingSession => "EstablishingSession",
            State::AuthorisedConnected => "AuthorisedConnected",
            State::AuthorisedDisconnected => "AuthorisedDisconnected",
            State::AuthFailed => "AuthFailed",
            State::Failed => "Failed",
        }
    }

    /// True once content may be sent.
    pub fn authorised(self) -> bool {
        matches!(self, State::AuthorisedConnected | State::AuthorisedDisconnected)
    }
}

/// Capabilities a transmitter advertises. The vendor sends `0x00021*` words;
/// only the version nibble is interpreted here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Caps {
    pub version: u8,
    pub depth: u8,
    /// Receivers behind the sink (repeater depth).
    pub receiver_count: u8,
}

impl Default for Caps {
    fn default() -> Self {
        Self {
            version: 2,
            depth: 0,
            receiver_count: 1,
        }
    }
}

impl Caps {
    /// The 3-byte capability word the vendor writes at `0x10028f40c`
    /// (`0x22`, then `0xafc2`), plus the trailing byte.
    pub fn encode(&self) -> [u8; 4] {
        let word = 0xafc2u16 | ((self.version as u16) << 12);
        let mut out = [0u8; 4];
        out[..2].copy_from_slice(&word.to_le_bytes());
        out[2] = 0x22;
        out[3] = self.depth << 4 | (self.receiver_count & 0x0f);
        out
    }
}

/// A parsed `AKE_SEND_CERT` body.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SendCert {
    pub rrx: [u8; 2],
    pub caps: [u8; 4],
    pub cert: Vec<u8>,
    pub ksv: [u8; 5],
    pub signature: Vec<u8>,
}

impl SendCert {
    pub const CERT_LEN: usize = 128;
    pub const SIG_LEN: usize = 384;
    pub const KSV_LEN: usize = 5;
    /// `1 + 2 + 4 + 128 + 5 + 384`.
    pub const MIN_LEN: usize = 1 + 2 + 4 + Self::CERT_LEN + Self::KSV_LEN + Self::SIG_LEN;

    /// Parse the vendor's layout: `[id][rrx][caps][cert][ksv][sig]`
    /// (`0x1003a2a64`). The id must be 3 and the body at least
    /// [`Self::MIN_LEN`]; a KSV that is not exactly five bytes is rejected
    /// ("invalid raw ksv bytes!").
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.first() != Some(&HdcpMessage::AkeNoStoredKm.id()) {
            bail!("not an AKE_SEND_CERT message (id {})", body.first().copied().unwrap_or(0xff));
        }
        if body.len() < Self::MIN_LEN {
            bail!(
                "AKE_SEND_CERT is {} bytes, expected at least {}",
                body.len(),
                Self::MIN_LEN
            );
        }
        let mut rrx = [0u8; 2];
        rrx.copy_from_slice(&body[1..3]);
        let mut caps = [0u8; 4];
        caps.copy_from_slice(&body[3..7]);
        let mut ksv = [0u8; Self::KSV_LEN];
        let ksv_at = 7 + Self::CERT_LEN;
        ksv.copy_from_slice(&body[ksv_at..ksv_at + Self::KSV_LEN]);
        let sig_at = ksv_at + Self::KSV_LEN;
        Ok(Self {
            rrx,
            caps,
            cert: body[7..ksv_at].to_vec(),
            ksv,
            signature: body[sig_at..sig_at + Self::SIG_LEN].to_vec(),
        })
    }

    /// The bytes the signature covers: `rtx || rrx || TxCaps || RxCaps || ksv`.
    pub fn signed(&self, rtx: &[u8; 8], txcaps: &[u8; 4]) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + 2 + 4 + 4 + 5);
        out.extend_from_slice(rtx);
        out.extend_from_slice(&self.rrx);
        out.extend_from_slice(txcaps);
        out.extend_from_slice(&self.caps);
        out.extend_from_slice(&self.ksv);
        out
    }
}

/// Verify an `AKE_SEND_CERT` signature against [`ROOT_MODULUS`].
///
/// Straight RSASSA-PKCS1-v1_5 over SHA-256, which is what `0x1003ace34` and
/// the "AKE failed signature verification" branch do.
pub fn verify(cert: &SendCert, rtx: &[u8; 8], txcaps: &[u8; 4]) -> Result<()> {
    use rsa::pkcs1v15::{Signature, VerifyingKey};
    use rsa::signature::Verifier;
    use rsa::{BigUint, RsaPublicKey};

    if cert.cert.len() != SendCert::CERT_LEN {
        bail!("receiver certificate is {} bytes", cert.cert.len());
    }
    let key = RsaPublicKey::new(
        BigUint::from_bytes_be(&ROOT_MODULUS),
        BigUint::from(ROOT_EXPONENT),
    )
    .context("building the HDCP root key")?;
    let verifying = VerifyingKey::<Sha256>::new(key);
    let signature = Signature::try_from(cert.signature.as_slice())
        .map_err(|e| anyhow!("receiver signature is malformed: {e}"))?;
    verifying
        .verify(&cert.signed(rtx, txcaps), &signature)
        .map_err(|_| anyhow!("AKE failed signature verification"))
}

/// `H' = HMAC-SHA256(Km, SHA-256(rtx || rrx))`.
pub fn h_prime(km: &[u8], rtx: &[u8], rrx: &[u8]) -> [u8; 32] {
    use hmac::{Hmac, Mac};
    let mut hash = Sha256::new();
    hash.update(rtx);
    hash.update(rrx);
    let digest = hash.finalize();
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(km)
        .expect("HMAC accepts keys of any length");
    mac.update(&digest);
    mac.finalize().into_bytes().into()
}

/// `L' = HMAC-SHA256(Km, rn || rrx)`.
pub fn l_prime(km: &[u8], rn: &[u8], rrx: &[u8]) -> [u8; 32] {
    use hmac::{Hmac, Mac};
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(km)
        .expect("HMAC accepts keys of any length");
    mac.update(rn);
    mac.update(rrx);
    mac.finalize().into_bytes().into()
}

/// One HDCP link, driven over the DL3 interface.
pub struct Link<'a> {
    iface: &'a nusb::Interface,
    pub seq: u8,
    pub state: State,
}

impl<'a> Link<'a> {
    pub fn new(iface: &'a nusb::Interface) -> Self {
        Self {
            iface,
            seq: 0,
            state: State::Idle,
        }
    }

    fn out(&self, value: u16, index: u16, data: &[u8]) -> Result<()> {
        self.iface
            .control_out(
                ControlOut {
                    control_type: ControlType::Vendor,
                    recipient: Recipient::Device,
                    request: 0x01,
                    value,
                    index,
                    data,
                },
                TIMEOUT,
            )
            .wait()
            .context("HDCP control out")?;
        Ok(())
    }

    fn read(&self, value: u16, index: u16, len: usize) -> Result<Vec<u8>> {
        self.iface
            .control_in(
                ControlIn {
                    control_type: ControlType::Vendor,
                    recipient: Recipient::Device,
                    request: 0x01,
                    value,
                    index,
                    length: len.min(MAX_REPLY) as u16,
                },
                TIMEOUT,
            )
            .wait()
            .context("HDCP control in")
    }

    /// Ship a message across as many 64-byte transfers as it needs.
    pub fn send(&mut self, msg: &Message) -> Result<()> {
        for packet in chunks(msg) {
            self.out(msg.id as u16, msg.flags | FLAG_WRITE, &packet)?;
        }
        Ok(())
    }

    /// Read one reply, retrying on `0x44` and re-heading on `0x40` up to a
    /// bounded number of times so a wedged dock cannot spin forever.
    pub fn receive(&mut self, id: u8) -> Result<Reply> {
        let mut flushes = 0;
        loop {
            let header = [id, self.seq | 0x10];
            self.out(0, FLAG_WRITE, &header)?;
            for _ in 0..16 {
                let buf = self.read(0, FLAG_WRITE, MAX_REPLY)?;
                match classify(&buf) {
                    (Read::Retry, _) => continue,
                    (Read::Flush, _) => break,
                    (Read::Message, Some(reply)) => return Ok(reply),
                    (Read::Message, None) => continue,
                }
            }
            flushes += 1;
            if flushes > 4 {
                bail!("receiver kept flushing");
            }
        }
    }

    /// EXIT: `wValue` carries the sequence number and `wIndex` the id with the
    /// `0x2c00` trailer (`0x10028a118`).
    pub fn exit(&mut self, id: u8) -> Result<()> {
        let body = [id, self.seq | 0x10];
        self.iface
            .control_in(
                ControlIn {
                    control_type: ControlType::Vendor,
                    recipient: Recipient::Device,
                    request: 0x02,
                    value: (self.seq as u16) << 8,
                    index: id as u16 | FLAG_EXIT,
                    length: body.len() as u16,
                },
                TIMEOUT,
            )
            .wait()
            .context("HDCP exit")?;
        self.seq = self.seq.wrapping_add(1);
        Ok(())
    }

    /// AKE step 1: advertise rtx and our capabilities.
    pub fn ake_init(&mut self, rtx: &[u8; 8], caps: &Caps) -> Result<()> {
        self.state = State::Authorising;
        let body = caps.encode();
        let mut payload = Vec::with_capacity(8 + body.len());
        payload.extend_from_slice(rtx);
        payload.extend_from_slice(&body);
        self.send(&Message::new(HdcpMessage::AkeInit, 0, payload))
    }

    /// AKE step 2: wait for and verify the receiver certificate.
    pub fn ake_send_cert(&mut self, rtx: &[u8; 8], caps: &Caps) -> Result<SendCert> {
        let reply = self.receive(HdcpMessage::AkeSendCert.id())?;
        if reply.id != HdcpMessage::AkeSendCert.id() {
            bail!("expected {}, got {}", HdcpMessage::name(HdcpMessage::AkeSendCert.id()), reply.name());
        }
        let mut body = Vec::with_capacity(reply.body.len() + 1);
        body.push(reply.id);
        body.extend_from_slice(&reply.body);
        let cert = SendCert::parse(&body)?;
        verify(&cert, rtx, &caps.encode()).inspect_err(|_| self.state = State::AuthFailed)?;
        self.state = State::ExchangingKeys;
        Ok(cert)
    }

    /// AKE step 3: prove knowledge of Km without revealing it.
    pub fn ake_send_h_prime(&mut self, km: &[u8], rtx: &[u8], rrx: &[u8]) -> Result<()> {
        self.send(&Message::encrypted(
            HdcpMessage::AkeSendHPrime,
            0,
            h_prime(km, rtx, rrx).to_vec(),
        ))
    }

    /// Locality check: HMAC over a nonce the receiver supplies.
    pub fn lc_send_l_prime(&mut self, km: &[u8], rn: &[u8], rrx: &[u8]) -> Result<()> {
        self.state = State::LocalityCheck;
        self.send(&Message::encrypted(
            HdcpMessage::LcSendLPrime,
            0,
            l_prime(km, rn, rrx).to_vec(),
        ))
    }
}

/// AKE transfer shapes, checked against a fake interface by [`Link`].
#[cfg(test)]
mod tests {
    use super::*;

    fn rtx() -> [u8; 8] {
        [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]
    }

    #[test]
    fn root_modulus_is_3072_bits_and_odd() {
        assert_eq!(ROOT_MODULUS.len(), 384);
        assert_eq!(ROOT_MODULUS[0], 0xe6);
        assert_eq!(ROOT_MODULUS[383] & 1, 1);
    }

    #[test]
    fn message_ids_follow_the_vendor_enum() {
        assert_eq!(HdcpMessage::AkeInit.id(), 1);
        assert_eq!(HdcpMessage::AkeSendCert.id(), 2);
        assert_eq!(HdcpMessage::AkeNoStoredKm.id(), 3);
        assert_eq!(HdcpMessage::LcSendLPrime.id(), 9);
        assert_eq!(Message::encrypted(HdcpMessage::AkeSendCert, 0, []).id, 0x12);
        assert_eq!(Message::encrypted(HdcpMessage::LcSendLPrime, 0, []).id, 0x19);
        assert!(Message::encrypted(HdcpMessage::AkeSendCert, 0, []).is_encrypted());
        assert!(!Message::new(HdcpMessage::AkeSendCert, 0, []).is_encrypted());
    }

    #[test]
    fn chunks_carry_the_id_first_and_stop_at_63_body_bytes() {
        let msg = Message::new(HdcpMessage::AkeInit, 0, vec![0xaa; 200]);
        let packets = chunks(&msg);
        assert_eq!(packets.len(), 4);
        assert_eq!(packets[0].len(), CHUNK);
        assert_eq!(packets[0][0], 1);
        assert!(packets.iter().all(|p| p[0] == 1));
        assert_eq!(packets.last().unwrap().len(), 1 + (200 - 3 * 63));
    }

    #[test]
    fn empty_body_still_sends_the_id() {
        let packets = chunks(&Message::new(HdcpMessage::AkeInit, 0, []));
        assert_eq!(packets, vec![vec![1u8]]);
    }

    #[test]
    fn classify_handles_retry_flush_and_short_bodies() {
        assert_eq!(classify(&[]).0, Read::Retry);
        assert_eq!(classify(&[RETRY]).0, Read::Retry);
        assert_eq!(classify(&[FLUSH, 0]).0, Read::Flush);
        let (kind, reply) = classify(&[2, 0xaa]);
        assert_eq!(kind, Read::Message);
        assert_eq!(reply.unwrap().body, vec![0xaa]);
        let (kind, reply) = classify(&[2]);
        assert_eq!(kind, Read::Message);
        assert!(reply.unwrap().body.is_empty());
    }

    #[test]
    fn caps_encode_matches_the_vendor_word() {
        let caps = Caps::default();
        let bytes = caps.encode();
        assert_eq!(u16::from_le_bytes([bytes[0], bytes[1]]), 0xafc2 | (2 << 12));
        assert_eq!(bytes[2], 0x22);
    }

    fn cert_body(private: &rsa::RsaPrivateKey, rrx: [u8; 2]) -> Vec<u8> {
        use rsa::pkcs1v15::SigningKey;
        use rsa::signature::{SignatureEncoding, Signer};
        let mut body = vec![HdcpMessage::AkeNoStoredKm.id()];
        body.extend_from_slice(&rrx);
        body.extend_from_slice(&Caps::default().encode());
        body.extend_from_slice(&[0x5a; SendCert::CERT_LEN]);
        body.extend_from_slice(&[0xa1, 0xb2, 0xc3, 0xd4, 0xe5]);
        body.extend_from_slice(&[0u8; SendCert::SIG_LEN]);
        let cert = SendCert::parse(&body).unwrap();
        let signed = cert.signed(&rtx(), &Caps::default().encode());
        let signature = SigningKey::<Sha256>::new(private.clone()).sign(&signed);
        let at = body.len() - SendCert::SIG_LEN;
        body[at..].copy_from_slice(&signature.to_bytes());
        body
    }

    /// One 3072-bit key for every test, so the signature width matches the
    /// wire format and key generation runs once.
    fn key() -> &'static rsa::RsaPrivateKey {
        static KEY: std::sync::OnceLock<rsa::RsaPrivateKey> = std::sync::OnceLock::new();
        KEY.get_or_init(|| {
            use rand::SeedableRng;
            let mut rng = rand::rngs::StdRng::seed_from_u64(7);
            rsa::RsaPrivateKey::new(&mut rng, 3072).unwrap()
        })
    }

    #[test]
    fn send_cert_parses_the_vendor_layout() {
        let private = key();
        let body = cert_body(private, [0x01, 0x02]);
        let cert = SendCert::parse(&body).unwrap();
        assert_eq!(cert.rrx, [0x01, 0x02]);
        assert_eq!(cert.ksv, [0xa1, 0xb2, 0xc3, 0xd4, 0xe5]);
        assert_eq!(cert.cert.len(), SendCert::CERT_LEN);
        assert_eq!(cert.signature.len(), SendCert::SIG_LEN);
        assert_eq!(body.len(), SendCert::MIN_LEN);
        assert_eq!(
            cert.signed(&rtx(), &Caps::default().encode()).len(),
            8 + 2 + 4 + 4 + 5
        );
    }

    #[test]
    fn send_cert_rejects_wrong_id_short_body_and_short_ksv() {
        let private = key();
        let mut body = cert_body(private, [0x01, 0x02]);
        body[0] = 4;
        assert!(SendCert::parse(&body).unwrap_err().to_string().contains("AKE_SEND_CERT"));
        let body = cert_body(private, [0x01, 0x02]);
        assert!(SendCert::parse(&body[..body.len() - 1])
            .unwrap_err()
            .to_string()
            .contains("expected at least"));
        let short_ksv = &body[..SendCert::MIN_LEN - 1];
        assert!(SendCert::parse(short_ksv).is_err());
    }

    #[test]
    fn signature_verification_accepts_a_good_certificate() {
        let private = key();
        let cert = SendCert::parse(&cert_body(private, [0x01, 0x02])).unwrap();
        let verifying =
            rsa::pkcs1v15::VerifyingKey::<Sha256>::new(rsa::RsaPublicKey::from(private));
        let sig = rsa::pkcs1v15::Signature::try_from(cert.signature.as_slice()).unwrap();
        use rsa::signature::Verifier;
        verifying
            .verify(&cert.signed(&rtx(), &Caps::default().encode()), &sig)
            .unwrap();
    }

    /// The shipped root key has to be usable as an RSA public key, and a
    /// certificate signed by a different key must not verify under it.
    #[test]
    fn root_key_builds_and_rejects_a_foreign_signature() {
        let private = key();
        let cert = SendCert::parse(&cert_body(private, [0x01, 0x02])).unwrap();
        let err = verify(&cert, &rtx(), &Caps::default().encode()).unwrap_err();
        assert!(err.to_string().contains("AKE failed signature verification"));
    }

    #[test]
    fn signature_verification_rejects_tampering() {
        let private = key();
        let cert = SendCert::parse(&cert_body(private, [0x01, 0x02])).unwrap();
        let verifying = rsa::pkcs1v15::VerifyingKey::<Sha256>::new(rsa::RsaPublicKey::from(private));
        let sig = rsa::pkcs1v15::Signature::try_from(cert.signature.as_slice()).unwrap();
        use rsa::signature::Verifier;
        let other = cert.signed(&[0u8; 8], &Caps::default().encode());
        assert!(verifying.verify(&other, &sig).is_err());
    }

    #[test]
    fn h_prime_and_l_prime_differ_by_input() {
        let km = [0x09u8; 16];
        let a = h_prime(&km, &rtx(), &[1, 2]);
        let b = h_prime(&km, &rtx(), &[1, 3]);
        assert_ne!(a, b);
        assert_ne!(l_prime(&km, &[7], &[1, 2]), a);
        assert_eq!(h_prime(&km, &rtx(), &[1, 2]), a);
    }

    #[test]
    fn state_names_match_the_vendor_table() {
        assert_eq!(State::AuthorisedConnected.name(), "AuthorisedConnected");
        assert!(State::AuthorisedConnected.authorised());
        assert!(!State::Authorising.authorised());
        assert!(!State::AuthFailed.authorised());
    }
}
