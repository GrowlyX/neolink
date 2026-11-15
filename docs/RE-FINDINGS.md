# DisplayLink Manager 17.1.56 — RE findings

Source: official `DisplayLink Manager Graphics Connectivity17.1-EXE.pkg`
(Synaptics, built 2026-11-05, `version="17.1.56"`), unpacked with
`pkgutil --expand-full`. Payload is `pbzx` (xar → pbzx → cpio).

## Bundle layout

```
DisplayLink Manager.app/Contents/
  MacOS/DisplayLinkUserAgent        14.9 MB  x86_64+arm64, Swift + C++ (dl::*)
  MacOS/DisplayLinkXpcService      425 KB    Swift only — the sandbox seam
  MacOS/DisplayLinkPrivilegedHelper.app       SMAppService helper
  XPCServices/FileVaultDetector.xpc
  Resources/{ella,navarro,ridge,firefly}-*-release.spkg   per-dock firmware/EDID blobs
  Library/LaunchAgents/com.displaylink.XpcService.plist
  Library/LaunchDaemons/com.displaylink.DisplayLinkManager.PrivilegedHelper.plist
```

`LSUIElement = true` (menu-bar-only agent), `LSMinimumSystemVersion = 14.0`.
No kexts, no DriverKit extension: since 15.x the transport is entirely user-space
(`IOUSBLib` / `IOUSBDevice` symbols), and frames are shipped over USB from a
`CGVirtualDisplay`-backed capture pipeline.

## The XPC seam (most useful part)

`DisplayLinkXpcService` vends three Mach services
(`com.displaylink.XpcService.V2`, `.internal`, `com.displaylink.XpcService`).

Protocols recovered from `__objc_classname` / `__TEXT,__objc_methname`:

- `DisplayLinkXpcServiceProtocolV2`
- `DisplayLinkXpcDriverProtocol`
- `DisplayLinkXpcInternalServiceProtocol`
- `DisplayLinkXpcClientNotificationProtocolV2`

Selectors (exact, from the ObjC method list):

```
- getDisplaysWithCompletionHandler:
- setDisplayEnabledWithPersistentDisplayId:enabled:completionHandler:
- setDisplayBrightnessWithPersistentDisplayId:brightness:completionHandler:
- setDisplayContrastWithPersistentDisplayId:contrast:completionHandler:
- setDisplayFlipWithPersistentDisplayId:flip:completionHandler:
- enableNotifications
```
Driver-bound notifications: `notifyDisplaysChangedWithDisplays:`,
`displayBrightnessChangedWithPersistentDisplayId:brightness:error:`,
`displayContrastChanged…:contrast:error:`, `displayFlipApplied…:flip:error:`.

Codable DTOs: `DisplayLinkDisplayV2` (`persistentDisplayId`, `name`, `CGID`,
`rotated`, `brightness`, `contrast`, `isEnabled`, `isFlipped`, `usbVid`,
`usbPid`, `usbSerialNumber`) wrapped by `DisplaysWithStatusCodeV2`.

Takeaway: the agent — not the daemon — owns display state, and it exposes an
XPC control surface keyed by *persistent* display id. Reimplementing the
transport (DL3, HDCP 2.x) is not required to build a compatible manager UI.

## Display creation

The agent links `CGVirtualDisplay` and holds one per attached dock
(`v16@?0@"CGVirtualDisplay"8`). Private CoreGraphics classes, verified live on
macOS 27.0.1 by `objc_getClass` + `method_copy`:

- `CGVirtualDisplayDescriptor` — `vendorID`, `productID`, `serialNumber`,
  `serialNum`, `name`, `sizeInMillimeters`, `maxPixelsWide/High`,
  `red/green/bluePrimary`, `whitePoint`, `queue`, `terminationHandler`
- `CGVirtualDisplay` — `initWithDescriptor:`, `applySettings:`, `displayID`,
  `hiDPI`, `rotation`, `modes`
- `CGVirtualDisplaySettings` — `modes`, `hiDPI`, `rotation`, `isReference`,
  `refreshDeadline`
- `CGVirtualDisplayMode` — `initWithWidth:height:refreshRate:[transferFunction:]`

`applySettings:` returns `BOOL` and a `displayID` becomes valid immediately.
Verified end-to-end from Rust: a display appeared in `CGGetOnlineDisplayList`
and stayed online. **This is the whole engine** for a non-USB virtual display.

## Private/undocumented but load-bearing

- `CoreDisplay_DisplayCreateInfoDictionary` → localised `DisplayProductName`
  (used by the existing `displays.rs`).
- `CGDisplayStream*` + ScreenCaptureKit (`SCStream`, `getShareableContent…`)
  both present; the agent switches recorders per OS/entitlement.
- `IOSurface`-backed `SurfaceBackedTextureCache`, Metal `ShaderLibrary.metallib`
  — the agent encodes with Metal on GPU (`m_supportsGpuEncoding`).

## DL3 packet header

Validator `0x100103148`, builders `0x1001270bc` / `0x1001271d4` / `0x1001273c8`.
The builder stages a 20-byte header (a 16-byte `ldur`/`stur` pair plus a `u32`)
and hands exactly 20 bytes to the pipe, so header and payload are contiguous.

```
off  size  field
 0    1    tag: 0x01 probe | op<<5 | last<<4 | cont<<3 | first<<2 | stream
 1    1    channel (probe only, else 0)
 2    2    payload length, big endian
 4    1    same tag byte as offset 0
 5    1    fragment index, 1-based, bumped per continued fragment
 6    2    packet counter (little endian, wraps)
 8    1    tile rect x      (16-pixel units)
 9    1    tile rect y
10    1    tile rect x_end
11    1    tile rect y_end
12    4    fragment offset inside the rectangle (little endian)
16    4    frame tag carried from the caller
```

Validation: byte 0 must be `0x01` (and byte 1 must equal the negotiated channel)
or `0x40`; anything else is `Invalid TG header.` (`0x1004e7a7a`). `len` is
reversed from a `ldrh` (big endian) and must not exceed `*(u32*)(x8+0x14)`
(`Invalid TG length.`, `0x1004e7b91`).

The rectangle is inclusive: the builder writes `x>>4` and `(x+w+0xf)>>4`, so one
packet describes the damaged span and the receiver walks it. `first` is set only
while the fragment index is still 0; `last` is set when the fragment offset is
zero, i.e. on the first packet of each rectangle. A continuation sets `cont`,
increments the index, and moves the previous rect's `x_end` into `x` so the
receiver resumes mid-row.

## HDCP 2.x

Status ops are `Edid 0x00`, `EdidExtension 0x02`, `Mode 0x05`,
`Attributes 0x06` — `monitor_status` at `0x10028e42c` switches on the first
body byte and passes `Attributes` a `u16` argument. `MonitorStatus:
is_valid_checksum` (`0x10026c668`) validates the trailing u16 over the payload,
which is the additive sum `dl3::checksum` mirrors.

Message ids, in enum order (`Unknown HDCP2 message id` `0x100507eaf` is the
bounds check): NULL, AKE_INIT, AKE_SEND_CERT, AKE_NO_STORED_KM, AKE_STORED_KM,
AKE_SEND_RRX, AKE_SEND_H_PRIME, AKE_SEND_PAIRING_INFO, LC_INIT, LC_SEND_L_PRIME,
SKE_SEND_EKS, REPEATERAUTH_SEND_RECEIVERID_LIST, RTT_READY, RTT_CHALLENGE,
REPEATERAUTH_SEND_ACK, REPEATERAUTH_STREAM_MANAGE, REPEATERAUTH_STREAM_READY,
RECEIVER_AUTHSTATUS, AKE_TRANSMITTER_INFO, AKE_RECEIVER_INFO. The sink's HDCP
revision and monitor capability are what gate repeater auth
(`repeater auth received but rx is not HDCP2.2`, `… is not a repeater`).

## Failure strings worth mirroring in diagnostics

`STATUS_OP_EDID not supported on this device`, `Invalid length of raw EDID data`,
`repeater auth received but rx is not HDCP2.2`,
`Failed to reset receiver - no echoed DL3 reset packets received`.

## Licence note

`Contents/Resources/3rd_party_licences.txt` is vendor-supplied; the per-dock
`.spkg` EDID/firmware blobs ship with the app and are covered by the Synaptics
EULA. Neolink only *reads* them if present; it never redistributes them.

## The `.spkg` per-dock packs

Four packs ship in `Contents/Resources` (`ella`, `firefly`, `navarro`, `ridge`),
one per dock family. They carry **dock firmware** (the agent's own strings are
`Firmware size … is not a multiple of 4`, `Firmware sent`, `DfuGetFirmwareDescriptor`),
not EDID: no pack contains the `00 ff ff ff ff ff ff 00` EDID preamble. They are
not redistributable, which is exactly why Neolink fetches the vendor package at
runtime instead of embedding them (see `crates/neolink-core/src/vendor.rs`).

Container. The TLV walk is at `0x10020a848`; the three error strings are at
`0x10020acbc`, `0x10020aca4` and `0x10020aee8`, and the `cmp` against `'FS'`
that gates the fragmented path is at `0x10020a844`.

```
0   "ELLA"
4   u32 total file size == real length
8   u16 TLV table offset
10  u16 TLV table length (<= 0x200000)
..  records: { u16 index, u16 len, body, 28-byte trailer } on 16-byte boundaries
..  TLV table: { u16 type, u16 len, body, pad-if-odd }
..  payload, resuming on the next 256-byte boundary after the table
```

TLV types the agent maps: `RD` record name, `OW` owner, `RE` revision (8/9 B),
`RL` release level (1 B), `VE` version (3 B), `DA`/`TI` build date/time, `BD`
body index, `BC` declared frame count (u32 big-endian, then a build hash), `PT`
payload. `FS` is the fragment marker the device path looks for.

A `PT` body is `u16 len`, then the first `len` bytes of the stream, then its
28-byte trailer. A record body is a run of fragments, each `u8 kind`, `u16 len`,
`len` bytes, closed by its own trailer. The record header's `index` is the frame
ordinal its first fragment continues, and a fragment's declared `u16` length is
the *full* length even when the record body is cut short; the missing bytes
follow in the stream after the TLV table.

That last point decides the order. Concatenating `PT`, then the bytes after the
table, then the record payloads *in index order* (`spkg::Pack::stream()`) walks
the most frames on every shipped pack; splicing the records in at their byte
offset instead walks fewer. Records continue the **last** frames of the pack, not
frame 0, so the old "frame 0 is the truncated one" reading was wrong.

The stream is a frame table:

```
u8  header[9]      byte 0 is per-frame (a tag), byte 1 is per-frame too;
                   the rest is opaque
u16 total          big-endian, counts the whole frame including this header
u8  body[total - 11]
```

Byte 0 is *not* a type: it differs on every frame of every pack. `BC` gives the
frame count (`55` for navarro, which walks 54 whole frames plus the truncated
55th), so trailing padding can never be read as another frame.

The final frames are delta-truncated: each record's first fragment names the frame
it continues and carries as much of it as fits (ella 29 691 declared bytes over
two fragments, of which only 24 456 are stored; ridge 20 829 declared, 12 405
stored; firefly 11 265 of 11 265). `spkg::Pack::frames()` supplies the stored
bytes at the overrun and reports what no record could finish as `rest` — always
the trailing edge of the pack, never an interior hole. On the shipped packs that
is 703 B (ridge) to 10 148 B (navarro); the traffic itself is cut at the CDN, so
this is not a parser artifact. `BC` gives the intended count (55 for navarro, of
which 54 walk whole).

The body bytes are high-entropy (≈7.99 b/byte) and contain no EDID magic. Apple
`compression_decode_buffer` with `COMPRESSION_ZLIB` *does* return bytes from
several offsets inside the stream, but every attempt decodes to a different,
still-entropy-8 result and the binary links no zlib and imports no inflate, so
the frames are best treated as stored firmware, not a compressed archive. The
remaining open items are what the firmware image means to the dock (DFU framing)
and the record trailer digest (below), not the container. Neither blocks the display path.

### Record trailer digest — open, not blocking

Every record and `PT` carries 28 bytes after its body, on a 16-byte boundary.
**Status: unresolved and deliberately parked.** It gates firmware flashing only;
the display path never reads it, and the shipped packs are CDN-truncated anyway,
so the digest very likely covers bytes that are not in the file we have.

Ruled out (zero matches, over each record's bytes):

- sha1, sha224, sha256, sha384, sha512, sha512/224, sha512/256, md5,
  sha3-224/256/512, shake128/256, blake2b, blake2s.
- Inputs: record body, fragment payloads, `kind`+payload, declared-length
  prefixes (LE and BE), payloads zero-padded to their declared length,
  fragment header + padded payload, record header, `PT` body, whole-file
  prefixes up to the body, the file with the trailer zeroed, index/length
  prefixes in both orders, `ELLA`+body, reversed forms.
- Pack-level slices: head, tail blob, concatenations of head/blob/body/payloads
  in all orders, the frame blob and the frame data (headers stripped), and the
  stream spliced as `head + record data + tail[missing..]`.
- Per-fragment chaining (`h = H(h || frag)` for each fragment) and per-fragment
  digests concatenated then re-hashed.
- Structural checksums: CRC-32 and Adler-32 of the whole body and of seven even
  body slices against the trailer read as 7×u32 in both byte orders.

Two corrections worth recording so the sweep is not repeated wrongly:

1. **`blake2b`/`blake2s` must be constructed with `digest_size=28`, not
   truncated.** `hashlib.blake2b(data, digest_size=28)` is a different function
   from `hashlib.blake2b(data).digest()[:28]`; an earlier sweep tested only the
   truncation. Both forms, for every digest size 20–40, now also fail.
2. **The earlier IV addresses in this file were off by one table.**
   `0x10047b7e0` is the SHA-384 IV; the tables in the arm64 slice are
   SHA-512 IV `0x47b7a0`, SHA-256 IV `0x47b820`, SHA-224 IV `0x47b840`, MD5 IV
   `0x47b870`, SHA-256 K `0x47cb9c`. That is why the first `adrp`/`add` scan
   missed: it looked for `+0x7e0` where SHA-224 is `+0x840`. Both inlined
   initialisers do exist and are reachable (`0x100254210` sets the SHA-224 IV,
   `0x1002541b0` the SHA-256 IV), so the hasher is inlined/static as suspected.

What is still not located is the *comparison* site, so the exact input is
unproven rather than disproven. `spkg::digests_ok()` reports SHA-224 agreement
without enforcing it, and `neolinkctl pack` prints the trailer but does not
treat a mismatch as an error. Reopen this only when implementing DFU flashing.

## Vendor package pin

URL, size and digest are constants in `vendor.rs`; the hash is checked before
`pkgutil` ever runs, so a changed or truncated download cannot reach `installer`.
`installer -pkg … -target /` via `osascript … with administrator privileges`
keeps signature checks and receipts with macOS instead of reimplementing them.
