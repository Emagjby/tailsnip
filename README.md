# tailsnip

`tailsnip` is a small Rust CLI for moving clipboard text between Tailscale-connected devices.

## Status

- MVP transport: authenticated HTTP between peers on your tailnet
- Supported clipboard hosts: macOS, Linux Wayland with `wl-copy`/`wl-paste`, Linux X11 with `xclip`
- Unsupported today: Windows, TLS termination, binary payloads, clipboard history, multi-user daemon hosting

## Quick start

1. Build the CLI with `cargo build --release`
2. Create a config template with `tailsnip init`
3. Fill in a shared token and device addresses in `~/.config/tailsnip/tailsnip.toml`
4. Start the daemon on each receiving device with `tailsnip daemon`
5. List aliases with `tailsnip devices`
6. Push local clipboard text with `tailsnip send <alias>` or pull remote clipboard text with `tailsnip get <alias>`

## Config example

See `docs/examples/config-example.toml` or use:

```toml
token = "replace-me"

[daemon]
listen = "0.0.0.0:3947"

[devices]
macbook = "100.10.0.10:3947"
desktop = "100.10.0.11:3947"
```

## Commands

- `tailsnip init` creates a template config in `~/.config/tailsnip/tailsnip.toml`
- `tailsnip daemon` runs the local clipboard API server until `Ctrl-C` or `SIGTERM`
- `tailsnip devices` prints configured aliases and addresses
- `tailsnip send <alias>` reads local clipboard text and writes it to the remote daemon
- `tailsnip get <alias>` fetches remote clipboard text and writes it locally

## Operator docs

- `docs/operator-guide.md` covers config shape, daemon behavior, clipboard dependencies, support boundaries, and Tailscale expectations
- `docs/release.md` covers builds, packaging expectations, and the MVP verification checklist

## Development

- Run tests with `cargo test`
- Build a debug binary with `cargo build`
- Install locally with `make install`
