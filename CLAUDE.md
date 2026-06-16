# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Qué es tawasuyu

Suite vertical en Rust (kernel propio, identidad, motor gráfico, navegador, ERP, shell, broker, simulador...) organizada como un solo Cargo workspace de ~210 crates. La arquitectura está embebida en el filesystem: cuatro cuadrantes (`00_unanchay`/`01_yachay`/`02_ruway`/`03_ukupacha`) corresponden a las cuatro fases del ciclo de la información (PERCIBIR / CONOCER / HACER / RAÍZ). Mover un dominio de cuadrante cambia su naturaleza — no son carpetas administrativas.

Lectura previa obligatoria al tocar cualquier cosa de fondo: `README.md`, `PLAN.md`, `WAWA.md` (este último describe el SO bare‑metal `wawa`, que vive aparte del workspace global). Hay SDDs específicos para dominios complejos: `02_ruway/llimphi/SDD.md`, `02_ruway/wawa/SDD.md`, `02_ruway/supay/SDD.md`, `02_ruway/tullpu/SDD.md`, `01_yachay/dominium/SDD.md`, `00_unanchay/puriy/SDD.md`, `shared/sandokan/SDD.md` (plano de control: quién arranca/para/supervisa/observa unidades en Linux y Wawa, sin duplicados) — son la fuente autoritativa cuando difieren con esta guía.

## Reglas duras del repo

1. **Un dominio = un crate raíz con subcrates plugin.** Nada de proliferación lateral. Splittear crates > ~1.500–2.000 LOC.
2. **Las UIs son frontends intercambiables sobre `*-core` agnósticos.** La lógica de dominio no sabe quién la pinta.
3. **GPUI está extinto** (2026-05-26). Todo gráfico nuevo va sobre **Llimphi** (`02_ruway/llimphi/*`: `hal/raster/layout/text/ui` + widgets + modules). Stack: `wgpu` + `vello` + `taffy` + `parley`, bucle Elm `input→update→view→layout→raster→present`. No agregar dependencias GPUI ni código nuevo sobre él. **Manual de uso (cómo construir una app, DSL `View<Msg>`, catálogo de ~44 widgets y 10 módulos, GPU directo, gotchas):** `02_ruway/llimphi/MANUAL.md`, verificado contra el código — leerlo antes de inventar UX o reimplementar widgets.
4. **Formatos ajenos entran por puentes en `shared/foreign-*`**, nunca al núcleo de las apps. Las apps siempre trabajan en formato nativo (BLAKE3 + DAG + postcard). En disco hoy (verificado 2026-06-16): `foreign-av` (ffmpeg), `foreign-fs`, `foreign-platform` (plataformas de video agnósticas), `foreign-psd` (import `.psd`, ~992 LOC sin stubs), `foreign-xlsx` (Excel ↔ `nakui_sheet::Sheet`, con round-trip y tests) y `foreign-ytdlp`. **No** están en disco `foreign-docx` ni `foreign-pptx` (planificados en `PLAN.md` §6.ter). Verificá con `ls shared/foreign-*` antes de asumir presencia o ausencia — esta lista envejece.
5. **`cargo check --workspace` debe pasar en `main` siempre** — es el smoke test mínimo.
6. **Nombres con carga semántica fuerte se respetan en su idioma** (mayormente quechua/español). No retraducir `khipu`, `rimay`, `pluma`, `wawa`, `mirada`, `nahual`, `chasqui`, `takiy`, `agora`, `arje`, `minga`, `shuma`, `nakui`, `iniy`, `tinkuy`, `chaka`, `pineal`, `puriy`, `supay`, `sandokan`, `dominium`, `cosmos`, `tullpu`, `yupay`, `llimphi`, `akasha`, `unanchay`, `yachay`, `ruway`, `ukupacha`.
7. **Comentarios y mensajes de commit en español.** Es la convención del repo (ver `git log`).

## Un término nombrado = un artefacto concreto, no un concepto a re-derivar (lección 2026-06-05)

Cuando el usuario nombra algo con una palabra del dominio (**diente**, khipu, mirada, rueda, pluma…), casi siempre es una **referencia dura** a un artefacto que **ya existe** en el repo (un widget, un crate, un patrón) con forma delimitada y, muchas veces, un **uso canónico** en alguna app. No es un concepto abstracto para rediseñar. Protocolo **obligatorio** antes de diseñar, afirmar o codear sobre uno de esos términos:

1. **Localizá el artefacto.** `grep` por el nombre + revisá `02_ruway/llimphi/MANUAL.md` (catálogo de widgets/modules). Si existe, **se usa**; está prohibido fabricar un sustituto paralelo (y peor, bautizarlo con el nombre del original).
2. **Leé su uso canónico y reproducilo.** Buscá qué app lo usa bien y copiá su composición exacta. No improvises un layout y lo llames como el original.
3. **No afirmes paridad sin verificar.** Nunca digas "esto es como X / es lo que querés" sin leer la fuente. Mostrá **evidencia** (render headless a PNG, código citado), no aserción.
4. **Ante una corrección, cambio mínimo dirigido.** No respondas un feedback puntual con un rewrite grande que mete un desvío nuevo — así no se converge.

Caso que originó la regla: **"diente" = el widget `llimphi-widget-dock-rail`** (pestaña que *sobresale* del panel, arrastrable, **representa un panel**). Layout canónico = rail como overlay pegado al borde interno + panel del item activo como pane al costado, exactamente como cosmos (`01_yachay/cosmos/cosmos-app-llimphi/src/chrome.rs`: `dock_rail_overlay` / `dock_panel_for`, y `src/main.rs` `view`). Costó una tarde entera porque se reinterpretó la palabra como concepto, se inventó una lista con rótulo llamada "diente", y se afirmó "así está cosmos" sin leer cosmos — violando además la Regla 3 ("leerlo antes de inventar UX o reimplementar widgets").

## Layout del workspace

```
00_unanchay/   PERCIBIR  — pluma · khipu · rimay · chaka · pineal · puriy
01_yachay/     CONOCER   — cosmos · dominium · nakui · iniy · tinkuy
02_ruway/      HACER     — mirada · shuma · nahual · chasqui · takiy · llimphi · supay · media · nada · wawa (host-side)
03_ukupacha/   RAÍZ      — arje · wawa (kernel + apps WASM) · agora · minga · wawa-explorer
shared/                  — sandokan · auth · card · ssh · format · foreign-* · rimay-localize · wawa-config · forth-emisor
web/                     — landing sobria (no producto)
```

Subcrates dentro de un dominio siguen el patrón `<dominio>-{core,app,cli,server,store,...}` o, para UI Llimphi, `<dominio>-<rol>-llimphi`. Demos ejecutables suelen vivir como `examples/` dentro del crate de UI correspondiente.

### `03_ukupacha/wawa` está **excluido** del workspace raíz

El kernel SASOS de wawa compila para `x86_64-unknown-none` con `panic = "abort"`, incompatible con el perfil global. `Cargo.toml` lo excluye explícitamente. `wawa-boot` lo consume como `[dependencies.kernel]` con `artifact = "bin"`. Los crates compartidos (`format`, `akasha`, `mirada-layout`, `forth-emisor`, `pluma-notebook-core`) cruzan la frontera referenciados por `path` — deben mantenerse `#![no_std]`.

## Comandos

### Workspace global (host)

```bash
cargo check --workspace                          # smoke test mínimo: debe pasar siempre
cargo build --workspace --release
cargo test -p <crate>                            # tests de un crate puntual
cargo run -p <crate> --example <demo> --release  # demos ejecutables (Llimphi)
```

Muchas apps tienen `examples/*_demo.rs` que son la forma esperada de probar features sin levantar la suite completa (ej. `pluma-editor-llimphi` tiene `multilienzo_demo`, `multilienzo_llm_demo`, `multilienzo_completo_demo`, `cuerpo_ide_demo`, `editor_unico_demo`; `pluma-notebook-kernel-{dominium,cosmos,llm}` traen `notebook_*_demo`).

### Landing web (`web/tawasuyu-web`)

```bash
./scripts/build-tawasuyu-web.sh dev       # cargo build + wasm-bindgen, ~10 s
./scripts/build-tawasuyu-web.sh release   # opt-level=3 + lto + strip, ~30 s
# output queda en web/tawasuyu-web/pkg/ (tawasuyu_web.js + tawasuyu_web_bg.wasm)
```

Necesita `wasm-bindgen-cli` en la versión **exacta** de `Cargo.lock` (hoy 0.2.121; `grep -A1 '^name = "wasm-bindgen"$' Cargo.lock | head` para confirmar) — si difiere, el JS no carga el `.wasm`. La landing es la única pieza del workspace que cruza el puente JS; no es producto, sólo cartel.

### Editor de archivos rápido (`nada`)

```bash
cargo run -p nada --release   # file tree + text-editor Llimphi sobre archivos reales del workspace
```

Útil para ejercitar features del `llimphi-widget-text-editor` (selección, undo, brackets, clipboard) sin levantar una app de dominio. (Antes se llamaba `tawasuyu-edit` — renombrado en 2026-05-27.)

### Núcleos `no_std` compartidos

```bash
./scripts/check-shared-cores.sh           # valida los 5 no_std (format, akasha, mirada-layout, forth-emisor, pluma-notebook-core)
./scripts/check-shared-cores.sh format    # un solo núcleo
```

Exige `rustup target add wasm32-unknown-unknown`. La ley: si un tipo viaja por Akasha, vive en disco direccionado por contenido, o se comparte entre kernel y userspace, su crate compila sin `std`.

### Kernel bare‑metal y apps WASM (`03_ukupacha/wawa`)

```bash
cd 03_ukupacha/wawa/wawa-kernel
cargo +nightly check --target x86_64-unknown-none -Z build-std=core,alloc

cd 03_ukupacha/wawa
cargo +nightly run -p boot -Z bindeps                 # forja imagen UEFI y arranca en QEMU

cd 03_ukupacha/wawa/apps/<app>
cargo build --target wasm32-unknown-unknown --release # luego copiar el .wasm a kernel/assets/

./scripts/build-pluma.sh                              # pipeline cargo + wasm-opt + consolidación en assets/
```

Toolchain: nightly con `rust-src`, targets `wasm32-unknown-unknown` y `x86_64-unknown-none`.

### LLM en pluma — backends y envs

`pluma-llm` es una fachada transparente (`LlmConfig{kind, model?, ...}` → `Arc<dyn ChatClient>`). Backends: Anthropic, Gemini, DeepSeek, Cohere, Ollama, Mock. `from_env()` autodetecta vía `PLUMA_LLM_BACKEND` o la primera env presente entre `ANTHROPIC_API_KEY`, `GEMINI_API_KEY`/`GOOGLE_API_KEY`, `DEEPSEEK_API_KEY`, `COHERE_API_KEY`. Sin credenciales cae a Mock — los demos arrancan igual.

### Daemon de embeddings

`rimay-verbo-daemon-bin` sirve un `Provider` por socket Unix. Consumidores (`pluma-semantic`, `pluma-align-embeddings`, `khipu`, `chasqui`) cambian `MockProvider::default()` por `DaemonClient::connect("$XDG_RUNTIME_DIR/verbo.sock").await?`. Una instancia = un modelo; multi-modelo = N daemons.

```bash
cargo run -p rimay-verbo-daemon-bin -- --provider fastembed   # default multilingual-e5-small (384d)
cargo run -p rimay-verbo-daemon-bin -- --provider mock --dim 384
```

## Arquitectura — el “porqué” detrás de varios archivos

- **Llimphi y el bucle Elm.** `llimphi-ui::App` define `update(Msg) -> ()` + `view() -> View`. `Handle<Msg>` es `Send + Clone` — workers de simulación reentran al `update` vía `Handle::dispatch(Msg::X)` o `Handle::spawn_periodic(Duration, Fn() -> Msg)`. Composición: `tiled_view_reorderable_cols(cols)` para paneles draggables (drag-to-swap por title bar), `splitter` (drag de divisores), `nodegraph` (DAG visual con pins y cables Bezier), `text-editor` (ropey + multi-cursor + undo). Paleta semántica en `llimphi-theme` (`Palette::from_theme(&theme)`). `View::paint_with(Fn(&mut Scene, &mut Typesetter, PaintRect))` para canvas custom; `View::image(peniko::Image)` para mostrar PNG/JPEG decodificados.
- **Pluma multilienzo (haz de cuerpos).** Un documento es un haz de `Cuerpo`s (lienzos: idioma, tono, audiencia, resumen, versión) sobre el mismo material, alineados párrafo-a-párrafo por `CartaHebras`. `NarrativeAtom` mantiene `id: Uuid` estable; `Transformacion` (Traducir/Tono/Resumir/Reescribir/Custom-Rhai) deriva un cuerpo hija de una madre. Si la madre cambia, la hija queda *stale* y la UI pinta la hebra punteada — un botón regenera. Persistencia en `pluma-store` (`sled` + trees nominales `atoms / cuerpos / transformaciones / cartas / estado_ui`). Ver `PLAN.md` §11 para el modelo completo.
- **Pluma notebook.** DAG reactivo de celdas (`pluma-notebook-core`) ejecutado por `pluma-notebook-exec::run_from` en orden topológico, con kernels intercambiables (`pluma-notebook-kernel-{llm,dominium,cosmos,python,wasm}`). Bindeo visual: `pluma-notebook-graph-llimphi` sobre `llimphi-widget-nodegraph` — drag pin→pin agrega dependencia y dispara `run_from` del cono; right-click ejecuta una celda. Outputs incluyen `OutputPayload::Image{bytes,mime}` para que un kernel produzca un PNG y la UI lo muestre directamente.
- **Wawa (SO bare‑metal).** Reactor cooperativo en `async_system/` (PIT 100Hz + IRQs). Apps son módulos WASM `cdylib` aislados por `wasmi`; capacidades **no se registran** en el linker si el bit del bitfield `Permisos` no está puesto (frontera física, no tabla de permisos). Almacenamiento direccionado por contenido (`almacen.rs`, BLAKE3 + log + GC mark/sweep/swap). Protocolo de red propio (`akasha`) en EtherType propio, sin TCP/IP. Ver `WAWA.md` §0–§14.
- **Cosmos refactor astrométrico puro.** `cosmos-ephemeris` + `cosmos-skywatch` + `cosmos-sundial` + `cosmos-tides` + `cosmos-transits` son extractos independientes del motor astrológico (`cosmos-engine`). Sirven sundial / mareas / navegación / planning sin tocar la maquinaria de cartas.
- **`agora` firma y verifica Ed25519 end-to-end.** `agora-core::verify_signature` + `agora-channel::{verificar_raiz, verificar_canal, verificar_manifiesto}` cubren el userspace; el kernel `wawa-kernel/src/claves.rs` espeja con `verificar_manifiesto_firmado / verificar_anuncio_canal / verificar_cuaderno_firmado` (zero-alloc, `ed25519-compact`). Lo que sí queda pendiente (WAWA.md §14.1.3) es la **tabla de capacidades por bytecode hash**: derivar permisos de la firma sobre `(hash_bytecode, permisos)` en lugar de declararlos en `EntradaApp`.

## Después de cada bloque funcional

`git add` específicos + commit + push a `origin/main`, sin pedir permiso (operaciones no destructivas). El estilo de commit del repo: `tipo(scope): mensaje corto en minúsculas y español` (`git log` para ver el patrón vigente).
