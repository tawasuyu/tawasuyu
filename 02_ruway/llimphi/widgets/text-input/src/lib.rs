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
}

/// Compone el input box: borde de 1 px (rect padre coloreado), relleno
/// interno, texto o placeholder, caret simulado al final si está focado.
/// Click sobre el box emite `on_focus` (típicamente `Msg::Focus(Field)`).
pub fn text_input_view<Msg: Clone + 'static>(
    state: &TextInputState,
    placeholder: &str,
    focused: bool,
    palette: &TextInputPalette,
    on_focus: Msg,
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

    // El texto va en un nodo HIJO de alto automático, centrado verticalmente
    // por el contenedor (`align_items: Center`). Antes el texto era el contenido
    // propio del nodo con alto fijo: `align_items` no centra el texto propio de
    // un nodo, así que quedaba pegado arriba («inputs descentrados hacia arriba»).
    let texto = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        ..Default::default()
    })
    .text_aligned(display, 13.0, text_color, Alignment::Start);
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
    // Caret real (cursor de inserción) cuando el input está focado: una barra
    // vertical fina a la derecha del prefijo medido. Se mide el ancho del
    // `caret_prefix` con el MISMO tamaño/fuente que el texto (13 px, sans) y se
    // ubica tras el padding-left (10 px). Pinta antes que el texto hijo, así que
    // a fin de línea (el caso típico al tipear) queda totalmente visible; en
    // medio del texto queda detrás del glifo (limitación v1).
    if focused {
        let caret_color = palette.caret;
        inner = inner.paint_with(move |scene, ts, rect| {
            use llimphi_ui::llimphi_raster::kurbo::{Affine, Rect as KRect};
            use llimphi_ui::llimphi_raster::peniko::Fill;
            use llimphi_ui::llimphi_text::{measure, TextBlock};
            let w = measure(
                ts,
                &TextBlock::simple(&caret_prefix, 13.0, caret_color, (0.0, 0.0)),
            )
            .width as f64;
            let x = rect.x as f64 + 10.0 + w;
            let h = 16.0_f64;
            let cy = rect.y as f64 + rect.h as f64 * 0.5;
            let bar = KRect::new(x, cy - h * 0.5, x + 1.5, cy + h * 0.5);
            scene.fill(Fill::NonZero, Affine::IDENTITY, caret_color, None, &bar);
        });
    }
    let inner = inner.children(vec![texto]);

    View::new(Style {
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
    .aria_description(if is_empty { placeholder.to_string() } else { String::new() })
    .on_click(on_focus)
    .cursor(llimphi_ui::Cursor::Text)
    .children(vec![inner])
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
    fn apply_key_inserts_printable_chars() {
        let mut s = TextInputState::new();
        let ev = key_press(Key::Character("a".into()), Some("a"));
        assert!(s.apply_key(&ev));
        assert_eq!(s.text(), "a");
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
