//! Box tree — output del engine, entrada de `llimphi-raster`.
//!
//! Un [`BoxNode`] es la unidad de pintado: rectángulo con fondo opcional
//! + texto opcional + lista ordenada de hijos. No hay layout real (no
//! corremos taffy todavía) — sólo posicionamiento naive: cada bloque
//! apila vertical, cada inline se concatena en la línea. Es suficiente
//! para que Llimphi pueda dibujar example.com legible.
//!
//! Fase 3 reemplazará este pase por `llimphi-layout` con taffy.

use markup5ever_rcdom::{Handle, NodeData};

use crate::dom::{self, DomTree};
use crate::style::{
    AlignItems, AlignSelf, BoxShadow, BoxSizing, ComputedStyle, Corners, FlexDirection, FlexWrap,
    GridTrackSize, JustifyContent, LengthVal, LinearGradient, ListStyleType, Outline, Overflow,
    PointerEvents, Position, Sides, StyleEngine, TextAlign, TextDecorationLine, TextShadow,
    TextTransform, Transform, VerticalAlign, Visibility, WhiteSpace,
};

/// Color RGBA, 8 bits por canal. Suficiente para CSS color values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Color = Color::rgb_const(0, 0, 0);
    pub const WHITE: Color = Color::rgb_const(255, 255, 255);
    pub const TRANSPARENT: Color = Color { r: 0, g: 0, b: 0, a: 0 };

    pub const fn rgb_const(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }
    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::rgb_const(r, g, b)
    }
}

/// Modos de visualización soportados.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    Block,
    Inline,
    InlineBlock,
    /// CSS flexbox container (block-level). El layout se delega a taffy
    /// con `flex_direction`, `justify_content`, `align_items`, `gap` y
    /// `flex_wrap` provistos por las propiedades del nodo.
    Flex,
    /// `inline-flex`: igual que Flex pero se comporta como inline en el
    /// flow del padre.
    InlineFlex,
    /// CSS grid container — mapea al algoritmo de grid de taffy con
    /// `grid_template_columns` y `grid_template_rows` del nodo.
    Grid,
    /// `inline-grid`: igual que Grid pero inline en el flow del padre.
    InlineGrid,
    None,
}

/// Un nodo del árbol de boxes — render-ready.
#[derive(Debug, Clone)]
pub struct BoxNode {
    pub display: Display,
    pub background: Option<Color>,
    pub color: Color,
    pub font_size: f32,
    /// 400 = normal, 700 = bold. Por ahora discreto: `< 600` se trata
    /// como normal y `>= 600` como bold (Llimphi text aún no expone
    /// weight axis arbitrario).
    pub font_weight: u16,
    /// CSS `font-style`: normal vs italic/oblique. Heredable.
    pub font_style: crate::style::FontStyle,
    /// CSS `font-family` como string CSS (acepta listas con fallbacks).
    /// `None` = default del runtime. Heredable.
    pub font_family: Option<String>,
    pub margin: Sides<f32>,
    pub padding: Sides<f32>,
    /// Ancho explícito CSS (`auto` por defecto).
    pub width: LengthVal,
    /// Tope superior del ancho.
    pub max_width: LengthVal,
    /// Alineación del texto inline dentro del bloque.
    pub text_align: TextAlign,
    /// Multiplicador line-height (font-size * line_height = altura
    /// de línea). `None` → caller usa 1.2 como default (matchea
    /// browser CSS `normal`; antes 1.4 — más generoso pero menos
    /// compacto que el render real).
    pub line_height: Option<f32>,
    /// Ancho del border en px por lado.
    pub border_widths: Sides<f32>,
    /// Color del border por lado. `None` = ese lado no se dibuja.
    pub border_colors: Sides<Option<Color>>,
    /// Radio corner-radius en px por esquina.
    pub border_radii: Corners<f32>,
    /// Background a aplicar cuando el nodo está bajo el mouse. `None` =
    /// no hay regla `:hover` que cambie el background del nodo. El
    /// chrome lo plug-ea vía `View::hover_fill`. Restyle completo en
    /// hover (cambios de color/border) queda fuera de scope por ahora.
    pub hover_background: Option<Color>,
    /// Background a aplicar cuando el nodo está focado (input/textarea
    /// actualmente focado por el usuario). Mismo modelo limitado que
    /// `hover_background`: sólo el delta de bg, no se propaga a
    /// ancestros (`:focus` aplica al sujeto del selector).
    pub focus_background: Option<Color>,
    /// Box-shadow propagado a `paint_with` en el chrome.
    pub box_shadow: Option<BoxShadow>,
    /// `z-index` aplicado al stacking order entre hermanos positioned.
    /// El chrome lo usa para reordenar children out-of-flow ascending —
    /// el mayor pinta encima. Para `position: static` se ignora.
    pub z_index: i32,
    /// Línea decorativa que el chrome dibuja sobre la hoja de texto
    /// (underline / line-through / overline). `None` = sin decoración.
    pub text_decoration: TextDecorationLine,
    /// Propiedades de flex container — sólo relevantes si `display` es
    /// `Flex`/`InlineFlex`. El chrome las mapea 1:1 a taffy.
    pub flex_direction: FlexDirection,
    pub justify_content: JustifyContent,
    pub align_items: AlignItems,
    pub flex_wrap: FlexWrap,
    pub gap_row: f32,
    pub gap_column: f32,
    /// Modelo de caja: cómo cuenta padding/border en width.
    pub box_sizing: BoxSizing,
    /// Mínimos y máximo extra del axis sizing (width/max_width ya existían).
    pub min_width: LengthVal,
    pub min_height: LengthVal,
    pub max_height: LengthVal,
    /// `hidden` aplica clip() en el chrome.
    pub overflow: Overflow,
    /// `white-space` define cómo collapse_whitespace trata el texto.
    pub white_space: WhiteSpace,
    /// Aplicado al texto del nodo (si es leaf) o propagado por
    /// herencia a hijos text leaf.
    pub text_transform: TextTransform,
    /// 0..1 — el chrome multiplica el alpha del background/border.
    pub opacity: f32,
    /// Item-side de flex.
    pub align_self: AlignSelf,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: LengthVal,
    /// Outline pintado fuera del border (sin afectar layout).
    pub outline: Outline,
    /// Gradiente de fondo (linear-gradient). Si Some, el chrome lo
    /// pinta encima/en lugar del background sólido.
    pub background_gradient: Option<LinearGradient>,
    pub position: Position,
    pub inset_top: LengthVal,
    pub inset_right: LengthVal,
    pub inset_bottom: LengthVal,
    pub inset_left: LengthVal,
    pub vertical_align: VerticalAlign,
    pub visibility: Visibility,
    pub pointer_events: PointerEvents,
    pub text_indent: f32,
    pub word_spacing: f32,
    pub text_shadows: Vec<TextShadow>,
    pub transforms: Vec<Transform>,
    pub grid_template_columns: Vec<GridTrackSize>,
    pub grid_template_rows: Vec<GridTrackSize>,
    /// Texto plano del nodo (sólo para hojas de texto). Para nodos con
    /// hijos el texto vive en los hijos.
    pub text: Option<String>,
    pub children: Vec<BoxNode>,
    /// Tag HTML que originó el box (para debug y feature detection).
    pub tag: Option<String>,
    /// Destino absoluto si el nodo es un `<a href="…">`. Ya resuelto
    /// contra la URL base del documento — los consumidores no tienen
    /// que conocer la base.
    pub link: Option<String>,
    /// Imagen decodificada (RGBA8) si el nodo es un `<img src>` que
    /// pudo descargarse y decodificarse. PNG/JPEG soportados; otros
    /// formatos dejan `None` y el chrome muestra un placeholder.
    pub image: Option<ImageData>,
    /// `true` si el nodo es un `<details>` que arrancó con el atributo
    /// `open`. El chrome usa esto para inicializar el estado open/closed
    /// del primer render; subsiguientes toggles los gestiona él. Para
    /// nodos que no son `<details>` queda en `false` y no se consulta.
    pub details_open_attr: bool,
    /// `true` si el `<a>` lleva `target="_blank"` (o cualquier target
    /// no-self). El chrome lo usa para abrir en nueva pestaña al click.
    /// `false` para todo lo demás.
    pub link_new_tab: bool,
    /// Si el `<a>` lleva `download[=filename]`, el chrome descarga el
    /// target en lugar de navegarlo. `Some(String::new())` = usar el
    /// filename del path; `Some("foo.pdf")` = filename override.
    pub link_download: Option<String>,
    /// Imagen decodificada del CSS `background-image: url(...)`. `None`
    /// si la propiedad no estaba o si la descarga/decode falló. El
    /// chrome la pinta como background (detrás del background sólido y
    /// gradient).
    pub background_image: Option<ImageData>,
    /// Si el nodo es un `<input>` de tipo texto o un `<textarea>`, el
    /// chrome lo renderea como widget editable. `None` para todo lo
    /// demás. Multilinea = textarea.
    pub input_kind: Option<InputKind>,
    /// Valor inicial del input (atributo `value`). Sólo se consulta al
    /// crear el `TextInputState` la primera vez por pestaña; los toggles
    /// y typings los maneja el chrome.
    pub input_initial: Option<String>,
    /// Para `<input type=checkbox|radio>`: estado `checked` inicial.
    /// `false` por default.
    pub input_checked_initial: bool,
    /// `true` si el `<input>`/`<textarea>` lleva el attr `autofocus`. El
    /// chrome busca el primer matching al recibir `Msg::Loaded` y le
    /// asigna `focused_input` para empezar la sesión con el cursor ahí.
    pub input_autofocus: bool,
    /// Placeholder del input — atributo `placeholder` del `<input>` /
    /// `<textarea>`. `None` si vacío.
    pub input_placeholder: Option<String>,
    /// Atributo `name` del input — clave del par `name=value` que va al
    /// query string al submit. `None` = el input no se envía.
    pub input_name: Option<String>,
    /// Índice (en `BoxTree.forms`) del `<form>` que contiene a este nodo
    /// (más cercano hacia arriba en la jerarquía). `None` = no está
    /// dentro de un form, no se puede submitear.
    pub form_idx: Option<usize>,
    /// Si el nodo es `<select>`, este campo lleva la lista de opciones
    /// (con `value` y `label`) y el índice por default. El chrome lo
    /// rendera como dropdown editable y guarda el índice seleccionado
    /// en su `TabState`.
    pub select: Option<SelectInfo>,
    /// Si el nodo es `<svg>`, lista de primitivas a pintar. El chrome
    /// las renderea adentro del rect del nodo (escalado por `viewBox` si
    /// existe; sino cada primitiva usa sus coords nativas).
    pub svg: Option<SvgScene>,
    /// Atributo HTML `id="..."` del elemento — usado por fragment
    /// navigation (`<a href="#foo">` busca el nodo con `element_id ==
    /// Some("foo")` y scrollea hasta él). `None` para nodos sin id y
    /// para nodos sintéticos (markers, wrappers Document, hojas Text).
    pub element_id: Option<String>,
    /// Clases CSS del nodo (atributo `class="a b c"` split por espacio).
    /// Vacío para nodos sin clase. Para que el snapshot pasado a `puriy-js`
    /// pueda indexar elementos por class y soportar `querySelector('.foo')`
    /// — Fase 7.8.
    pub class_list: Vec<String>,
    /// **Todos** los atributos HTML del elemento (name lowercased + value
    /// literal). Esto incluye `data-*`, `aria-*`, `href`, `src`, `title`,
    /// `role`, etc. Los atributos ya parseados como campos dedicados
    /// (`id`, `class`, `href` para links, `src` para imgs, `value` para
    /// inputs) también aparecen acá — son redundantes pero permiten que
    /// `getAttribute('id')` funcione uniformemente desde JS sin sub-rutas.
    /// Fase 7.16. Antes (7.11) este campo se llamaba `dataset` y sólo
    /// guardaba los `data-*` sin prefijo.
    pub attributes: Vec<(String, String)>,
    /// Animación CSS resuelta para el runtime de tween (`anim.rs`). `Some`
    /// sólo cuando el nodo tiene `animation: <name> …` Y el `<name>` matchea
    /// un `@keyframes` conocido. El chrome la consume por frame:
    /// `anim::animation_progress(&binding, elapsed)` da el progreso eased y
    /// `anim::sample_keyframes(&keyframes, p)` el overlay a mergear sobre el
    /// estilo base. `None` = nodo no animado.
    pub animation: Option<AnimationInstance>,
    /// Bindings `transition` declarados en el nodo. El chrome los consulta
    /// (`anim::transition_for`) para tweenear cambios de estado (hover, etc.).
    pub transitions: Vec<crate::style::TransitionBinding>,
    /// Identidad estable del nodo dentro del árbol (1..N en orden DFS
    /// pre-orden), asignada por un post-pass de `build`. Permite al chrome
    /// llevar estado por-nodo (p. ej. el tween de `transition` en hover)
    /// keyeado por id, sin depender de contar índices en walks paralelos
    /// frágiles. `0` = sin asignar (raíz vacía o nodos sintetizados por
    /// mutaciones JS post-load, que no participan de transiciones).
    pub node_id: u32,
}

/// Animación CSS lista para tween: el binding parseado + la definición de
/// `@keyframes` ya resuelta por nombre. Vive en el `BoxNode` para que el
/// chrome no tenga que cargar la tabla de keyframes del `StyleEngine`.
/// Rescatado del frente engine.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationInstance {
    pub binding: crate::style::AnimationBinding,
    pub keyframes: crate::style::Keyframes,
}

impl BoxNode {
    /// Filtra `attributes` por prefijo `data-` y devuelve `(suffix, value)`.
    /// Spec del `el.dataset` API: `data-foo-bar` → key `foo-bar`. Cada
    /// llamada recorre los atributos; para nodos con miles no es óptimo
    /// pero es lo esperado para el uso típico (<10 attrs por elemento).
    /// Fase 7.16 — antes vivía como campo separado `dataset`.
    pub fn dataset(&self) -> Vec<(&str, &str)> {
        let mut out = Vec::new();
        for (k, v) in &self.attributes {
            if let Some(rest) = strip_data_prefix(k) {
                out.push((rest, v.as_str()));
            }
        }
        out
    }
}

/// Devuelve el sufijo si `name` empieza con `data-` (case-insensitive),
/// o `None` en caso contrario. Helper local porque `attributes` guarda
/// nombres lowercased; el matching es sobre prefix de 5 bytes ASCII.
fn strip_data_prefix(name: &str) -> Option<&str> {
    let b = name.as_bytes();
    if b.len() > 5 && b[..5].eq_ignore_ascii_case(b"data-") {
        Some(&name[5..])
    } else {
        None
    }
}

/// Escena SVG minimal: lista de primitivas + viewBox opcional.
#[derive(Debug, Clone)]
pub struct SvgScene {
    pub width: f32,
    pub height: f32,
    /// `(min_x, min_y, w, h)` del viewBox, o `None` si el SVG no lo
    /// declaró (las primitivas van directo a coords del viewport del svg).
    pub view_box: Option<(f32, f32, f32, f32)>,
    pub prims: Vec<SvgPrim>,
}

#[derive(Debug, Clone)]
pub enum SvgPrim {
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        rx: f32,
        fill: Option<Color>,
        stroke: Option<Color>,
        stroke_w: f32,
    },
    Circle {
        cx: f32,
        cy: f32,
        r: f32,
        fill: Option<Color>,
        stroke: Option<Color>,
        stroke_w: f32,
    },
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        stroke: Color,
        stroke_w: f32,
    },
    /// Polygon (cerrado) o polyline (abierto) — los puntos vienen del
    /// atributo `points="x1,y1 x2,y2 …"`.
    Polyline {
        points: Vec<(f32, f32)>,
        closed: bool,
        fill: Option<Color>,
        stroke: Option<Color>,
        stroke_w: f32,
    },
    /// Path con secuencia de comandos. Subset: M (moveTo), L (lineTo),
    /// H/V (horizontal/vertical lineTo), C (cubic bezier), Q (quadratic
    /// bezier), Z (closepath). Todos en abs y rel (m/l/h/v/c/q/z).
    Path {
        d: Vec<PathCmd>,
        fill: Option<Color>,
        stroke: Option<Color>,
        stroke_w: f32,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum PathCmd {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    CubicTo(f32, f32, f32, f32, f32, f32),
    QuadTo(f32, f32, f32, f32),
    ClosePath,
}

/// Datos de un `<select>` para renderizarlo como dropdown.
#[derive(Debug, Clone)]
pub struct SelectInfo {
    pub options: Vec<SelectOption>,
    /// Índice del `<option selected>` inicial, o `0` si ninguno lo era.
    pub initial: usize,
}

#[derive(Debug, Clone)]
pub struct SelectOption {
    /// Texto que el usuario ve.
    pub label: String,
    /// Valor que va al querystring (cae al `label` si el HTML no
    /// proveyó atributo `value`).
    pub value: String,
}

/// Metadata por `<form>` del documento — el chrome la usa al submit.
#[derive(Debug, Clone)]
pub struct FormInfo {
    /// URL absoluta del action (resuelta contra el base). `None` =
    /// submit a la URL actual de la página (CSS spec).
    pub action: Option<String>,
    /// Método HTTP del form — sólo soportamos `GET` por ahora (el más
    /// común y el que funciona sin manejo de bodies/cookies en puriy).
    pub method: FormMethod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormMethod {
    Get,
    /// POST no está implementado todavía — el chrome trata como GET y
    /// muestra un hint en status.
    Post,
}

/// Subconjunto de `<input type=...>` que renderemos como widget de texto.
/// Todo lo demás (checkbox/radio/file/range/submit/...) se trata como
/// box normal por ahora.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    /// `<input type=text>`, `<input>` sin type, search, email, url, tel,
    /// number, password — todos se ven como una línea editable. password
    /// idealmente mostraría bullets, eso lo decide el chrome.
    Text,
    Password,
    Search,
    /// `<textarea>` — multilínea.
    TextArea,
    /// `<input type=checkbox>` — toggle booleano.
    Checkbox,
    /// `<input type=radio>` — exclusivo por nombre de grupo (`name`
    /// compartido entre múltiples radios del mismo form).
    Radio,
    /// `<input type=submit|button>` — botón con label desde `value` (o
    /// `Submit` por default). Click submitea el form si está dentro de
    /// uno; sino no-op.
    Submit,
}

/// Imagen RGBA8 lista para que el chrome la envuelva en `peniko::Image`.
/// `rgba` tiene exactamente `4 * width * height` bytes en orden RGBA.
#[derive(Debug, Clone)]
pub struct ImageData {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Árbol de boxes. Wrapper para poder agregar utilidades.
#[derive(Debug, Clone)]
pub struct BoxTree {
    pub root: BoxNode,
    /// Forms del documento en orden DFS. Cada `<input>` que cae dentro
    /// de uno tiene `BoxNode.form_idx = Some(i)`.
    pub forms: Vec<FormInfo>,
    /// Motor de estilos del documento, retenido para poder re-correr la
    /// cascada CSS tras una mutación que cambie qué reglas matchean
    /// (`classList.add/remove/toggle`, `className`, `setAttribute('class')`).
    /// El DOM original se dropea tras la carga (es `!Send`), así que el
    /// restyle reconstruye un DOM espejo del propio box tree (Fase 7.184).
    pub styles: StyleEngine,
}

impl BoxTree {
    /// Cuenta total de boxes (incluyendo la raíz).
    pub fn descendants_count(&self) -> usize {
        count(&self.root)
    }

    /// Recorre el árbol pre-order y aplica `f` a cada box.
    pub fn walk(&self, mut f: impl FnMut(&BoxNode)) {
        walk_inner(&self.root, &mut f);
    }

    /// Estima la posición vertical (px desde el top del documento) del
    /// nodo con `element_id == id`. Usada por fragment navigation
    /// (`<a href="#foo">`) — el chrome ajusta `scroll_y` a este valor.
    /// La estimación suma margin+padding de bloques y `font_size *
    /// line_height` de hojas de texto en orden DFS; ignora layout real
    /// (taffy todavía no corrió cuando el chrome resuelve el click), así
    /// que el salto puede caer ~1 línea arriba o abajo del target. Es
    /// suficiente para que el usuario vea el destino sin perderse.
    pub fn find_element_y(&self, id: &str) -> Option<f32> {
        let mut acc = 0.0_f32;
        find_y_inner(&self.root, id, &mut acc)
    }

    /// Reemplaza el contenido de texto del subárbol del nodo con
    /// `element_id == id` por `new_text`. Implementa el caso simple de
    /// `el.textContent = X` desde JS:
    ///
    /// 1. Si el nodo target tiene `.text` directo, se reemplaza.
    /// 2. Sino, se reemplaza el PRIMER text leaf en orden DFS del
    ///    subárbol; los hijos no-text quedan intactos.
    /// 3. Si no hay text leaves, no se hace nada (Fase 7.5c — caso raro;
    ///    requeriría sintetizar un nuevo BoxNode con estilo del padre).
    ///
    /// Devuelve `true` si se aplicó la mutación, `false` si no se
    /// encontró el id o no había text leaves. Spec real de `textContent`
    /// es "reemplazar TODO el subárbol con un único text node"; nuestra
    /// aproximación cubre el 90% de los usos reales (clocks, contadores,
    /// banners) sin un refactor del modelo del box tree.
    pub fn set_element_text_content(&mut self, id: &str, new_text: &str) -> bool {
        replace_text_content(&mut self.root, id, new_text)
    }

    /// Aplica una mutación de estilo (proveniente de `el.style.X = Y`)
    /// al nodo con `element_id == id`. `prop` en kebab-case (`color`,
    /// `background-color`, `display`, `font-size`, `visibility`).
    ///
    /// Devuelve `true` si la mutación se aplicó. Props desconocidas o
    /// values no parseables devuelven `false` (silencioso — los setters
    /// JS publican igual; el chrome aplica sólo lo que sabe). Subset
    /// limitado a propósito; ampliar cuando aparezcan casos reales.
    pub fn set_element_style(&mut self, id: &str, prop: &str, value: &str) -> bool {
        set_element_style_inner(&mut self.root, id, prop, value)
    }

    /// Reemplaza la lista de clases del nodo `element_id == id` por
    /// `classes`. NO re-corre la cascada — el caller debe llamar
    /// [`Self::restyle`] después (típicamente una sola vez tras drenar
    /// todas las mutaciones de un evento). Devuelve `true` si encontró el
    /// nodo. Mantiene el atributo `class` de `attributes` en sync para que
    /// el DOM espejo del restyle lea las clases nuevas. Fase 7.184.
    pub fn set_element_class_list(&mut self, id: &str, classes: Vec<String>) -> bool {
        set_class_list_inner(&mut self.root, id, classes)
    }

    /// Sincroniza el atributo `checked` (presencia) de cada control de
    /// formulario con `checks[i]`, en orden DFS (el mismo que indexa el
    /// `input_checks` del chrome), para que un restyle re-evalúe
    /// `:checked`/`:checked + label`. NO recascadea — el caller llama
    /// [`Self::restyle`] después. Fase 7.187.
    pub fn sync_checked_from(&mut self, checks: &[bool]) {
        let mut counter = 0usize;
        sync_checked_inner(&mut self.root, checks, &mut counter);
    }

    /// Re-aplica la cascada CSS a TODO el árbol reusando las reglas
    /// retenidas (`self.styles`). Necesario tras un cambio de `classList`
    /// u otra mutación que altere qué reglas matchean: un cambio en una
    /// clase puede afectar descendientes (selectores descendientes,
    /// herencia) y hermanos posteriores (`+`/`~`), así que recascadeamos
    /// el documento entero. Reconstruye un DOM rcdom-espejo (sólo
    /// elementos) del box tree y corre el MISMO motor de cascada que el
    /// build inicial — sin duplicar el matcher. Fase 7.184.
    ///
    /// Limitaciones (documentadas en el SDD): no re-dropea ni resucita
    /// nodos `display:none` (los que arrancaron ocultos al cargar nunca se
    /// boxearon; los que están en el árbol sí togglean display); no
    /// recolapsa márgenes (preserva el `margin` ya colapsado); no re-deriva
    /// contenido de pseudo-elements ni animaciones.
    pub fn restyle(&mut self) {
        let BoxTree { root, styles, .. } = self;
        if root.tag.is_some() {
            if let Some(mirror) = mirror_element(root) {
                restyle_apply(root, &mirror, None, styles);
            }
        } else {
            // Root sintético (wrapper sin tag): aplica a sus hijos elemento
            // como top-level (parent None), igual que `build` con `<body>`.
            let doc = markup5ever_rcdom::Node::new(markup5ever_rcdom::NodeData::Document);
            collect_mirror_children(root, &doc);
            let mc = doc.children.borrow();
            let mut mi = 0usize;
            restyle_children(&mut root.children, &mc, &mut mi, None, styles);
        }
    }

    /// Setea / actualiza el atributo `name` del nodo `id`. `name` va con
    /// su prefijo completo (`data-foo`, `aria-checked`, `href`, etc.) y
    /// debe venir ya en lowercase kebab. Devuelve `true` si encontró el
    /// nodo. Fase 7.16 — `el.setAttribute(name, value)` publica esta
    /// mutación; también la usan internamente los setters `data-*`.
    pub fn set_element_attribute(&mut self, id: &str, name: &str, value: &str) -> bool {
        set_attribute_inner(&mut self.root, id, name, Some(value))
    }

    /// Borra el atributo `name` del nodo `id`. Devuelve `true` si
    /// encontró el nodo (haya o no existido la key). Fase 7.16.
    pub fn remove_element_attribute(&mut self, id: &str, name: &str) -> bool {
        set_attribute_inner(&mut self.root, id, name, None)
    }

    /// Wrapper sobre `set_element_attribute` que reconstruye `data-<key>`
    /// para preservar la API de Fase 7.11. `key` va sin prefijo.
    pub fn set_element_dataset(&mut self, id: &str, key: &str, value: &str) -> bool {
        self.set_element_attribute(id, &format!("data-{}", key), value)
    }

    /// Wrapper sobre `remove_element_attribute`. Fase 7.11/7.16.
    pub fn remove_element_dataset(&mut self, id: &str, key: &str) -> bool {
        self.remove_element_attribute(id, &format!("data-{}", key))
    }

    /// Agrega `child` como último hijo del nodo `parent_id`. Fase 7.12.
    /// Devuelve `true` si encontró el parent. El `child` viene sintético
    /// (creado por `synthesize_box_node`); no se valida que su id sea
    /// único en el árbol.
    pub fn append_child_to(&mut self, parent_id: &str, child: BoxNode) -> bool {
        if let Some(parent) = find_node_mut(&mut self.root, parent_id) {
            // Fase 7.14 — heredar font/color/etc. del parent antes
            // de insertar. Sin esto, los nodos sintéticos quedan con
            // defaults (black/16px) ignorando el contexto visual del
            // padre.
            let mut child = child;
            inherit_style_to_child(parent, &mut child);
            parent.children.push(child);
            true
        } else {
            false
        }
    }

    /// Quita el primer descendiente con `element_id == child_id` que
    /// sea hijo directo del nodo `parent_id`. Devuelve `true` si quitó
    /// algo. Fase 7.12.
    pub fn remove_child_by_id(&mut self, parent_id: &str, child_id: &str) -> bool {
        if let Some(parent) = find_node_mut(&mut self.root, parent_id) {
            let before = parent.children.len();
            parent
                .children
                .retain(|c| c.element_id.as_deref() != Some(child_id));
            parent.children.len() < before
        } else {
            false
        }
    }

    /// Inserta `child` antes del primer hijo directo de `parent_id`
    /// cuyo `element_id == ref_id`. Si `ref_id` no se encuentra, hace
    /// fallback a append. Devuelve `true` si encontró el parent.
    /// Fase 7.14.
    pub fn insert_child_before(
        &mut self,
        parent_id: &str,
        child: BoxNode,
        ref_id: &str,
    ) -> bool {
        if let Some(parent) = find_node_mut(&mut self.root, parent_id) {
            let pos = parent
                .children
                .iter()
                .position(|c| c.element_id.as_deref() == Some(ref_id));
            let mut child = child;
            inherit_style_to_child(parent, &mut child);
            match pos {
                Some(i) => parent.children.insert(i, child),
                None => parent.children.push(child),
            }
            true
        } else {
            false
        }
    }
}

/// Fase 7.14 — copia las propiedades CSS-heredables del padre al child
/// sintético recién insertado, y propaga al text leaf interno si existe.
/// Sin esto, los nodos creados por `createElement` quedan con defaults
/// de `empty_root()` (color black, font_size 16px, etc.), ignorando el
/// contexto visual del padre.
///
/// Heredables (CSS spec): `color`, `font_size`, `font_weight`,
/// `font_style`, `font_family`, `line_height`, `text_align`,
/// `text_decoration`, `white_space`, `text_transform`. NO heredables:
/// `background`, `display`, `margin`, `padding`, `width`, etc.
fn inherit_style_to_child(parent: &BoxNode, child: &mut BoxNode) {
    child.color = parent.color;
    child.font_size = parent.font_size;
    child.font_weight = parent.font_weight;
    child.font_style = parent.font_style;
    child.font_family = parent.font_family.clone();
    child.line_height = parent.line_height;
    child.text_align = parent.text_align;
    child.text_decoration = parent.text_decoration;
    child.white_space = parent.white_space;
    child.text_transform = parent.text_transform;
    // Propagar al text leaf interno (primer hijo si es text node).
    for c in child.children.iter_mut() {
        if c.text.is_some() {
            c.color = child.color;
            c.font_size = child.font_size;
            c.font_weight = child.font_weight;
            c.font_style = child.font_style;
            c.font_family = child.font_family.clone();
            c.line_height = child.line_height;
            c.text_decoration = child.text_decoration;
        }
    }
}

/// Construye un `BoxNode` sintético para `el.appendChild(createElement(...))`.
/// Inicializa con defaults de `empty_root()` y customiza tag/id/text/
/// class_list/input_initial según los campos provenientes del payload
/// JS. Display elegido por tag: bloques comunes (`div`/`p`/`h1..h6`/
/// `ul`/`ol`/`li`/`section`/`article`/`header`/`footer`/`nav`/`main`)
/// son block; el resto es inline. UA stylesheet no se re-aplica — los
/// estilos se mantienen en defaults. Fase 7.12.
pub fn synthesize_box_node(
    tag: &str,
    id: Option<&str>,
    text_content: &str,
    class_list: Vec<String>,
    value: Option<&str>,
) -> BoxNode {
    // Fase 7.19 — tag vacío significa text node (createTextNode). El
    // BoxNode resultante es inline sin tag y con `text = Some(content)`.
    // El padre lo trata como cualquier otro text leaf; herencia de
    // estilos via inherit_style_to_child al append.
    if tag.is_empty() {
        let mut leaf = empty_root();
        leaf.display = Display::Inline;
        leaf.tag = None;
        leaf.text = Some(text_content.to_string());
        leaf.element_id = id.map(|s| s.to_string());
        return leaf;
    }
    let mut node = empty_root();
    node.tag = Some(tag.to_string());
    node.element_id = id.map(|s| s.to_string());
    node.class_list = class_list;
    // Display por tag — heurística simple (sin UA cascade). Suficiente
    // para que appendChild de `<li>`, `<div>`, `<p>` rendere como bloque.
    let display = match tag.to_ascii_lowercase().as_str() {
        "div" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol" | "li"
        | "section" | "article" | "header" | "footer" | "nav" | "main" | "aside"
        | "blockquote" | "pre" | "table" | "tr" | "tbody" | "thead" | "tfoot"
        | "figure" | "figcaption" | "address" | "hr" => Display::Block,
        "br" => Display::Block,
        "span" | "a" | "b" | "i" | "em" | "strong" | "small" | "code" | "u" | "s"
        | "del" | "ins" | "mark" | "sub" | "sup" | "kbd" | "var" | "samp" | "abbr"
        | "cite" | "q" | "time" | "label" => Display::Inline,
        _ => Display::Block,
    };
    node.display = display;
    // textContent: si es no-vacío, agregamos un text leaf como único
    // hijo. Hereda font_size/color del nodo padre via cascade — pero
    // como acá no corremos cascade, usamos los defaults.
    if !text_content.is_empty() {
        let mut leaf = empty_root();
        leaf.display = Display::Inline;
        leaf.tag = None;
        leaf.text = Some(text_content.to_string());
        leaf.font_size = node.font_size;
        leaf.color = node.color;
        node.children.push(leaf);
    }
    // input_initial para inputs con value pre-set.
    if let Some(v) = value {
        if !v.is_empty() {
            node.input_initial = Some(v.to_string());
        }
    }
    node
}

/// Busca el primer descendiente (incluyendo el root) con `element_id`
/// igual a `target` y devuelve `&mut` a él. Pre-order DFS. None si no
/// existe. Fase 7.12 — helper para mutaciones estructurales.
fn find_node_mut<'a>(root: &'a mut BoxNode, target: &str) -> Option<&'a mut BoxNode> {
    if root.element_id.as_deref() == Some(target) {
        return Some(root);
    }
    for c in root.children.iter_mut() {
        if let Some(found) = find_node_mut(c, target) {
            return Some(found);
        }
    }
    None
}

fn set_attribute_inner(
    node: &mut BoxNode,
    target: &str,
    name: &str,
    value: Option<&str>,
) -> bool {
    if node.element_id.as_deref() == Some(target) {
        node.attributes.retain(|(k, _)| k != name);
        if let Some(v) = value {
            node.attributes.push((name.to_string(), v.to_string()));
        }
        return true;
    }
    for c in node.children.iter_mut() {
        if set_attribute_inner(c, target, name, value) {
            return true;
        }
    }
    false
}

fn set_element_style_inner(
    node: &mut BoxNode,
    target: &str,
    prop: &str,
    value: &str,
) -> bool {
    if node.element_id.as_deref() == Some(target) {
        // Persistimos la declaración inline en el atributo `style` para que
        // un restyle posterior (classList) la re-aplique con prioridad inline
        // — sin esto, la cascada pisaría lo que JS seteó vía `el.style.X`.
        upsert_inline_style_attr(node, prop, value);
        return apply_style_to_node(node, prop, value);
    }
    for c in node.children.iter_mut() {
        if set_element_style_inner(c, target, prop, value) {
            return true;
        }
    }
    false
}

/// Inserta o actualiza una declaración `prop: value` en el atributo `style`
/// del nodo (kebab `prop`). Mantiene el resto de las declaraciones inline.
/// Usado para que `el.style.X = Y` (Fase 7.8) persista a través del restyle
/// (Fase 7.184), que re-parsea el atributo `style` desde el DOM espejo.
fn upsert_inline_style_attr(node: &mut BoxNode, prop: &str, value: &str) {
    let prop = prop.trim();
    if prop.is_empty() {
        return;
    }
    let existing = node
        .attributes
        .iter()
        .find(|(k, _)| k == "style")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    let mut decls: Vec<(String, String)> = Vec::new();
    for seg in existing.split(';') {
        let seg = seg.trim();
        if seg.is_empty() {
            continue;
        }
        if let Some((k, v)) = seg.split_once(':') {
            decls.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    if let Some(slot) = decls.iter_mut().find(|(k, _)| k == prop) {
        slot.1 = value.trim().to_string();
    } else {
        decls.push((prop.to_string(), value.trim().to_string()));
    }
    let serialized = decls
        .iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join("; ");
    if let Some(slot) = node.attributes.iter_mut().find(|(k, _)| k == "style") {
        slot.1 = serialized;
    } else {
        node.attributes.push(("style".to_string(), serialized));
    }
}

fn apply_style_to_node(node: &mut BoxNode, prop: &str, value: &str) -> bool {
    let val = value.trim();
    match prop {
        "color" => {
            if let Some(c) = parse_simple_color(val) {
                node.color = c;
                propagate_text_color(node, c);
                return true;
            }
        }
        "background" | "background-color" => {
            if val.eq_ignore_ascii_case("none") || val.eq_ignore_ascii_case("transparent") {
                node.background = None;
                return true;
            }
            if let Some(c) = parse_simple_color(val) {
                node.background = Some(c);
                return true;
            }
        }
        "display" => {
            let d = match val.to_ascii_lowercase().as_str() {
                "none" => Some(Display::None),
                "block" => Some(Display::Block),
                "inline" => Some(Display::Inline),
                "inline-block" => Some(Display::InlineBlock),
                "flex" => Some(Display::Flex),
                "grid" => Some(Display::Grid),
                _ => None,
            };
            if let Some(d) = d {
                node.display = d;
                return true;
            }
        }
        "font-size" => {
            if let Some(px) = parse_px(val) {
                node.font_size = px;
                propagate_font_size(node, px);
                return true;
            }
        }
        "visibility" => {
            // Aproximación: hidden → display:none (perdemos el espacio
            // reservado; spec real lo mantiene). Suficiente para toggle
            // show/hide del 90% de los casos.
            if val.eq_ignore_ascii_case("hidden") {
                node.display = Display::None;
                return true;
            }
            if val.eq_ignore_ascii_case("visible") {
                return true; // no-op por ahora
            }
        }
        _ => {}
    }
    false
}

// ===================== Fase 7.184 — restyle on classList =====================

/// Reemplaza recursivamente la `class_list` (y el atributo `class` espejo)
/// del nodo con `element_id == id`. Devuelve `true` si lo encontró.
fn set_class_list_inner(node: &mut BoxNode, id: &str, classes: Vec<String>) -> bool {
    if node.element_id.as_deref() == Some(id) {
        // Sincroniza el atributo `class` para que el DOM espejo del restyle
        // (que lee de `attributes`) y `class_list` no diverjan.
        let joined = classes.join(" ");
        if let Some(slot) = node.attributes.iter_mut().find(|(k, _)| k == "class") {
            slot.1 = joined;
        } else if !joined.is_empty() {
            node.attributes.push(("class".to_string(), joined));
        }
        node.class_list = classes;
        return true;
    }
    for c in node.children.iter_mut() {
        if set_class_list_inner(c, id, classes.clone()) {
            return true;
        }
    }
    false
}

/// Recorre en DFS los controles (`input_kind.is_some()`) y fija/quita el
/// atributo `checked` de cada uno según `checks[counter]`.
fn sync_checked_inner(node: &mut BoxNode, checks: &[bool], counter: &mut usize) {
    if node.input_kind.is_some() {
        let checked = checks.get(*counter).copied().unwrap_or(false);
        let has = node.attributes.iter().any(|(k, _)| k == "checked");
        if checked && !has {
            node.attributes.push(("checked".to_string(), String::new()));
        } else if !checked && has {
            node.attributes.retain(|(k, _)| k != "checked");
        }
        *counter += 1;
    }
    for c in node.children.iter_mut() {
        sync_checked_inner(c, checks, counter);
    }
}

/// Construye un Element rcdom espejo de un BoxNode elemento (`tag.is_some()`),
/// con `id`/`class`/`style` + el resto de `attributes`, y sus hijos elemento
/// (aplanando wrappers `tag=None`). Devuelve `None` si el box no es elemento.
fn mirror_element(b: &BoxNode) -> Option<Handle> {
    use markup5ever::interface::{Attribute, QualName};
    use markup5ever::{LocalName, Namespace};
    use markup5ever_rcdom::Node;
    use std::cell::RefCell;

    let tag = b.tag.as_deref()?;
    let mk_attr = |name: &str, val: &str| Attribute {
        name: QualName::new(None, Namespace::from(""), LocalName::from(name)),
        value: val.into(),
    };
    let mut attrs: Vec<Attribute> = Vec::new();
    // `id`/`class` desde los campos canónicos (la mutación de classList los
    // actualiza ahí); el resto desde `attributes` sin pisar id/class.
    if let Some(id) = b.element_id.as_deref() {
        attrs.push(mk_attr("id", id));
    }
    if !b.class_list.is_empty() {
        attrs.push(mk_attr("class", &b.class_list.join(" ")));
    }
    for (k, v) in &b.attributes {
        if k == "id" || k == "class" {
            continue;
        }
        attrs.push(mk_attr(k, v));
    }
    let elem = Node::new(NodeData::Element {
        name: QualName::new(None, Namespace::from(""), LocalName::from(tag)),
        attrs: RefCell::new(attrs),
        template_contents: RefCell::new(None),
        mathml_annotation_xml_integration_point: false,
    });
    collect_mirror_children(b, &elem);
    Some(elem)
}

/// Empuja los Element espejo de los hijos ELEMENTO de `b` bajo
/// `parent_mirror`, aplanando los wrappers `tag=None` (text leaves, markers,
/// pseudo-content) — en el DOM real esos no son ancestros de los elementos.
fn collect_mirror_children(b: &BoxNode, parent_mirror: &Handle) {
    use std::rc::Rc;
    for child in &b.children {
        if child.tag.is_some() {
            if let Some(cm) = mirror_element(child) {
                cm.parent.set(Some(Rc::downgrade(parent_mirror)));
                parent_mirror.children.borrow_mut().push(cm);
            }
        } else {
            collect_mirror_children(child, parent_mirror);
        }
    }
}

/// Computa el estilo re-cascadeado de `b` (un elemento, pareado con su
/// `mirror`) y lo aplica; luego recursa sobre sus hijos. `mirror.children`
/// está en el mismo orden (elementos aplanados) que recorre `restyle_children`.
fn restyle_apply(
    b: &mut BoxNode,
    mirror: &Handle,
    parent_cs: Option<&ComputedStyle>,
    styles: &StyleEngine,
) {
    let cs = styles.compute_with_parent(mirror, parent_cs);
    // Deltas de hover/focus, igual criterio que `build_node`.
    let hover_bg = {
        let h = styles.compute_with_parent_in_state(mirror, parent_cs, true);
        (h.background != cs.background).then_some(h.background).flatten()
    };
    let focus_bg = {
        let f = styles.compute_with_parent_for_state(mirror, parent_cs, false, true);
        (f.background != cs.background).then_some(f.background).flatten()
    };
    set_box_visual(b, &cs, hover_bg, focus_bg);
    let mc = mirror.children.borrow();
    let mut mi = 0usize;
    restyle_children(&mut b.children, &mc, &mut mi, Some(&cs), styles);
}

/// Recorre los hijos de un elemento, pareando cada hijo ELEMENTO con el
/// siguiente espejo (`mc[mi]`) y propagando estilo a los text leaves. Los
/// wrappers `tag=None` se atraviesan transparentes (sin consumir espejo).
fn restyle_children(
    children: &mut [BoxNode],
    mc: &[Handle],
    mi: &mut usize,
    parent_cs: Option<&ComputedStyle>,
    styles: &StyleEngine,
) {
    for child in children.iter_mut() {
        if child.tag.is_some() {
            if let Some(cm) = mc.get(*mi) {
                restyle_apply(child, cm, parent_cs, styles);
            }
            *mi += 1;
        } else {
            if let Some(p) = parent_cs {
                set_leaf_inherited(child, p);
            }
            // Wrapper sin tag: atravesar a sus hijos manteniendo el mismo
            // espejo/cursor (sus elementos son hijos del MISMO ancestro).
            restyle_children(&mut child.children, mc, mi, parent_cs, styles);
        }
    }
}

/// Sobrescribe los campos visuales derivados del estilo en un BoxNode
/// existente, preservando estructura/text/imagen/link/inputs y el `margin`
/// ya colapsado (no recolapsamos en restyle).
fn set_box_visual(b: &mut BoxNode, s: &ComputedStyle, hover_bg: Option<Color>, focus_bg: Option<Color>) {
    b.display = s.display;
    b.background = s.background;
    b.color = s.color;
    b.font_size = s.font_size;
    b.font_weight = s.font_weight;
    b.font_style = s.font_style;
    b.font_family = s.font_family.clone();
    b.padding = s.padding;
    b.width = s.width;
    b.max_width = s.max_width;
    b.text_align = s.text_align;
    b.line_height = s.line_height;
    b.border_widths = s.border_widths;
    b.border_colors = s.border_colors;
    b.border_radii = s.border_radii;
    b.hover_background = hover_bg;
    b.focus_background = focus_bg;
    b.box_shadow = s.box_shadow;
    b.z_index = s.z_index;
    b.flex_direction = s.flex_direction;
    b.justify_content = s.justify_content;
    b.align_items = s.align_items;
    b.flex_wrap = s.flex_wrap;
    b.gap_row = s.gap_row;
    b.gap_column = s.gap_column;
    b.box_sizing = s.box_sizing;
    b.min_width = s.min_width;
    b.min_height = s.min_height;
    b.max_height = s.max_height;
    b.overflow = s.overflow;
    b.white_space = s.white_space;
    b.text_transform = s.text_transform;
    b.opacity = s.opacity;
    b.align_self = s.align_self;
    b.flex_grow = s.flex_grow;
    b.flex_shrink = s.flex_shrink;
    b.flex_basis = s.flex_basis;
    b.outline = s.outline;
    b.background_gradient = s.background_gradient.clone();
    b.position = s.position;
    b.inset_top = s.inset_top;
    b.inset_right = s.inset_right;
    b.inset_bottom = s.inset_bottom;
    b.inset_left = s.inset_left;
    b.vertical_align = s.vertical_align;
    b.visibility = s.visibility;
    b.pointer_events = s.pointer_events;
    b.text_indent = s.text_indent;
    b.word_spacing = s.word_spacing;
    b.text_shadows = s.text_shadows.clone();
    b.transforms = s.transforms.clone();
    b.grid_template_columns = s.grid_template_columns.clone();
    b.grid_template_rows = s.grid_template_rows.clone();
    b.text_decoration = s.text_decoration;
}

/// Propaga las propiedades CSS heredables del estilo del padre a una hoja
/// de texto (mismo subconjunto que copia `compute_internal` del padre).
fn set_leaf_inherited(leaf: &mut BoxNode, p: &ComputedStyle) {
    leaf.color = p.color;
    leaf.font_size = p.font_size;
    leaf.font_weight = p.font_weight;
    leaf.font_style = p.font_style;
    leaf.font_family = p.font_family.clone();
    leaf.text_align = p.text_align;
    leaf.line_height = p.line_height;
    leaf.text_decoration = p.text_decoration;
    leaf.white_space = p.white_space;
    leaf.text_transform = p.text_transform;
    leaf.text_shadows = p.text_shadows.clone();
    leaf.word_spacing = p.word_spacing;
    leaf.text_indent = p.text_indent;
    leaf.visibility = p.visibility;
    leaf.pointer_events = p.pointer_events;
}

fn propagate_text_color(node: &mut BoxNode, c: Color) {
    if node.text.is_some() {
        node.color = c;
    }
    for child in node.children.iter_mut() {
        propagate_text_color(child, c);
    }
}

fn propagate_font_size(node: &mut BoxNode, size: f32) {
    if node.text.is_some() {
        node.font_size = size;
    }
    for child in node.children.iter_mut() {
        propagate_font_size(child, size);
    }
}

/// Parser mínimo de colores para `el.style.X = Y`. Acepta: `#rgb`,
/// `#rrggbb`, palabras CSS comunes (red, blue, green, black, white,
/// gray, yellow, orange, pink, purple, cyan, magenta, transparent).
fn parse_simple_color(s: &str) -> Option<Color> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        return parse_hex_color(hex);
    }
    let lower = s.to_ascii_lowercase();
    let (r, g, b) = match lower.as_str() {
        "black" => (0, 0, 0),
        "white" => (255, 255, 255),
        "red" => (255, 0, 0),
        "green" => (0, 128, 0),
        "blue" => (0, 0, 255),
        "yellow" => (255, 255, 0),
        "orange" => (255, 165, 0),
        "pink" => (255, 192, 203),
        "purple" => (128, 0, 128),
        "cyan" | "aqua" => (0, 255, 255),
        "magenta" | "fuchsia" => (255, 0, 255),
        "gray" | "grey" => (128, 128, 128),
        "lightgray" | "lightgrey" => (211, 211, 211),
        "darkgray" | "darkgrey" => (169, 169, 169),
        _ => return None,
    };
    Some(Color { r, g, b, a: 255 })
}

fn parse_hex_color(hex: &str) -> Option<Color> {
    let h = hex.trim();
    match h.len() {
        3 => {
            let r = u8::from_str_radix(&h[0..1].repeat(2), 16).ok()?;
            let g = u8::from_str_radix(&h[1..2].repeat(2), 16).ok()?;
            let b = u8::from_str_radix(&h[2..3].repeat(2), 16).ok()?;
            Some(Color { r, g, b, a: 255 })
        }
        6 => {
            let r = u8::from_str_radix(&h[0..2], 16).ok()?;
            let g = u8::from_str_radix(&h[2..4], 16).ok()?;
            let b = u8::from_str_radix(&h[4..6], 16).ok()?;
            Some(Color { r, g, b, a: 255 })
        }
        _ => None,
    }
}

fn parse_px(s: &str) -> Option<f32> {
    let t = s.trim();
    let stripped = t.strip_suffix("px").unwrap_or(t);
    stripped.trim().parse::<f32>().ok()
}

fn replace_text_content(node: &mut BoxNode, target: &str, new_text: &str) -> bool {
    if node.element_id.as_deref() == Some(target) {
        return replace_first_text_leaf(node, new_text);
    }
    for c in node.children.iter_mut() {
        if replace_text_content(c, target, new_text) {
            return true;
        }
    }
    false
}

fn replace_first_text_leaf(node: &mut BoxNode, new_text: &str) -> bool {
    if node.text.is_some() {
        node.text = Some(new_text.to_string());
        return true;
    }
    for c in node.children.iter_mut() {
        if replace_first_text_leaf(c, new_text) {
            return true;
        }
    }
    false
}

fn find_y_inner(b: &BoxNode, target: &str, acc: &mut f32) -> Option<f32> {
    if b.element_id.as_deref() == Some(target) {
        return Some(*acc);
    }
    if b.text.is_some() {
        // Hoja de texto: una línea de altura font_size * line_height.
        *acc += b.font_size * b.line_height.unwrap_or(1.2);
        return None;
    }
    // Block-ish: contribución de borders verticales del lado top.
    *acc += b.margin.top + b.padding.top;
    for c in &b.children {
        if let Some(y) = find_y_inner(c, target, acc) {
            return Some(y);
        }
    }
    *acc += b.padding.bottom + b.margin.bottom;
    None
}

impl BoxTree {
    /// Estima la y del N-ésimo (1-based) leaf de texto cuyo contenido
    /// contiene `query_lower` (la query debe venir ya lowercased — el
    /// caller suele hacerlo una vez fuera del walk). Usado por la find
    /// bar para auto-scroll al match actual con Enter/Shift+Enter.
    pub fn find_y_of_match(&self, query_lower: &str, nth_1based: usize) -> Option<f32> {
        if query_lower.is_empty() || nth_1based == 0 {
            return None;
        }
        let mut acc = 0.0_f32;
        let mut seen = 0_usize;
        find_match_y_inner(&self.root, query_lower, nth_1based, &mut acc, &mut seen)
    }
}

fn find_match_y_inner(
    b: &BoxNode,
    query: &str,
    target_nth: usize,
    acc: &mut f32,
    seen: &mut usize,
) -> Option<f32> {
    if let Some(text) = &b.text {
        if text.to_lowercase().contains(query) {
            *seen += 1;
            if *seen == target_nth {
                return Some(*acc);
            }
        }
        *acc += b.font_size * b.line_height.unwrap_or(1.2);
        return None;
    }
    *acc += b.margin.top + b.padding.top;
    for c in &b.children {
        if let Some(y) = find_match_y_inner(c, query, target_nth, acc, seen) {
            return Some(y);
        }
    }
    *acc += b.padding.bottom + b.margin.bottom;
    None
}

fn count(b: &BoxNode) -> usize {
    1 + b.children.iter().map(count).sum::<usize>()
}

fn walk_inner(b: &BoxNode, f: &mut impl FnMut(&BoxNode)) {
    f(b);
    for c in &b.children {
        walk_inner(c, f);
    }
}

/// Construye el árbol de boxes desde un DOM y un StyleEngine.
///
/// `base_url` se usa para resolver los `href` de `<a>` a URLs
/// absolutos. Pasale el URL del documento (puede ser `about:blank`
/// para HTML inline).
pub fn build(dom: &DomTree, styles: &StyleEngine, base_url: &str) -> BoxTree {
    // `<base href="...">` en el `<head>` override la base URL. Si está
    // ausente o inválido, fallback al URL del documento.
    let doc_base = url::Url::parse(base_url).ok();
    let base = dom
        .base_href()
        .as_deref()
        .and_then(|href| {
            // El base href puede ser absoluto o relativo al URL del doc.
            url::Url::parse(href)
                .ok()
                .or_else(|| doc_base.as_ref().and_then(|b| b.join(href).ok()))
        })
        .or(doc_base);
    let body = dom.find("body").unwrap_or_else(|| dom.document());
    // Prefetch paralelo de imágenes: pre-walk del DOM antes del build
    // recolecta todas las URLs de `<img>`/`<picture>` (resueltas contra
    // base) y las baja en paralelo con un pool de workers. Las bytes
    // quedan en la cache global; el `fetch_and_decode` síncrono dentro
    // de `build_node` después hace cache hit. Esto convierte el parse
    // de una página con 20 imágenes de "20 round-trips serializados"
    // a "ceil(20/N) round-trips". `background-image: url(...)` no
    // entra al pre-walk todavía — vive en CSS y requiere computar
    // styles primero.
    prefetch_image_urls(&dom.document(), base.as_ref());
    // Segundo pass de prefetch: `background-image: url(...)` vive en
    // CSS — necesita styles computados, así que va después del primer
    // pre-walk. Computamos sin parent style (background-image no es
    // heredable, así que el value es independiente del padre). Las
    // URLs descargadas también caen en la cache global.
    prefetch_background_image_urls(&dom.document(), styles, base.as_ref());
    let mut counters: std::collections::HashMap<String, i32> = std::collections::HashMap::new();
    let mut root = build_node(&body, styles, base.as_ref(), None, &mut counters)
        .unwrap_or_else(empty_root);
    let mut forms: Vec<FormInfo> = Vec::new();
    // Pre-walk del DOM para coleccionar `<form>` (orden DFS) con sus
    // attributes resueltos contra base. La asignación de form_idx por
    // input se hace en un post-pass sobre el box tree con el mismo
    // criterio DFS — ambos walks coinciden porque el box tree refleja
    // el DOM (sólo dropea text-whitespace inter-block; los <form> son
    // block-level y nunca se descartan).
    collect_forms_dom(&body, base.as_ref(), &mut forms);
    let mut form_stack: Vec<usize> = Vec::new();
    let mut form_cursor: usize = 0;
    assign_form_idx(&mut root, &mut form_stack, &mut form_cursor);
    // Identidad estable por nodo (1..N en DFS pre-orden). El chrome la usa
    // para llevar estado por-nodo (tween de `transition` en hover) keyeado
    // por id, sin contar índices en walks paralelos frágiles.
    let mut node_cursor: u32 = 1;
    assign_node_ids(&mut root, &mut node_cursor);
    BoxTree { root, forms, styles: styles.clone() }
}

/// Post-pass: numera cada nodo del árbol en orden DFS pre-orden empezando
/// en `*next`. Determinista y estable mientras la estructura del árbol no
/// cambie — exactamente la garantía que necesita el estado de hover del
/// chrome (keyeado por `node_id`).
fn assign_node_ids(node: &mut BoxNode, next: &mut u32) {
    node.node_id = *next;
    *next += 1;
    for child in &mut node.children {
        assign_node_ids(child, next);
    }
}

fn collect_forms_dom(node: &Handle, base: Option<&url::Url>, out: &mut Vec<FormInfo>) {
    if let markup5ever_rcdom::NodeData::Element { .. } = &node.data {
        if dom::element_name(node).as_deref() == Some("form") {
            let action = dom::attr(node, "action").and_then(|a| resolve_href(base, &a));
            let method = dom::attr(node, "method")
                .map(|m| {
                    if m.eq_ignore_ascii_case("post") {
                        FormMethod::Post
                    } else {
                        FormMethod::Get
                    }
                })
                .unwrap_or(FormMethod::Get);
            out.push(FormInfo { action, method });
        }
    }
    for c in node.children.borrow().iter() {
        collect_forms_dom(c, base, out);
    }
}

fn assign_form_idx(b: &mut BoxNode, stack: &mut Vec<usize>, cursor: &mut usize) {
    let is_form = b.tag.as_deref() == Some("form");
    if is_form {
        stack.push(*cursor);
        *cursor += 1;
    }
    if b.input_kind.is_some() || b.select.is_some() {
        b.form_idx = stack.last().copied();
    }
    for c in &mut b.children {
        assign_form_idx(c, stack, cursor);
    }
    if is_form {
        stack.pop();
    }
}

/// Recolecta primitivas de un `<svg>`: rect/circle/line directos.
/// Soporta atributos `viewBox`, `width`, `height`, `fill`, `stroke`,
/// `stroke-width`. Sin transforms ni groups recursivos.
fn collect_svg(svg_node: &Handle) -> SvgScene {
    let width = dom::attr(svg_node, "width")
        .and_then(|s| s.trim_end_matches("px").trim().parse::<f32>().ok())
        .unwrap_or(300.0);
    let height = dom::attr(svg_node, "height")
        .and_then(|s| s.trim_end_matches("px").trim().parse::<f32>().ok())
        .unwrap_or(150.0);
    let view_box = dom::attr(svg_node, "viewBox").and_then(|s| {
        let nums: Vec<f32> = s
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|p| !p.is_empty())
            .filter_map(|p| p.parse::<f32>().ok())
            .collect();
        if nums.len() == 4 {
            Some((nums[0], nums[1], nums[2], nums[3]))
        } else {
            None
        }
    });
    let mut prims: Vec<SvgPrim> = Vec::new();
    collect_svg_prims(svg_node, &mut prims);
    SvgScene { width, height, view_box, prims }
}

fn collect_svg_prims(node: &Handle, out: &mut Vec<SvgPrim>) {
    if let markup5ever_rcdom::NodeData::Element { .. } = &node.data {
        match dom::element_name(node).as_deref() {
            Some("rect") => {
                let x = svg_num(node, "x", 0.0);
                let y = svg_num(node, "y", 0.0);
                let w = svg_num(node, "width", 0.0);
                let h = svg_num(node, "height", 0.0);
                let rx = svg_num(node, "rx", 0.0);
                out.push(SvgPrim::Rect {
                    x, y, w, h, rx,
                    fill: svg_color(node, "fill"),
                    stroke: svg_color(node, "stroke"),
                    stroke_w: svg_num(node, "stroke-width", 1.0),
                });
            }
            Some("circle") => {
                let cx = svg_num(node, "cx", 0.0);
                let cy = svg_num(node, "cy", 0.0);
                let r = svg_num(node, "r", 0.0);
                out.push(SvgPrim::Circle {
                    cx, cy, r,
                    fill: svg_color(node, "fill"),
                    stroke: svg_color(node, "stroke"),
                    stroke_w: svg_num(node, "stroke-width", 1.0),
                });
            }
            Some("line") => {
                let x1 = svg_num(node, "x1", 0.0);
                let y1 = svg_num(node, "y1", 0.0);
                let x2 = svg_num(node, "x2", 0.0);
                let y2 = svg_num(node, "y2", 0.0);
                if let Some(stroke) = svg_color(node, "stroke") {
                    out.push(SvgPrim::Line {
                        x1, y1, x2, y2,
                        stroke,
                        stroke_w: svg_num(node, "stroke-width", 1.0),
                    });
                }
            }
            Some("polygon") | Some("polyline") => {
                let closed = dom::element_name(node).as_deref() == Some("polygon");
                let points = parse_svg_points(&dom::attr(node, "points").unwrap_or_default());
                if !points.is_empty() {
                    out.push(SvgPrim::Polyline {
                        points,
                        closed,
                        fill: svg_color(node, "fill"),
                        stroke: svg_color(node, "stroke"),
                        stroke_w: svg_num(node, "stroke-width", 1.0),
                    });
                }
            }
            Some("path") => {
                if let Some(d) = dom::attr(node, "d") {
                    let cmds = parse_svg_path(&d);
                    if !cmds.is_empty() {
                        out.push(SvgPrim::Path {
                            d: cmds,
                            fill: svg_color(node, "fill"),
                            stroke: svg_color(node, "stroke"),
                            stroke_w: svg_num(node, "stroke-width", 1.0),
                        });
                    }
                }
            }
            // Containers transparentes: recurrir adentro.
            Some("g") | Some("svg") => {}
            // Resto (`text`, `defs`, `mask`, etc.) ignorado.
            _ => return,
        }
    }
    for c in node.children.borrow().iter() {
        collect_svg_prims(c, out);
    }
}

fn svg_num(node: &Handle, name: &str, default: f32) -> f32 {
    dom::attr(node, name)
        .and_then(|s| s.trim_end_matches("px").trim().parse::<f32>().ok())
        .unwrap_or(default)
}

/// Elige una URL del `srcset` HTML. Subset: cada candidato es `url
/// [descriptor]` separados por `,`. Descriptor puede ser `Nx`
/// (densidad) o `Nw` (ancho) o ausente. Estrategia: preferimos la
/// más alta densidad (`Nx`) o el ancho más grande (`Nw`); sin
/// viewport conocido al tiempo de parse, asumimos high-DPI por default.
pub(crate) fn pick_srcset(srcset: &str) -> Option<String> {
    if srcset.trim().is_empty() {
        return None;
    }
    let mut best_score: f32 = -1.0;
    let mut best_url: Option<String> = None;
    for entry in srcset.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let (url, desc) = match entry.split_once(char::is_whitespace) {
            Some((u, d)) => (u.trim().to_string(), d.trim().to_string()),
            None => (entry.to_string(), String::new()),
        };
        let score: f32 = if let Some(rest) = desc.strip_suffix('x') {
            rest.parse::<f32>().unwrap_or(1.0) * 1000.0
        } else if let Some(rest) = desc.strip_suffix('w') {
            rest.parse::<f32>().unwrap_or(0.0)
        } else {
            // Sin descriptor — equivalente a 1x.
            1000.0
        };
        if score > best_score {
            best_score = score;
            best_url = Some(url);
        }
    }
    best_url
}

fn parse_svg_points(s: &str) -> Vec<(f32, f32)> {
    let nums: Vec<f32> = s
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.parse::<f32>().ok())
        .collect();
    nums.chunks_exact(2).map(|c| (c[0], c[1])).collect()
}

/// Parser de `d=` minimal: soporta M/m, L/l, H/h, V/v, C/c, Q/q, Z/z.
/// No soporta A (arcs), T, S (smooth bezier).
fn parse_svg_path(d: &str) -> Vec<PathCmd> {
    // Tokenize: cada comando es una letra, cada arg es un f32 (separados
    // por whitespace o coma; el signo `-` puede arrancar un nuevo número
    // sin separador).
    let bytes = d.as_bytes();
    let mut i = 0;
    let n = bytes.len();
    let mut out: Vec<PathCmd> = Vec::new();
    let mut cx = 0.0_f32; // cursor x absoluto
    let mut cy = 0.0_f32;
    let mut start_x = 0.0_f32;
    let mut start_y = 0.0_f32;
    let mut current_cmd: u8 = 0;
    while i < n {
        let c = bytes[i];
        if c.is_ascii_whitespace() || c == b',' {
            i += 1;
            continue;
        }
        if c.is_ascii_alphabetic() {
            current_cmd = c;
            i += 1;
            // Z/z no toma args — ejecutalo acá directamente, sino el
            // loop nunca llega al match (no hay número que dispare).
            if c == b'Z' || c == b'z' {
                out.push(PathCmd::ClosePath);
                cx = start_x;
                cy = start_y;
            }
            continue;
        }
        // c es dígito o `-`/`+`/`.`: leer un número.
        let read_num = |from: usize| -> Option<(f32, usize)> {
            let mut j = from;
            if j < n && (bytes[j] == b'-' || bytes[j] == b'+') {
                j += 1;
            }
            while j < n && (bytes[j].is_ascii_digit() || bytes[j] == b'.') {
                j += 1;
            }
            if j < n && (bytes[j] == b'e' || bytes[j] == b'E') {
                j += 1;
                if j < n && (bytes[j] == b'-' || bytes[j] == b'+') {
                    j += 1;
                }
                while j < n && bytes[j].is_ascii_digit() {
                    j += 1;
                }
            }
            std::str::from_utf8(&bytes[from..j])
                .ok()
                .and_then(|s| s.parse::<f32>().ok())
                .map(|v| (v, j))
        };
        let read_args = |from: usize, count: usize| -> Option<(Vec<f32>, usize)> {
            let mut nums = Vec::with_capacity(count);
            let mut k = from;
            while nums.len() < count {
                while k < n && (bytes[k].is_ascii_whitespace() || bytes[k] == b',') {
                    k += 1;
                }
                let (v, after) = read_num(k)?;
                nums.push(v);
                k = after;
            }
            Some((nums, k))
        };
        let rel = current_cmd.is_ascii_lowercase();
        match current_cmd.to_ascii_uppercase() {
            b'M' => {
                let (args, after) = match read_args(i, 2) {
                    Some(v) => v,
                    None => break,
                };
                let (mut x, mut y) = (args[0], args[1]);
                if rel { x += cx; y += cy; }
                out.push(PathCmd::MoveTo(x, y));
                cx = x; cy = y;
                start_x = x; start_y = y;
                i = after;
                // M con args extra implícitamente lineTo.
                current_cmd = if rel { b'l' } else { b'L' };
            }
            b'L' => {
                let (args, after) = match read_args(i, 2) {
                    Some(v) => v,
                    None => break,
                };
                let (mut x, mut y) = (args[0], args[1]);
                if rel { x += cx; y += cy; }
                out.push(PathCmd::LineTo(x, y));
                cx = x; cy = y;
                i = after;
            }
            b'H' => {
                let (args, after) = match read_args(i, 1) {
                    Some(v) => v,
                    None => break,
                };
                let mut x = args[0];
                if rel { x += cx; }
                out.push(PathCmd::LineTo(x, cy));
                cx = x;
                i = after;
            }
            b'V' => {
                let (args, after) = match read_args(i, 1) {
                    Some(v) => v,
                    None => break,
                };
                let mut y = args[0];
                if rel { y += cy; }
                out.push(PathCmd::LineTo(cx, y));
                cy = y;
                i = after;
            }
            b'C' => {
                let (args, after) = match read_args(i, 6) {
                    Some(v) => v,
                    None => break,
                };
                let (mut x1, mut y1, mut x2, mut y2, mut x, mut y) =
                    (args[0], args[1], args[2], args[3], args[4], args[5]);
                if rel {
                    x1 += cx; y1 += cy;
                    x2 += cx; y2 += cy;
                    x += cx; y += cy;
                }
                out.push(PathCmd::CubicTo(x1, y1, x2, y2, x, y));
                cx = x; cy = y;
                i = after;
            }
            b'Q' => {
                let (args, after) = match read_args(i, 4) {
                    Some(v) => v,
                    None => break,
                };
                let (mut x1, mut y1, mut x, mut y) = (args[0], args[1], args[2], args[3]);
                if rel {
                    x1 += cx; y1 += cy;
                    x += cx; y += cy;
                }
                out.push(PathCmd::QuadTo(x1, y1, x, y));
                cx = x; cy = y;
                i = after;
            }
            b'Z' => {
                out.push(PathCmd::ClosePath);
                cx = start_x;
                cy = start_y;
            }
            _ => {
                // Comando no soportado (`A`, `T`, `S`) — saltea un número
                // para evitar loops infinitos.
                if let Some((_, after)) = read_num(i) {
                    i = after;
                } else {
                    break;
                }
            }
        }
    }
    out
}

fn svg_color(node: &Handle, name: &str) -> Option<Color> {
    let v = dom::attr(node, name)?;
    let v = v.trim();
    if v.eq_ignore_ascii_case("none") {
        return None;
    }
    crate::style::parse_color_named_or_hex(v)
}

fn empty_root() -> BoxNode {
    BoxNode {
        display: Display::Block,
        background: None,
        color: Color::BLACK,
        font_size: 16.0,
        font_weight: 400,
        font_style: crate::style::FontStyle::Normal,
        font_family: None,
        margin: Sides::all(0.0),
        padding: Sides::all(0.0),
        width: LengthVal::Auto,
        max_width: LengthVal::Auto,
        text_align: TextAlign::Left,
        line_height: None,
        border_widths: Sides::all(0.0),
        border_colors: Sides::all(None),
        border_radii: Corners::all(0.0),
        hover_background: None,
        focus_background: None,
        box_shadow: None,
        z_index: 0,
        flex_direction: FlexDirection::Row,
        justify_content: JustifyContent::Start,
        align_items: AlignItems::Stretch,
        flex_wrap: FlexWrap::NoWrap,
        gap_row: 0.0,
        gap_column: 0.0,
        box_sizing: BoxSizing::ContentBox,
        min_width: LengthVal::Auto,
        min_height: LengthVal::Auto,
        max_height: LengthVal::Auto,
        overflow: Overflow::Visible,
        white_space: WhiteSpace::Normal,
        text_transform: TextTransform::None,
        opacity: 1.0,
        align_self: AlignSelf::Auto,
        flex_grow: 0.0,
        flex_shrink: 1.0,
        flex_basis: LengthVal::Auto,
        outline: Outline::default(),
        background_gradient: None,
        position: Position::Static,
        inset_top: LengthVal::Auto,
        inset_right: LengthVal::Auto,
        inset_bottom: LengthVal::Auto,
        inset_left: LengthVal::Auto,
        vertical_align: VerticalAlign::Baseline,
        visibility: Visibility::Visible,
        pointer_events: PointerEvents::Auto,
        text_indent: 0.0,
        word_spacing: 0.0,
        text_shadows: Vec::new(),
        transforms: Vec::new(),
        grid_template_columns: Vec::new(),
        grid_template_rows: Vec::new(),
        text_decoration: TextDecorationLine::None,
        text: None,
        children: Vec::new(),
        tag: Some("body".into()),
        link: None,
        image: None,
        details_open_attr: false,
        link_new_tab: false,
        link_download: None,
        background_image: None,
        input_kind: None,
        input_initial: None,
        input_placeholder: None,
        input_name: None,
        input_checked_initial: false,
        input_autofocus: false,
        form_idx: None,
        select: None,
        svg: None,
        element_id: None,
        class_list: Vec::new(),
        attributes: Vec::new(),
        animation: None,
        transitions: Vec::new(),
        node_id: 0,
    }
}

fn build_node(
    node: &Handle,
    styles: &StyleEngine,
    base: Option<&url::Url>,
    parent_style: Option<&ComputedStyle>,
    counters: &mut std::collections::HashMap<String, i32>,
) -> Option<BoxNode> {
    match &node.data {
        NodeData::Element { .. } => {
            let style = styles.compute_with_parent(node, parent_style);
            if style.display == Display::None {
                // Distinguimos el `display:none` de RUIDO UA (script/style/
                // option/colgroup/canvas/...) — que se descarta — del puesto
                // por el AUTOR (CSS de la página), que se RETIENE como box
                // oculto con su subárbol, para que un toggle de clase (restyle,
                // Fase 7.184) pueda mostrarlo. El chrome no pinta ni reserva
                // espacio para boxes `Display::None` (TaffyDisplay::None).
                // Fase 7.185.
                let tag = dom::element_name(node).unwrap_or_default();
                if crate::style::tag_defaults_to_none(&tag) {
                    return None;
                }
                // Cae a través: construye el box (display=None) y su subárbol.
            }
            // CSS counters: aplicar reset (sobrescribe) y luego
            // increment al entrar al nodo. Implementación pragmática
            // — un map global que sólo crece. CSS spec dice que reset
            // crea scope nuevo por subárbol, pero eso requiere un
            // stack y rara vez importa para los usos comunes (numbered
            // headings, breadcrumbs); cuando importe se mete el stack.
            for (name, val) in &style.counter_reset {
                counters.insert(name.clone(), *val);
            }
            for (name, delta) in &style.counter_increment {
                *counters.entry(name.clone()).or_insert(0) += *delta;
            }
            // Hover/focus styles: recomputamos con hover_active=true y
            // focus_active=true por separado y vemos si alguna pseudoclase
            // `:hover`/`:focus` cambió el background. Si sí, exponemos el
            // delta al chrome para que lo aplique cuando corresponda.
            // Resto del diff (color/border/etc.) queda fuera por ahora —
            // restyle completo requeriría re-mount del tree.
            let hover_style = styles.compute_with_parent_in_state(node, parent_style, true);
            let hover_background = if hover_style.background != style.background {
                hover_style.background
            } else {
                None
            };
            let focus_style =
                styles.compute_with_parent_for_state(node, parent_style, false, true);
            let focus_background = if focus_style.background != style.background {
                focus_style.background
            } else {
                None
            };
            let tag = dom::element_name(node);
            let link = match (tag.as_deref(), base) {
                (Some("a"), base) => dom::attr(node, "href").and_then(|h| resolve_href(base, &h)),
                _ => None,
            };
            let link_new_tab = tag.as_deref() == Some("a")
                && dom::attr(node, "target")
                    .map(|t| {
                        let t = t.trim().to_ascii_lowercase();
                        // `_blank` y cualquier target con nombre custom → nueva tab.
                        // `_self`/`_parent`/`_top` quedan como navegación in-place.
                        !t.is_empty() && t != "_self" && t != "_parent" && t != "_top"
                    })
                    .unwrap_or(false);
            let link_download = if tag.as_deref() == Some("a") {
                dom::attr(node, "download").map(|s| s.trim().to_string())
            } else {
                None
            };

            let input_kind = match tag.as_deref() {
                Some("textarea") => Some(InputKind::TextArea),
                Some("input") => {
                    let t = dom::attr(node, "type")
                        .map(|s| s.trim().to_ascii_lowercase())
                        .unwrap_or_else(|| "text".to_string());
                    match t.as_str() {
                        "" | "text" | "email" | "url" | "tel" | "number" => Some(InputKind::Text),
                        "search" => Some(InputKind::Search),
                        "password" => Some(InputKind::Password),
                        "checkbox" => Some(InputKind::Checkbox),
                        "radio" => Some(InputKind::Radio),
                        "submit" | "button" | "reset" => Some(InputKind::Submit),
                        _ => None, // file, range, color, hidden, etc.
                    }
                }
                _ => None,
            };
            let input_initial = input_kind.and_then(|_| {
                if tag.as_deref() == Some("textarea") {
                    // El "value" del textarea es su texto interior.
                    let mut s = String::new();
                    for child in node.children.borrow().iter() {
                        if let markup5ever_rcdom::NodeData::Text { contents } = &child.data {
                            s.push_str(&contents.borrow());
                        }
                    }
                    Some(s)
                } else {
                    dom::attr(node, "value")
                }
            });
            let input_placeholder = input_kind.and_then(|_| dom::attr(node, "placeholder"));
            let input_name = input_kind.and_then(|_| dom::attr(node, "name"));
            let input_checked_initial = matches!(
                input_kind,
                Some(InputKind::Checkbox) | Some(InputKind::Radio)
            ) && dom::attr(node, "checked").is_some();
            let input_autofocus = input_kind.is_some() && dom::attr(node, "autofocus").is_some();
            // `<svg>`: coleccionamos las primitivas (rect/circle/line) y
            // el viewBox. Las primitivas del subárbol del SVG no son
            // descendientes del box tree (el `display: inline-block` del
            // `<svg>` mantiene su rect pero los hijos quedan fuera del
            // flow). El chrome usa `b.svg` para paint_with.
            let svg = if tag.as_deref() == Some("svg") {
                Some(collect_svg(node))
            } else {
                None
            };
            // `<select>`: coleccionamos opciones y el inicial seleccionado.
            let select = if tag.as_deref() == Some("select") {
                let mut opts: Vec<SelectOption> = Vec::new();
                let mut initial = 0usize;
                let mut seen_selected = false;
                for child in node.children.borrow().iter() {
                    if dom::element_name(child).as_deref() == Some("option") {
                        let label = dom::collect_text(child);
                        let value = dom::attr(child, "value").unwrap_or_else(|| label.clone());
                        if dom::attr(child, "selected").is_some() && !seen_selected {
                            initial = opts.len();
                            seen_selected = true;
                        }
                        opts.push(SelectOption { label, value });
                    }
                }
                if opts.is_empty() {
                    None
                } else {
                    Some(SelectInfo { options: opts, initial })
                }
            } else {
                None
            };
            // <img>: descarga + decode sync. Si falla, el campo queda
            // None y el chrome muestra placeholder con el alt. Resuelve
            // `srcset` antes que `src` (responsive images).
            let image = if tag.as_deref() == Some("img") {
                let src_candidate = pick_srcset(&dom::attr(node, "srcset").unwrap_or_default())
                    .or_else(|| dom::attr(node, "src"));
                src_candidate.and_then(|s| fetch_image_src(base, &s))
            } else if tag.as_deref() == Some("picture") {
                // `<picture>`: el primer `<source srcset>` que sirva
                // gana; sino caemos al `<img>` interno (que ya entra
                // como child y trae su src/srcset).
                let mut chosen: Option<String> = None;
                for child in node.children.borrow().iter() {
                    if dom::element_name(child).as_deref() == Some("source") {
                        if let Some(s) = dom::attr(child, "srcset") {
                            if let Some(c) = pick_srcset(&s) {
                                chosen = Some(c);
                                break;
                            }
                        }
                    }
                }
                chosen.and_then(|s| fetch_image_src(base, &s))
            } else {
                None
            };
            // `background-image: url(...)` — resolver contra base y
            // descargar/decode. Misma cache que `<img>` por la fetch::
            // global. Falla silenciosa → background_image queda None.
            let background_image = style
                .background_image_url
                .as_deref()
                .and_then(|u| fetch_image_src(base, u));
            let mut children = Vec::new();
            // `::before` pseudo-element. Se inyecta ANTES que el marker
            // de `<li>` y que los children reales — matchea spec ("the
            // first thing inside the box").
            if let Some(ps) =
                styles.compute_pseudo(node, crate::style::PseudoElement::Before, Some(&style))
            {
                // Aplicar reset/increment declarados en la regla del
                // pseudo (`h2::before { counter-increment: sec }`).
                // El pseudo es lo "primero adentro" del nodo, así que
                // sus contadores cuentan antes de resolver su content.
                for (name, val) in &ps.counter_reset {
                    counters.insert(name.clone(), *val);
                }
                for (name, delta) in &ps.counter_increment {
                    *counters.entry(name.clone()).or_insert(0) += *delta;
                }
                if let Some(items) = &ps.content {
                    emit_content_items(items, node, counters, &ps, base, &mut children);
                }
            }
            // <li>: prefija con marker (bullet o numeral según
            // `list-style-type`). Lo agregamos como un hijo Text inline
            // antes de procesar los hijos reales — hereda
            // color/font-size de `style`. Si `list-style-type: none` o
            // no estamos dentro de una lista reconocible, no se inyecta
            // marker.
            if tag.as_deref() == Some("li") {
                if let Some(marker) = li_marker(node, style.list_style_type) {
                    children.push(inline_text_with_style(marker, &style));
                }
            }
            // `<iframe>` placeholder: sin engine de sub-página todavía,
            // mostramos un label con la URL para que el lector vea QUE
            // hay contenido embebido y dónde apunta.
            if tag.as_deref() == Some("iframe") {
                let src = dom::attr(node, "src").unwrap_or_default();
                let label = if src.is_empty() {
                    "[iframe sin src]".to_string()
                } else {
                    format!("[iframe: {src}]")
                };
                children.push(inline_text_with_style(label, &style));
            }
            // <img> sin imagen decodificada: muestra `alt`.
            if tag.as_deref() == Some("img") && image.is_none() {
                if let Some(alt) = dom::attr(node, "alt") {
                    if !alt.trim().is_empty() {
                        children.push(inline_text_with_style(format!("[img: {alt}]"), &style));
                    }
                }
            }
            for child in node.children.borrow().iter() {
                if let Some(b) = build_node(child, styles, base, Some(&style), counters) {
                    children.push(b);
                }
            }
            // `::after` pseudo-element. Se appendea al final, después
            // de los children reales. Igual que before, aplicamos
            // reset/increment del pseudo antes de resolver content.
            if let Some(ps) =
                styles.compute_pseudo(node, crate::style::PseudoElement::After, Some(&style))
            {
                for (name, val) in &ps.counter_reset {
                    counters.insert(name.clone(), *val);
                }
                for (name, delta) in &ps.counter_increment {
                    *counters.entry(name.clone()).or_insert(0) += *delta;
                }
                if let Some(items) = &ps.content {
                    emit_content_items(items, node, counters, &ps, base, &mut children);
                }
            }
            let children = strip_block_adjacent_whitespace(children, style.display);
            let children = collapse_vertical_margins(children);
            // Margin collapsing contra el padre. CSS spec: si el padre
            // no tiene border-top ni padding-top, el margin-top del
            // primer hijo block in-flow se promueve al padre (queda
            // como max(parent.margin_top, child.margin_top)). Idem
            // para el último hijo y margin-bottom. Solo aplica si el
            // padre es Block-ish (no Flex/Grid/Inline); en esos casos
            // hay un context distinto que no colapsa.
            let parent_no_top_barrier = style.padding.top == 0.0
                && style.border_widths.top == 0.0;
            let parent_no_bot_barrier = style.padding.bottom == 0.0
                && style.border_widths.bottom == 0.0;
            let parent_is_block_flow = matches!(style.display, Display::Block);
            let mut effective_margin = style.margin;
            let children = if parent_is_block_flow {
                collapse_margins_against_parent(
                    children,
                    &mut effective_margin,
                    parent_no_top_barrier,
                    parent_no_bot_barrier,
                )
            } else {
                children
            };
            Some(BoxNode {
                display: style.display,
                background: style.background,
                color: style.color,
                font_size: style.font_size,
                font_weight: style.font_weight,
                font_style: style.font_style,
                font_family: style.font_family.clone(),
                margin: effective_margin,
                padding: style.padding,
                width: style.width,
                max_width: style.max_width,
                text_align: style.text_align,
                line_height: style.line_height,
                border_widths: style.border_widths,
                border_colors: style.border_colors,
                border_radii: style.border_radii,
                hover_background,
                focus_background,
                box_shadow: style.box_shadow,
                z_index: style.z_index,
                flex_direction: style.flex_direction,
                justify_content: style.justify_content,
                align_items: style.align_items,
                flex_wrap: style.flex_wrap,
                gap_row: style.gap_row,
                gap_column: style.gap_column,
                box_sizing: style.box_sizing,
                min_width: style.min_width,
                min_height: style.min_height,
                max_height: style.max_height,
                overflow: style.overflow,
                white_space: style.white_space,
                text_transform: style.text_transform,
                opacity: style.opacity,
                align_self: style.align_self,
                flex_grow: style.flex_grow,
                flex_shrink: style.flex_shrink,
                flex_basis: style.flex_basis,
                outline: style.outline,
                background_gradient: style.background_gradient.clone(),
                position: style.position,
                inset_top: style.inset_top,
                inset_right: style.inset_right,
                inset_bottom: style.inset_bottom,
                inset_left: style.inset_left,
                vertical_align: style.vertical_align,
                visibility: style.visibility,
                pointer_events: style.pointer_events,
                text_indent: style.text_indent,
                word_spacing: style.word_spacing,
                text_shadows: style.text_shadows.clone(),
                transforms: style.transforms.clone(),
                grid_template_columns: style.grid_template_columns.clone(),
                grid_template_rows: style.grid_template_rows.clone(),
                text_decoration: style.text_decoration,
                text: None,
                children,
                tag: tag.clone(),
                link,
                image,
                details_open_attr: tag.as_deref() == Some("details")
                    && dom::attr(node, "open").is_some(),
                link_new_tab,
                link_download,
                background_image,
                input_kind,
                input_initial,
                input_placeholder,
                input_name: input_name.or_else(|| {
                    // `<select>` también necesita un `name` para submitear.
                    if tag.as_deref() == Some("select") {
                        dom::attr(node, "name")
                    } else {
                        None
                    }
                }),
                input_checked_initial,
                input_autofocus,
                form_idx: None,
                select,
                svg,
                element_id: dom::attr(node, "id").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
                class_list: dom::attr(node, "class")
                    .map(|s| {
                        s.split_whitespace()
                            .filter(|p| !p.is_empty())
                            .map(|p| p.to_string())
                            .collect()
                    })
                    .unwrap_or_default(),
                attributes: dom::all_attrs(node),
                // Resuelve `animation: <name>` contra la tabla de @keyframes
                // del stylesheet; sólo Some si el nombre matchea.
                animation: style.animation.as_ref().and_then(|b| {
                    styles
                        .keyframes()
                        .get(&b.name)
                        .map(|kf| AnimationInstance {
                            binding: b.clone(),
                            keyframes: kf.clone(),
                        })
                }),
                transitions: style.transitions.clone(),
                node_id: 0,
            })
        }
        NodeData::Text { contents } => {
            let raw = contents.borrow().to_string();
            // CSS whitespace collapse: colapsa runs internos a un solo
            // espacio, preserva un espacio al inicio o fin si lo había
            // (caso clásico: `foo <a>bar</a> baz` debe rendear "foo bar
            // baz" — sin el espacio adyacente al link los tokens se
            // pegan al renderizarse en views vecinas).
            let parent = parent_style.unwrap_or(&ComputedStyle::default()).clone();
            let collapsed = collapse_whitespace(&raw, parent.white_space);
            let collapsed = apply_text_transform(collapsed, parent.text_transform);
            if collapsed.is_empty() {
                return None;
            }
            // El leaf de texto hereda las propiedades inheritables del
            // padre (color, font-size, font-weight, text-align,
            // line-height). Sin esto, todo texto sale negro 16px aunque
            // el `<p>` padre indique color rojo.
            Some(inline_text_with_style(collapsed, &parent))
        }
        _ => {
            // Document / Doctype / Comment → recurrir sólo en hijos. El
            // wrapper que producimos abajo es siempre `Display::Block`, así
            // que filtramos con ese display.
            let mut children = Vec::new();
            for child in node.children.borrow().iter() {
                if let Some(b) = build_node(child, styles, base, parent_style, counters) {
                    children.push(b);
                }
            }
            let children = strip_block_adjacent_whitespace(children, Display::Block);
            let children = collapse_vertical_margins(children);
            if children.is_empty() {
                return None;
            }
            // Wrapeamos los hijos en un block transparente para no
            // perder la jerarquía. Heredamos lo del padre si lo hay.
            let p = parent_style.cloned().unwrap_or_default();
            Some(BoxNode {
                display: Display::Block,
                background: None,
                color: p.color,
                font_size: p.font_size,
                font_weight: p.font_weight,
                font_style: p.font_style,
                font_family: p.font_family.clone(),
                margin: Sides::all(0.0),
                padding: Sides::all(0.0),
                width: LengthVal::Auto,
                max_width: LengthVal::Auto,
                text_align: p.text_align,
                line_height: p.line_height,
                border_widths: Sides::all(0.0),
                border_colors: Sides::all(None),
                border_radii: Corners::all(0.0),
                hover_background: None,
        focus_background: None,
                box_shadow: None,
        z_index: 0,
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::Start,
                align_items: AlignItems::Stretch,
                flex_wrap: FlexWrap::NoWrap,
                gap_row: 0.0,
                gap_column: 0.0,
                box_sizing: BoxSizing::ContentBox,
                min_width: LengthVal::Auto,
                min_height: LengthVal::Auto,
                max_height: LengthVal::Auto,
                overflow: Overflow::Visible,
                white_space: WhiteSpace::Normal,
                text_transform: TextTransform::None,
                opacity: 1.0,
                align_self: AlignSelf::Auto,
                flex_grow: 0.0,
                flex_shrink: 1.0,
                flex_basis: LengthVal::Auto,
                outline: Outline::default(),
                background_gradient: None,
                position: Position::Static,
                inset_top: LengthVal::Auto,
                inset_right: LengthVal::Auto,
                inset_bottom: LengthVal::Auto,
                inset_left: LengthVal::Auto,
                vertical_align: VerticalAlign::Baseline,
                visibility: Visibility::Visible,
                pointer_events: PointerEvents::Auto,
                text_indent: 0.0,
                word_spacing: 0.0,
                text_shadows: Vec::new(),
                transforms: Vec::new(),
                grid_template_columns: Vec::new(),
                grid_template_rows: Vec::new(),
                text_decoration: p.text_decoration,
                text: None,
                children,
                tag: None,
                link: None,
                image: None,
                details_open_attr: false,
                link_new_tab: false,
        link_download: None,
                background_image: None,
                input_kind: None,
                input_initial: None,
                input_placeholder: None,
        input_name: None,
        input_checked_initial: false,
        input_autofocus: false,
        form_idx: None,
        select: None,
        svg: None,
        element_id: None,
        class_list: Vec::new(),
        attributes: Vec::new(),
                animation: None,
                transitions: Vec::new(),
                node_id: 0,
            })
        }
    }
}

/// Construye un nodo Text inline con el color/font/text-align/line-height
/// del estilo dado — usado tanto por hojas Text reales como por los
/// markers sintéticos (`•` de `<li>`, `[img: alt]` de `<img>` roto).
fn inline_text_with_style(s: String, style: &ComputedStyle) -> BoxNode {
    BoxNode {
        display: Display::Inline,
        background: None,
        color: style.color,
        font_size: style.font_size,
        font_weight: style.font_weight,
        font_style: style.font_style,
        font_family: style.font_family.clone(),
        margin: Sides::all(0.0),
        padding: Sides::all(0.0),
        width: LengthVal::Auto,
        max_width: LengthVal::Auto,
        text_align: style.text_align,
        line_height: style.line_height,
        border_widths: Sides::all(0.0),
        border_colors: Sides::all(None),
        border_radii: Corners::all(0.0),
        hover_background: None,
        focus_background: None,
        box_shadow: None,
        z_index: 0,
        flex_direction: FlexDirection::Row,
        justify_content: JustifyContent::Start,
        align_items: AlignItems::Stretch,
        flex_wrap: FlexWrap::NoWrap,
        gap_row: 0.0,
        gap_column: 0.0,
        box_sizing: BoxSizing::ContentBox,
        min_width: LengthVal::Auto,
        min_height: LengthVal::Auto,
        max_height: LengthVal::Auto,
        overflow: Overflow::Visible,
        white_space: WhiteSpace::Normal,
        text_transform: TextTransform::None,
        opacity: 1.0,
        align_self: AlignSelf::Auto,
        flex_grow: 0.0,
        flex_shrink: 1.0,
        flex_basis: LengthVal::Auto,
        outline: Outline::default(),
        background_gradient: None,
        position: Position::Static,
        inset_top: LengthVal::Auto,
        inset_right: LengthVal::Auto,
        inset_bottom: LengthVal::Auto,
        inset_left: LengthVal::Auto,
        vertical_align: VerticalAlign::Baseline,
        visibility: Visibility::Visible,
        pointer_events: PointerEvents::Auto,
        text_indent: 0.0,
        word_spacing: 0.0,
        text_shadows: Vec::new(),
        transforms: Vec::new(),
        grid_template_columns: Vec::new(),
        grid_template_rows: Vec::new(),
        text_decoration: style.text_decoration,
        text: Some(s),
        children: Vec::new(),
        tag: None,
        link: None,
        image: None,
        details_open_attr: false,
        link_new_tab: false,
        link_download: None,
        background_image: None,
        input_kind: None,
        input_initial: None,
        input_placeholder: None,
        input_name: None,
        input_checked_initial: false,
        input_autofocus: false,
        form_idx: None,
        select: None,
        svg: None,
        element_id: None,
        class_list: Vec::new(),
        attributes: Vec::new(),
        animation: None,
        transitions: Vec::new(),
        node_id: 0,
    }
}

/// `true` si el nodo se comporta como block-level para el flujo (Block,
/// Flex, Grid, None). `Inline*` queda fuera — son del flow inline.
fn is_block_level(b: &BoxNode) -> bool {
    !matches!(
        b.display,
        Display::Inline | Display::InlineBlock | Display::InlineFlex | Display::InlineGrid
    )
}

/// `true` si el nodo es un leaf de texto inline cuyo contenido se reduce
/// a whitespace (incluye el caso post-collapse del CSS, que deja " "
/// como "espacio entre tokens"). `<br>` y otros inlines sin texto no
/// matchean (b.text es None).
fn is_ws_only_inline(b: &BoxNode) -> bool {
    matches!(b.display, Display::Inline | Display::InlineBlock)
        && b
            .text
            .as_ref()
            .map(|s| !s.is_empty() && s.chars().all(|c| c.is_whitespace()))
            .unwrap_or(false)
}

/// Quita los text-nodes whitespace-only que separan block siblings o
/// quedan adyacentes al borde de un block. Replica el comportamiento
/// estándar de los browsers: en HTML, el `\n  ` entre `</p>\n  <h2>`
/// produce un Text node " " que NO debe rendear (sino cada tag aporta
/// una línea visible vacía). Se preserva si está rodeado de inlines
/// (ahí sí lleva valor: separa tokens).
fn strip_block_adjacent_whitespace(
    children: Vec<BoxNode>,
    parent_display: Display,
) -> Vec<BoxNode> {
    // Cuando el padre es Inline (`<span>`, `<em>`, etc.) los hijos viven
    // en el inline-flow del *abuelo* block; los whitespace que tengan
    // dentro pueden ser parte de un token relevante ("foo<span> </span>
    // bar" debe mantener los dos espacios). No filtramos a este nivel —
    // el filtrado real ocurre cuando el padre sí establece un contexto
    // block (Block/Flex/Grid/InlineBlock/etc.).
    if matches!(parent_display, Display::Inline) {
        return children;
    }
    if children.iter().all(|c| !is_ws_only_inline(c)) {
        return children;
    }
    let block_levels: Vec<bool> = children.iter().map(is_block_level).collect();
    let ws_only: Vec<bool> = children.iter().map(is_ws_only_inline).collect();
    let n = children.len();
    // Para cada nodo whitespace-only, buscamos el primer vecino no-ws
    // (antes y después). Si ambos son block-level (o son edge), drop —
    // la run entera de whitespace entre dos blocks no aporta nada
    // visual. Antes mirábamos sólo el vecino inmediato, lo que dejaba
    // que runs consecutivas se preservaran al final del body
    // ("<blockquote>X</blockquote>  \n  ").
    let mut out = Vec::with_capacity(n);
    for (i, c) in children.into_iter().enumerate() {
        if ws_only[i] {
            let prev_is_block_or_edge = {
                let mut j = i;
                loop {
                    if j == 0 {
                        break true;
                    }
                    j -= 1;
                    if !ws_only[j] {
                        break block_levels[j];
                    }
                }
            };
            let next_is_block_or_edge = {
                let mut j = i + 1;
                loop {
                    if j >= n {
                        break true;
                    }
                    if !ws_only[j] {
                        break block_levels[j];
                    }
                    j += 1;
                }
            };
            if prev_is_block_or_edge && next_is_block_or_edge {
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// Descarga `url` y la decodifica a RGBA8. Devuelve `None` si la URL no
/// es HTTP(S), si la descarga falla, si el MIME no es imagen, o si el
/// decoder no soporta el formato. Sync: bloquea el thread caller — el
/// chrome ya está en un worker thread durante `Engine::load`. Pasa por
/// la cache global de bytes — recargas y navegación entre tabs no
/// re-descargan.
/// Materializa los `ContentItem`s del `content:` del pseudo en boxes
/// hijos. Strings/counters/attrs adjacentes se concatenan en un
/// `inline_text_with_style`; cada `Url` genera un `<img>` sintético
/// inline-block. Orden preservado. Items que producen string vacía o
/// urls que fallan al decode se omiten silenciosamente.
fn emit_content_items(
    items: &[crate::style::ContentItem],
    node: &Handle,
    counters: &std::collections::HashMap<String, i32>,
    pseudo_style: &ComputedStyle,
    base: Option<&url::Url>,
    out: &mut Vec<BoxNode>,
) {
    use crate::style::ContentItem;
    let mut text_buf = String::new();
    let flush_text = |buf: &mut String, out: &mut Vec<BoxNode>| {
        if !buf.is_empty() {
            out.push(inline_text_with_style(std::mem::take(buf), pseudo_style));
        }
    };
    for it in items {
        match it {
            ContentItem::Text(s) => text_buf.push_str(s),
            ContentItem::Counter(name) => {
                let v = counters.get(name).copied().unwrap_or(0);
                text_buf.push_str(&v.to_string());
            }
            ContentItem::Attr(name) => {
                if let Some(v) = dom::attr(node, name) {
                    text_buf.push_str(&v);
                }
            }
            ContentItem::Url(u) => {
                flush_text(&mut text_buf, out);
                if let Some(abs) = resolve_href(base, u) {
                    if let Some(img) = fetch_and_decode(&abs) {
                        out.push(synthetic_image_box(img, pseudo_style));
                    }
                    // Si fetch/decode falla, lo omitimos (matchea CSS
                    // spec: url() inválido suprime la generación).
                }
            }
        }
    }
    flush_text(&mut text_buf, out);
}

/// Construye un BoxNode inline-block con una imagen ya decodificada,
/// hereda el estilo del pseudo. Se usa para `content: url(...)`.
fn synthetic_image_box(img: ImageData, style: &ComputedStyle) -> BoxNode {
    let mut b = inline_text_with_style(String::new(), style);
    b.display = Display::InlineBlock;
    b.image = Some(img);
    b.text = None;
    b
}

/// Margin collapsing contra el padre. CSS spec:
/// - Si el padre NO tiene border-top ni padding-top, el margin-top
///   del primer hijo block in-flow "se ve" como parte del padre —
///   se promueve y queda en `max(parent.margin_top, child.margin_top)`.
///   El hijo se setea a 0 para evitar doble cuenta.
/// - Idem para el último hijo y bottom.
///
/// Esto destraba el caso típico: `body { margin: 8px }` con un primer
/// `<h1 style="margin: 21px 0">` — sin collapse el body tiene 8px +
/// 21px = 29px arriba; con collapse, max(8, 21) = 21px, que es lo que
/// hacen los browsers reales.
fn collapse_margins_against_parent(
    mut children: Vec<BoxNode>,
    parent_margin: &mut Sides<f32>,
    no_top_barrier: bool,
    no_bot_barrier: bool,
) -> Vec<BoxNode> {
    if no_top_barrier {
        if let Some(first) = children.first_mut() {
            if is_block_level(first) && first.margin.top > 0.0 {
                parent_margin.top = parent_margin.top.max(first.margin.top);
                first.margin.top = 0.0;
            }
        }
    }
    if no_bot_barrier {
        if let Some(last) = children.last_mut() {
            if is_block_level(last) && last.margin.bottom > 0.0 {
                parent_margin.bottom = parent_margin.bottom.max(last.margin.bottom);
                last.margin.bottom = 0.0;
            }
        }
    }
    children
}

/// Margin collapsing CSS — entre hermanos block adyacentes, el gap
/// vertical es `max(prev.margin_bottom, next.margin_top)` (NO la suma).
/// Sin esto, raw HTML pages como motherfucking se ven con gaps el
/// doble entre `<h2>` y `<p>` consecutivos. Implementación simple:
/// para cada par (block, block) consecutivo, restamos del margin_top
/// del segundo el min(prev.margin_bottom, next.margin_top). El total
/// `prev.margin_bottom + next.margin_top_modificado` queda igual a
/// `max(prev.margin_bottom, next.margin_top)`.
///
/// Casos NO cubiertos (queda para una iteración más completa):
/// - Collapse con el padre (cuando primer/último hijo block no tiene
///   padding/border arriba/abajo, su margin colapsa contra el padre).
/// - Negative margins (CSS spec dice que se tratan separadamente).
/// - Through-block collapsing en blocks vacíos.
fn collapse_vertical_margins(children: Vec<BoxNode>) -> Vec<BoxNode> {
    if children.len() < 2 {
        return children;
    }
    let mut out: Vec<BoxNode> = Vec::with_capacity(children.len());
    for c in children {
        if let Some(prev) = out.last() {
            if is_block_level(prev) && is_block_level(&c) {
                let prev_bot = prev.margin.bottom.max(0.0);
                let next_top = c.margin.top.max(0.0);
                let reduction = prev_bot.min(next_top);
                if reduction > 0.0 {
                    let mut adjusted = c;
                    adjusted.margin.top -= reduction;
                    out.push(adjusted);
                    continue;
                }
            }
        }
        out.push(c);
    }
    out
}

/// Workers paralelos para el prefetch. 6 es un compromiso razonable:
/// alto enough para esconder latencia de TCP/TLS (cada handshake ~50-
/// 200ms), bajo enough para no saturar servidores ni el ulimit de
/// sockets del proceso. Browsers reales usan 6-8 por host.
const PREFETCH_WORKERS: usize = 6;

/// Pre-walk del DOM coleccionando URLs absolutas de `<img src>`,
/// `<img srcset>`, `<picture><source srcset>`, y disparando descargas
/// paralelas. La cache global de bytes guarda los resultados —
/// `fetch_and_decode` en `build_node` después hace cache hit.
fn prefetch_image_urls(root: &Handle, base: Option<&url::Url>) {
    let mut urls: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut push = |u: String| {
        if seen.insert(u.clone()) {
            urls.push(u);
        }
    };
    dom::walk(root, &mut |node| {
        let tag = dom::element_name(node);
        match tag.as_deref() {
            Some("img") => {
                if let Some(src) = pick_srcset(&dom::attr(node, "srcset").unwrap_or_default())
                    .or_else(|| dom::attr(node, "src"))
                {
                    if let Some(abs) = resolve_href(base, &src) {
                        push(abs);
                    }
                }
            }
            Some("picture") => {
                for child in node.children.borrow().iter() {
                    if dom::element_name(child).as_deref() == Some("source") {
                        if let Some(s) = dom::attr(child, "srcset") {
                            if let Some(c) = pick_srcset(&s) {
                                if let Some(abs) = resolve_href(base, &c) {
                                    push(abs);
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    });
    if urls.is_empty() {
        return;
    }
    // Cache hits no necesitan fetch; los filtramos para ahorrar threads.
    // Además filtramos schemes no-HTTP (`about:`, `file:`, `data:`) —
    // ureq haría un round-trip al timeout para nada.
    let pending: Vec<String> = urls
        .into_iter()
        .filter(|u| {
            url::Url::parse(u)
                .ok()
                .map(|p| matches!(p.scheme(), "http" | "https"))
                .unwrap_or(false)
        })
        .filter(|u| crate::cache::get(u).is_none())
        .collect();
    if pending.is_empty() {
        return;
    }
    // Pool simple: dividir las URLs en chunks de tamaño ceil(N/W) y un
    // thread por chunk. Más simple que un channel + N workers, y para
    // 6-30 URLs típicas de una página el balance es suficiente.
    let chunk_size = pending.len().div_ceil(PREFETCH_WORKERS).max(1);
    let mut handles = Vec::new();
    for chunk in pending.chunks(chunk_size) {
        let chunk = chunk.to_vec();
        handles.push(std::thread::spawn(move || {
            for url in chunk {
                // Best-effort: errores se ignoran. El build_node
                // posterior los reintentará serializado y muestra el
                // alt del `<img>` si igual falla.
                let _ = crate::fetch::fetch_bytes(&url);
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }
}

/// Segundo pass de prefetch: recolecta URLs de `background-image:
/// url(...)` después de computar styles. Reusa el mismo pool de
/// workers que `prefetch_image_urls`. Computamos sin parent porque
/// `background-image` no se hereda y los valores son independientes
/// del contexto del padre (cosa que sí valdría para `color` o
/// `font-size`).
fn prefetch_background_image_urls(
    root: &Handle,
    styles: &StyleEngine,
    base: Option<&url::Url>,
) {
    let mut urls: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    dom::walk(root, &mut |node| {
        if !matches!(node.data, markup5ever_rcdom::NodeData::Element { .. }) {
            return;
        }
        let style = styles.compute(node);
        if let Some(u) = style.background_image_url.as_deref() {
            if let Some(abs) = resolve_href(base, u) {
                if seen.insert(abs.clone()) {
                    urls.push(abs);
                }
            }
        }
    });
    if urls.is_empty() {
        return;
    }
    let pending: Vec<String> = urls
        .into_iter()
        .filter(|u| {
            url::Url::parse(u)
                .ok()
                .map(|p| matches!(p.scheme(), "http" | "https"))
                .unwrap_or(false)
        })
        .filter(|u| crate::cache::get(u).is_none())
        .collect();
    if pending.is_empty() {
        return;
    }
    let chunk_size = pending.len().div_ceil(PREFETCH_WORKERS).max(1);
    let mut handles = Vec::new();
    for chunk in pending.chunks(chunk_size) {
        let chunk = chunk.to_vec();
        handles.push(std::thread::spawn(move || {
            for url in chunk {
                let _ = crate::fetch::fetch_bytes(&url);
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }
}

fn fetch_and_decode(url: &str) -> Option<ImageData> {
    let bytes = crate::fetch::fetch_bytes(url).ok()?;
    decode_image_bytes(&bytes)
}

/// Decodifica bytes de imagen (PNG/JPEG por las features de `image`) a RGBA8.
/// `None` si el formato no está habilitado o el decode falla.
fn decode_image_bytes(bytes: &[u8]) -> Option<ImageData> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    reader.format()?; // formato no habilitado por features → None
    let img = reader.decode().ok()?;
    let rgba = img.to_rgba8();
    let (width, height) = (rgba.width(), rgba.height());
    Some(ImageData { rgba: rgba.into_raw(), width, height })
}

/// Resuelve+decodifica la imagen de un `src`/`srcset`/`background-image`.
/// Los `data:` URLs se decodifican inline (RFC 2397) — `resolve_href` los
/// bloquea a propósito (no son navegables como `<a href>`), pero como fuente
/// de un recurso son legítimos. El resto resuelve contra `base` y baja por
/// HTTP/file. `None` si falta src o falla la decodificación.
fn fetch_image_src(base: Option<&url::Url>, src: &str) -> Option<ImageData> {
    if crate::fetch::is_data_url(src.trim()) {
        return decode_image_bytes(&crate::fetch::decode_data_url(src.trim())?);
    }
    let abs = resolve_href(base, src)?;
    fetch_and_decode(&abs)
}

/// Colapso de whitespace según `white-space`:
/// - `Normal` / `NoWrap`: runs internos → un espacio, leading/trailing
///   reducidos a uno; newlines colapsan igual.
/// - `Pre`: todo preservado.
/// - `PreWrap`: igual que Pre — el wrap es responsabilidad del layout.
/// - `PreLine`: runs de espacio/tab → un espacio, newlines preservados.
fn collapse_whitespace(s: &str, ws: WhiteSpace) -> String {
    match ws {
        WhiteSpace::Pre | WhiteSpace::PreWrap => s.to_string(),
        WhiteSpace::PreLine => {
            // Colapsa espacios/tabs (no '\n') a uno solo, preserva newlines.
            let mut out = String::with_capacity(s.len());
            let mut prev_space = false;
            for c in s.chars() {
                if c == '\n' {
                    out.push(c);
                    prev_space = false;
                } else if c.is_whitespace() {
                    if !prev_space {
                        out.push(' ');
                        prev_space = true;
                    }
                } else {
                    out.push(c);
                    prev_space = false;
                }
            }
            out
        }
        WhiteSpace::Normal | WhiteSpace::NoWrap => {
            let leading = s.chars().next().is_some_and(|c| c.is_whitespace());
            let trailing = s.chars().last().is_some_and(|c| c.is_whitespace());
            let core: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
            if core.is_empty() {
                // Sólo whitespace: lo dejamos como " " para no perder el
                // separador entre inlines vecinos.
                return if leading || trailing { " ".to_string() } else { String::new() };
            }
            let mut out = String::with_capacity(core.len() + 2);
            if leading {
                out.push(' ');
            }
            out.push_str(&core);
            if trailing {
                out.push(' ');
            }
            out
        }
    }
}

/// Aplica `text-transform` al texto. Capitalize convierte la primera
/// letra de cada palabra (separada por whitespace) a mayúscula.
fn apply_text_transform(s: String, t: TextTransform) -> String {
    match t {
        TextTransform::None => s,
        TextTransform::Uppercase => s.to_uppercase(),
        TextTransform::Lowercase => s.to_lowercase(),
        TextTransform::Capitalize => {
            let mut out = String::with_capacity(s.len());
            let mut start_of_word = true;
            for c in s.chars() {
                if c.is_whitespace() {
                    out.push(c);
                    start_of_word = true;
                } else if start_of_word {
                    out.extend(c.to_uppercase());
                    start_of_word = false;
                } else {
                    out.push(c);
                }
            }
            out
        }
    }
}

/// Construye el texto del marker de un `<li>`. Para tipos numerados
/// (`decimal`/`*-alpha`/`*-roman`) calcula la posición del item entre sus
/// hermanos `<li>` del mismo padre, respetando `<ol start>` y
/// `<li value>`. Devuelve `None` si `list-style-type: none`.
///
/// Marcadores con número usan `"N. "` (período + un espacio) — alineado
/// con el comportamiento de browsers. Marcadores con símbolo usan
/// `"<sym>  "` (doble espacio) para dar el airecito que tenía el bullet
/// hardcoded original.
fn li_marker(node: &Handle, kind: ListStyleType) -> Option<String> {
    match kind {
        ListStyleType::None => None,
        ListStyleType::Disc => Some("• ".into()),
        ListStyleType::Circle => Some("◦ ".into()),
        ListStyleType::Square => Some("▪ ".into()),
        ListStyleType::Decimal => Some(format!("{}. ", ol_item_position(node))),
        ListStyleType::LowerAlpha => {
            Some(format!("{}. ", to_alpha(ol_item_position(node), false)))
        }
        ListStyleType::UpperAlpha => {
            Some(format!("{}. ", to_alpha(ol_item_position(node), true)))
        }
        ListStyleType::LowerRoman => {
            Some(format!("{}. ", to_roman(ol_item_position(node), false)))
        }
        ListStyleType::UpperRoman => {
            Some(format!("{}. ", to_roman(ol_item_position(node), true)))
        }
    }
}

/// Posición 1-indexed del `<li>` entre sus hermanos `<li>` del padre.
/// Respeta `<ol start="N">` (arranca el contador en N) y `<li value="N">`
/// (resetea el contador al valor dado para ese item y los siguientes).
/// Si `node` no es un `<li>` o no tiene padre, devuelve 1.
fn ol_item_position(node: &Handle) -> i32 {
    let Some(parent) = parent_handle(node) else { return 1 };
    let parent_is_ol = dom::element_name(&parent).as_deref() == Some("ol");
    let mut counter: i32 = if parent_is_ol {
        dom::attr(&parent, "start").and_then(|s| s.trim().parse().ok()).unwrap_or(1)
    } else {
        1
    };
    for child in parent.children.borrow().iter() {
        if dom::element_name(child).as_deref() != Some("li") {
            continue;
        }
        if let Some(v) = dom::attr(child, "value").and_then(|s| s.trim().parse::<i32>().ok()) {
            counter = v;
        }
        if std::rc::Rc::ptr_eq(child, node) {
            return counter;
        }
        counter += 1;
    }
    counter
}

/// Misma idea que `style::parent_of`. Lo duplicamos acá para no tocar
/// la visibilidad del helper en `style.rs`.
fn parent_handle(node: &Handle) -> Option<Handle> {
    let weak = node.parent.take();
    let restored = weak.clone();
    node.parent.set(restored);
    weak.and_then(|w| w.upgrade())
}

/// Convierte 1..N a alpha bijectiva base-26 (1=a, 26=z, 27=aa, 28=ab…).
/// Valores `<= 0` caen a `"0"` — el marker numérico igual se imprime.
fn to_alpha(mut n: i32, upper: bool) -> String {
    if n <= 0 {
        return n.to_string();
    }
    let mut buf: Vec<u8> = Vec::new();
    while n > 0 {
        n -= 1;
        let d = (n % 26) as u8;
        buf.push(if upper { b'A' + d } else { b'a' + d });
        n /= 26;
    }
    buf.reverse();
    // SAFETY: sólo ASCII A-Z/a-z.
    String::from_utf8(buf).expect("alpha ascii-only")
}

/// Romanos 1..3999. Fuera del rango caemos a decimal — matchea el
/// comportamiento de browsers (Chromium también).
fn to_roman(n: i32, upper: bool) -> String {
    if !(1..=3999).contains(&n) {
        return n.to_string();
    }
    const VALUES: &[(i32, &str, &str)] = &[
        (1000, "M", "m"),
        (900, "CM", "cm"),
        (500, "D", "d"),
        (400, "CD", "cd"),
        (100, "C", "c"),
        (90, "XC", "xc"),
        (50, "L", "l"),
        (40, "XL", "xl"),
        (10, "X", "x"),
        (9, "IX", "ix"),
        (5, "V", "v"),
        (4, "IV", "iv"),
        (1, "I", "i"),
    ];
    let mut n = n;
    let mut out = String::new();
    for (val, up, lo) in VALUES {
        while n >= *val {
            out.push_str(if upper { up } else { lo });
            n -= val;
        }
    }
    out
}

fn resolve_href(base: Option<&url::Url>, href: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty() {
        return None;
    }
    // Schemes que NO son web: el chrome no debería intentar navegar a ellos.
    let lc = href.to_ascii_lowercase();
    if lc.starts_with("javascript:")
        || lc.starts_with("mailto:")
        || lc.starts_with("tel:")
        || lc.starts_with("sms:")
        || lc.starts_with("data:")
    {
        return None;
    }
    // Fragmentos puros (`#foo`): resuelven a la URL actual + fragment.
    // El chrome detecta same-page navigation (mismo URL sans fragment)
    // y scrollea al elemento con id matching en lugar de recargar.
    if href.starts_with('#') {
        return base.and_then(|b| b.join(href).ok()).map(|u| u.to_string());
    }
    if let Ok(abs) = url::Url::parse(href) {
        // Sólo http/https son navegables por puriy hoy. file://, ftp://,
        // etc. quedan ignorados para no romper la pestaña.
        return match abs.scheme() {
            "http" | "https" | "about" => Some(abs.into()),
            _ => None,
        };
    }
    base.and_then(|b| b.join(href).ok()).and_then(|abs| {
        match abs.scheme() {
            "http" | "https" | "about" => Some(abs.into()),
            _ => None,
        }
    })
}

impl ComputedStyle {
    // Asegura que ComputedStyle es referenciable desde boxes (sin re-export
    // cycles). Sin este impl no haría falta; lo dejamos para forzar el
    // link en docs.
    #[doc(hidden)]
    pub fn _link(_: &Self) {}
}

#[cfg(test)]
mod tests {
    use super::Display;
    use crate::Engine;

    #[test]
    fn box_tree_no_vacio() {
        let html = "<html><body><h1>Hola</h1><p>Mundo</p></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        assert!(doc.box_tree.descendants_count() >= 3);
    }

    #[test]
    fn node_ids_son_unicos_y_no_cero() {
        let html = "<html><body><div><h1>Hola</h1><p>Mundo</p></div></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut ids = Vec::new();
        doc.box_tree.walk(|b| ids.push(b.node_id));
        assert!(ids.iter().all(|&id| id != 0), "ningún nodo queda en 0");
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "los node_id son únicos");
        // DFS pre-orden arranca en 1 sobre la raíz (body).
        assert_eq!(doc.box_tree.root.node_id, 1);
    }

    #[test]
    fn display_none_excluye_head() {
        let html = "<html><head><title>t</title></head><body><p>x</p></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        // El árbol parte de body — head no debe haber aportado nada.
        let mut tags = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.tag {
                tags.push(t.clone());
            }
        });
        assert!(!tags.contains(&"title".to_string()));
        assert!(!tags.contains(&"head".to_string()));
    }

    #[test]
    fn ol_li_recibe_marker_decimal() {
        let html =
            "<html><body><ol><li>uno</li><li>dos</li><li>tres</li></ol></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut markers = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.ends_with(". ") {
                    markers.push(t.clone());
                }
            }
        });
        assert_eq!(markers, vec!["1. ".to_string(), "2. ".into(), "3. ".into()]);
    }

    #[test]
    fn ul_li_recibe_marker_bullet() {
        let html = "<html><body><ul><li>a</li><li>b</li></ul></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut markers = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.starts_with('•') {
                    markers.push(t.clone());
                }
            }
        });
        assert_eq!(markers.len(), 2);
    }

    #[test]
    fn unidades_viewport_resuelven_contra_el_viewport_real() {
        use crate::style::{LengthVal, Viewport};
        // `vw/vh/vmin/vmax` deben resolver contra el ancho/alto REAL de la
        // ventana, no contra DEFAULT_VIEWPORT (1280×800). Con viewport 800×600
        // y `style="…"` inline (que parsea `boxes::build`, no la hoja):
        //   50vw   = 50% de 800            = 400
        //   50vh   = 50% de 600            = 300
        //   50vmin = 50% de min(800,600)   = 300
        //   50vmax = 50% de max(800,600)   = 400
        let html = r#"<html><body>
            <div id="vw" style="width:50vw"></div>
            <div id="vh" style="width:50vh"></div>
            <div id="vmin" style="width:50vmin"></div>
            <div id="vmax" style="width:50vmax"></div>
        </body></html>"#;
        let vp = Viewport { width: 800.0, height: 600.0, dpr: 1.0 };
        let doc = Engine::new().with_viewport(vp).load_html("about:test", html);
        let mut widths = std::collections::HashMap::new();
        doc.box_tree.walk(|b| {
            if let Some(id) = b.element_id.as_deref() {
                widths.insert(id.to_string(), b.width);
            }
        });
        assert_eq!(widths.get("vw"), Some(&LengthVal::Px(400.0)));
        assert_eq!(widths.get("vh"), Some(&LengthVal::Px(300.0)));
        assert_eq!(widths.get("vmin"), Some(&LengthVal::Px(300.0)));
        assert_eq!(widths.get("vmax"), Some(&LengthVal::Px(400.0)));
    }

    #[test]
    fn unidades_viewport_default_sin_viewport_real() {
        use crate::style::LengthVal;
        // Sin `with_viewport`, el Engine usa DEFAULT_VIEWPORT (1280×800):
        // 50vw = 640. Garantiza que el scope no contamina el path por defecto
        // (se restaura al dropear al volver de `load_html`).
        let html = r#"<html><body><div id="x" style="width:50vw"></div></body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        let mut w = None;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("x") {
                w = Some(b.width);
            }
        });
        assert_eq!(w, Some(LengthVal::Px(640.0)));
    }

    fn box_by_id(bt: &super::BoxTree, id: &str) -> Option<super::BoxNode> {
        let mut found = None;
        bt.walk(|b| {
            if found.is_none() && b.element_id.as_deref() == Some(id) {
                found = Some(b.clone());
            }
        });
        found
    }

    #[test]
    fn restyle_aplica_regla_de_clase_agregada() {
        // `.on` (no presente al cargar) + un selector descendiente `.on .child`.
        // Tras agregar la clase y recascadear, el fondo del box y el color del
        // hijo deben aparecer.
        let html = r#"<html><head><style>
            .on { background: red; }
            .on .child { color: blue; }
        </style></head><body>
            <div id="box"><p id="p" class="child">x</p></div>
        </body></html>"#;
        let mut doc = Engine::new().load_html("about:test", html);
        assert_eq!(box_by_id(&doc.box_tree, "box").unwrap().background, None);
        assert!(doc.box_tree.set_element_class_list("box", vec!["on".to_string()]));
        doc.box_tree.restyle();
        assert_eq!(
            box_by_id(&doc.box_tree, "box").unwrap().background,
            Some(super::Color::rgb(255, 0, 0))
        );
        assert_eq!(box_by_id(&doc.box_tree, "p").unwrap().color, super::Color::rgb(0, 0, 255));
    }

    #[test]
    fn restyle_quitar_clase_revierte_estilo() {
        let html = r#"<html><head><style>
            #box { background: green; }
            #box.on { background: red; }
        </style></head><body><div id="box" class="on">x</div></body></html>"#;
        let mut doc = Engine::new().load_html("about:test", html);
        assert_eq!(
            box_by_id(&doc.box_tree, "box").unwrap().background,
            Some(super::Color::rgb(255, 0, 0))
        );
        doc.box_tree.set_element_class_list("box", vec![]);
        doc.box_tree.restyle();
        // Sin `.on`, gana la regla base `#box { background: green }`.
        assert_eq!(
            box_by_id(&doc.box_tree, "box").unwrap().background,
            Some(super::Color::rgb(0, 128, 0))
        );
    }

    #[test]
    fn restyle_combinador_hermano_afecta_posterior() {
        // Cambiar la clase de #t debe afectar a su HERMANO #pnl vía `+`.
        // Sólo posible recascadeando el árbol entero, no sólo el subárbol.
        let html = r#"<html><head><style>
            .open + .panel { background: red; }
        </style></head><body>
            <div id="t" class="tab"></div>
            <div id="pnl" class="panel">x</div>
        </body></html>"#;
        let mut doc = Engine::new().load_html("about:test", html);
        assert_eq!(box_by_id(&doc.box_tree, "pnl").unwrap().background, None);
        doc.box_tree
            .set_element_class_list("t", vec!["tab".into(), "open".into()]);
        doc.box_tree.restyle();
        assert_eq!(
            box_by_id(&doc.box_tree, "pnl").unwrap().background,
            Some(super::Color::rgb(255, 0, 0))
        );
    }

    #[test]
    fn restyle_toggle_display_none_oculta_y_muestra() {
        let html = r#"<html><head><style>
            .hidden { display: none; }
        </style></head><body><div id="box">x</div></body></html>"#;
        let mut doc = Engine::new().load_html("about:test", html);
        assert_ne!(box_by_id(&doc.box_tree, "box").unwrap().display, super::Display::None);
        doc.box_tree.set_element_class_list("box", vec!["hidden".into()]);
        doc.box_tree.restyle();
        assert_eq!(box_by_id(&doc.box_tree, "box").unwrap().display, super::Display::None);
        doc.box_tree.set_element_class_list("box", vec![]);
        doc.box_tree.restyle();
        assert_ne!(box_by_id(&doc.box_tree, "box").unwrap().display, super::Display::None);
    }

    #[test]
    fn restyle_sin_cambios_es_idempotente() {
        let html = r#"<html><head><style>
            #box { background: red; color: green; padding: 5px; font-size: 20px; }
        </style></head><body><div id="box"><span id="s">hi</span></div></body></html>"#;
        let mut doc = Engine::new().load_html("about:test", html);
        let before_box = box_by_id(&doc.box_tree, "box").unwrap();
        let before_s = box_by_id(&doc.box_tree, "s").unwrap();
        doc.box_tree.restyle();
        let after_box = box_by_id(&doc.box_tree, "box").unwrap();
        let after_s = box_by_id(&doc.box_tree, "s").unwrap();
        assert_eq!(before_box.background, after_box.background);
        assert_eq!(before_box.color, after_box.color);
        assert_eq!(before_box.display, after_box.display);
        assert_eq!(before_box.padding.top, after_box.padding.top);
        assert_eq!(before_box.font_size, after_box.font_size);
        // El span hereda color/font del padre, igual antes y después.
        assert_eq!(before_s.color, after_s.color);
        assert_eq!(before_s.font_size, after_s.font_size);
    }

    #[test]
    fn restyle_preserva_estilo_inline_seteado_por_js() {
        // `el.style.color='red'` (via set_element_style) debe sobrevivir a un
        // restyle posterior por classList: la cascada re-parsea el atributo
        // `style` y el inline gana sobre la regla `.on { color: blue }`.
        let html = r#"<html><head><style>.on { color: blue; }</style></head>
            <body><p id="p">x</p></body></html>"#;
        let mut doc = Engine::new().load_html("about:test", html);
        doc.box_tree.set_element_style("p", "color", "red");
        assert_eq!(box_by_id(&doc.box_tree, "p").unwrap().color, super::Color::rgb(255, 0, 0));
        doc.box_tree.set_element_class_list("p", vec!["on".into()]);
        doc.box_tree.restyle();
        assert_eq!(box_by_id(&doc.box_tree, "p").unwrap().color, super::Color::rgb(255, 0, 0));
    }

    #[test]
    fn build_retiene_display_none_de_autor_y_descarta_ua() {
        // Fase 7.185 — un elemento ocultado por CSS de autor se RETIENE en el
        // box tree (oculto, con su subárbol) para poder mostrarlo luego; el
        // ruido UA (`<script>`) se sigue descartando.
        let html = r#"<html><head><style>
            .modal { display: none; }
        </style></head><body>
            <div id="m" class="modal"><p id="inner">contenido</p></div>
            <script>var x = 1;</script>
            <span id="s">visible</span>
        </body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        let m = box_by_id(&doc.box_tree, "m").expect("modal de autor retenido");
        assert_eq!(m.display, super::Display::None);
        assert!(box_by_id(&doc.box_tree, "inner").is_some(), "subárbol retenido");
        let mut script_text = false;
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.contains("var x") {
                    script_text = true;
                }
            }
        });
        assert!(!script_text, "el texto del <script> no debe filtrarse al box tree");
        assert!(box_by_id(&doc.box_tree, "s").is_some());
    }

    #[test]
    fn restyle_muestra_modal_oculto_al_cargar() {
        // El patrón clásico: modal arranca `display:none`, JS agrega `.open`
        // para mostrarlo. Posible porque retenemos el box oculto al cargar.
        let html = r#"<html><head><style>
            .modal { display: none; }
            .modal.open { display: block; }
        </style></head><body>
            <div id="m" class="modal">hola</div>
        </body></html>"#;
        let mut doc = Engine::new().load_html("about:test", html);
        assert_eq!(box_by_id(&doc.box_tree, "m").unwrap().display, super::Display::None);
        doc.box_tree
            .set_element_class_list("m", vec!["modal".into(), "open".into()]);
        doc.box_tree.restyle();
        assert_eq!(box_by_id(&doc.box_tree, "m").unwrap().display, super::Display::Block);
    }

    #[test]
    fn pseudo_estado_checked_disabled_enabled() {
        let html = r#"<html><head><style>
            input:checked { background: red; }
            input:disabled { color: green; }
            input:enabled { color: blue; }
            input:required { background: yellow; }
        </style></head><body>
            <input id="a" type="checkbox" checked>
            <input id="b" type="checkbox">
            <input id="c" type="text" disabled>
            <input id="d" type="text" required>
        </body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        let red = super::Color::rgb(255, 0, 0);
        let blue = super::Color::rgb(0, 0, 255);
        // a: checked → fondo rojo; enabled → color azul.
        assert_eq!(box_by_id(&doc.box_tree, "a").unwrap().background, Some(red));
        assert_eq!(box_by_id(&doc.box_tree, "a").unwrap().color, blue);
        // b: no checked → no rojo (conserva su fondo UA); enabled → azul.
        assert_ne!(box_by_id(&doc.box_tree, "b").unwrap().background, Some(red));
        assert_eq!(box_by_id(&doc.box_tree, "b").unwrap().color, blue);
        // c: disabled → verde; NO enabled (no azul).
        assert_eq!(box_by_id(&doc.box_tree, "c").unwrap().color, super::Color::rgb(0, 128, 0));
        // d: required → fondo amarillo.
        assert_eq!(box_by_id(&doc.box_tree, "d").unwrap().background, Some(super::Color::rgb(255, 255, 0)));
    }

    #[test]
    fn pseudo_nth_of_type_y_only_of_type_y_nth_last() {
        let html = r#"<html><head><style>
            p:nth-of-type(2) { color: red; }
            li:nth-last-child(1) { color: green; }
            span:only-of-type { color: blue; }
        </style></head><body>
            <div><span id="sp">x</span><p id="p1">1</p><p id="p2">2</p></div>
            <ul><li id="l1">a</li><li id="l2">b</li></ul>
        </body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        assert_eq!(box_by_id(&doc.box_tree, "p2").unwrap().color, super::Color::rgb(255, 0, 0));
        assert_ne!(box_by_id(&doc.box_tree, "p1").unwrap().color, super::Color::rgb(255, 0, 0));
        assert_eq!(box_by_id(&doc.box_tree, "l2").unwrap().color, super::Color::rgb(0, 128, 0));
        assert_ne!(box_by_id(&doc.box_tree, "l1").unwrap().color, super::Color::rgb(0, 128, 0));
        assert_eq!(box_by_id(&doc.box_tree, "sp").unwrap().color, super::Color::rgb(0, 0, 255));
    }

    #[test]
    fn sync_checked_y_restyle_actualiza_pseudo_checked() {
        // Fase 7.187 — togglear un checkbox actualiza el atributo `checked` y
        // recascadea: `:checked` y `:checked + label` aplican en vivo.
        let html = r#"<html><head><style>
            input:checked { background: red; }
            input:checked + label { color: blue; }
        </style></head><body>
            <input id="cb" type="checkbox"><label id="lb">L</label>
        </body></html>"#;
        let mut doc = Engine::new().load_html("about:test", html);
        let red = super::Color::rgb(255, 0, 0);
        let blue = super::Color::rgb(0, 0, 255);
        assert_ne!(box_by_id(&doc.box_tree, "cb").unwrap().background, Some(red));
        // Marcar (el checkbox es el control índice 0).
        doc.box_tree.sync_checked_from(&[true]);
        doc.box_tree.restyle();
        assert_eq!(box_by_id(&doc.box_tree, "cb").unwrap().background, Some(red));
        assert_eq!(box_by_id(&doc.box_tree, "lb").unwrap().color, blue);
        // Desmarcar revierte ambos.
        doc.box_tree.sync_checked_from(&[false]);
        doc.box_tree.restyle();
        assert_ne!(box_by_id(&doc.box_tree, "cb").unwrap().background, Some(red));
        assert_ne!(box_by_id(&doc.box_tree, "lb").unwrap().color, blue);
    }

    #[test]
    fn pseudo_is_y_where_matchean_lista() {
        let html = r#"<html><head><style>
            :is(h1, h2) { color: red; }
            .box :where(.a, .b) { background: green; }
            #x:is(.on, .off) { color: blue; }
        </style></head><body>
            <h2 id="h">t</h2>
            <div class="box"><span id="s" class="b">x</span></div>
            <p id="x" class="on">p</p>
        </body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        assert_eq!(box_by_id(&doc.box_tree, "h").unwrap().color, super::Color::rgb(255, 0, 0));
        assert_eq!(
            box_by_id(&doc.box_tree, "s").unwrap().background,
            Some(super::Color::rgb(0, 128, 0))
        );
        assert_eq!(box_by_id(&doc.box_tree, "x").unwrap().color, super::Color::rgb(0, 0, 255));
    }

    #[test]
    fn pseudo_where_no_aporta_especificidad() {
        // `:where(#hero)` tiene especificidad 0 → lo vence el selector de tag
        // `p` (que llega después y tiene especificidad 1). Si `:where` aportara
        // los 100 del `#id`, ganaría el rojo.
        let html = r#"<html><head><style>
            :where(#hero) { color: red; }
            p { color: green; }
        </style></head><body><p id="hero">x</p></body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        assert_eq!(box_by_id(&doc.box_tree, "hero").unwrap().color, super::Color::rgb(0, 128, 0));
    }

    #[test]
    fn shorthand_inset_y_flex_flow() {
        use crate::style::LengthVal;
        let html = r#"<html><head><style>
            #a { position: absolute; inset: 10px 20px; }
            #b { display: flex; flex-flow: column wrap; }
        </style></head><body>
            <div id="a">x</div><div id="b">y</div>
        </body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        let a = box_by_id(&doc.box_tree, "a").unwrap();
        // `inset: 10px 20px` → top/bottom=10, right/left=20.
        assert_eq!(a.inset_top, LengthVal::Px(10.0));
        assert_eq!(a.inset_right, LengthVal::Px(20.0));
        assert_eq!(a.inset_bottom, LengthVal::Px(10.0));
        assert_eq!(a.inset_left, LengthVal::Px(20.0));
        let b = box_by_id(&doc.box_tree, "b").unwrap();
        assert_eq!(b.flex_direction, super::FlexDirection::Column);
        assert_eq!(b.flex_wrap, super::FlexWrap::Wrap);
    }

    #[test]
    fn pseudo_not_con_lista() {
        // CSS4: `:not(.a, .b)` no matchea si el elemento tiene .a O .b.
        let html = r#"<html><head><style>
            li:not(.skip, .hidden) { color: red; }
        </style></head><body><ul>
            <li id="n1">uno</li>
            <li id="n2" class="skip">dos</li>
            <li id="n3" class="hidden">tres</li>
        </ul></body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        let red = super::Color::rgb(255, 0, 0);
        assert_eq!(box_by_id(&doc.box_tree, "n1").unwrap().color, red); // sin clases → rojo
        assert_ne!(box_by_id(&doc.box_tree, "n2").unwrap().color, red); // .skip → excluido
        assert_ne!(box_by_id(&doc.box_tree, "n3").unwrap().color, red); // .hidden → excluido
    }

    #[test]
    fn propiedades_logicas_de_caja() {
        let html = r#"<html><head><style>
            #a { margin-inline: 10px 20px; padding-block: 5px; }
            #b { margin-inline-start: 8px; padding-block-end: 12px; }
        </style></head><body><div id="a">x</div><div id="b">y</div></body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        let a = box_by_id(&doc.box_tree, "a").unwrap();
        // margin-inline: 10 20 → left=10 (start), right=20 (end), LTR.
        assert_eq!(a.margin.left, 10.0);
        assert_eq!(a.margin.right, 20.0);
        // padding-block: 5 → top=bottom=5.
        assert_eq!(a.padding.top, 5.0);
        assert_eq!(a.padding.bottom, 5.0);
        let b = box_by_id(&doc.box_tree, "b").unwrap();
        assert_eq!(b.margin.left, 8.0); // inline-start = left (LTR)
        assert_eq!(b.padding.bottom, 12.0); // block-end = bottom
    }

    #[test]
    fn list_style_none_suprime_marker() {
        let html = r#"<html><head><style>
            ul { list-style-type: none }
        </style></head><body><ul><li>uno</li><li>dos</li></ul></body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut has_bullet = false;
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.contains('•') {
                    has_bullet = true;
                }
            }
        });
        assert!(!has_bullet, "no debería haber marker con list-style-type:none");
    }

    #[test]
    fn ol_start_corre_el_contador() {
        let html =
            "<html><body><ol start=\"5\"><li>x</li><li>y</li></ol></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut markers = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.ends_with(". ") {
                    markers.push(t.clone());
                }
            }
        });
        assert_eq!(markers, vec!["5. ".to_string(), "6. ".into()]);
    }

    #[test]
    fn li_value_resetea_el_contador() {
        let html = "<html><body><ol><li>x</li><li value=\"10\">y</li><li>z</li></ol></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut markers = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.ends_with(". ") {
                    markers.push(t.clone());
                }
            }
        });
        assert_eq!(markers, vec!["1. ".to_string(), "10. ".into(), "11. ".into()]);
    }

    #[test]
    fn lower_roman_y_lower_alpha_aplican() {
        let html = r#"<html><head><style>
            .roman { list-style-type: lower-roman }
            .alpha { list-style-type: upper-alpha }
        </style></head><body>
          <ol class="roman"><li>a</li><li>b</li><li>c</li></ol>
          <ol class="alpha"><li>a</li><li>b</li></ol>
        </body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut markers = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.ends_with(". ") {
                    markers.push(t.clone());
                }
            }
        });
        // ol.roman → i. ii. iii.   ol.alpha → A. B.
        assert_eq!(
            markers,
            vec![
                "i. ".to_string(),
                "ii. ".into(),
                "iii. ".into(),
                "A. ".into(),
                "B. ".into(),
            ]
        );
    }

    #[test]
    fn to_alpha_y_to_roman_son_correctos() {
        use super::{to_alpha, to_roman};
        assert_eq!(to_alpha(1, false), "a");
        assert_eq!(to_alpha(26, false), "z");
        assert_eq!(to_alpha(27, false), "aa");
        assert_eq!(to_alpha(28, false), "ab");
        assert_eq!(to_alpha(52, true), "AZ");
        assert_eq!(to_roman(4, false), "iv");
        assert_eq!(to_roman(9, true), "IX");
        assert_eq!(to_roman(1994, false), "mcmxciv");
        assert_eq!(to_roman(3999, true), "MMMCMXCIX");
        // Fuera de rango → decimal fallback.
        assert_eq!(to_roman(4000, false), "4000");
        assert_eq!(to_roman(0, true), "0");
    }

    #[test]
    fn estilo_inline_aplica_color() {
        let html = r#"<html><body><p style="color: #ff0000">x</p></body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found_red = false;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("p") && b.color == super::Color::rgb(255, 0, 0) {
                found_red = true;
            }
        });
        assert!(found_red, "no se encontró <p> con color rojo");
    }

    #[test]
    fn link_stylesheet_externo_data_url_aplica() {
        // `<link rel="stylesheet" href="data:text/css,...">` — la hoja externa
        // se baja (acá vía data:, sin red) y sus reglas entran a la cascada.
        let html = r##"<html><head>
            <link rel="stylesheet" href="data:text/css,p%7Bcolor%3A%23008000%7D">
        </head><body><p>verde</p></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found = false;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("p") && b.color == super::Color::rgb(0, 128, 0) {
                found = true;
            }
        });
        assert!(found, "la regla de la hoja externa data: no se aplicó al <p>");
    }

    #[test]
    fn link_relativo_resuelve_contra_base_href() {
        // `<base href="file://<dir>/">` + `<link href="x.css">` relativo debe
        // bajar `<dir>/x.css` (no contra la URL del documento). file:// = sin red.
        let mut dir = std::env::temp_dir();
        dir.push(format!("puriy_basehref_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.css"), "p { color: #00ff00 }").unwrap();
        let base = format!("file://{}/", dir.display());
        let html = format!(
            r##"<html><head><base href="{base}"><link rel="stylesheet" href="x.css"></head><body><p>v</p></body></html>"##
        );
        let eng = Engine::new();
        let doc = eng.load_html("about:test", &html);
        let mut found = false;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("p") && b.color == super::Color::rgb(0, 255, 0) {
                found = true;
            }
        });
        let _ = std::fs::remove_dir_all(&dir);
        assert!(found, "el <link> relativo no resolvió contra <base href>");
    }

    #[test]
    fn import_en_style_inline_se_sigue() {
        // `@import` de un data: CSS dentro de un <style> — sus reglas aplican.
        let html = r##"<html><head><style>
            @import url("data:text/css,p%7Bcolor%3A%23ff0000%7D");
        </style></head><body><p>x</p></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found = false;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("p") && b.color == super::Color::rgb(255, 0, 0) {
                found = true;
            }
        });
        assert!(found, "la regla del @import no se aplicó");
    }

    #[test]
    fn import_precede_a_las_reglas_propias_en_cascada() {
        // @import pone rojo; la regla propia (después) lo pisa a azul → azul.
        let html = r##"<html><head><style>
            @import url("data:text/css,p%7Bcolor%3Ared%7D");
            p { color: #0000ff }
        </style></head><body><p>x</p></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut p_color = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("p") {
                p_color = Some(b.color);
            }
        });
        assert_eq!(p_color, Some(super::Color::rgb(0, 0, 255)), "la regla propia debe ganar al @import");
    }

    #[test]
    fn link_media_print_no_aplica_en_pantalla() {
        // `<link media="print">` no debe aplicar al render de pantalla; la
        // misma regla con `media="screen"` sí. DEFAULT_VIEWPORT es screen.
        let print = r##"<html><head>
            <link rel="stylesheet" href="data:text/css,p%7Bcolor%3Ared%7D" media="print">
        </head><body><p>x</p></body></html>"##;
        let screen = r##"<html><head>
            <link rel="stylesheet" href="data:text/css,p%7Bcolor%3Ared%7D" media="screen">
        </head><body><p>x</p></body></html>"##;
        let eng = Engine::new();
        let red = super::Color::rgb(255, 0, 0);
        let color_of = |html: &str| {
            let doc = eng.load_html("about:test", html);
            let mut c = None;
            doc.box_tree.walk(|b| {
                if b.tag.as_deref() == Some("p") {
                    c = Some(b.color);
                }
            });
            c
        };
        assert_ne!(color_of(print), Some(red), "media=print no debía aplicar en pantalla");
        assert_eq!(color_of(screen), Some(red), "media=screen sí debía aplicar");
    }

    #[test]
    fn link_stylesheet_cascada_respeta_orden_de_documento() {
        // Hoja externa (data:) declara color rojo; un `<style>` posterior lo
        // pisa a azul — el orden de documento debe ganar (azul), no el externo.
        let html = r##"<html><head>
            <link rel="stylesheet" href="data:text/css,p%7Bcolor%3Ared%7D">
            <style>p { color: #0000ff }</style>
        </head><body><p>azul</p></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut p_color = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("p") {
                p_color = Some(b.color);
            }
        });
        assert_eq!(p_color, Some(super::Color::rgb(0, 0, 255)), "el <style> posterior debe ganar");
    }

    #[test]
    fn details_sin_open_attr_arranca_cerrado() {
        let html = r#"<html><body>
            <details><summary>Tit</summary><p>Contenido</p></details>
        </body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut details_attr: Vec<bool> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("details") {
                details_attr.push(b.details_open_attr);
            }
        });
        assert_eq!(details_attr, vec![false]);
    }

    #[test]
    fn details_con_open_attr_lo_refleja() {
        let html = r#"<html><body>
            <details open><summary>Tit</summary><p>Contenido</p></details>
        </body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut details_attr: Vec<bool> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("details") {
                details_attr.push(b.details_open_attr);
            }
        });
        assert_eq!(details_attr, vec![true]);
    }

    #[test]
    fn details_summary_se_parsean_como_tags() {
        let html = r#"<html><body>
            <details><summary>Tit</summary><p>Contenido</p></details>
        </body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut saw_details = false;
        let mut saw_summary = false;
        doc.box_tree.walk(|b| {
            match b.tag.as_deref() {
                Some("details") => saw_details = true,
                Some("summary") => saw_summary = true,
                _ => {}
            }
        });
        assert!(saw_details, "no se encontró <details> en el box tree");
        assert!(saw_summary, "no se encontró <summary> en el box tree");
    }

    #[test]
    fn details_open_attr_es_false_para_nodos_no_details() {
        let html = "<html><body><p>x</p><h1>y</h1></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() != Some("details") {
                assert!(!b.details_open_attr, "{:?} no debería tener details_open_attr=true", b.tag);
            }
        });
    }

    #[test]
    fn ws_entre_blocks_se_filtra() {
        // El "\n  " entre </h1> y <p> produce un Text node " " que NO
        // debería rendear como un row vacío.
        let html = "<html><body><h1>A</h1>\n  <p>B</p>\n  <h2>C</h2></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        // Walk del body. Esperamos sólo h1, p, h2 como children directos
        // (sin text-leaves de whitespace entre ellos).
        let body = &doc.box_tree.root;
        // Body envuelve un Inline de transición (collapse_whitespace puede
        // dejar uno leading o trailing). Recorremos directamente.
        let mut top_tags: Vec<Option<String>> = body
            .children
            .iter()
            .filter(|c| !super::is_ws_only_inline(c))
            .map(|c| c.tag.clone())
            .collect();
        // Aseguramos que el filtrado sólo dejó tags reales.
        top_tags.retain(|t| t.is_some());
        let names: Vec<&str> = top_tags
            .iter()
            .map(|t| t.as_deref().unwrap_or(""))
            .collect();
        assert_eq!(names, vec!["h1", "p", "h2"]);
        // Y verificamos que NO hay inlines whitespace-only entre ellos en
        // el árbol real (post-strip).
        for c in &body.children {
            assert!(
                !super::is_ws_only_inline(c),
                "el body no debería tener inlines ws-only entre blocks: {:?}",
                c.text
            );
        }
    }

    #[test]
    fn ws_alrededor_de_inline_se_preserva() {
        // El espacio entre "foo " y <strong>bar</strong> y " baz" sí
        // tiene valor — debe quedarse para no pegar tokens.
        let html = "<html><body><p>foo <strong>bar</strong> baz</p></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        // Encontramos el <p> y verificamos que sus children contengan
        // textos con espacios donde corresponde.
        let mut texts: Vec<String> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("p") {
                for c in &b.children {
                    if let Some(t) = &c.text {
                        texts.push(t.clone());
                    }
                    // Si es <strong>, mirá su hijo
                    if c.tag.as_deref() == Some("strong") {
                        for cc in &c.children {
                            if let Some(t) = &cc.text {
                                texts.push(format!("[strong]{t}"));
                            }
                        }
                    }
                }
            }
        });
        // Esperamos que "foo " conserve el espacio trailing y " baz" el leading.
        assert!(
            texts.iter().any(|t| t.ends_with(' ')),
            "esperaba un text con espacio trailing en {:?}",
            texts
        );
        assert!(
            texts.iter().any(|t| t.starts_with(' ')),
            "esperaba un text con espacio leading en {:?}",
            texts
        );
        assert!(
            texts.iter().any(|t| t == "[strong]bar"),
            "esperaba `bar` dentro de strong en {:?}",
            texts
        );
    }

    #[test]
    fn link_target_blank_marca_link_new_tab() {
        let html = r#"<html><body>
            <a href="https://a.test/" target="_blank">A</a>
            <a href="https://b.test/">B</a>
            <a href="https://c.test/" target="_self">C</a>
        </body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut links: Vec<(String, bool)> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("a") {
                if let Some(target) = &b.link {
                    links.push((target.clone(), b.link_new_tab));
                }
            }
        });
        assert!(links.iter().any(|(u, nt)| u.contains("a.test") && *nt));
        assert!(links.iter().any(|(u, nt)| u.contains("b.test") && !*nt));
        assert!(links.iter().any(|(u, nt)| u.contains("c.test") && !*nt));
    }

    #[test]
    fn link_mailto_y_tel_y_javascript_se_ignoran() {
        let html = r#"<html><body>
            <a href="mailto:foo@bar">M</a>
            <a href="tel:+541112345678">T</a>
            <a href="javascript:alert(1)">J</a>
            <a href="data:text/plain,hi">D</a>
            <a href="ftp://example.com/">F</a>
        </body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut clickable: Vec<String> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("a") {
                if let Some(t) = &b.link {
                    clickable.push(t.clone());
                }
            }
        });
        assert!(clickable.is_empty(), "ningún href no-web debería ser clickable: {clickable:?}");
    }

    #[test]
    fn srcset_elige_la_densidad_mas_alta() {
        let url = super::pick_srcset("foo.png 1x, foo-2x.png 2x, foo-3x.png 3x");
        assert_eq!(url.as_deref(), Some("foo-3x.png"));
    }

    #[test]
    fn srcset_elige_el_ancho_mas_grande() {
        let url = super::pick_srcset("a.png 320w, b.png 800w, c.png 1600w");
        assert_eq!(url.as_deref(), Some("c.png"));
    }

    #[test]
    fn srcset_sin_descriptor_usa_la_primera_con_1x_implicito() {
        // En la práctica un srcset sin descriptor es equivalente a 1x.
        let url = super::pick_srcset("a.png, b.png");
        // No importa el orden interno — basta con que devuelva alguno.
        assert!(url.is_some());
    }

    #[test]
    fn svg_parsea_polygon_y_polyline() {
        let html = r##"<html><body>
            <svg width="100" height="100">
                <polygon points="0,0 50,0 50,50" fill="red"/>
                <polyline points="0,100 100,50 100,0" stroke="blue"/>
            </svg>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut prim_count = 0;
        let mut had_closed = false;
        let mut had_open = false;
        doc.box_tree.walk(|b| {
            if let Some(s) = &b.svg {
                for p in &s.prims {
                    if let crate::SvgPrim::Polyline { points, closed, .. } = p {
                        prim_count += 1;
                        if *closed {
                            had_closed = true;
                            assert_eq!(points.len(), 3);
                        } else {
                            had_open = true;
                        }
                    }
                }
            }
        });
        assert_eq!(prim_count, 2);
        assert!(had_closed);
        assert!(had_open);
    }

    #[test]
    fn svg_parsea_path_minimal() {
        let html = r##"<html><body>
            <svg width="100" height="100">
                <path d="M 10 10 L 90 10 L 50 90 Z" fill="green"/>
            </svg>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut cmds_count = 0;
        doc.box_tree.walk(|b| {
            if let Some(s) = &b.svg {
                for p in &s.prims {
                    if let crate::SvgPrim::Path { d, .. } = p {
                        cmds_count = d.len();
                    }
                }
            }
        });
        // M, L, L, Z → 4 cmds.
        assert_eq!(cmds_count, 4);
    }

    #[test]
    fn svg_recolecta_rect_circle_y_line() {
        let html = r##"<html><body>
            <svg width="200" height="100" viewBox="0 0 200 100">
                <rect x="10" y="10" width="50" height="30" fill="red" stroke="black" stroke-width="2"/>
                <circle cx="120" cy="50" r="20" fill="blue"/>
                <line x1="0" y1="0" x2="200" y2="100" stroke="green" stroke-width="3"/>
            </svg>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut scene: Option<crate::SvgScene> = None;
        doc.box_tree.walk(|b| {
            if let Some(s) = &b.svg {
                scene = Some(s.clone());
            }
        });
        let scene = scene.expect("debería haber un <svg>");
        assert_eq!(scene.width, 200.0);
        assert_eq!(scene.height, 100.0);
        assert_eq!(scene.view_box, Some((0.0, 0.0, 200.0, 100.0)));
        assert_eq!(scene.prims.len(), 3);
        match &scene.prims[0] {
            crate::SvgPrim::Rect { x, y, w, h, fill, stroke, .. } => {
                assert_eq!(*x, 10.0);
                assert_eq!(*y, 10.0);
                assert_eq!(*w, 50.0);
                assert_eq!(*h, 30.0);
                assert!(fill.is_some());
                assert!(stroke.is_some());
            }
            _ => panic!("primera prim debería ser Rect"),
        }
        match &scene.prims[1] {
            crate::SvgPrim::Circle { cx, cy, r, .. } => {
                assert_eq!(*cx, 120.0);
                assert_eq!(*cy, 50.0);
                assert_eq!(*r, 20.0);
            }
            _ => panic!("segunda prim debería ser Circle"),
        }
        match &scene.prims[2] {
            crate::SvgPrim::Line { x1, y2, .. } => {
                assert_eq!(*x1, 0.0);
                assert_eq!(*y2, 100.0);
            }
            _ => panic!("tercera prim debería ser Line"),
        }
    }

    #[test]
    fn select_recolecta_options_y_seleccionado_inicial() {
        let html = r##"<html><body>
            <form action="/p">
                <select name="lang">
                    <option value="es">Español</option>
                    <option value="en" selected>English</option>
                    <option>Otro</option>
                </select>
            </form>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("https://example.com/", html);
        let mut info: Option<crate::SelectInfo> = None;
        doc.box_tree.walk(|b| {
            if let Some(s) = &b.select {
                info = Some(s.clone());
                assert_eq!(b.input_name.as_deref(), Some("lang"));
                assert_eq!(b.form_idx, Some(0));
            }
        });
        let info = info.expect("debería haber un <select>");
        assert_eq!(info.options.len(), 3);
        assert_eq!(info.options[0].value, "es");
        assert_eq!(info.options[0].label, "Español");
        assert_eq!(info.options[2].label, "Otro");
        assert_eq!(info.options[2].value, "Otro"); // fallback al label
        assert_eq!(info.initial, 1); // <option selected> es el segundo
    }

    #[test]
    fn form_asigna_form_idx_a_inputs_que_contiene() {
        let html = r##"<html><body>
            <form action="/search" method="get">
                <input type="text" name="q" value="hola">
                <input type="text" name="lang" value="es">
            </form>
            <input type="text" name="outside">
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("https://example.com/", html);
        assert_eq!(doc.box_tree.forms.len(), 1);
        let mut names_inside: Vec<String> = Vec::new();
        let mut outside_form_idx: Option<usize> = None;
        doc.box_tree.walk(|b| {
            if let Some(name) = &b.input_name {
                if b.form_idx == Some(0) {
                    names_inside.push(name.clone());
                } else if b.input_kind.is_some() && name == "outside" {
                    outside_form_idx = b.form_idx;
                }
            }
        });
        assert_eq!(names_inside, vec!["q".to_string(), "lang".into()]);
        assert_eq!(outside_form_idx, None);
        assert_eq!(
            doc.box_tree.forms[0].action.as_deref(),
            Some("https://example.com/search")
        );
    }

    #[test]
    fn em_y_i_y_cite_son_italic_por_default() {
        let html = "<html><body><em>a</em><i>b</i><cite>c</cite><p>d</p></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found: Vec<(String, crate::FontStyle)> = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(tag) = &b.tag {
                if matches!(tag.as_str(), "em" | "i" | "cite" | "p") {
                    found.push((tag.clone(), b.font_style));
                }
            }
        });
        let em = found.iter().find(|(t, _)| t == "em").unwrap();
        let i = found.iter().find(|(t, _)| t == "i").unwrap();
        let cite = found.iter().find(|(t, _)| t == "cite").unwrap();
        let p = found.iter().find(|(t, _)| t == "p").unwrap();
        assert_eq!(em.1, crate::FontStyle::Italic);
        assert_eq!(i.1, crate::FontStyle::Italic);
        assert_eq!(cite.1, crate::FontStyle::Italic);
        assert_eq!(p.1, crate::FontStyle::Normal);
    }

    #[test]
    fn font_style_normal_override_padre_italic() {
        let html = r##"<html><head><style>
            .x { font-style: normal }
        </style></head><body><em>fuera<span class="x">dentro</span></em></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut span_style: Option<crate::FontStyle> = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("span") {
                span_style = Some(b.font_style);
            }
        });
        assert_eq!(span_style, Some(crate::FontStyle::Normal));
    }

    #[test]
    fn focus_pseudo_aporta_a_focus_background() {
        use crate::StyleEngine;
        let html = r##"<html><head><style>
            input { background: white }
            input:focus { background: #ffeecc }
        </style></head><body><input type="text"></body></html>"##;
        let dom = crate::DomTree::parse(html);
        let styles = StyleEngine::from_dom(&dom);
        let input = dom.find("input").unwrap();
        let base = styles.compute_with_parent_for_state(&input, None, false, false);
        let focused = styles.compute_with_parent_for_state(&input, None, false, true);
        // base es blanco (255,255,255), focused es #ffeecc (255,238,204).
        assert_eq!(base.background.map(|c| (c.r, c.g, c.b)), Some((255, 255, 255)));
        assert_eq!(focused.background.map(|c| (c.r, c.g, c.b)), Some((255, 238, 204)));
    }

    #[test]
    fn box_tree_expone_focus_background() {
        let html = r##"<html><head><style>
            input:focus { background: #abcdef }
        </style></head><body><input type="text"></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found = false;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("input") {
                assert_eq!(
                    b.focus_background.map(|c| (c.r, c.g, c.b)),
                    Some((0xab, 0xcd, 0xef))
                );
                found = true;
            }
        });
        assert!(found, "no se encontró <input> en el box tree");
    }

    #[test]
    fn parsea_background_image_url_a_computed_style_y_no_descarga_si_url_no_resuelve() {
        // Sin red, fetch_and_decode falla y background_image queda None.
        // Pero el url SÍ debe quedar capturado en computed.background_image_url
        // (visible al re-parsear el stylesheet).
        use crate::StyleEngine;
        let html = r##"<html><head><style>
            .hero { background-image: url("https://nope.invalid/bg.png") }
        </style></head><body><div class="hero">x</div></body></html>"##;
        let dom = crate::DomTree::parse(html);
        let styles = StyleEngine::from_dom(&dom);
        let div = dom.find("div").expect("debería encontrar <div>");
        let s = styles.compute_with_parent(&div, None);
        assert_eq!(
            s.background_image_url.as_deref(),
            Some("https://nope.invalid/bg.png")
        );
    }

    #[test]
    fn background_image_none_limpia_url() {
        use crate::StyleEngine;
        let html = r##"<html><head><style>
            .hero { background-image: url(a.png) }
            .hero.off { background-image: none }
        </style></head><body><div class="hero off">x</div></body></html>"##;
        let dom = crate::DomTree::parse(html);
        let styles = StyleEngine::from_dom(&dom);
        let div = dom.find("div").expect("debería encontrar <div>");
        let s = styles.compute_with_parent(&div, None);
        assert!(s.background_image_url.is_none());
    }

    #[test]
    fn link_fragmento_se_resuelve_a_base_mas_frag() {
        // Antes: `#top` se ignoraba (None). Ahora resuelve contra la
        // base — el chrome detecta same-page y scrollea en lugar de
        // recargar la URL.
        let html = r##"<html><body><a href="#top">arriba</a></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("https://example.com/doc", html);
        let mut links: Vec<String> = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(l) = &b.link {
                links.push(l.clone());
            }
        });
        assert_eq!(links, vec!["https://example.com/doc#top".to_string()]);
    }

    #[test]
    fn iframe_se_renderea_como_placeholder_con_url() {
        let html = r##"<html><body>
            <iframe src="https://embed.example.com/video"></iframe>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found: Option<String> = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("iframe") {
                if let Some(first) = b.children.first() {
                    found = first.text.clone();
                }
            }
        });
        assert_eq!(
            found.as_deref(),
            Some("[iframe: https://embed.example.com/video]")
        );
    }

    #[test]
    fn iframe_sin_src_muestra_label_generico() {
        let html = "<html><body><iframe></iframe></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found: Option<String> = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("iframe") {
                found = b.children.first().and_then(|c| c.text.clone());
            }
        });
        assert_eq!(found.as_deref(), Some("[iframe sin src]"));
    }

    #[test]
    fn content_url_parser_acepta_quoted_y_unquoted() {
        use crate::ContentItem;
        let html = r##"<html><head><style>
            .a::before { content: url("https://x/y.png") }
            .b::before { content: url(https://x/z.png) }
        </style></head><body>
            <p class="a"></p>
            <p class="b"></p>
        </body></html>"##;
        let dom = crate::DomTree::parse(html);
        let eng = crate::StyleEngine::from_dom(&dom);
        let ps_a = dom.find("p").unwrap();
        let before = eng.compute_pseudo(&ps_a, crate::PseudoElement::Before, None);
        assert_eq!(
            before.and_then(|s| s.content),
            Some(vec![ContentItem::Url("https://x/y.png".into())])
        );
    }

    #[test]
    fn margin_collapse_padre_promueve_margin_del_primer_hijo() {
        // <body style="margin: 8px"> con primer hijo
        // <div style="margin: 20px 0 0 0">: el body no tiene padding/
        // border arriba, así que el margin_top del div se promueve al
        // body. Final: body.margin.top = max(8, 20) = 20; div.margin.top = 0.
        let html = r##"<html><body style="margin: 8px">
            <div style="margin: 20px 0 0 0">x</div>
            <div style="margin: 0 0 12px 0">y</div>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        // body es el root del box tree (BoxTree.root viene de
        // dom.find("body")).
        assert_eq!(doc.box_tree.root.tag.as_deref(), Some("body"));
        assert_eq!(doc.box_tree.root.margin.top, 20.0);
        assert_eq!(doc.box_tree.root.margin.bottom, 12.0);
        // El primer hijo div quedó con margin.top = 0 (promovido).
        let first_div = &doc.box_tree.root.children[0];
        assert_eq!(first_div.margin.top, 0.0);
        // El último div: margin.bottom promovido al body.
        let last_div = doc.box_tree.root.children.last().unwrap();
        assert_eq!(last_div.margin.bottom, 0.0);
    }

    #[test]
    fn margin_collapse_padre_bloqueado_por_padding() {
        // Si el body tiene padding-top, el margin del primer hijo NO
        // colapsa contra el body — el padding es la "barrera".
        let html = r##"<html><body style="margin: 8px; padding: 10px 0 0 0">
            <div style="margin: 20px 0 0 0">x</div>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        assert_eq!(doc.box_tree.root.margin.top, 8.0);
        let first_div = &doc.box_tree.root.children[0];
        assert_eq!(first_div.margin.top, 20.0);
    }

    #[test]
    fn margin_collapsing_max_entre_block_siblings() {
        // `<h2 style="margin: 0 0 20px 0">` seguido de `<p style="margin: 10px 0 0 0">`:
        // gap esperado es max(20, 10) = 20. El margin_bottom del h2
        // queda intacto (20), el margin_top del p baja a 0.
        let html = r##"<html><body>
            <h2 style="margin: 0 0 20px 0">Heading</h2>
            <p style="margin: 10px 0 0 0">Para</p>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut h2_margin_bottom: Option<f32> = None;
        let mut p_margin_top: Option<f32> = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("h2") {
                h2_margin_bottom = Some(b.margin.bottom);
            }
            if b.tag.as_deref() == Some("p") {
                p_margin_top = Some(b.margin.top);
            }
        });
        assert_eq!(h2_margin_bottom, Some(20.0));
        // 10 - min(20, 10) = 10 - 10 = 0. Gap total = 20 + 0 = 20 = max.
        assert_eq!(p_margin_top, Some(0.0));
    }

    #[test]
    fn margin_collapsing_no_aplica_a_inline() {
        // Block + inline no colapsan — el inline vive en otro flow.
        let html = r##"<html><body>
            <p style="margin: 0 0 10px 0">Para</p>
            <span style="margin: 5px 0 0 0">inline</span>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut span_margin_top: Option<f32> = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("span") {
                span_margin_top = Some(b.margin.top);
            }
        });
        // No tocado.
        assert_eq!(span_margin_top, Some(5.0));
    }

    #[test]
    fn prefetch_no_crashea_sin_imagenes() {
        // Sanity: páginas sin imágenes no deben fallar el prefetch.
        let html = "<html><body><p>solo texto</p></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        // Si llegó acá sin panic, OK.
        assert!(doc.box_tree.descendants_count() > 0);
    }

    #[test]
    fn prefetch_skip_de_urls_no_http() {
        // URLs `about:`/`file:`/`data:` no deben encolarse al pool —
        // sería un round-trip al timeout para nada. El test pone una
        // base `about:test` con `<img src="...">` que resuelve a
        // about:... y verifica que la carga termina rápido (sin
        // esperar timeouts de red).
        let html = r##"<html><body><img src="x.png"></body></html>"##;
        let eng = Engine::new();
        let t0 = std::time::Instant::now();
        let _ = eng.load_html("about:test", html);
        let elapsed = t0.elapsed();
        assert!(
            elapsed.as_millis() < 500,
            "load_html con base about: y un <img> debería ser instantáneo, fue {elapsed:?}"
        );
    }

    #[test]
    fn img_data_url_se_decodifica_inline() {
        // `<img src="data:image/png;base64,...">` con un PNG 1×1 (un pixel rojo).
        // `resolve_href` bloquea data: (no navegable), pero como fuente de
        // imagen `fetch_image_src` lo decodifica sin tocar la red.
        let png_1x1 = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";
        let html = format!(r##"<html><body><img src="{png_1x1}"></body></html>"##);
        let eng = Engine::new();
        let doc = eng.load_html("about:test", &html);
        let mut img_dims: Option<(u32, u32)> = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("img") {
                if let Some(img) = &b.image {
                    img_dims = Some((img.width, img.height));
                }
            }
        });
        assert_eq!(img_dims, Some((1, 1)), "el PNG data: debería decodificar a 1×1");
    }

    #[test]
    fn counter_numera_h2_sequencialmente() {
        // Patrón clásico: body resetea el contador a 0, cada h2::before
        // lo incrementa y muestra el valor — h2 numerados 1, 2, 3.
        let html = r##"<html><head><style>
            body { counter-reset: sec }
            h2::before { counter-increment: sec; content: counter(sec) ". " }
        </style></head><body>
            <h2>Intro</h2>
            <h2>Cuerpo</h2>
            <h2>Cierre</h2>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        // Recolectamos el primer text leaf de cada h2 (el ::before).
        let mut h2_prefixes: Vec<String> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("h2") {
                if let Some(first) = b.children.first() {
                    if let Some(t) = &first.text {
                        h2_prefixes.push(t.clone());
                    }
                }
            }
        });
        assert_eq!(h2_prefixes, vec!["1. ", "2. ", "3. "]);
    }

    #[test]
    fn attr_en_content_lee_del_padre_del_pseudo() {
        // `<a data-tag="X">` con `a::after { content: " [" attr(data-tag) "]" }`
        // debe inyectar " [X]" después del texto del link.
        let html = r##"<html><head><style>
            a::after { content: " [" attr(data-tag) "]" }
        </style></head><body>
            <a href="#" data-tag="ALPHA">link</a>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut a_children: Vec<String> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("a") && a_children.is_empty() {
                a_children = b
                    .children
                    .iter()
                    .filter_map(|c| c.text.clone())
                    .collect();
            }
        });
        assert_eq!(a_children, vec!["link".to_string(), " [ALPHA]".to_string()]);
    }

    #[test]
    fn before_y_after_se_inyectan_como_children() {
        let html = r##"<html><head><style>
            .badge::before { content: "▸ " }
            .badge::after  { content: " !" }
        </style></head><body>
            <p class="badge">Hola</p>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        // El `<p>` tiene 3 hijos: el ::before, el text leaf "Hola", el ::after.
        let mut p_children: Option<Vec<String>> = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("p") && p_children.is_none() {
                p_children = Some(
                    b.children
                        .iter()
                        .filter_map(|c| c.text.clone())
                        .collect(),
                );
            }
        });
        let texts = p_children.expect("debería encontrar <p>");
        assert_eq!(texts, vec!["▸ ".to_string(), "Hola".to_string(), " !".to_string()]);
    }

    #[test]
    fn find_y_of_match_devuelve_y_creciente_por_match() {
        let html = r##"<html><body>
            <p>alfa</p><p>beta</p><p>alfa beta</p><p>alfa</p>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let bt = &doc.box_tree;
        let y1 = bt.find_y_of_match("alfa", 1).expect("match 1");
        let y2 = bt.find_y_of_match("alfa", 2).expect("match 2");
        let y3 = bt.find_y_of_match("alfa", 3).expect("match 3");
        assert!(y2 > y1, "match 2 debe quedar más abajo que match 1");
        assert!(y3 > y2);
        // Sin match para el 4to.
        assert!(bt.find_y_of_match("alfa", 4).is_none());
        // Query vacía o nth=0 devuelven None.
        assert!(bt.find_y_of_match("", 1).is_none());
        assert!(bt.find_y_of_match("alfa", 0).is_none());
    }

    #[test]
    fn input_autofocus_se_marca_solo_para_inputs_con_attr() {
        let html = r##"<html><body>
            <form>
                <input type="text" name="a">
                <input type="text" name="b" autofocus>
                <input type="text" name="c" autofocus>
            </form>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut flags: Vec<bool> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.input_kind.is_some() {
                flags.push(b.input_autofocus);
            }
        });
        assert_eq!(flags, vec![false, true, true]);
    }

    #[test]
    fn element_id_se_extrae_del_attr() {
        let html = r##"<html><body>
            <h2 id="intro">Intro</h2>
            <p id="">vacío no cuenta</p>
            <p>sin id</p>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut ids: Vec<String> = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(id) = &b.element_id {
                ids.push(id.clone());
            }
        });
        assert_eq!(ids, vec!["intro".to_string()]);
    }

    #[test]
    fn ws_solo_inline_no_se_dropea_si_padre_es_inline_flow() {
        // <p>foo<span> </span>bar</p> — el espacio dentro de span sí debe
        // quedar porque separa "foo" de "bar".
        let html = "<html><body><p>foo<span> </span>bar</p></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found_space = false;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("span") {
                for c in &b.children {
                    if c.text.as_deref().map(|s| s.contains(' ')).unwrap_or(false) {
                        found_space = true;
                    }
                }
            }
        });
        assert!(found_space, "el espacio dentro de <span> debería preservarse");
    }

    #[test]
    fn set_element_text_content_reemplaza_hoja() {
        let html = r#"<html><body><h1 id="hero">Hola</h1></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        let ok = doc.box_tree.set_element_text_content("hero", "Adiós");
        assert!(ok);
        // Verificar que la hoja de texto se actualizó.
        let mut found = false;
        doc.box_tree.walk(|b| {
            if b.text.as_deref() == Some("Adiós") {
                found = true;
            }
        });
        assert!(found, "no se encontró 'Adiós' en el árbol post-mutación");
    }

    #[test]
    fn set_element_text_content_no_encuentra_id_devuelve_false() {
        let html = r#"<html><body><p>x</p></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        let ok = doc.box_tree.set_element_text_content("fantasma", "x");
        assert!(!ok);
    }

    #[test]
    fn set_element_style_color_actualiza_text_leaves() {
        let html = r#"<html><body><h1 id="h">hola</h1></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        let ok = doc.box_tree.set_element_style("h", "color", "red");
        assert!(ok);
        // El leaf de texto debe haber heredado el color rojo.
        let mut color_changed = false;
        doc.box_tree.walk(|b| {
            if b.text.as_deref() == Some("hola") {
                if b.color.r == 255 && b.color.g == 0 && b.color.b == 0 {
                    color_changed = true;
                }
            }
        });
        assert!(color_changed);
    }

    #[test]
    fn set_element_style_background_hex() {
        let html = r#"<html><body><div id="d">x</div></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        assert!(doc.box_tree.set_element_style("d", "background", "#abc"));
        let mut bg_set = false;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("d") {
                if let Some(c) = b.background {
                    if c.r == 0xaa && c.g == 0xbb && c.b == 0xcc {
                        bg_set = true;
                    }
                }
            }
        });
        assert!(bg_set);
    }

    #[test]
    fn set_element_style_display_none_oculta() {
        let html = r#"<html><body><div id="d">x</div></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        assert!(doc.box_tree.set_element_style("d", "display", "none"));
        let mut hidden = false;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("d") {
                if matches!(b.display, Display::None) {
                    hidden = true;
                }
            }
        });
        assert!(hidden);
    }

    #[test]
    fn set_element_style_prop_desconocida_devuelve_false() {
        let html = r#"<html><body><div id="d">x</div></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        assert!(!doc.box_tree.set_element_style("d", "transform", "rotate(45deg)"));
    }

    #[test]
    fn set_element_style_id_inexistente_devuelve_false() {
        let html = r#"<html><body><p>x</p></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        assert!(!doc.box_tree.set_element_style("fantasma", "color", "red"));
    }

    // ============= Fase 7.16 — attributes genéricos =============

    #[test]
    fn box_node_attributes_contiene_todos_los_attrs_html() {
        let html = r#"<html><body><a id="x" href="https://gioser.net" aria-current="page" data-track="hero" rel="noopener">x</a></body></html>"#;
        let doc = Engine::new().load_html("about:t", html);
        let mut found: Option<Vec<(String, String)>> = None;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("x") {
                found = Some(b.attributes.clone());
            }
        });
        let attrs = found.expect("a#x existe");
        // Todos los attrs aparecen, lowercase names, values literales.
        assert!(attrs.iter().any(|(k, v)| k == "href" && v == "https://gioser.net"));
        assert!(attrs.iter().any(|(k, v)| k == "aria-current" && v == "page"));
        assert!(attrs.iter().any(|(k, v)| k == "data-track" && v == "hero"));
        assert!(attrs.iter().any(|(k, v)| k == "rel" && v == "noopener"));
        // El attr id también aparece — no se filtra (el getAttribute('id')
        // resuelve por la rama especial del JS, pero el campo se mantiene
        // uniforme para evitar ramas adicionales en el chrome).
        assert!(attrs.iter().any(|(k, v)| k == "id" && v == "x"));
    }

    #[test]
    fn box_node_dataset_filter_view_devuelve_solo_data_attrs() {
        let html = r##"<html><body><div id="x" data-foo="1" aria-label="hi" data-bar-baz="2" href="#">y</div></body></html>"##;
        let doc = Engine::new().load_html("about:t", html);
        let mut found: Option<Vec<(String, String)>> = None;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("x") {
                found = Some(b.dataset().into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect());
            }
        });
        let ds = found.expect("div#x existe");
        assert_eq!(ds.len(), 2);
        assert!(ds.iter().any(|(k, v)| k == "foo" && v == "1"));
        assert!(ds.iter().any(|(k, v)| k == "bar-baz" && v == "2"));
    }

    #[test]
    fn set_element_attribute_agrega_attr_nuevo() {
        let html = r#"<html><body><div id="x">y</div></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        assert!(doc.box_tree.set_element_attribute("x", "aria-current", "step"));
        let mut found = false;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("x")
                && b.attributes.iter().any(|(k, v)| k == "aria-current" && v == "step")
            {
                found = true;
            }
        });
        assert!(found);
    }

    #[test]
    fn set_element_attribute_reemplaza_attr_existente() {
        let html = r#"<html><body><a id="x" href="/old">y</a></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        assert!(doc.box_tree.set_element_attribute("x", "href", "/nuevo"));
        let mut count_href = 0;
        let mut val = String::new();
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("x") {
                for (k, v) in &b.attributes {
                    if k == "href" {
                        count_href += 1;
                        val = v.clone();
                    }
                }
            }
        });
        assert_eq!(count_href, 1, "href no debe duplicarse al reemplazar");
        assert_eq!(val, "/nuevo");
    }

    #[test]
    fn remove_element_attribute_quita_la_key() {
        let html = r#"<html><body><a id="x" href="/x" aria-label="hi">y</a></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        assert!(doc.box_tree.remove_element_attribute("x", "aria-label"));
        let mut still = false;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("x")
                && b.attributes.iter().any(|(k, _)| k == "aria-label")
            {
                still = true;
            }
        });
        assert!(!still);
    }

    #[test]
    fn set_element_dataset_wrapper_usa_set_element_attribute() {
        let html = r#"<html><body><div id="x">y</div></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        // El wrapper de Fase 7.11 ahora delega a set_element_attribute
        // con el prefijo data-; verificamos que ambos vean el mismo store.
        assert!(doc.box_tree.set_element_dataset("x", "role", "main"));
        let mut found = false;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("x")
                && b.attributes.iter().any(|(k, v)| k == "data-role" && v == "main")
            {
                found = true;
            }
        });
        assert!(found, "set_element_dataset debe poblar attributes con data-<key>");
    }

    #[test]
    fn set_element_attribute_id_inexistente_devuelve_false() {
        let html = r#"<html><body><p>x</p></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        assert!(!doc.box_tree.set_element_attribute("fantasma", "href", "/"));
    }

    #[test]
    fn set_element_text_content_reemplaza_primer_leaf_no_los_demas() {
        let html = r#"<html><body><div id="d"><span>uno</span><span>dos</span></div></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        let ok = doc.box_tree.set_element_text_content("d", "X");
        assert!(ok);
        let mut texts = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if !t.trim().is_empty() {
                    texts.push(t.clone());
                }
            }
        });
        // El primer text leaf "uno" pasa a "X"; "dos" sigue intacto.
        assert!(texts.contains(&"X".to_string()), "texts: {texts:?}");
        assert!(texts.contains(&"dos".to_string()), "texts: {texts:?}");
        assert!(!texts.contains(&"uno".to_string()), "texts: {texts:?}");
    }

    #[test]
    fn box_tree_resuelve_animation_contra_keyframes() {
        // `animation: fade …` + `@keyframes fade` debe poblar BoxNode.animation
        // (Tier B: wiring del runtime de tween rescatado de engine).
        let html = r##"<html><head><style>
            @keyframes fade { from { opacity: 0 } to { opacity: 1 } }
            #target { animation: fade 2s linear }
        </style></head><body><div id="target">hola</div></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found = false;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("target") {
                let inst = b.animation.as_ref().expect("div animado sin AnimationInstance");
                assert_eq!(inst.binding.name, "fade");
                // A mitad de los 2s (linear) la opacity interpolada ≈ 0.5.
                let p = crate::anim::animation_progress(&inst.binding, 1.0).unwrap();
                let ov = crate::anim::sample_keyframes(&inst.keyframes, p);
                let op = ov.opacity.expect("keyframes fade interpola opacity");
                assert!((op - 0.5).abs() < 0.05, "opacity a mitad: {op}");
                found = true;
            }
        });
        assert!(found, "no se encontró #target en el box tree");
    }

    #[test]
    fn box_tree_animation_none_sin_keyframes_match() {
        // `animation: <name>` sin `@keyframes <name>` → animation: None.
        let html = r##"<html><head><style>
            #x { animation: noexiste 1s }
        </style></head><body><div id="x">a</div></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut checked = false;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("x") {
                assert!(b.animation.is_none(), "no debería resolver sin @keyframes");
                checked = true;
            }
        });
        assert!(checked);
    }
}
