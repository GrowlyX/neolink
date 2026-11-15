# Neolink

A native, Rust display manager for macOS that can drive displays the way
DisplayLink Manager 17.x does, with our own UI, transport and arrangement
engine. Nothing from Synaptics is bundled: the official package is fetched at
runtime, hash-pinned, and installed only when you ask.

Two halves:

- **`neolink-core`** — CoreGraphics enumeration/reconfiguration, `CGVirtualDisplay`
  backing store, a DL3 USB transport, an NSXPC client for the vendor agent, and
  the on-demand vendor package fetcher.
- **`neolink-app`** — a GPUI window (device list / arrangement canvas / inspector)
  over that core.

`docs/RE-FINDINGS.md` records how the vendor 17.1.56 package works and which
private APIs are used.

## Status

Early but functional on macOS 14+. Virtual displays created through
`CGVirtualDisplay` come online, keep their identity, and survive mode, origin,
rotation and HiDPI changes. See [COMPATIBILITY.md](COMPATIBILITY.md) for the
honest parity matrix — this is **not** yet a drop-in replacement for DisplayLink
Manager on real docks.

## Install

There are two install paths, and they are not exclusive:

1. **The vendor driver** (needed for a real dock to show a picture). Press
   **Get DisplayLink** in the app, or:

   ```sh
   neolinkctl vendor provision
   ```

   That downloads the official 17.1.56 package from Synaptics, verifies it
   against the pinned SHA-256, expands it into
   `~/Library/Application Support/Neolink/vendor/`, and runs the system
   installer once (one admin prompt). `vendor status`, `vendor expand`,
   `vendor packs` and `vendor purge` manage the cache separately.

2. **Neolink itself**:

   ```sh
   scripts/bundle.sh --release          # target/release/Neolink.app
   scripts/bundle.sh --release --install
   ```

   The bundle is universal and ad-hoc signed. For distribution, swap the
   `codesign --sign -` line for a Developer ID identity and notarise with
   `notarytool`.

## Build

```sh
cargo build -p neolink
cargo test -p neolink-core --lib
```

`NEOLINK_DEMO=1` (or `2` for the full action path) exercises the app on launch
without Accessibility permission.

`neolinkctl pack <file.spkg> [--out DIR]` reassembles a dock pack's frames and
optionally writes the image out; `neolinkctl login on|off|status` manages the
LaunchAgent.

## Layout

```
crates/neolink-core/src/virtual_display.rs   CGVirtualDisplay engine
crates/neolink-core/src/displays.rs          CG enumeration + reconfiguration
crates/neolink-core/src/agent.rs             vendor agent supervision + XPC
crates/neolink-core/src/layout.rs            profile capture/apply
crates/neolink-core/src/dl3.rs               DL3 wire format
crates/neolink-core/src/transport.rs         USB transport
crates/neolink-core/src/spkg.rs              .spkg container + frame table
crates/neolink-core/src/bin/spkgdump.rs      inspect a pack's frame table
crates/neolink-core/src/bin/edidgen.rs       synthesise an EDID for the dock
crates/neolink-core/src/vendor.rs            pinned package fetch/verify/install
crates/neolink-app/                          GPUI UI
```

## Licence

MIT for Neolink's own code.

DisplayLink Manager and the per-dock `.spkg` firmware/EDID packs are Synaptics'
and stay under their EULA. Neolink ships none of them: it stores a URL and a
SHA-256 and pulls the bytes from Synaptics' own server at install time. The RE
notes in `docs/RE-FINDINGS.md` describe behaviour and structure only.
