# Development

macOS 14+ only. The core crate links CoreGraphics, the Objective-C runtime and a
compiled `.m` shim, so there is no Linux or Windows build.

```sh
cargo fmt
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
scripts/bundle.sh            # target/debug/Neolink.app
```

`NEOLINK_DEMO=2 ./target/debug/neolink` creates a virtual display and runs every
inspector action, so the whole path can be checked headlessly without granting
Accessibility or clicking.

## What CI checks

`ci.yml`, on every push to `main` and every PR:

- **lint**: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo doc` with
  `-D warnings`, and `cargo deny check` for licences, advisories and sources.
- **test**: the workspace on `macos-latest` and `macos-14`, plus doctests.
- **bundle**: `scripts/bundle.sh --release`, then verifies the result is a
  signed app with a lint-clean `Info.plist`, and runs `neolinkctl probe` from
  inside the bundle.

Nothing here needs a dock. `neolinkctl usb`, `neolinkctl edid`, `neolinkctl
transport` all fail cleanly with "no DisplayLink adapter attached", so a green
run only proves the code compiles, links and its own tests pass — not that the
wire format is right.

## Releasing

Push a `v*` tag matching the `Cargo.toml` version. The release workflow re-runs
fmt, clippy and the tests, builds the universal bundle, and attaches a zip to a
new GitHub release.

The signature is ad-hoc, so first launch still needs
`xattr -d com.apple.quarantine Neolink.app` or an Open Anyway. To distribute
properly, set a Developer ID identity in `scripts/bundle.sh` and notarise with
`notarytool` — both TODO.

## Layout

```
crates/neolink-core/src/virtual_display.rs  CGVirtualDisplay engine
crates/neolink-core/src/displays.rs         CG enumeration + reconfiguration
crates/neolink-core/src/dl3.rs              DL3 wire format
crates/neolink-core/src/transport.rs        USB transport (nusb)
crates/neolink-core/src/spkg.rs             vendor .spkg container
crates/neolink-core/src/agent.rs            vendor agent + XPC client
crates/neolink-core/src/vendor.rs           pinned package fetcher
crates/neolink-core/src/layout.rs           profile capture/apply
crates/neolink-app/                         GPUI UI
```

The `bin/` helpers are scratch tools used while reversing (`dbg`, `hd`,
`spkgdump`, `usbprobe`, …). They are not installed by the bundle.
