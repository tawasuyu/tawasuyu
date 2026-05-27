<!-- Quechua (Cusco/Collao). Revisión bienvenida. -->

# pineal

> Backend mana hap'iq rikuchikuy. Monorepuq "kimsa-ñawin".

Canvas tukuylla (cartesiano · polar · mesh · treemap · phosphor · flow · heatmap · stream · financial · umbrella), huk Llimphi `SceneCanvas` patapi. Ima willay kawsasqakunapas (cosmos, dominium, nakui, tinkuy, chasqui) pineal-man k'antinkunata haywariy atin, sapan grafica pipeline mana kanchu.

## Churay

```sh
cargo run --release -p pineal-demo
cargo run --release -p pineal-financial-demo
cargo run --release -p pineal-phosphor-demo
cargo run --release -p pineal-stream-demo
```

## Tinkuy

- **Linux / macOS / Windows** — Llimphi (vello/wgpu) renderiy.
- **Wawa bare-metal** — Llimphi framebuffer patapi; kikin escena sach'a.

## Crateskuna

Tukuy crates yachachiyninwan iskay versionkunapi: [`pineal-core`](pineal-core/README.md) escena modelo, [`pineal-render`](pineal-render/README.md) Llimphi rikuchikuy, [`pineal-{cartesian,polar,mesh,treemap,phosphor,flow,heatmap,stream,financial,umbrella}`](pineal-cartesian/README.md) sapanka layakuna, [`pineal-export`](pineal-export/README.md) PNG/SVG/GIF qatinapaq, [`pineal-{demo,financial-demo,phosphor-demo,stream-demo}`](pineal-demo/README.md) muestrakuna.

## Yuyaykunaq

- pineal **manan yupanchu** — siq'illanmi. Simulación ruwananpaqqa [`dominium`](../../01_yachay/dominium/README.md), [`tinkuy`](../../01_yachay/tinkuy/README.md), [`cosmos`](../../01_yachay/cosmos/README.md), chaykunawan rimay, hinaspa kutichiyninta pineal-man qun.
- SVG qatinaqa cheqaq vector kanmi (mana píxeles laqasqachu).
