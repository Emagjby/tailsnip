# tailsnip operator guide

## Config shape

The CLI reads `tailsnip.toml` from `~/.config/tailsnip/tailsnip.toml` or `$XDG_CONFIG_HOME/tailsnip/tailsnip.toml`.

```toml
token = "shared-secret-used-by-all-peers"

[daemon]
listen = "0.0.0.0:3947"

[devices]
macbook = "100.10.0.10:3947"
desktop = "100.10.0.11:3947"
```

- `token` is required and must be non-empty
- `daemon.listen` is optional; default is `127.0.0.1:3947`
- `devices` is optional, but if present it must not be empty
- device aliases may only use lowercase letters, digits, `_`, and `-`
- device addresses must be explicit `IP:PORT` socket addresses

## Command behavior

- `tailsnip init` writes a template config and fails if the file already exists
- `tailsnip devices` prints one `alias<TAB>address` entry per configured peer
- `tailsnip send <alias>` reads the local clipboard, resolves the alias, and POSTs text to the remote daemon
- `tailsnip get <alias>` reads remote clipboard text, refuses to overwrite the local clipboard with an empty payload, and then writes locally
- `tailsnip daemon` starts the HTTP service and shuts down cleanly on `Ctrl-C` or `SIGTERM`

## Daemon runtime expectations

- startup fails fast for invalid config, bind failures, or unsupported clipboard backends
- authenticated requests must send `Authorization: Bearer <token>`
- auth failures return `401`
- unavailable clipboard backends return `503`
- clipboard command failures return `500`
- shutdown cleans up the retained Wayland clipboard owner process before exit

## Clipboard support

Supported backends:

- macOS: `pbcopy` and `pbpaste`; `osascript` is used first for writes when available, then `pbcopy` is used as fallback
- Linux Wayland: `wl-copy` and `wl-paste`
- Linux X11: `xclip`

Explicit support boundaries:

- only macOS and Linux are supported today
- Linux write support requires either Wayland with `wl-copy` or X11 with `xclip`
- Linux read support requires Wayland with both `wl-copy` and `wl-paste`, or X11 with `xclip`
- Windows and mixed desktop-session fallback logic are out of scope for this MVP

Common failure modes:

- missing backend tools: daemon start or local clipboard operations fail with `clipboard unavailable`
- backend command errors: requests fail with `failed to read clipboard` or `failed to write clipboard`
- Wayland ownership not retained: writes fail if `wl-copy --foreground` exits during the grace period
- remote peer offline or port closed: `send`/`get` fail with `remote device unreachable`
- wrong shared token: remote requests fail with `authentication failed`

## Tailscale setup expectations

- each device should be reachable over the same tailnet
- use the peer's Tailscale IP in `[devices]`
- ensure the daemon port is reachable on the receiving device
- this MVP assumes HTTP over the tailnet; there is no built-in TLS layer yet
- if you bind the daemon to `0.0.0.0`, rely on Tailscale reachability and host firewall rules to keep exposure intentional
