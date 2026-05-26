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
- **2026-05-26 (tarde):** Fase 3 polish. (1) **Scroll vertical**: wheel + PageUp/Dn + ArrowUp/Dn + Home/End, mediante `Position::Absolute` + `inset.top = -scroll_y` + `clip(true)` en el outer. (2) **Links clickables**: `BoxNode.link` resuelve `<a href>` contra base URL (engine), el chrome dispara `Msg::Navigate(url)` y recolorea el subárbol del `<a>` en azul. (3) **Address bar editable**: `TextInputState` (single-line del editor compartido) sobre `text_input_view`; click foca el input, Enter navega, Esc cancela. (4) **Polish**: UA stylesheet con `font_weight` (h1..h6/b/strong/th = 700), parser CSS para `font-weight`, `<li>` prefija bullet, `pre/blockquote/hr` añadidos a block defaults. Bold se simula con `font_size × 1.1` mientras `llimphi-text` no exponga el eje weight.
- **2026-05-26 (noche):** Features de chrome. (a) **Historial por pestaña**: `TabState.history: Vec<String>` + `cursor`. Navigate trunca y empuja; Back/Forward recargan sin push. Botones ◀ ▶ ⟳ + atajos Alt+←/→. (b) **Pestañas múltiples**: `Model.tabs: Vec<TabState>` + `active`. Barra superior con click-para-activar y "✕" para cerrar; "+" abre nueva. Atajos Ctrl+T / Ctrl+W / Ctrl+Tab / Ctrl+Shift+Tab. Cada Msg async lleva `tab: TabId, gen: u64` — si la pestaña fue cerrada o pisada por otra navegación, el resultado se descarta. (c) **`<img>` mínimo**: engine descarga + decodifica (`image` crate, PNG+JPEG) sync dentro del worker; `BoxNode.image: Option<ImageData>` (RGBA8 + w/h). Chrome envuelve en `peniko::Image` y lo aplica con `.image(img)`. Fallback `[img: alt]` si la decodificación falla.
- **2026-05-26 (madrugada):** Fix amontonado. Hojas de texto reciben `height = font_size × 1.4` (line-height aproximado) — sin esto, taffy colapsaba los inlines al top del bloque. Bloques con sólo hijos inline/inline-block conmutan a `FlexDirection::Row + FlexWrap::Wrap` para que los tokens fluyan en múltiples líneas. Bloques con hijos block siguen en `Column` sin wrap.
- **2026-05-26 (siguiente):** Fidelidad de render. (a) **Whitespace inline**: `collapse_whitespace` en `NodeData::Text` reemplaza el `trim` duro — colapsa runs internos a un espacio, preserva uno leading/trailing si existía. Resuelve casos `foo <a>bar</a> baz` que antes salía como `foobarbaz` pegado. Whitespace-only nodes entre inlines se conservan como `" "` para no perder la separación. (b) **Selectores `.class` y `#id`**: `Selector` enum (`Universal | Type | Id | Class`); el parser parsea selectores simples con prefijo `.` o `#` (alfanuméricos + `-_`); combinadores/pseudoclases/atributos se ignoran en silencio. UA stylesheet migrado al enum. Tests `selector_class_matchea` y `selector_id_matchea`.
- **2026-05-26 (continuación):** Network + CSS. (a) **Cache de bytes**: módulo `puriy_engine::cache` con `HashMap<String,Vec<u8>>` global protegido por `Mutex`, LRU por orden de inserción, cap 64 MB con eviction. `fetch_bytes` y `fetch_and_decode` consultan cache antes de salir a la red — recargas (F5), back/forward y navegación entre tabs del mismo origen son instantáneas. (b) **Selectores descendientes**: `Selector` ahora es compound (`parts: Vec<Simple>`); `Simple` mantiene `Universal | Type | Id | Class`. El matcher recorre ancestros greedy de derecha a izquierda usando `node.parent` de `markup5ever_rcdom`. Acepta `.menu li`, `nav a`, `#hero h2`, etc. Pseudoclases y combinadores `> + ~` siguen ignorándose. Test `selector_descendiente_matchea` (15/15 verde).
- **2026-05-26 (sgte):** Selectores CSS — combinadores y compounds. (a) **Simples compound**: el viejo `enum Simple` se aplastó en `struct Compound { tag, ids, classes }` — un único compound puede mezclar tag + N ids + N classes (`a.btn`, `p#hero.alert`, `*.foo`). El parser barre el token con un cursor de bytes en vez de `chars().all(is_ident_char)`. (b) **Combinadores `> + ~`**: `Selector` ahora lleva `compounds: Vec<Compound>` + `combinators: Vec<Combinator>` (descendant/child/adjacent-sibling/general-sibling). `normalize_combinators` inyecta espacios alrededor de `>`/`+`/`~` antes de tokenizar. El matcher viaja derecha→izquierda y por cada combinador salta a parent o a `prev_element_sibling` (que saltea text nodes). Sin pseudoclases ni `[attr]` todavía — la regla se ignora si aparecen. Tests: `selector_compound_matchea`, `selector_hijo_directo_matchea`, `selector_hermano_adyacente_matchea`, `selector_hermano_general_matchea` (19/19 verde).
- **2026-05-26 (sgte+1):** Selectores CSS — atributos y pseudoclases estructurales. `Compound` se infló con `attrs: Vec<AttrMatch>` y `pseudos: Vec<Pseudo>`. (a) **Atributos**: `[attr]` (presencia), `[attr=v]` (igual), `[attr^=v]` (prefijo), `[attr$=v]` (sufijo), `[attr*=v]` (substring). `parse_attr_match` busca operadores en orden `^=`/`$=`/`*=`/`=` y trimea comillas. (b) **Pseudoclases estructurales**: `:first-child`, `:last-child`, `:only-child`, `:first-of-type`, `:last-of-type`. `pseudo_matches` filtra hijos Element del padre (saltea Text nodes) y resuelve por posición / tipo. `:hover`/`:focus`/`:active` quedan fuera porque requieren tracking del chrome; `:not(...)` y `:nth-child(...)` también: cualquier paréntesis tras `:` rechaza el selector entero. (c) **`normalize_combinators`** ahora respeta lo que vive dentro de `[…]` o `(…)` — `[href*="a>b"]` no se rompe. Tests nuevos: `selector_attr_presente`, `selector_attr_equals`, `selector_attr_prefix_suffix_contains`, `selector_first_last_only_child`, `selector_first_last_of_type` (24/24 verde).
- **2026-05-26 (sgte+2):** Propiedades CSS de layout. `ComputedStyle` gana `width`/`max_width: LengthVal` (`Auto`/`Px`/`Pct`), `text_align: TextAlign` (`Left`/`Center`/`Right`/`Justify`) y `line_height: Option<f32>` (multiplicador). `parse_length_or_pct` acepta `auto`, `Npx`, `Nem`/`Nrem`, `N%`; `parse_line_height` traga `1.5` (mult adimensional), `24px`, `1.5em`. `BoxNode` propaga los cuatro campos; `puriy-llimphi::box_style` mapea `width`/`max_width` a `Size`/`max_size` de taffy (`Pct(80)` → `percent(0.8)`), `text-align` de un bloque con hijos inline a `justify_content` del Row (`Center`/`End`), y `line-height` reemplaza el `1.4` hardcodeado en la altura de hojas de texto. Tests nuevos: `parsea_width_max_width`, `parsea_text_align`, `parsea_line_height`, `computa_width_y_text_align` (28/28 verde).
- **2026-05-26 (sgte+3):** Inheritance CSS real. **Bug original**: cada hoja de texto se construía con defaults hardcoded (`BLACK`, `16px`, `400`), aunque su `<p>` padre dijera `color:red; font-size:20px`. **Fix**: `StyleEngine::compute_with_parent(node, parent)` arranca el `ComputedStyle` copiando del padre las propiedades CSS-inheritables (`color`, `font_size`, `font_weight`, `text_align`, `line_height`) y deja en default las no-heredables (`background`, `display`, `margin`, `padding`, `width`, `max_width`). `boxes::build_node` threadea el `parent_style` recursivamente; las hojas Text se construyen con `inline_text_with_style(s, parent_style)`. Refinamiento: `font_weight` por tag (`<b>`/`<strong>`/`h1..h6`/`<th>` = 700) sólo se aplica si el tag pinta bold; si no, se respeta lo heredado — así un `<b>` dentro de un `<p>` no-bold sigue siendo bold sin pisar herencia desde adentro hacia afuera. Tests nuevos: `hereda_color_y_font_size_del_padre`, `no_hereda_propiedades_no_heredables`, `font_weight_bold_local_no_propaga_a_padre_no_bold`, `box_tree_propaga_color_a_hoja_de_texto` (32/32 verde).
- **Bloqueado por:** nada. Siguiente: font-weight real cuando llimphi-text lo abra, fetch async / cancelable, `:hover`/`:focus` (requiere mouse tracking en el chrome), `:not(simple)` y `:nth-child(an+b)`, persistencia de la cache entre sesiones, `border` + `border-radius` + `box-shadow`, especificidad real (hoy "último gana" — un `#id` no vence a `body p` si llega antes en el stylesheet).

## Relacionados

- [[project-llimphi]] — la pila gráfica que puriy consume
- [[project-mirada]] — compositor Wayland donde puriy abre ventana en Linux
- [[project-wawa]] — kernel SASOS donde puriy abre framebuffer bare-metal
- [[project-pluma]] — visor markdown hermano (ambos en 00_unanchay, ambos visualizadores)
