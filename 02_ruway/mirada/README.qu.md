<!-- Quechua (Cusco/Collao). Revisión bienvenida. -->

# mirada

> Tawasuyu-pa display pacha: compositor + portal + greeter + launcher.

`mirada` (kastellano: *qhaway*) sistema haqarispa rikuna haywariy: Wayland compositor, XDG portal (file pickers, screenshare), login greeter + minimo launcher. Tukuy UI Llimphi patapi; `bar-*` cratekuna tikrana status barras.

## Churay

```sh
cargo run --release -p mirada-compositor
cargo run --release -p mirada-greeter
cargo run --release -p mirada-launcher
```

## Tinkuy

- **Linux DRM/KMS** — natural compositor.
- **Linux nested** — host Wayland ukhupi (dev modo).
- **Wawa** — minimo compositor kernel framebuffer-pi.

Crateskuna listako [README.md](README.md)-pi.

## Yuyaykunaq

- **Manan `weston`/`sway` qhipa-saqi** estabilidad; *Llimphi-HAL tinkuy*-pi qhipa-saqi.
- DRM/KMS permiso munan: greeter-manta kawsachiy.
- XDG portal **hunt'asqa**: `pluma`, `nada`, etc. portal-rayku file picker mañakuy atinku.
