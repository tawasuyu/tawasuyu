<!-- Quechua (Cusco/Collao). Revisión bienvenida. -->

# shuma

> Shell interactivo zsh/fish kasqaqlla, Llimphi chasis patanpi.

`shuma` zsh + tmux + mosh suyaspa, sapan p'aki hina: shell history/completion/job-control, natural multiplexing (manan `tmux`), karu sesiones (manan `mosh`), Llimphi 4-slot chasispi (TopBar, Main, BottomBar, DrawerTab + Quake drawer). 8-bloque roadmap (suyay 2026-05-25). `matilda` huk multi-host declarativo herramienta.

## Churay

```sh
cargo run --release -p shuma-shell-llimphi
cargo run --release -p shuma-cli
cargo run --release -p shuma-daemon
```

## Tinkuy

- **Linux / macOS / Windows** — shell + Llimphi UI.
- **Wawa** — kernel ukhupi.
- `shuma-protocol` lokal-cliente + karu-server, mana SSHwan.

Crateskuna [README.md](README.md)-pi.

## Yuyaykunaq

- **Suyay, mana aymachay.** Shuma usaspa, zsh/tmux/mosh wikch'ay atinki.
- **`intent → comando`** opcional; mana LLM tradicional shell hina.
- Karu sesiones **`shuma-protocol`** TCP/TLS patanpi — manan SSH daemon munana.
