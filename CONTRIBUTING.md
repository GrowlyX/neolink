# Contributing

Thanks for the interest. This is pre-1.0 and the transport half is still being
reverse engineered, so the most useful contributions are reproductions, packet
captures and hardware reports.

## Before you start

- Open an issue before a large change. The display engine has a lot of
  constraints that aren't obvious from the code.
- `main` is the branch. There is a single maintainer; a PR that needs a design
  discussion will take longer than one that fixes a bug or a doc.

## Development

```sh
cargo fmt
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p neolink-core --lib
```

CI runs exactly those, plus a bundle build and a check that `Cargo.lock` and
`go`-style formatting are clean. Keep `cargo fmt` and clippy clean before you
push; a red CI won't be reviewed.

`NEOLINK_DEMO=2 ./target/debug/neolink` exercises the whole UI action path
without Accessibility permission. `neolinkctl probe`, `neolinkctl vendor
status` and `neolinkctl usb` are the manual checks.

## Hardware reports

The most valuable thing you can attach to an issue:

- dock/adapter model and USB vendor:product id (`neolinkctl usb`)
- macOS version and build
- whether the vendor agent was running, and whether Neolink's transport could
  open the device
- a `neolinkctl usb` trace, and `log show --predicate 'process == "neolink"'`

If you can capture a USB trace while DisplayLink Manager drives the dock, say so
in the issue — that closes gaps in `docs/RE-FINDINGS.md` faster than anything.

## Code style

- Comments explain *why*, and cite the disassembly address when a constant came
  from reversing (`0x1001273c8`), so it can be re-checked.
- No vendor bytes in the tree: EDID templates are synthesised, firmware blobs
  are read from the installed package.
- Keep `unsafe` blocks small and next to the FFI declaration they belong to.

## Licensing

Neolink is MIT. By opening a PR you agree your contribution is MIT too. Don't
paste code from Synaptics' binary beyond short, cited constants.
