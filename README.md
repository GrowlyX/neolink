# Neolink

A native, Rust display manager for macOS that can drive displays the way
DisplayLink Manager 17.x does — without a kext, DriverKit extension, or the
vendor's daemon.

Two halves:

- **`neolink-core`** — CoreGraphics enumeration/reconfiguration, `CGVirtualDisplay`
  backing store, saved arrangements, and an optional NSXPC client for the real
  DisplayLink agent.
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

## Build

```sh
cargo build --release -p neolink      # app
cargo run -p neolink-core --bin neolinkctl -- probe
```

`NEOLINK_DEMO=1` (or `2` for the full action path) exercises the app on launch
without Accessibility permission.

## Layout

```
crates/neolink-core/src/virtual_display.rs   CGVirtualDisplay engine
crates/neolink-core/src/displays.rs          CG enumeration + reconfiguration
crates/neolink-core/src/agent.rs             vendor agent supervision + XPC
crates/neolink-core/src/layout.rs            profile capture/apply
crates/neolink-app/                          GPUI UI
```

## Licence

MIT for Neolink's own code. Synaptics vendor bits (firmware/EDID blobs) are read
from an installed DisplayLink Manager and never redistributed.
