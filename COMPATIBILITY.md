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
| Login-window agent | `neolinkctl loginscreen on` installs a `LoginWindow` agent modelled on the vendor's `LoginScreenExt.pkg` (one admin prompt) |
| Bundling | `scripts/bundle.sh` builds a universal, ad-hoc signed `.app` with icon and `Info.plist`; optional LaunchAgent for launch-at-login |
| Screen capture | `displays::grab` reads a display through `CGDisplayCreateImage` and hands `Transport::frame` a strided BGRA buffer |

## Missing or partial

| Area | Gap |
| --- | --- |
| Live validation of the USB transport | No dock in this session. `find_endpoints`, the reset echo, the 20-byte header and the status checksum are all derived from the disassembly, never measured on a device |
| Frame encoding | `Transport::frame` ships the recovered 20-byte header (BE length, inclusive tile rect, fragment index and offset, `first`/`last`/`cont`) from a `CGDisplayCreateImage` capture, so capture→USB is wired. `Transport::frame_dirty` adds a 16×16 tile damage tracker (`diff::DirtyTracker`): an unchanged frame sends nothing, a moving window sends only its tiles, and damage past 60% of the frame falls back to one full-frame rect. `--codec h264` encodes each damage rect through VideoToolbox (`h264::EncoderPool`, hardware requested, intra-only): every rect is a self-contained IDR with SPS/PPS in band, odd rects are padded one pixel (coded size is always even) while the packet keeps the true rect, and the Annex-B bitstream replaces the BGRA payload. **Raw is still the default and the only format ever measured** — the 20-byte header has no codec/format field (every byte is accounted for, and the low two tag bits are the carried-over `stream` nibble, not a selector), so the receiver's format contract is out of band and unverified. Validation for the encoded path is an encode→decode round-trip plus payload assertions in unit tests, not a dock |
| HDCP 2.x | Status surface only: `neolinkctl hdcp` reads `Attributes`, decodes protection state and the sink's revision, and lists the 20 message ids. The AKE/LC/SKE handshake itself is not implemented, so HDCP-protected content will not play on the dock |
| Per-dock EDID | `dl3::dock_edid(family, w, h)` derives a stable, family-specific block (product code and serial hash from the family name) instead of one shared template, and `neolinkctl edid --dock NAME` emits it. It is still a synthesised block, not the vendor's per-dock data |
| `.spkg` payload | Contents, fragment splice and frame table are implemented and checked against all four shipped packs (`neolinkctl pack F --out D`), and the payload is confirmed as dock firmware, not EDID. The 28-byte record trailer digest is **open and not blocking**: it only gates firmware flashing, the display path never reads it, and the shipped packs are CDN-truncated so it probably covers bytes we do not have. Ruled out: sha1/224/256/384/512, sha512/224, sha512/256, md5, sha3-224/256/512, shake, blake2b/s (including `digest_size=28`, which is not a truncated digest), CRC-32/Adler-32, per-fragment chaining, and head/blob/body orderings. DFU framing around the bodies is likewise not implemented |
| Privileged helper | The vendor's `SMAppService` helper is used as-is; we have no helper of our own |
| Login-screen extension semantics | Our agent re-runs the whole app at the login window; the vendor's `LoginScreenExt.pkg` also rewrites ownership and tears down stale per-user agents first, and its app runs the session as root-adjacent `loginwindow`. Not exercised on a locked machine |
| Login-screen uninstall | `loginscreen off` boots the agent out and removes the plist but does not restore vendor agents it displaced |
| Notarisation | Ad-hoc signature only; no Developer ID or `notarytool` run |
| Multi-display edge cases | Mirror/arrangement restore matches by UUID only; unplugged docks are skipped silently |

## Test coverage

89 unit tests cover the DL3 header/flags/rectangle/checksum rules, EDID
validation plus the per-dock derivation, HDCP attribute decoding, the SPKG
container, fragment splice and frame table against synthesised and all four real
packs, profile mirror round-trip and tidy, the vendor pin/login-item helpers, and
the damage tracker (first frame, unchanged frame, single-pixel damage, run and
vertical coalescing, disjoint damage, the full-frame threshold, resize, sizes
that are not tile multiples, stride padding, short buffers, and the invariant
that every dirty tile is covered by a returned rect), the H.264 path (codec
parsing, SPS/PPS/IDR ordering, a VideoToolbox encode→decode round-trip,
flat-rect size reduction, session-pool reuse and LRU retirement, odd/1x1 rect padding, and that a compressed
payload — not BGRA — is what gets fragmented), and the refresh preference
(round-trip, old profiles, restore only at a matching size).

Manual checks this session: the universal bundle from `scripts/bundle.sh
--release` codesigns, launches and stays up with its window rendered; all four
shipped packs parse and walk their frame tables via `neolinkctl pack`. Also
available: `neolinkctl probe`, `neolinkctl vendor status|fetch|expand|packs`,
`neolinkctl pack F --out D`, `neolinkctl hdcp`, `neolinkctl edid --dock NAME`
and `neolinkctl loginscreen on|off`. `neolinkctl frame [ID] [N] [--codec
raw|h264]` reports per-frame dirty tiles, rect count and bytes versus raw, and
prints the session-wide saving.
