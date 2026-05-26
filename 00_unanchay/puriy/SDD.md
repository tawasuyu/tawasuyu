# Puriy — navegador web soberano

> Puriy (quechua: *viajar, recorrer, caminar*). Tipo: **Web browser engine + chrome over Llimphi**.

## Tesis

Adaptar **Servo** (motor web Rust) como engine de DOM/CSS/JS/networking, y delegar **TODO el render** a [[Llimphi]]. Resultado: un navegador que corre idéntico en:

- **Linux/Wayland** — Llimphi monta sobre `mirada` (compositor).
- **Wawa bare-metal** — Llimphi monta sobre framebuffer directo (sin OS).

Una sola pila gráfica (`wgpu + vello + taffy + DAG monádico`), una sola superficie abstracta (`llimphi-hal::Surface`), dos targets.

## Por qué Servo, no Chromium/WebKit

- **Rust nativo** — sin FFI a C++. Tipos seguros, sin segfaults heredados.
- **Modular** — `servo` se compone de crates separados (style, layout, script, net) embebibles individualmente.
- **Sin política corporativa** — no Google, no Apple, no Mozilla mainstream. Linux Foundation desde 2024.
- **Compatible con wawa** — Servo no asume X11/Win32/macOS. Su superficie es abstraíble.

## Anatomía — 4 crates

```
[ CUADRANTE I · 0x00 UNANCHAY ]

4. puriy-app          — Binario lanzable (en mirada o en wawa)
   │                    (parsea CLI, instancia engine, abre Llimphi)
   ▼
3. puriy-llimphi      — Chrome del navegador
   │                    (toolbar, tabs, address bar, bookmarks)
   │                    Construido sobre llimphi-ui (DAG monádico)
   ▼
2. puriy-engine       — Bridge a Servo
   │                    (embebe script + style + layout + net)
   │                    Output: primitivas geométricas → llimphi-raster
   ▼
1. puriy-core         — Modelo agnóstico
   │                    (sesiones, tabs, history, bookmarks, perfiles)
   │                    Sin deps de Servo ni de Llimphi
   ▼
[ Estado puro ]
```

## Fases de forja

### Fase 1 — `puriy-core` (modelo agnóstico)

- `Session`, `Tab`, `History`, `Bookmark`, `Profile` puros.
- Sin deps de gráficos ni de Servo.
- Testeable con `cargo test`.
- **Hito:** abrir un Profile en disco, crear/cerrar tabs, navegar (mock).

### Fase 2 — `puriy-engine` (embed de Servo)

- Agregar deps de los crates Servo necesarios (no todo Servo, solo `script`, `style`, `layout`, `net`, `webrender_api` quizá).
- Bridge entre `puriy-core::Tab` y la pipeline Servo.
- **Decisión arquitectónica clave:** ¿usamos `webrender` interno de Servo o forzamos toda primitiva a pasar por `llimphi-raster`?
  - **Opción A (pragmática):** `webrender` para el viewport del documento, Llimphi para el chrome. Servo se mantiene cerca de upstream.
  - **Opción B (purista):** Interceptar el `Display List` de Servo y traducirlo a primitivas Vello dentro de `llimphi-raster`. Más trabajo, soberanía total.
  - **Decisión:** Empezar con A en Fase 2; migrar a B cuando Llimphi madure y haya un caso de uso real (ej: renderizar páginas en wawa sin pulling webrender entero).
- **Hito:** Cargar `https://example.com` y renderizar el DOM parseado en una textura wgpu.

### Fase 3 — `puriy-llimphi` (chrome)

- Toolbar (back/fwd/reload/url) + tabs + sidebar opcional.
- Construido sobre `llimphi-ui` (DAG monádico).
- Eventos de teclado: Ctrl+T (nuevo tab), Ctrl+W (cerrar), Ctrl+L (focus address bar), etc.
- **Hito:** Chrome funcional sin engine (engine devuelve mocks).

### Fase 4 — `puriy-app` (binario)

- CLI: `puriy [URL] [--profile NAME] [--target wayland|framebuffer]`.
- Detección automática del target (si hay variable `WAYLAND_DISPLAY` → mirada; si no, framebuffer wawa).
- **Hito:** `puriy https://gioser.net` abre y renderiza la landing del propio repo.

## Pila exacta

| Capa | Crate raíz | Deps externas |
|---|---|---|
| Core | `puriy-core` | (puro Rust) |
| Engine | `puriy-engine` | `servo` (selección de crates), `tokio`, `url` |
| Chrome | `puriy-llimphi` | `llimphi-ui`, `llimphi-layout`, `llimphi-raster` |
| App | `puriy-app` | todo lo anterior + `clap` |

## Targets de salida (vía `llimphi-hal::Surface`)

| Target | Surface impl | Cuándo |
|---|---|---|
| Wayland (dev / desktop normal) | `WinitSurface` sobre `mirada-compositor` | Linux con sesión gráfica |
| Framebuffer bare-metal | `WawaFramebufferSurface` (impl en `03_ukupacha/wawa/`) | Cuando `wawa` es PID 1 y no hay OS host |
| Headless (tests / CI) | `HeadlessSurface` (sin display) | `cargo test`, screenshots |

## Estado

- **2026-05-25:** SDD escrito. Esqueletos de los 4 crates creados (sin deps de Servo todavía).
- **2026-05-26:** Fase 2 — `puriy-engine` real. Deps de Servo embebidos: `html5ever 0.39` (parser HTML), `markup5ever_rcdom 0.39` (DOM), `cssparser 0.35` (anchor; el subset CSS se parsea con un mini-parser propio porque la API de cssparser rotó entre 0.33→0.35 y nuestro subset es trivial), `url 2`. Net síncrono con `ureq` (no tokio en el engine). Pipeline `fetch → parse_html → parse_styles → build_box_tree → BoxTree` operativo: `cargo run -p puriy-app -- https://example.com` baja la página, parsea DOM + UA stylesheet + `<style>` inline + atributo `style="..."`, y dumpea el árbol de boxes. 10/10 tests verde. **Decisión arquitectónica:** se eligió Opción A (pragmática) — webrender se mantiene fuera por ahora, el box tree pasa directo a `llimphi-raster`. Opción B (interceptar Display List Servo→Vello) se reconsidera cuando el motor adopte Stylo entero.
- **2026-05-26:** Fase 3 — `puriy-llimphi` real. `App` Llimphi (`Puriy`) con header (URL + status) + viewport blanco. Worker thread carga la URL; el `BoxTree` cruza al UI thread por `Handle::dispatch` (el `DomTree` con `Rc<Node>` queda en el worker y se dropea ahí — es `!Send`). Conversión recursiva `BoxNode → View<Msg>`: blocks columnan, inlines fluyen en row, colores y spacing mapean a `Style` de taffy. F5 recarga. `puriy-app` autodetecta target: `WAYLAND_DISPLAY`/`DISPLAY` → ventana Llimphi; sino → headless. **Probar:** `cargo run -p puriy-app -- https://example.com` (abre ventana). `cargo run -p puriy-app -- https://example.com --target headless` (dumpea árbol).
- **Bloqueado por:** nada. Fase 4 (binario polish + perfil persistente) y mejoras visuales (scroll, links clickables, raster background-image) quedan como siguientes incrementos.

## Relacionados

- [[project-llimphi]] — la pila gráfica que puriy consume
- [[project-mirada]] — compositor Wayland donde puriy abre ventana en Linux
- [[project-wawa]] — kernel SASOS donde puriy abre framebuffer bare-metal
- [[project-pluma]] — visor markdown hermano (ambos en 00_unanchay, ambos visualizadores)
