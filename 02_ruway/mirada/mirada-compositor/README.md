# mirada-compositor

> Tiling Wayland compositor — the "Body" of [mirada](../README.md).

A real tiling Wayland compositor built on [`smithay`]. It's the **Body** of `mirada`'s Brain↔Body architecture: it speaks the Wayland protocol with apps, owns surfaces and inputs, executes the layout decisions [`mirada-brain`](../mirada-brain/README.md) emits. Without `mirada-brain`, it still runs as a minimal default layout.

## Usage

```sh
cargo run --release -p mirada-compositor
```

## Deps

- `smithay`, [`mirada-protocol`](../mirada-protocol/README.md), [`mirada-body`](../mirada-body/README.md)
