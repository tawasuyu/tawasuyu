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
- **Wawa** — runs inside the kernel.
- `shuma-protocol` enables local-client + remote-server without SSH.

Crates listed in [README.md](README.md) (shuma + matilda).

## Considerations

- **Replacement, not addition.** If you use shuma, you can uninstall zsh/tmux/mosh; behavior fully covered.
- **`intent → command`** is optional; without LLM the traditional shell runs unchanged.
- Remote sessions use **`shuma-protocol`** over TCP/TLS — no SSH daemon required.
