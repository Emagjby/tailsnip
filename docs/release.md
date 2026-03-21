# tailsnip release basics

## Build instructions

- debug build: `cargo build`
- release build: `cargo build --release`
- run tests before shipping: `cargo test`
- install locally for smoke testing: `make install`

Expected release artifact:

- single Rust CLI binary named `tailsnip`
- default output path: `target/release/tailsnip`

## Packaging expectations

- version comes from `Cargo.toml`
- tag releases with the same semantic version used in `Cargo.toml`
- ship binaries with the operator docs and example config alongside the release notes
- document the supported platforms for the tagged build: macOS, Linux Wayland, Linux X11
- do not claim Windows support until clipboard and daemon behavior are implemented and tested there

## Minimal manual verification checklist

Before an MVP release, verify:

1. `cargo test` passes on the release commit
2. `cargo build --release` succeeds from a clean checkout
3. `tailsnip init` creates the template config in the expected config directory
4. `tailsnip devices` prints configured aliases in stable sorted order
5. `tailsnip daemon` starts, prints the selected clipboard backend, and exits cleanly on `Ctrl-C`
6. `tailsnip send <alias>` succeeds between two tailnet peers with a valid token
7. `tailsnip get <alias>` succeeds between two tailnet peers with a valid token
8. auth mismatch returns a clear `authentication failed` error
9. unreachable peer returns a clear `remote device unreachable` error
10. missing clipboard tools fail with a clear `clipboard unavailable` error

## Suggested release notes topics

- supported clipboard environments
- required external clipboard tools on Linux
- tailnet-only transport expectation
- known MVP limitations and unsupported platforms
