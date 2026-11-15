<p align="center">
  <a href="https://github.com/GrowlyX/neolink/actions/workflows/ci.yml"><img src="https://github.com/GrowlyX/neolink/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/GrowlyX/neolink/releases/latest"><img src="https://img.shields.io/github/v/release/GrowlyX/neolink" alt="latest release"></a>
  <img src="https://img.shields.io/badge/platform-macOS%2014%2B-blue" alt="macOS 14+">
  <a href="LICENSE"><img src="https://img.shields.io/badge/licence-MIT-blue" alt="MIT"></a>
</p>

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

## Docs

- [COMPATIBILITY.md](COMPATIBILITY.md): what works, what doesn't, measured
- [docs/RE-FINDINGS.md](docs/RE-FINDINGS.md): how the vendor package and the
  private APIs actually work
- [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md): building, testing, what CI checks
- [CONTRIBUTING.md](CONTRIBUTING.md) · [SECURITY.md](SECURITY.md)

## Status

Pre-1.0. The virtual-display side works; the USB side is reverse engineered and
**unvalidated on hardware**. See COMPATIBILITY.md before trusting it with a dock.
