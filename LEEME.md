<p align="center"><img src="docs/brand/chakana-512.png" alt="tawasuyu" width="116"></p>

# tawasuyu

**Una suite vertical de software, construida desde el metal hacia arriba.**

*Read this in English: [README.md](README.md).*

tawasuyu es la respuesta de una persona a una pregunta simple: *¿cómo sería
la computación si fueras dueño de todas sus capas?* Es un único workspace de
Rust de ~520 crates que contiene, entre otras cosas:

- un **sistema operativo** que bootea en metal desnudo, sin Linux abajo (*wawa*),
- un **motor gráfico** con sus propios widgets, layout, texto y pipeline GPU (*llimphi*),
- un **compositor Wayland y window manager** (*mirada*),
- un **motor de navegador web** (*puriy*),
- un **entorno de escritura** donde un documento vive como muchos cuerpos paralelos (*pluma*),
- un **ERP**, un **motor de astronomía**, un **DSL de física**, un **sistema de
  notas P2P**, un **editor de imágenes**, un **motor musical**, un **cliente de
  correo nativo**, una **terminal**… y el pegamento que los hace un sistema
  coherente en vez de una pila de programas.

Todo se apoya en las mismas bases nativas — almacenamiento direccionado por
contenido (BLAKE3 + DAG + postcard), identidad Ed25519, una capa P2P — y los
formatos ajenos (psd, xlsx…) entran sólo por puentes explícitos. Sin
Electron, sin stack web en las apps de escritorio, sin toolkit de UI heredado.

Es un sistema vivo en movimiento, no un producto pulido. El código es la
documentación de una arquitectura; este documento es la puerta de entrada.

## Probalo en cinco minutos

Necesitás Rust estable (nightly sólo para el SO bare-metal). Después:

```bash
git clone https://git.tawasuyu.net/tawasuyu/tawasuyu.git
cd tawasuyu
cargo check --workspace   # el smoke test mínimo de la suite
```

Elegí algo para correr:

| Querés ver… | Corré |
|---|---|
| La galería de widgets del motor gráfico | `cargo run -p llimphi-gallery --release` |
| Un editor rápido con árbol de archivos | `cargo run -p nada --release` |
| Un documento como muchos cuerpos paralelos (traducción/tono/resumen) | `cargo run -p pluma-editor-llimphi --example multilienzo_completo_demo --release` |
| El banco de trabajo de astronomía/astrología | `cargo run -p cosmos-app-llimphi --release` |
| Física de partículas desde un DSL | `cargo run -p tinkuy-llimphi --example tinkuy_demo --release` |
| Un editor de imágenes por capas, no destructivo | `cargo run -p tullpu-app-llimphi --release` |
| La terminal / shell de espacios de trabajo | `cargo run -p shuma-shell-llimphi --release` |
| Un cliente de correo nativo (IMAP/SMTP, búsqueda semántica, "rail" P2P firmado) | `cargo run -p paloma-app --release` |
| Un gestor de procesos (unidades Linux, controles vivos) | `SANDOKAN_MONITOR_SEED=1 cargo run -p sandokan-monitor-llimphi --release` |
| Un launcher de escritorio (barras, dock, menú global) | `cargo run -p launcher-llimphi --example launcher_demo` |
| **El sistema operativo booteando en QEMU** | `cd 03_ukupacha/wawa && cargo +nightly run -p boot -Z bindeps` |

Muchos crates traen más `examples/*_demo.rs` — son la forma esperada de
probar una feature sin levantar la suite completa.

¿Sin toolchain? Hay una **imagen demo pre-forjada de wawa** (~1.3 MB):
descargá [wawa-latest.tar.zst](https://tawasuyu.net/dist/wawa-latest.tar.zst),
extraé, `./correr.sh` — el SO bootea en QEMU en menos de un minuto (necesita
`qemu-system-x86_64` + OVMF).

## El mapa

El filesystem *es* la arquitectura. El workspace se organiza en cuatro
cuadrantes que espejan las cuatro fases del ciclo de la información:

```
tawasuyu/
├── 00_unanchay/   PERCIBIR — pluma · khipu · rimay · chaka · pineal · puriy
├── 01_yachay/     CONOCER  — cosmos · dominium · nakui · iniy · tinkuy
├── 02_ruway/      HACER    — mirada · shuma · nahual · chasqui · takiy · llimphi · paloma
│                             supay · media · nada · tullpu · churay · hapiy · cards · wawa (host)
├── 03_ukupacha/   RAÍZ     — arje · wawa (kernel + apps WASM) · agora · minga
│                             sandokan · wawa-explorer
├── shared/        núcleos transversales — sandokan · format · card · auth · ssh
│                             foreign-psd · rimay-localize · app-bus · launcher
└── web/           la landing por la que quizás llegaste (no es producto)
```

Mover un dominio de cuadrante cambia su naturaleza — no son carpetas
administrativas. Un quién-es-quién rápido:

- **pluma** — documentos vivos: un material, muchos cuerpos (idioma, tono,
  audiencia), alineados párrafo a párrafo; más un notebook reactivo.
- **khipu** — notas que se desvanecen si la atención no las mantiene vivas;
  P2P, local-first.
- **rimay** — lenguaje: daemon de embeddings, localización.
- **puriy** — un motor de navegador web (CSS/layout/JS vía QuickJS).
- **cosmos** — astronomía + astrología: efemérides, cielo, mareas, cartas, y
  una esfera celeste 3D corriendo sobre el motor GPU de llimphi.
- **dominium** — simulación; **tinkuy** — DSL de física; **nakui** — un ERP;
  **iniy** — verificación de afirmaciones.
- **llimphi** — el motor gráfico sobre el que se construye todo lo visual
  (`wgpu` + `vello` + `taffy` + `parley`, bucle Elm, ~44 widgets).
- **mirada** — compositor Wayland / window manager / display manager.
- **shuma** — terminal y runtime de espacios; **nada** — un editor rápido;
  **nahual** — visores universales; **tullpu** — editor de imágenes;
  **takiy** — música; **media** — audio/video; **supay** — un motor 3D retro;
  **chasqui** — broker de mensajes.
- **paloma** — correo nativo: IMAP/SMTP, búsqueda semántica, resumir/borrador
  con LLM (local-first), mensajes firmados Ed25519, y un *rail* soberano P2P
  donde la dirección *es* la clave pública — sin spoofing del remitente.
- **churay** — instalador/actualizador gráfico estilo Office para la suite en
  cualquier Linux (catálogo de apps, instalación con un clic, entradas
  `.desktop`); comparte un formato de hash direccionado por contenido con **hammer**.
- **hapiy** — captura de pantalla (el "Spectacle"): un cliente `zwlr_screencopy`
  soberano que atrapa lo que pinta mirada y pasa la captura a tullpu para anotar.
- **arje** — init; **agora** — identidad y firmas Ed25519 end-to-end;
  **minga** — colaboración P2P; **sandokan** — el plano de control (quién
  arranca, para, supervisa y observa unidades en Linux y en wawa).
- **wawa** — el sistema operativo: kernel SASOS para `x86_64-unknown-none`,
  reactor cooperativo, apps WASM aisladas por bits de capacidad,
  almacenamiento direccionado por contenido, y su propio protocolo de red
  (*akasha*) sobre un EtherType crudo — sin TCP/IP.

Cada carpeta de dominio tiene su `README.md` (inglés) y `LEEME.md`
(español); los dominios complejos llevan además un `SDD.md` — el documento
de diseño autoritativo. Estos mismos archivos son los que sirve
[tawasuyu.net](https://tawasuyu.net).

## La arquitectura, en breve

Cinco reglas le dan forma a todo:

1. **Un dominio = un crate raíz con subcrates plugin.** Sin proliferación
   lateral; los crates se parten al pasar ~1.500–2.000 LOC.
2. **Las UIs son frontends intercambiables sobre crates `*-core` agnósticos.**
   La lógica de dominio nunca sabe quién la pinta.
3. **Todo lo gráfico pasa por llimphi.** Un motor, un bucle estilo Elm
   (`input → update → view → layout → raster → present`), widgets y theme
   compartidos.
4. **Los formatos ajenos entran por puentes `shared/foreign-*`**, nunca al
   núcleo de una app. Las apps trabajan en el formato nativo: DAGs
   direccionados por BLAKE3, serializados con postcard.
5. **`cargo check --workspace` debe pasar siempre en `main`.** El CI lo
   custodia.

Los tipos que cruzan fronteras — por la red *akasha*, a disco direccionado
por contenido, o entre kernel y userspace — viven en crates `no_std`
(`format`, `akasha`, `mirada-layout`, `forth-emisor`, `pluma-notebook-core`),
validados por `./scripts/check-shared-cores.sh`.

## Compilar las partes inusuales

El workspace compila con Rust estable. Dos piezas son especiales:

**El SO (`03_ukupacha/wawa`)** está excluido del workspace raíz: el kernel
apunta a `x86_64-unknown-none` con `panic = "abort"`. Necesita nightly con
`rust-src`, más los targets `wasm32-unknown-unknown` y `x86_64-unknown-none`:

```bash
cd 03_ukupacha/wawa/wawa-kernel
cargo +nightly check --target x86_64-unknown-none -Z build-std=core,alloc

cd 03_ukupacha/wawa
cargo +nightly run -p boot -Z bindeps      # forja imagen UEFI y bootea QEMU
./scripts/build-wawa-image.sh              # imagen QEMU/USB publicable
```

**La landing web (`web/tawasuyu-web`)** es el único cruce del puente JS en
el repo (wasm-bindgen):

```bash
./scripts/build-tawasuyu-web.sh dev        # o `release`
```

## Estado

Sistema de investigación personal activo, moviéndose rápido, con bordes
ásperos honestos. El kernel bootea end-to-end en QEMU; el compositor corre
sesiones reales sobre GPUs Intel; la mayoría de las apps son MVPs usables
("feo pero sirve" acá es una postura de diseño, no un accidente). Hay
extractos standalone de algunos dominios publicados como repos de entrada:
[llimphi](https://git.tawasuyu.net/tawasuyu/llimphi),
[mirada](https://git.tawasuyu.net/tawasuyu/mirada), y otros.

## Contribuir

Ver [CONTRIBUTING.md](CONTRIBUTING.md). Dos cosas para saber de entrada:
los mensajes de commit se escriben en **español** (convención del repo), y
los nombres con carga semántica fuerte (quechua/español: *khipu*, *rimay*,
*wawa*…) no se traducen nunca.

## Licencia

Triple licencia por área, ver [LICENSE.md](LICENSE.md): el default del
workspace es **MIT OR Apache-2.0**; seis crates fundacionales (`format`,
`forth-emisor`, `foreign-fs`, `wawa`, `wawa-kernel`, `wawa-fs`) son
**MPL-2.0**.

## Enlaces

- **Sitio:** [tawasuyu.net](https://tawasuyu.net) — sirve estos mismos documentos.
- **Fuente:** [git.tawasuyu.net/tawasuyu/tawasuyu](https://git.tawasuyu.net/tawasuyu/tawasuyu)
- **Plan y diseño:** [PLAN.md](PLAN.md), [WAWA.md](WAWA.md), `SDD.md` por dominio.
