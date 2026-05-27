# mirada

> Gioser's display stack: compositor + portal + greeter + launcher.

`mirada` (Spanish *look, gaze*) delivers what the user sees on boot: the Wayland compositor, the XDG portal (file pickers, screenshare), the login greeter and a minimal launcher. All UI runs on Llimphi; the `bar-*` crates provide swappable status bars.

## Install

```sh
cargo run --release -p mirada-compositor
cargo run --release -p mirada-greeter
cargo run --release -p mirada-launcher
```

## Compatibility

- **Linux DRM/KMS** — native compositor.
- **Linux nested** — runs inside a host Wayland (dev mode).
- **Wawa** — minimal compositor on the kernel's framebuffer.

Crates listed in [README.md](README.md).

## Considerations

- **Doesn't replace `weston`/`sway`** in stability; replaces them in *Llimphi-HAL compatibility*. For full-stack monorepo, you want `mirada`.
- DRM/KMS requires permissions: launch from a greeter, not a user terminal.
- XDG portal is **complete**: `pluma`, `nada`, etc. can request file pickers via portal with no app-specific code.
