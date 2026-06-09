# Contributing to tawasuyu

*La versión en español está más abajo: [Contribuir a tawasuyu](#contribuir-a-tawasuyu).*

tawasuyu is a personal, opinionated system. Contributions are welcome, but
the architecture is not up for vote — read this first so your effort lands.

## Ground rules

1. **`cargo check --workspace` must pass on `main`, always.** It is the
   suite's minimal smoke test and CI enforces it. Never push something that
   breaks it.
2. **Commit messages are written in Spanish**, following the repo style:
   `tipo(scope): mensaje corto en minúsculas` — e.g.
   `fix(mirada): restaurar foco al cerrar popup`. Look at `git log` for the
   living pattern. Code comments are also in Spanish.
3. **One domain = one root crate with plugin subcrates.** Don't add lateral
   crates outside a domain's folder. Split any crate that grows past
   ~1,500–2,000 LOC.
4. **UIs are interchangeable frontends over UI-agnostic `*-core` crates.**
   Domain logic must not know who paints it. All new graphics go through
   **llimphi** (read `02_ruway/llimphi/MANUAL.md` before inventing UX or
   reimplementing a widget — there are ~44 widgets and 10 modules already).
5. **Foreign formats enter through `shared/foreign-*` bridges**, never into
   an app's core. Apps work in the native format (BLAKE3 + DAG + postcard).
6. **Names with strong semantic load are never translated** (*khipu*,
   *rimay*, *pluma*, *wawa*, *mirada*, *chasqui*, *llimphi*…). They are hard
   references to concrete artifacts — if a word names something, find that
   artifact and use it; don't re-derive the concept.

## Before touching anything deep

Read, in this order: the domain's `README.md`, its `SDD.md` if present
(SDDs are authoritative when they disagree with anything else), and
`PLAN.md` / `WAWA.md` for the system-wide picture.

## Setup

```bash
git clone https://git.tawasuyu.net/tawasuyu/tawasuyu.git
cd tawasuyu
cargo check --workspace                  # stable Rust is enough here
./scripts/check-shared-cores.sh          # validates the no_std shared cores
```

The bare-metal OS (`03_ukupacha/wawa`) is excluded from the root workspace
and needs nightly (`rust-src`, targets `wasm32-unknown-unknown` and
`x86_64-unknown-none`) — see the root [README](README.md#building-the-unusual-parts).

## Tests

Run the tests of the crate you touched: `cargo test -p <crate>`. For wide
runs prefer `cargo nextest` (per-test timeouts; some cosmos searches are
slow in debug). Types that cross boundaries (network, content-addressed
disk, kernel↔userspace) must stay `no_std` — `check-shared-cores.sh` is the
referee.

## Licensing of contributions

By contributing you agree your work is licensed under the file's area
license (workspace default **MIT OR Apache-2.0**; some foundational crates
are **MPL-2.0** — see [LICENSE.md](LICENSE.md)). No CLA.

---

# Contribuir a tawasuyu

tawasuyu es un sistema personal y opinado. Las contribuciones son
bienvenidas, pero la arquitectura no se vota — leé esto primero para que tu
esfuerzo aterrice.

## Reglas de base

1. **`cargo check --workspace` debe pasar en `main`, siempre.** Es el smoke
   test mínimo de la suite y el CI lo exige. Nunca pushees algo que lo rompa.
2. **Los mensajes de commit se escriben en español**, siguiendo el estilo
   del repo: `tipo(scope): mensaje corto en minúsculas` — p. ej.
   `fix(mirada): restaurar foco al cerrar popup`. Mirá `git log` para ver el
   patrón vivo. Los comentarios de código también van en español.
3. **Un dominio = un crate raíz con subcrates plugin.** No agregues crates
   laterales fuera de la carpeta de un dominio. Partí cualquier crate que
   pase las ~1.500–2.000 LOC.
4. **Las UIs son frontends intercambiables sobre crates `*-core` agnósticos.**
   La lógica de dominio no debe saber quién la pinta. Todo lo gráfico nuevo
   va sobre **llimphi** (leé `02_ruway/llimphi/MANUAL.md` antes de inventar
   UX o reimplementar un widget — ya hay ~44 widgets y 10 módulos).
5. **Los formatos ajenos entran por puentes `shared/foreign-*`**, nunca al
   núcleo de una app. Las apps trabajan en el formato nativo
   (BLAKE3 + DAG + postcard).
6. **Los nombres con carga semántica fuerte no se traducen nunca** (*khipu*,
   *rimay*, *pluma*, *wawa*, *mirada*, *chasqui*, *llimphi*…). Son
   referencias duras a artefactos concretos — si una palabra nombra algo,
   encontrá ese artefacto y usalo; no re-derives el concepto.

## Antes de tocar algo profundo

Leé, en este orden: el `README.md` del dominio, su `SDD.md` si existe (los
SDD son autoritativos cuando difieren con cualquier otra cosa), y
`PLAN.md` / `WAWA.md` para la foto del sistema completo.

## Setup

```bash
git clone https://git.tawasuyu.net/tawasuyu/tawasuyu.git
cd tawasuyu
cargo check --workspace                  # acá alcanza Rust estable
./scripts/check-shared-cores.sh          # valida los núcleos no_std compartidos
```

El SO bare-metal (`03_ukupacha/wawa`) está excluido del workspace raíz y
necesita nightly (`rust-src`, targets `wasm32-unknown-unknown` y
`x86_64-unknown-none`) — ver el [LEEME](LEEME.md#compilar-las-partes-inusuales) raíz.

## Tests

Corré los tests del crate que tocaste: `cargo test -p <crate>`. Para
corridas anchas preferí `cargo nextest` (timeout por test; algunas búsquedas
de cosmos son lentas en debug). Los tipos que cruzan fronteras (red, disco
direccionado por contenido, kernel↔userspace) deben mantenerse `no_std` —
`check-shared-cores.sh` es el árbitro.

## Licencia de las contribuciones

Al contribuir aceptás que tu trabajo queda bajo la licencia del área del
archivo (default del workspace **MIT OR Apache-2.0**; algunos crates
fundacionales son **MPL-2.0** — ver [LICENSE.md](LICENSE.md)). Sin CLA.
