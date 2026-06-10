# shuma

> Interactive shell with zsh/fish parity, on a Llimphi chassis.

`shuma` replaces zsh + tmux + mosh with a single piece: shell with history/completion/job-control, native multiplexing (no `tmux`), remote sessions (no `mosh`), all inside a Llimphi 4-slot chassis (TopBar, Main, BottomBar, DrawerTab + Quake drawer). 8-block roadmap (target 2026-05-25). `matilda` is the sibling tool for declarative multi-host configuration.

## Install

```sh
cargo run --release -p shuma-shell-llimphi
cargo run --release -p shuma-cli
cargo run --release -p shuma-daemon
```

## Compatibility

- **Linux / macOS / Windows** — shell + Llimphi UI.
- **Wawa** — planned (no kernel-side port yet).
- `shuma-daemon` + `shuma-protocol` enable local-client + remote-server without SSH.

Crates listed in [LEEME.md](LEEME.md) (shuma + matilda).

## Considerations

- **Replacement, not addition.** If you use shuma, you can uninstall zsh/tmux/mosh; behavior fully covered.
- **`intent → command`** is optional; without LLM the traditional shell runs unchanged.
- Remote sessions go through **`shuma-daemon` over TCP, encrypted and authenticated with Noise XK** (`shuma-link`, known-peers pinning) — no SSH daemon and no TLS/CA required. `shuma-protocol` is the wire framing (length-prefix + postcard).

## Status (2026-06-09)

- **Terminal surface is the default render path** (SDD-TERMINAL phases 0–5: append-only scrollback store, virtualized line mode, command blocks + chrome, selection/copy + Ctrl+F find, GPU cell grid behind `SHUMA_GPU_GRID=1`). Legacy pane stays reachable with `SHUMA_TERMINAL_LEGACY=1`. Persistent scrollback spills to disk; `:scrollback` / `:scrollback grep <pat>` inspect the archive. See [SDD-TERMINAL.md](SDD-TERMINAL.md).
- **Workspaces with real isolation engines**: `unshare` (default), `bwrap`, `podman` — a workspace can run inside an actual OCI container (`Source::Container`), selectable in the session form.
- **`sudo` works**: `shuma-askpass` is a `SUDO_ASKPASS`-compatible Llimphi popup, so bare `sudo` no longer hangs.
- **Live streaming output** (progress bars, byte counters), per-command sub-collapsibles (`ls -R`) and sortable tables (`ls -l`).
- **Cards & pipelines**: `shuma-card` models workspaces and `PipelineSpec` DAGs (commands joined by flow edges) served by the daemon.
- **Remote PTY/TUI is full-duplex** over the encrypted daemon channel (Unix socket locally, Noise-XK TCP remotely).
- The reusable surface lives in `02_ruway/llimphi/widgets/terminal` (`llimphi-widget-terminal`); `llimphi-module-shuma-term` embeds a Ctrl+`-style terminal in any Llimphi app.
