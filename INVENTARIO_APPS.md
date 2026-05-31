# Inventario de apps gioser — estado y completitud

> Estado al **2026-05-31**. Los **% son estimaciones** (basadas en fases ✓/pendiente de `PLAN.md`, SDDs, tests verdes y código real vs stubs — no auditoría formal). Ordenado por cuadrante.

## 00_unanchay — PERCIBIR

| App | % | Qué falta |
|---|---|---|
| **pluma** (`pluma-app`, `pluma-deck-app`, `pluma-notebook-app`) | ~82% | Embeddings reales conectados (verbo-daemon), backend LLM que *genere* traducciones (hoy tabla pre-poblada), vista matriz, búsqueda transversal viva, federación minga, export docx lado-a-lado. Deck: PDF + notas de orador + ruta SVG en HTML. |
| **khipu** (`khipu-app`) | ~70% | Integrar embeddings al algoritmo de gravedad (similitud → refuerzo), UI de búsqueda/filtros, export/import, tests E2E de federación. |
| **rimay** (`verbo-daemon`) | ~95% | Backend remoto Cohere, unit systemd para auto-arranque, compilar a WASM. |
| **chaka** (`chaka-app`, CLI) | ~65% | UI Llimphi (hoy CLI-only), ampliar COBOL (CICS, SQL embebido, Db2/IMS), ficheros indexed/relative, target WASM (`no_std`). |
| **pineal** (10 demos) | ~90% | Consumidores vivos reales (cosmos/dominium/nakui/takiy), interactividad (click/zoom persistido), leyendas con drill-down, animaciones, vista 3D. |
| **puriy** (`puriy-app`, navegador) | ~75% | Motor JavaScript (hoy stub), forms interactivos, media-queries/responsive, flexbox/grid expuestos, **bare-metal wawa** (bloquea el Hito #2). |

## 01_yachay — CONOCER

| App | % | Qué falta |
|---|---|---|
| **cosmos** (`cosmos-app-llimphi` + CLIs/server) | ~70% | Fondo de continentes en AstroCarto, form de birth-data in-situ (hoy JSON manual), store de cartas con sidebar tree, corpus humano de interpretación, validación research. |
| **dominium** (`dominium-app-llimphi`, `dominium-cli`) | ~75% | Sliders de parámetros en panel (UI), editor avanzado de Conceptos, export `.webm`, validación contra simulaciones externas. |
| **nakui** (`nakui-ui-llimphi`, `nakui-sheet-llimphi`, `nakui-explorer-llimphi`) | ~70% | Módulos ERP verticales reales (inventory/CRM — hoy solo `ventas`+`tesoro` demo), **yupay** (motor de fórmulas es/qu, no empezado), validación Rhai integrada en UI. |
| **iniy** (`iniy-cli`, `iniy-server`, `iniy-wiki`, `iniy-explorer-llimphi`) | ~40% | Pipeline end-to-end (graph+NLI+reportería incompletos), backend NLI local operativo, backend LLM cableado, dirección de subjetividad, validación con datasets. |
| **tinkuy** (`tinkuy-sim`, `tinkuy-llimphi`) | ~50% | DSL validada + benchmarks de optimizador, ABI WASM, integración Wawa (kernel cdylib), validación física real. |

## 02_ruway — HACER

| App | % | Qué falta |
|---|---|---|
| **llimphi** (motor: hal/raster/layout/text/ui + gallery) | ~85% ✓ | Sistema de animaciones (tweens/springs), a11y (screen-reader), theming por archivo, **`View::custom_pass(wgpu)` 3D** (lo necesita supay Fase 3). |
| **media** (`media-app`, `media-recorder-app`) | ~70% | Recorder con UI pulida (región, preview, progreso), playlist avanzada, spectrogram/phase, streaming de red (RTMP/HLS). |
| **nada** (editor) | ~75% | File watcher (cambios externos), save-as/rename, confirmación de overwrites, multi-cursor, sesiones persistentes. |
| **shuma** (`shuma-shell-llimphi`, `shuma`, `shuma-daemon`, `shuma-gateway`) | ~65% | Integración visual de **Matilda** (crear/arrancar VMs), persistencia de sesiones, completions context-aware. |
| **nahual** (`nahual-shell-llimphi`, `nahual-gallery-llimphi`) | ~60% | Visores editables (hoy read-only), full-text search, thumbnails de video, exploración de archives sin descomprimir. |
| **mirada** (`mirada-llimphi`, `-asistente`, `-launcher`, `-compositor`, `-greeter`, `-ctl`) | ~55% | Shell + DM en hardware real (Wayland nativo, gestor de sesión, autostart, persistencia de workspace), asistente con LLM, multi-pantalla/DPI. |
| **chasqui** (`chasqui-broker-explorer-llimphi`, `chasqui-explorer-llimphi`) | ~50% | Productividad: WAL persistente, replicación P2P, routing tipo MQTT, ACLs+cifrado, ingesta de eventos reales. |
| **takiy** (`takiy-app-llimphi`) | ~45% | **Generador IA de sonidos** (no existe), browser de SoundFonts, secuenciador pro (patterns/arpegiadores), grabación de audio. |
| **supay** (`supay-app-llimphi`, `supay-doom-llimphi`) | ~35% | Fase 2 (extracción completa BSP/segs/nodes), **Fase 3 renderer 3D** (bloqueada por `custom_pass` de Llimphi), validación bit-exact, vendor doomgeneric documentado. |

## 03_ukupacha — RAÍZ

| App | % | Qué falta |
|---|---|---|
| **wawa** (kernel SASOS, **V1.0.0-GOLD** + 14 apps WASM) | ~88% | Ring buffer de N raíces en superbloque + menú "anclas recientes" en boot (rollback), enum multi-output GPU (hoy 1 scanout), revocación de identidades kernel↔userspace. Apps maduras: pluma 90%, mudanza 85%, asistente/bitácora 75-80%; las demás 50-75%. |
| **agora** (`agora-cli`, `agora-app`) | ~75% | CLI: UI de revocación, gossip anti-entropía propio. App Llimphi (~60%): edición in-situ de identidades, TrustGraph interactivo, políticas de confianza evaluables. |
| **minga** (`minga-cli`, `minga-explorer-llimphi`) | ~72% | `minga pull` selectivo, merge multi-rama (rebase/cherry-pick), blame/annotate, firma de commits (agora). Explorer: drill-down en árbol, búsqueda, diff visual. |
| **wawa-explorer** (`wawa-explorer-llimphi`) | ~75% | Búsqueda por hash, renderizado contextual del nodo, export de subárbol a `.tar`, sync remota AoE inline. |
| **arje** (`arje-packager`, `arje-installer`, `arje-absorb`) | ~52% | Soporte aarch64 completo, compresión de imágenes, particionado inteligente + rollback automático del installer, dedup entre instantáneas. |
| **sandokan** (shared; `sandokan-app`, `sandokan-daemon`) | ~30-40% | Fase 0: handlers vacíos en la app, sin consumidores reales (debe ser consumido por shuma). |

## Lectura rápida

- **Más maduros (>80%):** rimay (95), pineal (90), wawa-kernel (88), llimphi (85), pluma (82).
- **Núcleo gráfico es el cuello de botella:** llimphi al 85% bloquea el 3D de supay y refinamientos de varias UIs (todas las apps GUI dependen de sus 4 fases — hoy estables).
- **Patrón común de lo que falta:** validación con datos reales/externos, backends remotos (LLM/embeddings vía daemon), y pasar de *demos* a *casos de usuario reales* (módulos ERP de nakui, productividad de chasqui, generador IA de takiy).
- **Menos avanzados:** sandokan (~35%), supay (~35%), iniy (~40%), takiy (~45%).
