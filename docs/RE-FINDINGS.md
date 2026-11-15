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
