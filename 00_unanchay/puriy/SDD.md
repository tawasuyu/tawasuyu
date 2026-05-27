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
- **2026-05-26 (sgte+4):** Especificidad CSS real. **Bug original**: cascada por "último gana" — `#hero{color:blue}` perdía contra `body p{color:red}` si llegaba antes en el stylesheet. **Fix**: `Selector::specificity()` calcula el clásico `a*100 + b*10 + c` (a = #id, b = .class + [attr] + :pseudo-class, c = tags). En `compute_with_parent`, las reglas que matchean se recolectan con `(specificity, source_index, &Rule)` y se ordenan ASC antes de aplicarse — empate de especificidad lo desempata el orden de aparición. Inline `style="..."` sigue al final con especificidad implícita 1000. Tests: `specificity_calculada_correctamente` (incluye `a.btn[href^="https"]:first-child` = 31, `nav > a#x.y` = 112), `id_vence_a_tag_aunque_llegue_antes`, `clase_vence_a_tag`, `inline_style_vence_a_id`, `empate_de_especificidad_gana_el_ultimo` (37/37 verde).
- **2026-05-27:** `!important`. `Decl` ahora es `struct Decl { kind: DeclKind, important: bool }` — cada declaración puede ser marcada individualmente. El parser detecta el sufijo `!important` (case-insensitive) al final de cada value antes de pasar el resto a `decl_kind_from_pair`. La cascada hace dos pasadas en orden: (1) normales ordenadas por (specificity, source_index) más inline normal, (2) `!important` con mismo orden más inline `!important`. Cualquier important vence cualquier normal del mismo origen, lo cual abre la puerta a que stylesheets de autor "fuerzan" overrides. Tests: `important_vence_normal_de_mayor_especificidad`, `important_inline_vence_important_de_id`, `normal_inline_pierde_contra_important_de_regla` (40/40 verde).
- **2026-05-27 (sgte):** `border` + `border-radius`. (a) **Engine**: `ComputedStyle` gana `border_width: f32`, `border_color: Option<Color>`, `border_radius: f32`. `DeclKind` agrega `BorderWidth`, `BorderColor`, `BorderEnabled(bool)`, `BorderRadius`. `border-style: solid|dashed|dotted|double` activa el dibujo; `none|hidden` lo desactiva (color → None, width → 0). El shorthand `border: 2px solid #f00` se expande en `parse_declarations` a 3 decls atómicas vía `parse_border_shorthand` — los tokens pueden venir en cualquier orden (cada uno se prueba contra parse_length_px / parse_color / parse_border_style). (b) **Chrome**: `apply_border(view, &BoxNode)` aplica `View::radius(border_radius)` + `paint_with(...)` con `vello::Scene::stroke` sobre un `RoundedRect` insetado por `width/2` para que el trazo caiga dentro del rect (vello pinta centrado al path). Tests: `parsea_border_shorthand` (incluye `!important` y `border: none`), `parsea_border_propiedades_individuales` (42/42 verde).
- **2026-05-27 (sgte+1):** `:hover` con scope limitado. `Pseudo::Hover` se evalúa contra un flag externo `hover_active: bool` que el caller threadea (`compute_with_parent_in_state` lo expone). En `boxes::build_node` cada Element computa dos veces — sin hover (estilo base) y con hover_active=true; el delta sólo se captura en `hover_background: Option<Color>` por ahora. El chrome lo plug-ea con `View::hover_fill(...)` que ya tenía Llimphi para transiciones de bg. **Limitaciones explícitas**: (a) sólo se propaga el delta de background — color/border/font-weight no cambian on hover; restyle completo requeriría re-mount del view tree por evento de mouse. (b) `:hover` sólo aplica al sujeto del selector (último compound); `nav:hover a` no propaga el chain de hover a los ancestros. Suficiente para 90% del uso real (`.btn:hover`, `a:hover`). Tests: `hover_state_activa_regla_solo_cuando_corresponde`, `hover_pseudo_aporta_a_specificity`, `box_tree_expone_hover_background` (45/45 verde).
- **2026-05-27 (sgte+2):** Persistencia de cache entre sesiones. `cache::load_from_disk()` y `cache::flush()` serializan el bytes-cache a `$XDG_CACHE_HOME/puriy/cache.bin` (fallback `$HOME/.cache/puriy/cache.bin`). Formato binario manual sin deps externas: magic `PUYC` + versión u8 + count u32 LE + por entrada `[u32 url_len][url][u32 data_len][data]`. Escritura atómica vía `cache.bin.tmp` + rename. `puriy-app::main` llama `load_from_disk` al startup y `flush` al cierre; el worker del chrome también flush después de cada navegación exitosa para no perder todo si el proceso muere. Best-effort: errores I/O son silenciosos (perder el flush no rompe la sesión, sólo significa cold start la próxima). Tests: `codec_round_trip`, `decode_rechaza_magic_invalida` (47/47 verde).
- **2026-05-27 (cierre fase 4):** **HITO ALCANZADO** — `puriy https://gioser.net --target headless` parsea la landing del propio repo (título correcto, 455 boxes, jerarquía completa: header/main/aside/ul/li, footer, SVG). Cableado de `puriy-core` que faltaba: (a) **`--profile NAME` funcional**: `load_or_create_profile` resuelve `$XDG_CONFIG_HOME/puriy/profiles/NAME/` (fallback `~/.config/...`), carga `profile.json` con `puriy_core::store::load` o crea fresh con `Profile::nuevo`. Cada profile tiene su propio `cache.bin` (vía `cache::set_persist_path`). (b) **History global persistente**: en `run_headless` y en el chrome (`Msg::Loaded`), cada navegación exitosa llama `profile.history.record(url, title, now)` y `persist_all`/`persist_profile` guarda el JSON atómicamente. (c) **Bookmark con Ctrl+D**: nueva `Msg::Bookmark` que toma la URL+título de la pestaña activa, llama `profile.bookmarks.add` si no estaba (dedup por url), actualiza el status bar (`⭐ guardado · N bookmarks` / `⭐ ya estaba guardado`), y persiste. (d) **Tests E2E**: `puriy --target headless --profile demo` corrido dos veces preserva `history.len == 2` con ambas URLs (gioser.net + example.com). 62 tests verde (47 engine + 15 core).
- **Estado Fase 4:** CERRADA. Lo que queda es para fases siguientes: font-weight axis real (depende de llimphi-text), fetch async paralelo de assets (refactor del worker), `:hover` restyle completo (color/border), `:focus`/`:active`, `:not(simple)`, `:nth-child(an+b)`, cache con TTL respetando `Cache-Control`, `box-shadow`, JS engine via Servo `script` crate (Fase 5+).

## Relacionados

- [[project-llimphi]] — la pila gráfica que puriy consume
- [[project-mirada]] — compositor Wayland donde puriy abre ventana en Linux
- [[project-wawa]] — kernel SASOS donde puriy abre framebuffer bare-metal
- [[project-pluma]] — visor markdown hermano (ambos en 00_unanchay, ambos visualizadores)
