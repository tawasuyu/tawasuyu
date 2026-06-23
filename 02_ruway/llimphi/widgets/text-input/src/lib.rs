//! `llimphi-widget-text-input` — input de texto single-line para Llimphi.
//!
//! Después del refactor 2026-05-25, [`TextInputState`] es un wrapper fino
//! sobre [`llimphi_widget_text_editor::EditorState`] con
//! `options.single_line = true` + un flag `masked` para passwords. La
//! API pública (`new`, `masked`, `text`, `set_text`, `clear`, `apply_key`,
//! `is_empty`, `push_str`, `pop`, `is_masked`) se mantiene salvo que
//! `text()` ahora devuelve `String` (antes `&str`) — los callers que
//! hacían `.text().trim().to_string()` siguen funcionando idénticos.
//!
//! Beneficios heredados del editor: selección con Shift+arrows, undo/
//! redo con Ctrl+Z/Y, salto de palabra con Ctrl+arrows, Home/End,
//! Delete (además de Backspace). Tab/Enter siguen ignorados (single_line).

#![forbid(unsafe_code)]

use std::sync::Arc;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{KeyEvent, View};
use llimphi_widget_text_editor::{EditorOptions, EditorState};

/// Paleta del input. Defaults son una variante dark con borde tenue que
/// se enciende al focar, equivalente conceptual al `nahual-theme` dark.
#[derive(Debug, Clone, Copy)]
pub struct TextInputPalette {
    pub bg: Color,
    pub bg_focus: Color,
    pub border: Color,
    pub border_focus: Color,
    pub fg_text: Color,
    pub fg_placeholder: Color,
    /// Color del caret (cursor de inserción) que se pinta cuando el input
    /// está focado. Default = `fg_text` (sigue al texto, como `caret-color:
    /// auto` en CSS).
    pub caret: Color,
    /// Fondo del texto seleccionado (con Shift+flechas o doble/triple click).
    pub selection: Color,
}

impl Default for TextInputPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl TextInputPalette {
    /// Construye la paleta desde un `Theme` semántico.
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg: t.bg_input,
            bg_focus: t.bg_input_focus,
            border: t.border,
            border_focus: t.border_focus,
            fg_text: t.fg_text,
            fg_placeholder: t.fg_placeholder,
            caret: t.fg_text,
            selection: t.bg_selected,
        }
    }
}

/// Estado del input. Wrappea un `EditorState` single-line.
#[derive(Debug, Clone, Default)]
pub struct TextInputState {
    inner: EditorState,
    masked: bool,
}

impl TextInputState {
    /// Input vacío visible (texto plano).
    pub fn new() -> Self {
        Self {
            inner: EditorState::with_options(EditorOptions {
                single_line: true,
                ..EditorOptions::default()
            }),
            masked: false,
        }
    }

    /// Input enmascarado — para campos de contraseña.
    pub fn masked() -> Self {
        Self { masked: true, ..Self::new() }
    }

    /// Texto actual. Devuelve `String` (antes `&str` — el rope no expone
    /// slice borrowed sin clone). Para evitar copias innecesarias, los
    /// callers que sólo necesitan derivar `.trim()` o `.is_empty()`
    /// pueden hacerlo directo sobre el `String` devuelto.
    pub fn text(&self) -> String {
        self.inner.text()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn is_masked(&self) -> bool {
        self.masked
    }

    pub fn clear(&mut self) {
        self.inner.set_text("");
    }

    pub fn set_text(&mut self, s: impl Into<String>) {
        let s = s.into();
        self.inner.set_text(&s);
    }

    pub fn push_str(&mut self, s: &str) {
        let combined = format!("{}{}", self.inner.text(), s);
        self.inner.set_text(&combined);
    }

    pub fn pop(&mut self) -> Option<char> {
        let mut t = self.inner.text();
        let ch = t.pop()?;
        self.inner.set_text(&t);
        Some(ch)
    }

    /// Aplica una tecla al estado. Devuelve `true` si cambió el contenido
    /// **o** sólo se movió el cursor (cualquier cosa que requiera repintar).
    pub fn apply_key(&mut self, event: &KeyEvent) -> bool {
        self.inner.apply_key(event).touched()
    }

    /// Acceso de bajo nivel al editor interno — útil si el caller
    /// quiere consultar cursor/selección o aplicar ops avanzadas.
    pub fn editor(&self) -> &EditorState {
        &self.inner
    }
    pub fn editor_mut(&mut self) -> &mut EditorState {
        &mut self.inner
    }

    /// Registra un click del mouse a `local_x` px del borde izquierdo del box
    /// (coords del nodo, incluido el borde de 1 px + el padding de 10 px).
    /// Mapea x → columna de carácter y delega en
    /// [`EditorState::register_click`], que escala **1 click = caret**,
    /// **2 = palabra**, **3 = todo** por ventana de tiempo. `font_size` es el
    /// de render (13.0 por defecto en `text_input_view`).
    ///
    /// El mapeo usa un ancho de carácter aproximado (`font_size * 0.5`): para
    /// una fuente proporcional no es exacto al píxel, pero como doble/triple
    /// click snapean a límites de palabra/línea, basta caer dentro de la
    /// palabra. (No contempla scroll horizontal — pensado para campos cortos.)
    pub fn pointer_click(&mut self, local_x: f32, font_size: f32) {
        const PAD: f32 = 11.0; // borde 1px + padding interno 10px
        let char_w = (font_size * 0.5).max(1.0);
        let col = (((local_x - PAD) / char_w).round().max(0.0)) as usize;
        // Single-line ⇒ línea 0; register_click clampa la columna al texto.
        self.inner.register_click(0, col);
    }

    /// Selecciona todo el texto (equivalente a triple-click / Ctrl+A).
    pub fn select_all(&mut self) {
        self.inner.select_all();
    }
}

/// Compone el input box: borde de 1 px (rect padre coloreado), relleno
/// interno, texto o placeholder, y el caret (cursor de inserción) sobre el
/// texto si está focado. Caret v3 (Fase 7.1255): cuando está focado la hoja
/// pinta texto+caret en un `paint_over` con **scroll horizontal** — el texto
/// se desplaza para mantener el caret a la vista cuando desborda la caja, y se
/// recorta al área de contenido. Sin foco usa un nodo-hijo de texto (sin caret).
/// Click sobre el box emite `on_focus` (típicamente `Msg::Focus(Field)`).
pub fn text_input_view<Msg: Clone + 'static>(
    state: &TextInputState,
    placeholder: &str,
    focused: bool,
    palette: &TextInputPalette,
    on_focus: Msg,
) -> View<Msg> {
    build_input(state, placeholder, focused, palette, ClickHandler::Focus(on_focus))
}

/// Cómo responde el box al click.
enum ClickHandler<Msg> {
    /// Click plano → un solo `Msg` (toma foco). Sin selección por mouse.
    Focus(Msg),
    /// Click con posición → `Msg(local_x)`. El caller suele enfocar el campo
    /// **y** llamar [`TextInputState::pointer_click`] para posicionar el caret
    /// / seleccionar palabra (2 clicks) / todo (3 clicks).
    At(Arc<dyn Fn(f32) -> Msg + Send + Sync>),
}

/// Como [`text_input_view`] pero el click lleva la posición `local_x` (px
/// desde el borde izquierdo del box) — habilita posicionar el caret y la
/// selección por **doble/triple click** (vía [`TextInputState::pointer_click`]).
/// La selección activa se pinta resaltada.
pub fn text_input_view_mouse<Msg, F>(
    state: &TextInputState,
    placeholder: &str,
    focused: bool,
    palette: &TextInputPalette,
    on_click_at: F,
) -> View<Msg>
where
    Msg: Clone + 'static,
    F: Fn(f32) -> Msg + Send + Sync + 'static,
{
    build_input(
        state,
        placeholder,
        focused,
        palette,
        ClickHandler::At(Arc::new(on_click_at)),
    )
}

fn build_input<Msg: Clone + 'static>(
    state: &TextInputState,
    placeholder: &str,
    focused: bool,
    palette: &TextInputPalette,
    click: ClickHandler<Msg>,
) -> View<Msg> {
    let raw = state.text();
    let is_empty = raw.is_empty();
    let shown = if is_empty {
        placeholder.to_string()
    } else if state.masked {
        "•".repeat(raw.chars().count())
    } else {
        raw
    };
    let display = shown;
    // Prefijo del texto visible hasta el caret (cursor de inserción), para
    // medir su ancho y posicionar la barra del caret. La columna es índice de
    // carácter (single-line ⇒ `line == 0`); `take(col)` sobre el texto MOSTRADO
    // (placeholder/`•`/crudo) alinea el caret con lo que se ve. Cuando el input
    // está vacío el `col` es 0 ⇒ prefijo vacío ⇒ caret al inicio (no se mide el
    // placeholder).
    let caret_prefix: String = if focused {
        display.chars().take(state.editor().cursor.caret.col).collect()
    } else {
        String::new()
    };
    // Rango de selección en columnas (= char offsets, single-line). `None` si
    // no hay selección o el input no está focado. Se captura por valor para el
    // closure de pintado (no puede tomar prestado `state`).
    let sel_cols: Option<(usize, usize)> = if focused {
        let ed = state.editor();
        let (s, e) = ed.cursor.selection_range(&ed.buffer);
        (s != e).then_some((s, e))
    } else {
        None
    };
    let sel_color = palette.selection;
    let text_color = if is_empty {
        palette.fg_placeholder
    } else {
        palette.fg_text
    };
    let (bg, border) = if focused {
        (palette.bg_focus, palette.border_focus)
    } else {
        (palette.bg, palette.border)
    };

    let mut inner = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .radius(3.0);
    let inner = if focused {
        // Caret v3 — scroll horizontal (Fase 7.1255). Cuando el input está
        // focado, la propia hoja pinta el texto Y el caret en un solo `paint_over`
        // (pasada vello FINAL): así puede DESPLAZAR el texto a la izquierda cuando
        // el cursor se saldría por el borde derecho, manteniéndolo visible — el
        // clásico scroll del caret de los `<input>`. El offset (`scroll`) depende
        // del ancho de layout y de la posición del caret, ambos conocidos sólo en
        // tiempo de pintado (acá `rect.w` ya está resuelto y `ts` puede medir), no
        // en `view()` — por eso no se hace con un nodo hijo + `transform` estático.
        // Sin foco se usa el camino de nodo-hijo de abajo (sin caret, sin scroll).
        let caret_color = palette.caret;
        let display_c = display;
        let caret_prefix_c = caret_prefix;
        let tcolor = text_color;
        let sel_cols_c = sel_cols;
        inner.paint_over(move |scene, ts, rect| {
            use llimphi_ui::llimphi_raster::kurbo::{Affine, Rect as KRect};
            use llimphi_ui::llimphi_raster::peniko::{BlendMode, Fill};
            use llimphi_ui::llimphi_text::{draw_layout, measurement, Alignment};
            let pad = 10.0_f64;
            // Ancho visible interno (entre los dos paddings de 10 px).
            let vis_w = (rect.w as f64 - 2.0 * pad).max(0.0);
            // Layout del texto completo en una sola línea (sin wrap).
            let layout = ts.layout(
                &display_c, 13.0, None, Alignment::Start, 1.2, false, None, 400.0, false, false,
                0.0, 0.0,
            );
            let th = measurement(&layout).height as f64;
            // Ancho del prefijo hasta el caret = posición x del caret en el texto.
            let caret_w = if caret_prefix_c.is_empty() {
                0.0
            } else {
                let lp = ts.layout(
                    &caret_prefix_c, 13.0, None, Alignment::Start, 1.2, false, None, 400.0, false,
                    false, 0.0, 0.0,
                );
                measurement(&lp).width as f64
            };
            // Scroll: si el caret cae más allá del ancho visible, corre el texto a
            // la izquierda lo justo para que el caret quede al borde (con 2 px de
            // aire). Texto que entra ⇒ scroll 0 (anclado al padding-left).
            let scroll = (caret_w - vis_w + 2.0).max(0.0);
            let cx0 = rect.x as f64 + pad;
            // Recorte al área de contenido para que el texto desplazado no se
            // derrame sobre el padding ni fuera de la caja.
            let clip = KRect::new(
                cx0,
                rect.y as f64,
                rect.x as f64 + rect.w as f64 - pad,
                rect.y as f64 + rect.h as f64,
            );
            scene.push_layer(Fill::NonZero, BlendMode::default(), 1.0, Affine::IDENTITY, &clip);
            let oy = rect.y as f64 + (rect.h as f64 - th) * 0.5;
            // Resaltado de selección (debajo del texto). Mide el prefijo hasta
            // el inicio y el fin del rango para ubicar el rect en x.
            if let Some((s, e)) = sel_cols_c {
                let mut prefix_w = |n: usize| -> f64 {
                    if n == 0 {
                        return 0.0;
                    }
                    let p: String = display_c.chars().take(n).collect();
                    let lp = ts.layout(
                        &p, 13.0, None, Alignment::Start, 1.2, false, None, 400.0, false, false,
                        0.0, 0.0,
                    );
                    measurement(&lp).width as f64
                };
                let xs = cx0 + prefix_w(s) - scroll;
                let xe = cx0 + prefix_w(e) - scroll;
                let sel_rect = KRect::new(xs, oy, xe.max(xs + 1.0), oy + th);
                scene.fill(Fill::NonZero, Affine::IDENTITY, sel_color, None, &sel_rect);
            }
            draw_layout(scene, &layout, tcolor, (cx0 - scroll, oy));
            scene.pop_layer();
            // Caret: barra vertical en la posición del caret, desplazada por el
            // mismo scroll. Fuera del clip para que nunca se recorte en el borde.
            let x = cx0 + caret_w - scroll;
            let h = 16.0_f64;
            let cy = rect.y as f64 + rect.h as f64 * 0.5;
            let bar = KRect::new(x, cy - h * 0.5, x + 1.5, cy + h * 0.5);
            scene.fill(Fill::NonZero, Affine::IDENTITY, caret_color, None, &bar);
        })
    } else {
        // Sin foco: el texto va en un nodo HIJO de alto automático, centrado
        // verticalmente por el contenedor (`align_items: Center`). (`align_items`
        // no centra el texto PROPIO de un nodo — por eso el hijo.)
        let texto = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: auto(),
            },
            ..Default::default()
        })
        .text_aligned(display, 13.0, text_color, Alignment::Start);
        inner.children(vec![texto])
    };

    let node = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(34.0_f32),
        },
        padding: Rect {
            left: length(1.0_f32),
            right: length(1.0_f32),
            top: length(1.0_f32),
            bottom: length(1.0_f32),
        },
        ..Default::default()
    })
    .fill(border)
    .radius(4.0)
    // Semántica: input de texto + el valor crudo como `value` (no el "•"
    // del modo masked — los lectores no deben dictar la contraseña en
    // voz alta; AccessKit ya marca el control como TextInput y el lector
    // sustituye por "punto" cuando el contexto lo requiere). El
    // placeholder va como `description` cuando el campo está vacío para
    // que el lector lo enuncie como pista. `value` queda vacío en masked.
    .role(llimphi_ui::Role::TextInput)
    .aria_value(if state.masked { String::new() } else { state.text() })
    .aria_description(if is_empty { placeholder.to_string() } else { String::new() });
    // Click: plano (toma foco) o con posición (foco + caret/selección). El
    // `on_click_at` da `x` relativo al rect del box; el caller lo pasa a
    // `pointer_click`.
    let node = match click {
        ClickHandler::Focus(m) => node.on_click(m),
        ClickHandler::At(cb) => node.on_click_at(move |x, _y, _w, _h| Some(cb(x))),
    };
    node.cursor(llimphi_ui::Cursor::Text).children(vec![inner])
}

#[cfg(test)]
mod tests {
    use super::*;
    use llimphi_ui::{Key, KeyState, NamedKey};

    fn key_press(key: Key, text: Option<&str>) -> KeyEvent {
        KeyEvent {
            key,
            state: KeyState::Pressed,
            text: text.map(|s| s.to_string()),
            modifiers: Default::default(),
            repeat: false,
        }
    }

    #[test]
    fn palette_caret_default_sigue_al_texto() {
        // El caret por default sigue al color del texto (`caret-color: auto`):
        // `from_theme` y `Default` lo igualan a `fg_text`.
        let t = llimphi_theme::Theme::dark();
        let pal = TextInputPalette::from_theme(&t);
        assert_eq!(pal.caret, pal.fg_text);
        assert_eq!(pal.caret, t.fg_text);
        assert_eq!(TextInputPalette::default().caret, TextInputPalette::default().fg_text);
    }

    #[test]
    fn caret_se_registra_como_over_painter_solo_focado() {
        // Caret v2 (Fase 7.1249): el caret se pinta con `paint_over` (pasada
        // FINAL sobre el glifo). Verificamos el wiring montando la vista: con
        // foco hay un over-painter registrado; sin foco no hay ninguno.
        use llimphi_ui::llimphi_layout::LayoutTree;
        use llimphi_ui::{has_over_painter, mount};
        let mut st = TextInputState::new();
        st.set_text("hola");
        let pal = TextInputPalette::default();

        let mut lt = LayoutTree::new();
        let focado = mount(&mut lt, text_input_view(&st, "ph", true, &pal, ()));
        assert!(
            has_over_painter(&focado),
            "input focado debe registrar el caret como over-painter"
        );

        let mut lt2 = LayoutTree::new();
        let sin_foco = mount(&mut lt2, text_input_view(&st, "ph", false, &pal, ()));
        assert!(
            !has_over_painter(&sin_foco),
            "input sin foco no pinta caret"
        );
    }

    #[test]
    fn apply_key_inserts_printable_chars() {
        let mut s = TextInputState::new();
        let ev = key_press(Key::Character("a".into()), Some("a"));
        assert!(s.apply_key(&ev));
        assert_eq!(s.text(), "a");
    }

    #[test]
    fn pointer_click_escalates_caret_word_all() {
        let mut s = TextInputState::new();
        s.set_text("hola mundo");
        // x cae dentro de "mundo" (col ~7 con char_w = 13*0.5 = 6.5; x = 11 + 7*6.5).
        let x = 11.0 + 7.0 * 6.5;
        // 1 click: caret simple, sin selección.
        s.pointer_click(x, 13.0);
        assert!(!s.editor().has_selection(), "1 click no debe seleccionar");
        // 2 clicks: palabra bajo el cursor.
        s.pointer_click(x, 13.0);
        assert!(s.editor().has_selection(), "2 clicks debe seleccionar palabra");
        assert_eq!(s.editor().selected_text().as_deref(), Some("mundo"));
        // 3 clicks: todo (párrafo = la línea entera en single-line).
        s.pointer_click(x, 13.0);
        assert_eq!(s.editor().selected_text().as_deref(), Some("hola mundo"));
    }

    #[test]
    fn select_all_selecciona_todo() {
        let mut s = TextInputState::new();
        s.set_text("Frida Kahlo");
        s.select_all();
        assert_eq!(s.editor().selected_text().as_deref(), Some("Frida Kahlo"));
    }

    #[test]
    fn apply_key_backspace_pops() {
        let mut s = TextInputState::new();
        s.set_text("hola");
        let ev = key_press(Key::Named(NamedKey::Backspace), None);
        assert!(s.apply_key(&ev));
        assert_eq!(s.text(), "hol");
    }

    #[test]
    fn enter_ignorado_en_single_line() {
        let mut s = TextInputState::new();
        s.set_text("hola");
        let enter = key_press(Key::Named(NamedKey::Enter), None);
        assert!(!s.apply_key(&enter));
        assert_eq!(s.text(), "hola");
    }

    #[test]
    fn masked_state_is_masked() {
        let s = TextInputState::masked();
        assert!(s.is_masked());
    }

    #[test]
    fn flecha_izquierda_mueve_cursor() {
        // El refactor agrega esta capacidad — antes no había movimiento.
        let mut s = TextInputState::new();
        s.set_text("hola");
        let arr = key_press(Key::Named(NamedKey::ArrowLeft), None);
        assert!(s.apply_key(&arr));
        assert_eq!(s.editor().cursor.caret.col, 3);
    }

    #[test]
    fn push_str_y_pop_funcionan() {
        let mut s = TextInputState::new();
        s.push_str("hola");
        assert_eq!(s.text(), "hola");
        assert_eq!(s.pop(), Some('a'));
        assert_eq!(s.text(), "hol");
    }
}
