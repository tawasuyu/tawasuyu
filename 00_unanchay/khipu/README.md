# khipu

> `khipu` (Quechua: knotted cord recordkeeping). Notes with temporal gravity.

Quick-note capture where forgetting is part of the model: each note has a mass that decays with time and reinforces with each access. What's recurrent stays visible; what isn't touched fades until it falls off the horizon.

## Install

```sh
cargo run --release -p khipu-app
```

## Compatibility

- **Linux / macOS / Windows** — Llimphi UI (Wayland/X11/Win32 via `winit`).
- Local persistence in `$XDG_DATA_HOME/khipu/`.

## Crates

| Crate | Role |
|---|---|
| [`khipu-core`](khipu-core/README.md) | Note model + store; no UI. |
| [`khipu-gravity`](khipu-gravity/README.md) | Mass/decay algorithm; reinforcement on access. |
| [`khipu-app`](khipu-app/README.md) | Llimphi UI over the core. |

## Considerations

- **It's not a "todo" system** — no due dates, no reminders; it's a notebook with its own physics.
- Decay is transparent: each note shows its current mass; the user decides whether to save it.
- Plays well with the [agora](../../03_ukupacha/agora/README.md) network: notes can be shared without losing their local gravity.
