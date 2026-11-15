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
one per dock family: firmware plus a synthesised EDID/mode set. They are *not*
redistributable, which is exactly why Neolink fetches the vendor package at
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
body index, `PT` payload. `FS` is the fragment marker the device path looks for;
each record body in the shipped packs opens with it. Records are stored out of
order and the `u16 index` is the reassembly key.

A `PT` body is `u16 len`, then the first `len` bytes of the stream, then its
28-byte trailer. A record body is a run of fragments, each `u8 kind`, `u16 len`,
`len` bytes, closed by its own trailer. Concatenating the `PT` bytes, the bytes
after the table and the record fragments *in index order* yields the stream,
which is covered by `spkg::Pack::stream()`.

The stream is a frame table:

```
u8  kind
u16 len            counts the 10-byte preamble plus the data
u8  scratch[7]
u8  preamble[10]
u8  data[len - 10]
```

`kind` is per-fragment noise, not a type. The last frame of every shipped pack is
cut by the `FRAGMENTED_SPKG` path: its tail lives in the record and the frame
parses cleanly once spliced. `spkg::Pack::frames()` does that splice and reports
the remainder.

The data itself is not compressed. Apple `compression_decode_buffer` with
`COMPRESSION_ZLIB` accepts it and returns it verbatim, which is why earlier
notes claimed a zlib stream: the "successful decode" was a copy. `flate2`
rejects it at every offset. What the reassembled bytes mean per dock is still
unknown — they are high-entropy (≈7.99 B/byte) and contain no EDID magic, so the
image format on top of the frames is the open item. The container, the splice
and the frame table are all real and enforced by tests against the four vended
packs.

## Vendor package pin

URL, size and digest are constants in `vendor.rs`; the hash is checked before
`pkgutil` ever runs, so a changed or truncated download cannot reach `installer`.
`installer -pkg … -target /` via `osascript … with administrator privileges`
keeps signature checks and receipts with macOS instead of reimplementing them.
