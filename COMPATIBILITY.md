# DisplayLink parity

Honest matrix against DisplayLink Manager 17.1.56 on macOS 27.0.1. No dock was
attached while the USB half was written, so the transport is RE-derived and
**unvalidated on hardware**. Everything in the first table was exercised locally.

## Works

| Area | Notes |
| --- | --- |
| Vendor package acquisition | Pinned URL + SHA-256, cached under Application Support, expanded on demand, installed with `installer` in one admin prompt. No Synaptics bytes are redistributed |
| Virtual display creation | `CGVirtualDisplay` + `applySettings:`, the same object the vendor agent holds per dock |
| Mode list, HiDPI, rotation | Re-applied in place; re-asserted after every CG reconfiguration |
| Arrangement + persistence | UUID-keyed profile, tidy, atomic save |
| Mirror persistence | `mirror_of` resolved to the master's UUID in `save()`/`tidy()` and restored by `apply()` |
| Vendor agent detection/start/stop | `launchctl asuser` + `open`, version from `Info.plist` |
| Vendor XPC read | `getDisplaysWithCompletionHandler:` decoded via an `NSSecureCoding` shim; `reachable()` pings it rather than grepping `launchctl` |
| Vendor enable / brightness / contrast / flip | `setDisplay*WithPersistentDisplayId:…`; setters report failure only on a real XPC error or timeout, since the vendor replies with `nil` |
| Agent change notifications | `enableNotifications` + a real `DisplayLinkXpcClientNotificationProtocolV2` exported object; the UI drains a channel and only polls as a fallback |
| Bundling | `scripts/bundle.sh` builds a universal, ad-hoc signed `.app` with icon and `Info.plist`; optional LaunchAgent for launch-at-login |

## Missing or partial

| Area | Gap |
| --- | --- |
| Live validation of the USB transport | No dock in this session. `find_endpoints`, the reset echo and the status checksum are derived from the disassembly, not measured |
| Frame path | `Transport::frame` ships tile fragments with a caller-supplied pixel source; no capture→encode pipeline is wired, so a physical dock shows nothing yet without the vendor agent |
| HDCP 2.x | Not implemented; only mentioned in the failure strings we mirror |
| EDID override | Read/write plumbing exists (`edid`, `edid-write`). The `.spkg` container, fragment splice and frame table are implemented (`neolinkctl pack F --out D`), but the frame payload itself still decodes to high-entropy bytes with no EDID magic, so there is no per-dock EDID yet |
| Refresh-rate selection | Modes carry Hz, but virtual modes are fixed at 60 and there is no picker |
| Privileged helper | The vendor's `SMAppService` helper is used as-is; we have no helper of our own |
| Notarisation | Ad-hoc signature only; no Developer ID or `notarytool` run |
| Multi-display edge cases | Mirror/arrangement restore matches by UUID only; unplugged docks are skipped silently |

## Test coverage

26 unit tests cover the DL3 header/fragment/checksum rules, EDID validation, the
SPKG container, fragment splice and frame table against synthesised and all four
real packs, profile mirror round-trip and tidy, and the vendor pin/login-item
helpers. Manual checks:
`neolinkctl probe`, `neolinkctl vendor status|fetch|expand|packs`,
`NEOLINK_DEMO=2`, and launching the signed bundle.
