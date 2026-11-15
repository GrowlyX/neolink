# DisplayLink parity

Honest matrix, measured against DisplayLink Manager 17.1.56 on macOS 27.0.1.

## Works

| Area | Notes |
| --- | --- |
| Virtual display creation | `CGVirtualDisplay` + `applySettings:`, the same object the vendor agent holds per dock |
| Mode list, HiDPI, rotation | Re-applied in place; re-asserted after every CG reconfiguration |
| Arrangement + persistence | UUID-keyed profile, tidy, atomic save |
| Vendor agent detection/start/stop | `launchctl asuser` + `open`, version from `Info.plist` |
| Vendor XPC read | `getDisplaysWithCompletionHandler:` decoded via an `NSSecureCoding` shim |
| Vendor enable / brightness / contrast / flip | `setDisplay*WithPersistentDisplayId:…` |

## Missing or partial

| Area | Gap |
| --- | --- |
| USB transport (DL3, HDCP 2.x, EDID writes) | Not reimplemented. Real docks still need the vendor agent running; `nusb` is a dependency but unused |
| Display stream | No `CGDisplayStream`/ScreenCaptureKit pipeline — a virtual display is composited locally, so nothing is encoded to a dock |
| Vendor notifications | `enableNotifications` is exported but `displayBrightnessChanged…`/`displaysChanged` are not consumed; the UI polls every 1.5 s |
| Brightness/contrast in the UI | Read-only meters; no slider writes them back to `vendor_set` |
| Refresh-rate selection | Modes carry Hz, but virtual modes are fixed at 60 and there is no picker |
| Mirror persistence | `mirror_of` is written as `None` in `save()` and `tidy()` |
| Multi-display edge cases | Mirror/arrangement restore matches by UUID only; unplugged docks are skipped silently |
| Packaging | No `.app` bundle, icon, launch agent or notarisation; `cargo run` only |

## Test coverage

No automated tests yet; verification is `neolinkctl probe`, `NEOLINK_DEMO=2`,
and `neolinkctl list` against a live session.
