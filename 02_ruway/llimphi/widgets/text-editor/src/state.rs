//! [`EditorState`] — la unión de buffer + cursor + undo + opciones, con
//! `apply_key` que mapea un `KeyEvent` de llimphi-ui a operaciones de
//! edición o movimiento. Este es el tipo que el caller pone en su
//! `Model` y mete en el `update` Elm.

use std::cell::RefCell;

use llimphi_ui::{ImeEvent, Key, KeyEvent, KeyState, NamedKey};

use crate::buffer::Buffer;
use crate::clipboard::{Clipboard, NullClipboard};
use crate::cursor::{Cursor, Pos};
use crate::highlight::{Highlighter, Language, Span};
use crate::ops::{
    dedent, delete_backward, delete_forward, delete_word_backward, delete_word_forward,
    indent_or_insert_tab, insert_newline_auto_indent, replace_selection,
};
use crate::undo::UndoStack;

/// Opciones del editor — afectan indent + límite de undo + page size.
#[derive(Debug, Clone, Copy)]
pub struct EditorOptions {
    /// `true` = Tab inserta `indent_size` spaces; `false` = inserta `\t`.
    pub tab_to_spaces: bool,
    pub indent_size: usize,
    /// Cuántas líneas avanza PageUp/PageDown.
    pub page_size: usize,
    /// `true` = Enter no inserta `\n`; el caller maneja submit. (modo
    /// single-line para el text-input refactorizado).
    pub single_line: bool,
}

impl Default for EditorOptions {
    fn default() -> Self {
        Self {
            tab_to_spaces: true,
            indent_size: 2,
            page_size: 12,
            single_line: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EditorState {
    pub buffer: Buffer,
    /// Cursor primario — el que la API legacy expone como "el" cursor.
    /// Edit ops aplican al primary + todos los `extra_cursors` en orden.
    pub cursor: Cursor,
    /// Cursores adicionales (multi-cursor). Vacío en el caso típico.
    /// Cuando hay extras, las ediciones aplican a todos; Esc los colapsa
    /// dejando sólo el primary.
    pub extra_cursors: Vec<Cursor>,
    /// Diagnostics del LSP (o equivalente). El client externo los popa
    /// vía `set_diagnostics`; el render del editor los pinta como
    /// subrayado bajo el rango con color según severity.
    pub diagnostics: Vec<crate::diagnostics::Diagnostic>,
    pub options: EditorOptions,
    /// Líneas-guarda: índices ordenados (ascendente, sin duplicados) de
    /// líneas que el caret no puede ocupar y que el gutter no numera.
    /// El widget no decide qué es guarda — lo decide el caller (el
    /// `cuerpo_ide` lo computa a partir de la estructura de átomos +
    /// flags de fusión). La lista debe mantenerse al día tras cada
    /// edición que cambie la cantidad o posición de líneas: el caller
    /// es responsable de actualizar este campo cuando reaccione a
    /// `Changed`. Vacío = sin guardas, comportamiento clásico de IDE.
    pub guard_lines: Vec<usize>,
    /// Tinte de fondo por línea. `line_tints[i]` controla la línea
    /// `i` del buffer: `Some(color)` pinta un rectángulo del ancho
    /// completo del área de contenido al ALPHA del color, **debajo**
    /// del texto y de cualquier highlight; `None` deja la línea sin
    /// tinte. Vacío o ausente = sin tintes (modo IDE clásico). Pensado
    /// para colorear zonas en editores narrativos sin afectar la
    /// lectura — los callers deben elegir colores con alpha bajo (≤
    /// ~40/255 sobre el bg).
    pub line_tints: Vec<Option<llimphi_ui::llimphi_raster::peniko::Color>>,
    pub undo: UndoStack,
    /// Texto en composición del IME (dead keys de acentos, CJK, emoji
    /// picker). No vive en el buffer hasta el `Commit`: el view lo pinta
    /// subrayado en el caret y el caret se corre detrás. `None` = sin
    /// composición activa (el caso normal). Lo administra
    /// [`Self::apply_ime_event`].
    pub preedit: Option<Preedit>,
    /// Línea inicial visible — el viewport renderiza
    /// `[scroll_offset, scroll_offset + visible)`. El caller llama a
    /// [`Self::ensure_caret_visible`] tras movimientos para auto-scrollear.
    pub scroll_offset: usize,
    /// Contador monotónico que se incrementa con cada edición del buffer.
    /// Lo usa el cache de highlight para invalidarse sin re-hashear el
    /// texto entero por frame.
    pub edit_seq: u64,
    /// InputEdits que el editor produjo y todavía no fueron aplicados
    /// al `Tree` cached del highlighter. El highlight, antes de
    /// reparsear, los drena y los aplica al tree → parseo incremental
    /// real (tree-sitter sólo reconstruye los subtrees afectados).
    pub pending_input_edits: RefCell<Vec<tree_sitter::InputEdit>>,
    /// Cache memoizado del syntax highlight. Interior mutability vía
    /// `RefCell` para que el view (que recibe `&EditorState`) lo
    /// actualice on-demand. Se invalida cuando cambian `edit_seq` o el
    /// `Language` solicitado.
    pub highlight_cache: RefCell<Option<HighlightCache>>,
    /// Último click registrado por [`Self::register_click`]: instante +
    /// posición. Sirve para contar clicks consecutivos (doble/triple) sin
    /// que el caller lleve el tiempo. `None` = no hubo click reciente.
    last_click: Option<(std::time::Instant, Pos)>,
    /// Cuántos clicks consecutivos cercanos lleva la racha actual (1 = simple,
    /// 2 = palabra, 3 = párrafo). Cicla 1→2→3→1. Ver [`Self::register_click`].
    click_count: u32,
}

/// Ventana máxima entre dos clicks para que cuenten como doble/triple.
const MULTI_CLICK_WINDOW: std::time::Duration = std::time::Duration::from_millis(450);

/// Texto en composición del IME — el que el método de entrada está
/// armando antes de confirmarlo (un acento muerto a medio componer, una
/// sílaba CJK con su ventana de candidatos, un emoji del picker). No
/// pertenece al buffer todavía; el view lo dibuja en el caret con
/// subrayado para que el usuario vea lo que está tecleando.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preedit {
    /// Texto provisional a pintar en el caret.
    pub text: String,
    /// Rango `(inicio, fin)` en **bytes** dentro de `text` que el IME
    /// quiere resaltar (la "clause" activa), si lo reporta. Hoy el view
    /// subraya todo el preedit por igual; el campo se conserva para un
    /// resaltado más fino cuando haga falta.
    pub cursor: Option<(usize, usize)>,
}

/// Entrada del cache: spans por línea + clave que la generó.
#[derive(Debug, Clone)]
pub struct HighlightCache {
    pub seq: u64,
    pub language: Language,
    pub spans: Vec<Vec<Span>>,
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorState {
    pub fn new() -> Self {
        Self {
            buffer: Buffer::new(),
            cursor: Cursor::new(),
            extra_cursors: Vec::new(),
            diagnostics: Vec::new(),
            options: EditorOptions::default(),
            guard_lines: Vec::new(),
            line_tints: Vec::new(),
            undo: UndoStack::new(),
            preedit: None,
            scroll_offset: 0,
            edit_seq: 0,
            pending_input_edits: RefCell::new(Vec::new()),
            highlight_cache: RefCell::new(None),
            last_click: None,
            click_count: 0,
        }
    }

    /// Devuelve todos los cursores en orden: primary + extras. Útil para
    /// el render que dibuja un caret + selección por cada uno.
    pub fn all_cursors(&self) -> impl Iterator<Item = &Cursor> {
        std::iter::once(&self.cursor).chain(self.extra_cursors.iter())
    }

    /// Agrega un cursor adicional con caret en `(line, col)`. Si ya hay
    /// un cursor exactamente ahí, no duplica.
    pub fn add_cursor_at(&mut self, line: usize, col: usize) {
        let line = line.min(self.buffer.len_lines().saturating_sub(1));
        let col = col.min(self.buffer.line_len_chars(line));
        let pos = Pos::new(line, col);
        if self.cursor.caret == pos {
            return;
        }
        if self.extra_cursors.iter().any(|c| c.caret == pos) {
            return;
        }
        self.extra_cursors.push(Cursor::at(line, col));
    }

    /// Colapsa multi-cursor: descarta los `extra_cursors`. No toca el
    /// primary.
    pub fn collapse_to_primary(&mut self) {
        self.extra_cursors.clear();
    }

    pub fn has_multi_cursor(&self) -> bool {
        !self.extra_cursors.is_empty()
    }

    /// Reemplaza los diagnostics del editor. Usado por el client LSP
    /// cuando recibe `textDocument/publishDiagnostics`.
    pub fn set_diagnostics(&mut self, diags: Vec<crate::diagnostics::Diagnostic>) {
        self.diagnostics = diags;
    }

    pub fn with_options(options: EditorOptions) -> Self {
        Self {
            options,
            ..Self::new()
        }
    }

    /// Ajusta `scroll_offset` para que la línea del caret quede dentro
    /// de `[scroll_offset, scroll_offset + visible_lines)`. Si el caret
    /// está arriba, scrollea para arriba; si está abajo, scrollea para
    /// abajo dejando el caret en la última línea visible.
    pub fn ensure_caret_visible(&mut self, visible_lines: usize) {
        if visible_lines == 0 {
            return;
        }
        let line = self.cursor.caret.line;
        if line < self.scroll_offset {
            self.scroll_offset = line;
        } else if line >= self.scroll_offset + visible_lines {
            self.scroll_offset = line + 1 - visible_lines;
        }
        // Clampea al rango válido — no scrollear más allá del fin del
        // buffer (deja la última línea siempre visible).
        let max_scroll = self.line_count().saturating_sub(1);
        if self.scroll_offset > max_scroll {
            self.scroll_offset = max_scroll;
        }
    }

    /// Scrollea relativo (positivo = abajo). Clampea a 0..line_count-1.
    pub fn scroll_by(&mut self, delta: i32) {
        let new = (self.scroll_offset as i32 + delta).max(0) as usize;
        let max = self.line_count().saturating_sub(1);
        self.scroll_offset = new.min(max);
    }

    pub fn text(&self) -> String {
        self.buffer.text()
    }

    pub fn set_text(&mut self, s: &str) {
        self.buffer.set_text(s);
        // Las guardas y tintes previos referían al texto viejo:
        // limpiar. El caller los repuebla cuando reaccione al cambio.
        self.guard_lines.clear();
        self.line_tints.clear();
        // Clampea el caret a la nueva longitud.
        let last_line = self.buffer.len_lines().saturating_sub(1);
        let col = self.buffer.line_len_chars(last_line);
        self.cursor = Cursor::at(last_line, col);
        self.undo.clear();
        self.bump_edit_seq();
        // Cambio masivo de buffer — el árbol cached del highlighter
        // queda inválido. Lo borramos para forzar full parse próximo.
        for lang in [Language::Rust, Language::Python] {
            crate::highlight::invalidate_tree_cache(lang);
        }
    }

    /// Incrementa el contador de ediciones — invalidando el cache de
    /// highlight automáticamente.
    pub fn bump_edit_seq(&mut self) {
        self.edit_seq = self.edit_seq.wrapping_add(1);
    }

    /// Devuelve los spans del highlight cacheados. Si el cache no matchea
    /// (distinto `edit_seq` o `language`), reparsea con tree-sitter
    /// incremental — aplica los `pending_input_edits` al tree previo
    /// antes de parsear, y guarda el nuevo tree.
    pub fn highlighted_spans(&self, language: Language) -> Vec<Vec<Span>> {
        if matches!(language, Language::Plain) {
            return Vec::new();
        }
        let mut cache = self.highlight_cache.borrow_mut();
        if let Some(c) = cache.as_ref() {
            if c.seq == self.edit_seq && c.language == language {
                return c.spans.clone();
            }
        }
        // Aplica los InputEdits pending al tree cached antes de parsear
        // — eso convierte el parseo de "full" a "incremental real".
        let edits: Vec<tree_sitter::InputEdit> =
            self.pending_input_edits.borrow_mut().drain(..).collect();
        crate::highlight::apply_pending_edits(language, &edits);

        let mut h = Highlighter::new(language);
        let spans = h.highlight(&self.buffer.text());
        *cache = Some(HighlightCache {
            seq: self.edit_seq,
            language,
            spans: spans.clone(),
        });
        spans
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    pub fn line_count(&self) -> usize {
        self.buffer.len_lines()
    }

    /// Posiciona el caret en `(line, col)`, clampeando al rango válido
    /// del buffer. Colapsa la selección. Usado por el caller cuando el
    /// usuario clickea en el área de texto.
    ///
    /// Si la línea destino está en [`Self::guard_lines`], el caret
    /// salta a la línea no-guarda más cercana (privilegia hacia
    /// abajo). Así un click "en la franja entre zonas" aterriza en el
    /// inicio de la zona siguiente.
    pub fn set_caret_at(&mut self, line: usize, col: usize) {
        self.cursor.set_caret(&self.buffer, Pos::new(line, col));
        if !self.guard_lines.is_empty() {
            snap_cursor_off_guard(&mut self.cursor, &self.buffer, &self.guard_lines, 0);
        }
    }

    /// Selecciona la palabra bajo `(line, col)` (doble-click). Delegado a
    /// [`Cursor::select_word`].
    pub fn select_word_at(&mut self, line: usize, col: usize) {
        self.cursor.select_word(&self.buffer, Pos::new(line, col));
    }

    /// Selecciona el párrafo bajo `(line, col)` (triple-click). Delegado a
    /// [`Cursor::select_paragraph`].
    pub fn select_paragraph_at(&mut self, line: usize, col: usize) {
        self.cursor.select_paragraph(&self.buffer, Pos::new(line, col));
    }

    /// Registra un click del mouse en `(line, col)` y aplica la selección
    /// según cuántos clicks consecutivos cercanos hubo:
    /// **1** = caret simple · **2** = palabra · **3** = párrafo · **4+** =
    /// vuelve a caret. La racha se reinicia si pasó más de
    /// [`MULTI_CLICK_WINDOW`] desde el click anterior o si éste cayó lejos
    /// (otra línea, o más de un char de distancia). Reemplaza a
    /// [`Self::set_caret_at`] como handler de `PointerEvent::Click` cuando
    /// se quiere doble/triple click.
    pub fn register_click(&mut self, line: usize, col: usize) {
        let now = std::time::Instant::now();
        let pos = Pos::new(line, col);
        let n = match self.last_click {
            Some((t, p))
                if now.duration_since(t) <= MULTI_CLICK_WINDOW
                    && p.line == pos.line
                    && p.col.abs_diff(pos.col) <= 1 =>
            {
                self.click_count = self.click_count % 3 + 1;
                self.click_count
            }
            _ => {
                self.click_count = 1;
                1
            }
        };
        self.last_click = Some((now, pos));
        match n {
            2 => self.select_word_at(line, col),
            3 => self.select_paragraph_at(line, col),
            _ => self.set_caret_at(line, col),
        }
    }

    /// `true` si la línea `line` figura en `guard_lines`.
    pub fn is_guard_line(&self, line: usize) -> bool {
        self.guard_lines.binary_search(&line).is_ok()
    }

    /// Reemplaza la lista de líneas-guarda. La entrada se ordena y
    /// deduplica — el caller puede pasarlas en cualquier orden. Tras
    /// el cambio NO se snappea el caret automáticamente: si tu nueva
    /// lista deja al caret sobre una guarda, llamá a
    /// [`Self::snap_off_guards`] explícitamente.
    pub fn set_guard_lines(&mut self, mut lines: Vec<usize>) {
        lines.sort_unstable();
        lines.dedup();
        self.guard_lines = lines;
    }

    /// Salta el primary cursor + extras fuera de cualquier línea
    /// guarda. `dir` orienta la búsqueda: `+1` busca primero abajo,
    /// `-1` arriba, `0` igual a `+1` (con fallback al opuesto).
    /// No-op si `guard_lines` está vacío.
    pub fn snap_off_guards(&mut self, dir: i32) {
        if self.guard_lines.is_empty() {
            return;
        }
        snap_cursor_off_guard(&mut self.cursor, &self.buffer, &self.guard_lines, dir);
        for c in &mut self.extra_cursors {
            snap_cursor_off_guard(c, &self.buffer, &self.guard_lines, dir);
        }
    }

    /// Extiende la selección hasta `(line, col)`. Si no había anchor,
    /// lo planta en el caret actual antes de mover. Usado por drag del
    /// mouse: cada `Move` del drag llama esto con la nueva pos.
    pub fn extend_selection_to(&mut self, line: usize, col: usize) {
        let line = line.min(self.buffer.len_lines().saturating_sub(1));
        let col = col.min(self.buffer.line_len_chars(line));
        if self.cursor.anchor.is_none() {
            self.cursor.anchor = Some(self.cursor.caret);
        }
        self.cursor.caret = Pos::new(line, col);
        self.cursor.desired_col = col;
    }

    /// Selecciona todo el buffer: anchor en `(0,0)`, caret al final de
    /// la última línea. Colapsa los multi-cursor extras. Operación de
    /// sólo-cursor (no edita) — la usan el menú de edición y Ctrl+A.
    pub fn select_all(&mut self) {
        self.collapse_to_primary();
        let last_line = self.buffer.len_lines().saturating_sub(1);
        let last_col = self.buffer.line_len_chars(last_line);
        self.cursor.anchor = Some(Pos::ORIGIN);
        self.cursor.caret = Pos::new(last_line, last_col);
        self.cursor.desired_col = last_col;
    }

    /// `true` si hay algo que deshacer (para habilitar "Deshacer" en el
    /// menú de edición).
    pub fn can_undo(&self) -> bool {
        self.undo.can_undo()
    }

    /// `true` si hay algo que rehacer.
    pub fn can_redo(&self) -> bool {
        self.undo.can_redo()
    }

    /// `true` si hay una selección no-vacía (para habilitar Cortar/
    /// Copiar/Eliminar en el menú de edición).
    pub fn has_selection(&self) -> bool {
        self.cursor.has_selection()
    }

    /// Texto seleccionado, si hay selección no-vacía. `None` cuando el
    /// cursor está colapsado.
    pub fn selected_text(&self) -> Option<String> {
        if !self.cursor.has_selection() {
            return None;
        }
        let (s, e) = self.cursor.selection_range(&self.buffer);
        if s == e {
            return None;
        }
        Some(self.buffer.slice(s, e))
    }

    /// Resultado: `Changed` si la tecla modificó el buffer o el cursor;
    /// `Ignored` si la tecla no aplica al editor. Útil para que el
    /// caller decida si rebuildear el view.
    ///
    /// Copy/cut/paste (Ctrl+C/X/V) son ignorados — para habilitarlos,
    /// usá [`Self::apply_key_with_clipboard`] pasando un backend.
    pub fn apply_key(&mut self, event: &KeyEvent) -> ApplyResult {
        self.apply_key_with_clipboard(event, &mut NullClipboard)
    }

    /// Aplica un evento de IME al editor — el camino por el que llegan los
    /// acentos compuestos (dead keys), CJK y el emoji picker cuando la app
    /// habilita `App::ime_allowed`. El flujo del IME es
    /// `Enabled → Preedit* → Commit | Disabled`:
    ///
    /// - `Enabled`: el IME tomó el foco; no hay nada que insertar aún.
    /// - `Preedit{text,..}`: texto en composición. Lo guardamos en
    ///   [`Self::preedit`] para que el view lo pinte subrayado en el caret
    ///   **sin** tocar el buffer. Un `Preedit` con `text` vacío cierra la
    ///   composición sin confirmar.
    /// - `Commit(text)`: texto confirmado. Limpiamos el preedit e
    ///   insertamos `text` en todos los cursores, exactamente como si se
    ///   hubiera tecleado (mismo camino de undo + parseo incremental).
    /// - `Disabled`: el IME soltó el foco; descartamos cualquier
    ///   composición pendiente.
    ///
    /// Devuelve [`ApplyResult::Changed`] sólo en el `Commit` no-vacío (lo
    /// único que persiste); el resto es `CursorMoved` (hubo que
    /// redibujar el preedit) o `Ignored`.
    pub fn apply_ime_event(&mut self, event: &ImeEvent) -> ApplyResult {
        match event {
            ImeEvent::Enabled => ApplyResult::Ignored,
            ImeEvent::Preedit { text, cursor } => {
                let had = self.preedit.is_some();
                if text.is_empty() {
                    self.preedit = None;
                    if had { ApplyResult::CursorMoved } else { ApplyResult::Ignored }
                } else {
                    self.preedit = Some(Preedit { text: text.clone(), cursor: *cursor });
                    ApplyResult::CursorMoved
                }
            }
            ImeEvent::Commit(text) => {
                let had = self.preedit.take().is_some();
                if text.is_empty() {
                    return if had { ApplyResult::CursorMoved } else { ApplyResult::Ignored };
                }
                let text = text.clone();
                let changed =
                    self.apply_edit_all(|b, c, _opts| Some(replace_selection(b, c, &text)));
                if changed {
                    self.bump_edit_seq();
                    ApplyResult::Changed
                } else {
                    ApplyResult::Ignored
                }
            }
            ImeEvent::Disabled => {
                let had = self.preedit.take().is_some();
                if had { ApplyResult::CursorMoved } else { ApplyResult::Ignored }
            }
        }
    }

    /// Como [`Self::apply_key`] pero con backend de clipboard activo:
    /// Ctrl+C copia la selección, Ctrl+X la corta, Ctrl+V pega lo que
    /// haya en el clipboard.
    pub fn apply_key_with_clipboard(
        &mut self,
        event: &KeyEvent,
        clipboard: &mut dyn Clipboard,
    ) -> ApplyResult {
        // Antes de aplicar la tecla guardamos la línea del primary
        // cursor: si la edición/movimiento termina parando en una
        // guarda, la dirección del salto es la diferencia
        // post-pre. Up → snap arriba, Down → snap abajo, click/edit
        // en el mismo sitio → snap abajo por default.
        let pre_line = self.cursor.caret.line as i32;
        let r = self.apply_key_inner(event, clipboard);
        if r.changed() {
            self.bump_edit_seq();
        }
        if r.touched() && !self.guard_lines.is_empty() && !self.cursor.has_selection() {
            // Si hay selección viva (shift+arrow / drag) no snappeamos:
            // el usuario está seleccionando a través de la guarda y
            // forzar el caret afuera rompería la selección.
            let dir = (self.cursor.caret.line as i32 - pre_line).signum();
            self.snap_off_guards(dir);
        }
        r
    }

    fn apply_key_inner(
        &mut self,
        event: &KeyEvent,
        clipboard: &mut dyn Clipboard,
    ) -> ApplyResult {
        if event.state != KeyState::Pressed {
            return ApplyResult::Ignored;
        }
        let extending = event.modifiers.shift;
        let ctrl = event.modifiers.ctrl || event.modifiers.meta;
        let alt = event.modifiers.alt;

        // Esc colapsa multi-cursor (sin extras = ignorado, el caller
        // decide qué más hacer — cancelar edit, cerrar find, etc.).
        if matches!(&event.key, Key::Named(NamedKey::Escape)) {
            if self.has_multi_cursor() {
                self.collapse_to_primary();
                return ApplyResult::CursorMoved;
            }
            return ApplyResult::Ignored;
        }

        // Multi-cursor: Ctrl+Alt+ArrowDown/Up agrega un cursor en la
        // línea siguiente/anterior usando la misma desired_col. Esc del
        // caller debería colapsar — no lo manejamos acá porque el caller
        // puede querer usar Esc para otras cosas (cerrar find, cancelar
        // edit). El caller chequea has_multi_cursor() antes.
        if ctrl && alt {
            match &event.key {
                Key::Named(NamedKey::ArrowDown) => {
                    let line = self.cursor.caret.line + 1;
                    if line < self.buffer.len_lines() {
                        self.add_cursor_at(line, self.cursor.desired_col);
                        return ApplyResult::CursorMoved;
                    }
                    return ApplyResult::Ignored;
                }
                Key::Named(NamedKey::ArrowUp) => {
                    if self.cursor.caret.line > 0 {
                        self.add_cursor_at(self.cursor.caret.line - 1, self.cursor.desired_col);
                        return ApplyResult::CursorMoved;
                    }
                    return ApplyResult::Ignored;
                }
                _ => {}
            }
        }

        let page = self.options.page_size;
        match &event.key {
            // Movimiento
            Key::Named(NamedKey::ArrowLeft) => {
                if ctrl {
                    self.apply_move_all(|b, c| c.move_word_left(b, extending));
                } else {
                    self.apply_move_all(|b, c| c.move_left(b, extending));
                }
                ApplyResult::CursorMoved
            }
            Key::Named(NamedKey::ArrowRight) => {
                if ctrl {
                    self.apply_move_all(|b, c| c.move_word_right(b, extending));
                } else {
                    self.apply_move_all(|b, c| c.move_right(b, extending));
                }
                ApplyResult::CursorMoved
            }
            Key::Named(NamedKey::ArrowUp) => {
                self.apply_move_all(|b, c| c.move_up(b, extending));
                ApplyResult::CursorMoved
            }
            Key::Named(NamedKey::ArrowDown) => {
                self.apply_move_all(|b, c| c.move_down(b, extending));
                ApplyResult::CursorMoved
            }
            Key::Named(NamedKey::Home) => {
                if ctrl {
                    self.apply_move_all(|b, c| c.move_doc_start(b, extending));
                } else {
                    self.apply_move_all(|b, c| c.move_home(b, extending));
                }
                ApplyResult::CursorMoved
            }
            Key::Named(NamedKey::End) => {
                if ctrl {
                    self.apply_move_all(|b, c| c.move_doc_end(b, extending));
                } else {
                    self.apply_move_all(|b, c| c.move_end(b, extending));
                }
                ApplyResult::CursorMoved
            }
            Key::Named(NamedKey::PageUp) => {
                self.apply_move_all(|b, c| c.move_page_up(b, extending, page));
                ApplyResult::CursorMoved
            }
            Key::Named(NamedKey::PageDown) => {
                self.apply_move_all(|b, c| c.move_page_down(b, extending, page));
                ApplyResult::CursorMoved
            }

            // Edición
            Key::Named(NamedKey::Enter) => {
                if self.options.single_line {
                    return ApplyResult::Ignored;
                }
                self.apply_edit_all(|b, c, _opts| Some(insert_newline_auto_indent(b, c)));
                ApplyResult::Changed
            }
            Key::Named(NamedKey::Backspace) => {
                let changed = if ctrl {
                    self.apply_edit_all(|b, c, _opts| delete_word_backward(b, c))
                } else {
                    self.apply_edit_all(|b, c, _opts| delete_backward(b, c))
                };
                if changed {
                    ApplyResult::Changed
                } else {
                    ApplyResult::Ignored
                }
            }
            Key::Named(NamedKey::Delete) => {
                let changed = if ctrl {
                    self.apply_edit_all(|b, c, _opts| delete_word_forward(b, c))
                } else {
                    self.apply_edit_all(|b, c, _opts| delete_forward(b, c))
                };
                if changed {
                    ApplyResult::Changed
                } else {
                    ApplyResult::Ignored
                }
            }
            Key::Named(NamedKey::Tab) => {
                let any = if extending {
                    self.apply_edit_all(|b, c, opts| {
                        dedent(b, c, opts.tab_to_spaces, opts.indent_size)
                    })
                } else {
                    self.apply_edit_all(|b, c, opts| {
                        Some(indent_or_insert_tab(b, c, opts.tab_to_spaces, opts.indent_size))
                    })
                };
                if any { ApplyResult::Changed } else { ApplyResult::Ignored }
            }

            // Clipboard
            Key::Character(s) if ctrl && s.as_str().eq_ignore_ascii_case("c") => {
                if let Some(text) = self.selected_text() {
                    clipboard.set(&text);
                    ApplyResult::CursorMoved
                } else {
                    ApplyResult::Ignored
                }
            }
            Key::Character(s) if ctrl && s.as_str().eq_ignore_ascii_case("x") => {
                if let Some(text) = self.selected_text() {
                    clipboard.set(&text);
                    let d = replace_selection(&mut self.buffer, &mut self.cursor, "");
                    self.undo.push(d);
                    ApplyResult::Changed
                } else {
                    ApplyResult::Ignored
                }
            }
            Key::Character(s) if ctrl && s.as_str().eq_ignore_ascii_case("v") => {
                let Some(text) = clipboard.get() else {
                    return ApplyResult::Ignored;
                };
                if text.is_empty() {
                    return ApplyResult::Ignored;
                }
                // En single-line, los `\n` del clipboard se aplanan.
                let to_insert = if self.options.single_line {
                    text.replace(['\n', '\r'], " ")
                } else {
                    text
                };
                let d = replace_selection(&mut self.buffer, &mut self.cursor, &to_insert);
                self.undo.push(d);
                ApplyResult::Changed
            }

            // Undo / Redo
            Key::Character(s) if ctrl && s.as_str().eq_ignore_ascii_case("z") => {
                let did = if extending {
                    self.undo.redo(&mut self.buffer, &mut self.cursor)
                } else {
                    self.undo.undo(&mut self.buffer, &mut self.cursor)
                };
                if did { ApplyResult::Changed } else { ApplyResult::Ignored }
            }
            Key::Character(s) if ctrl && s.as_str().eq_ignore_ascii_case("y") => {
                let did = self.undo.redo(&mut self.buffer, &mut self.cursor);
                if did { ApplyResult::Changed } else { ApplyResult::Ignored }
            }

            // Inserción de chars imprimibles vía event.text (respeta IME +
            // layouts no-US). Ignoramos cuando ctrl/meta están activos
            // para no comernos Ctrl+S, Ctrl+C, etc. (eso lo hace el
            // caller registrando shortcuts).
            _ => {
                if ctrl {
                    return ApplyResult::Ignored;
                }
                let Some(text) = event.text.as_ref() else {
                    return ApplyResult::Ignored;
                };
                if text.is_empty() || text.chars().any(|c| c.is_control()) {
                    return ApplyResult::Ignored;
                }
                let text = text.clone();
                self.apply_edit_all(|b, c, _opts| Some(replace_selection(b, c, &text)));
                ApplyResult::Changed
            }
        }
    }

    // ----- Multi-cursor helpers -----

    /// Aplica un movimiento (no edita el buffer) a todos los cursores:
    /// primary + extras. Después dedupa para evitar cursores que terminan
    /// en el mismo punto.
    fn apply_move_all<F>(&mut self, mut f: F)
    where
        F: FnMut(&Buffer, &mut Cursor),
    {
        f(&self.buffer, &mut self.cursor);
        for c in &mut self.extra_cursors {
            f(&self.buffer, c);
        }
        self.dedupe_cursors();
    }

    /// Aplica una edición (que puede modificar el buffer) a todos los
    /// cursores. Procesa en orden de offset descendente para que las
    /// ediciones tempranas no desplacen las posiciones de las
    /// posteriores. Devuelve `true` si al menos uno produjo un delta.
    /// Cada delta también genera un `tree_sitter::InputEdit` que va a
    /// `pending_input_edits` para alimentar el incremental parsing.
    fn apply_edit_all<F>(&mut self, mut f: F) -> bool
    where
        F: FnMut(&mut Buffer, &mut Cursor, &EditorOptions) -> Option<crate::ops::EditDelta>,
    {
        let mut all: Vec<(Option<usize>, usize)> = Vec::with_capacity(1 + self.extra_cursors.len());
        let p_off = self.buffer.pos_to_offset(self.cursor.caret.line, self.cursor.caret.col);
        all.push((None, p_off));
        for (i, c) in self.extra_cursors.iter().enumerate() {
            let off = self.buffer.pos_to_offset(c.caret.line, c.caret.col);
            all.push((Some(i), off));
        }
        all.sort_by_key(|(_, off)| std::cmp::Reverse(*off));

        let opts = self.options;
        let mut any = false;
        for (which, _) in all {
            let cursor: &mut Cursor = match which {
                None => &mut self.cursor,
                Some(i) => &mut self.extra_cursors[i],
            };
            // Pre-edit positions del start del delta — necesitamos las
            // coordenadas BYTE del buffer ANTES de la edición.
            let start_char = self.buffer.pos_to_offset(cursor.caret.line, cursor.caret.col);
            // Pero si hay selección, el start real es el min de la sel.
            let (sel_start, _) = cursor.selection_range(&self.buffer);
            let start_char = start_char.min(sel_start);
            let start_byte = self.buffer.char_to_byte(start_char);
            let start_line = self.buffer.char_to_line(start_char);
            let start_col_byte = start_byte - self.buffer.line_to_byte(start_line);
            let pre_pt = tree_sitter::Point { row: start_line, column: start_col_byte };

            if let Some(d) = f(&mut self.buffer, cursor, &opts) {
                let edit = compute_input_edit(start_byte, pre_pt, &d);
                self.pending_input_edits.borrow_mut().push(edit);
                self.undo.push(d);
                any = true;
            }
        }
        self.dedupe_cursors();
        any
    }

    /// Elimina cursores extras que están en la misma posición que el
    /// primary o que otros extras (después de una edición pueden
    /// converger).
    fn dedupe_cursors(&mut self) {
        let primary = self.cursor.caret;
        let mut seen: Vec<Pos> = vec![primary];
        self.extra_cursors.retain(|c| {
            if seen.contains(&c.caret) {
                false
            } else {
                seen.push(c.caret);
                true
            }
        });
    }
}

/// Si `cursor.caret.line` cae sobre una línea presente en `guards`,
/// mueve el caret a la línea no-guarda más cercana siguiendo `dir`:
///
/// - `dir > 0` → busca primero abajo, luego arriba.
/// - `dir < 0` → busca primero arriba, luego abajo.
/// - `dir == 0` → equivalente a `dir > 0`.
///
/// Colapsa la selección y reposiciona el `col` clampeado al ancho de
/// la línea destino. `guards` debe estar ordenado ascendente; el
/// chequeo usa `binary_search`. Si TODAS las líneas son guardas, no
/// puede hacer nada y el caret queda donde está.
fn snap_cursor_off_guard(
    cursor: &mut Cursor,
    buffer: &Buffer,
    guards: &[usize],
    dir: i32,
) {
    let n = buffer.len_lines();
    if n == 0 || guards.is_empty() {
        return;
    }
    let line = cursor.caret.line.min(n - 1);
    if guards.binary_search(&line).is_err() {
        return;
    }
    // Orden de búsqueda: primero la dirección preferida, luego la opuesta.
    let primary: i32 = if dir < 0 { -1 } else { 1 };
    let secondary: i32 = -primary;
    for d in [primary, secondary] {
        let mut probe = line as i32 + d;
        while probe >= 0 && (probe as usize) < n {
            let p = probe as usize;
            if guards.binary_search(&p).is_err() {
                let col = cursor.desired_col.min(buffer.line_len_chars(p));
                cursor.caret = Pos::new(p, col);
                cursor.anchor = None;
                return;
            }
            probe += d;
        }
    }
    // Todas las líneas son guardas — no podemos hacer nada útil.
}

/// Convierte un `EditDelta` + posiciones pre-edit a un `InputEdit` de
/// tree-sitter. tree-sitter trabaja en bytes y `Point { row, column_byte }`;
/// el editor trabaja en chars (y col_byte para esto).
///
/// `start_byte` y `start_point` son las coords del inicio del delta
/// ANTES del cambio (el caller las captura).
fn compute_input_edit(
    start_byte: usize,
    start_point: tree_sitter::Point,
    delta: &crate::ops::EditDelta,
) -> tree_sitter::InputEdit {
    let removed_bytes = delta.removed.len();
    let inserted_bytes = delta.inserted.len();

    let old_end_byte = start_byte + removed_bytes;
    let new_end_byte = start_byte + inserted_bytes;

    let old_end_point = advance_point(start_point, &delta.removed);
    let new_end_point = advance_point(start_point, &delta.inserted);

    tree_sitter::InputEdit {
        start_byte,
        old_end_byte,
        new_end_byte,
        start_position: start_point,
        old_end_position: old_end_point,
        new_end_position: new_end_point,
    }
}

/// Avanza un Point por el contenido de `text`: cuenta `\n` para filas,
/// bytes de la última línea para columna.
fn advance_point(start: tree_sitter::Point, text: &str) -> tree_sitter::Point {
    let newlines = text.bytes().filter(|b| *b == b'\n').count();
    if newlines == 0 {
        tree_sitter::Point {
            row: start.row,
            column: start.column + text.len(),
        }
    } else {
        let after_last_nl = text.rsplit('\n').next().unwrap_or("").len();
        tree_sitter::Point {
            row: start.row + newlines,
            column: after_last_nl,
        }
    }
}

/// Resultado de `apply_key`. El caller usa esto para decidir si
/// rebuildear el view o ignorar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyResult {
    /// La tecla cambió el buffer (o sea, hay edición persistible).
    Changed,
    /// Sólo se movió el cursor — el view se redibuja, pero el `source`
    /// del notebook no cambia.
    CursorMoved,
    /// La tecla no aplicaba al editor.
    Ignored,
}

impl ApplyResult {
    pub fn changed(self) -> bool {
        matches!(self, ApplyResult::Changed)
    }
    pub fn touched(self) -> bool {
        !matches!(self, ApplyResult::Ignored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llimphi_ui::Modifiers;

    fn ev(named: NamedKey, shift: bool, ctrl: bool) -> KeyEvent {
        KeyEvent {
            key: Key::Named(named),
            state: KeyState::Pressed,
            text: None,
            modifiers: Modifiers { shift, ctrl, alt: false, meta: false },
            repeat: false,
        }
    }
    fn evtext(s: &str, shift: bool, ctrl: bool) -> KeyEvent {
        KeyEvent {
            key: Key::Character(s.into()),
            state: KeyState::Pressed,
            text: Some(s.to_owned()),
            modifiers: Modifiers { shift, ctrl, alt: false, meta: false },
            repeat: false,
        }
    }

    #[test]
    fn escribir_chars_inserta() {
        let mut s = EditorState::new();
        s.apply_key(&evtext("h", false, false));
        s.apply_key(&evtext("i", false, false));
        assert_eq!(s.text(), "hi");
    }

    #[test]
    fn register_click_cuenta_simple_doble_triple() {
        let mut s = EditorState::new();
        s.set_text("hola mundo\notro\n\nparrafo dos");

        // Click simple en "mundo": caret, sin selección.
        s.register_click(0, 7);
        assert!(!s.cursor.has_selection());

        // Doble click (mismo punto, dentro de la ventana): palabra.
        s.register_click(0, 7);
        let (a, b) = s.cursor.selection_range(&s.buffer);
        assert_eq!(s.buffer.slice(a, b), "mundo");

        // Triple click: párrafo (bloque de líneas no vacías).
        s.register_click(0, 7);
        let (a, b) = s.cursor.selection_range(&s.buffer);
        assert_eq!(s.buffer.slice(a, b), "hola mundo\notro");

        // Cuarto click: vuelve a caret simple.
        s.register_click(0, 7);
        assert!(!s.cursor.has_selection());
    }

    #[test]
    fn register_click_lejos_reinicia_a_simple() {
        let mut s = EditorState::new();
        s.set_text("hola mundo");
        s.register_click(0, 1);
        // Segundo click en otra columna lejana → racha reiniciada, no palabra.
        s.register_click(0, 8);
        assert!(!s.cursor.has_selection());
    }

    #[test]
    fn enter_con_indent_auto() {
        let mut s = EditorState::new();
        s.set_text("    hola");
        s.cursor = Cursor::at(0, 8);
        s.apply_key(&ev(NamedKey::Enter, false, false));
        assert_eq!(s.text(), "    hola\n    ");
    }

    #[test]
    fn enter_en_single_line_ignorado() {
        let mut s = EditorState::with_options(EditorOptions {
            single_line: true,
            ..Default::default()
        });
        s.set_text("a");
        s.cursor = Cursor::at(0, 1);
        let r = s.apply_key(&ev(NamedKey::Enter, false, false));
        assert_eq!(r, ApplyResult::Ignored);
        assert_eq!(s.text(), "a");
    }

    #[test]
    fn tab_inserta_indent() {
        let mut s = EditorState::new();
        s.apply_key(&ev(NamedKey::Tab, false, false));
        assert_eq!(s.text(), "  "); // indent_size por defecto = 2
    }

    #[test]
    fn shift_tab_dedenta() {
        let mut s = EditorState::new();
        s.set_text("    hola");
        s.cursor = Cursor::at(0, 4);
        s.apply_key(&ev(NamedKey::Tab, true, false));
        // indent_size=2 → quita 2 espacios
        assert_eq!(s.text(), "  hola");
    }

    #[test]
    fn ctrl_z_y_ctrl_y_son_undo_redo() {
        let mut s = EditorState::new();
        s.apply_key(&evtext("a", false, false));
        s.apply_key(&evtext("b", false, false));
        assert_eq!(s.text(), "ab");
        s.apply_key(&evtext("z", false, true));
        assert_eq!(s.text(), "a");
        s.apply_key(&evtext("y", false, true));
        assert_eq!(s.text(), "ab");
    }

    #[test]
    fn ctrl_shift_z_es_redo() {
        let mut s = EditorState::new();
        s.apply_key(&evtext("a", false, false));
        s.apply_key(&evtext("z", false, true));
        assert!(s.is_empty());
        s.apply_key(&evtext("z", true, true));
        assert_eq!(s.text(), "a");
    }

    #[test]
    fn ctrl_arrow_left_salta_palabra() {
        let mut s = EditorState::new();
        s.set_text("hola mundo");
        s.cursor = Cursor::at(0, 10);
        s.apply_key(&ev(NamedKey::ArrowLeft, false, true));
        assert_eq!(s.cursor.caret, Pos::new(0, 5)); // inicio de "mundo"
        s.apply_key(&ev(NamedKey::ArrowLeft, false, true));
        assert_eq!(s.cursor.caret, Pos::new(0, 0)); // inicio de "hola"
    }

    #[test]
    fn shift_arrow_selecciona_y_chars_reemplazan() {
        let mut s = EditorState::new();
        s.set_text("abc");
        s.cursor = Cursor::at(0, 0);
        s.apply_key(&ev(NamedKey::ArrowRight, true, false));
        s.apply_key(&ev(NamedKey::ArrowRight, true, false));
        assert!(s.cursor.has_selection());
        s.apply_key(&evtext("X", false, false));
        assert_eq!(s.text(), "Xc");
    }

    #[test]
    fn ctrl_chars_se_ignoran_en_input_normal() {
        // Ctrl+S no debería insertar "s".
        let mut s = EditorState::new();
        let r = s.apply_key(&evtext("s", false, true));
        assert_eq!(r, ApplyResult::Ignored);
        assert!(s.is_empty());
    }

    #[test]
    fn ctrl_c_copia_la_seleccion_al_clipboard() {
        use crate::clipboard::MemClipboard;
        let mut s = EditorState::new();
        s.set_text("hola mundo");
        s.cursor = Cursor {
            anchor: Some(Pos::new(0, 0)),
            caret: Pos::new(0, 4),
            desired_col: 4,
        };
        let mut clip = MemClipboard::new();
        let r = s.apply_key_with_clipboard(&evtext("c", false, true), &mut clip);
        assert_eq!(r, ApplyResult::CursorMoved);
        assert_eq!(clip.get().as_deref(), Some("hola"));
        // El buffer no cambia.
        assert_eq!(s.text(), "hola mundo");
    }

    #[test]
    fn ctrl_x_corta_y_borra() {
        use crate::clipboard::MemClipboard;
        let mut s = EditorState::new();
        s.set_text("hola mundo");
        s.cursor = Cursor {
            anchor: Some(Pos::new(0, 0)),
            caret: Pos::new(0, 5),
            desired_col: 5,
        };
        let mut clip = MemClipboard::new();
        let r = s.apply_key_with_clipboard(&evtext("x", false, true), &mut clip);
        assert_eq!(r, ApplyResult::Changed);
        assert_eq!(clip.get().as_deref(), Some("hola "));
        assert_eq!(s.text(), "mundo");
    }

    #[test]
    fn ctrl_v_pega_en_el_caret() {
        use crate::clipboard::MemClipboard;
        let mut s = EditorState::new();
        s.set_text("ab");
        s.cursor = Cursor::at(0, 1);
        let mut clip = MemClipboard::with("XYZ");
        s.apply_key_with_clipboard(&evtext("v", false, true), &mut clip);
        assert_eq!(s.text(), "aXYZb");
    }

    #[test]
    fn ctrl_v_aplana_newlines_en_single_line() {
        use crate::clipboard::MemClipboard;
        let mut s = EditorState::with_options(EditorOptions {
            single_line: true,
            ..Default::default()
        });
        let mut clip = MemClipboard::with("a\nb\nc");
        s.apply_key_with_clipboard(&evtext("v", false, true), &mut clip);
        assert_eq!(s.text(), "a b c");
    }

    #[test]
    fn ensure_caret_visible_scrollea_hacia_abajo() {
        let mut s = EditorState::new();
        let lines: String = (0..100).map(|n| format!("line {n}\n")).collect();
        s.set_text(&lines);
        s.cursor = Cursor::at(50, 0);
        s.ensure_caret_visible(20);
        // Caret en línea 50, visible_lines = 20 → scroll = 50 - 19 = 31.
        assert_eq!(s.scroll_offset, 31);
        // El caret debe estar dentro del viewport.
        assert!(s.cursor.caret.line >= s.scroll_offset);
        assert!(s.cursor.caret.line < s.scroll_offset + 20);
    }

    #[test]
    fn ensure_caret_visible_scrollea_hacia_arriba() {
        let mut s = EditorState::new();
        let lines: String = (0..100).map(|n| format!("line {n}\n")).collect();
        s.set_text(&lines);
        s.scroll_offset = 50;
        s.cursor = Cursor::at(5, 0);
        s.ensure_caret_visible(20);
        assert_eq!(s.scroll_offset, 5);
    }

    #[test]
    fn ensure_caret_visible_no_mueve_si_ya_visible() {
        let mut s = EditorState::new();
        let lines: String = (0..50).map(|n| format!("line {n}\n")).collect();
        s.set_text(&lines);
        s.scroll_offset = 10;
        s.cursor = Cursor::at(15, 0);
        s.ensure_caret_visible(20);
        assert_eq!(s.scroll_offset, 10);
    }

    #[test]
    fn input_edits_se_acumulan_y_drenan_en_highlight() {
        use crate::highlight::Language;
        let mut s = EditorState::new();
        s.set_text("fn main() {}");
        // Set_text invalida pero NO pushea InputEdit (es replace_all).
        // Después de una edit normal, sí debería haber 1 pending.
        s.cursor = Cursor::at(0, 12);
        s.apply_key(&evtext("x", false, false));
        assert_eq!(s.pending_input_edits.borrow().len(), 1);
        // El parse drena los pending.
        let _ = s.highlighted_spans(Language::Rust);
        assert!(s.pending_input_edits.borrow().is_empty());
    }

    #[test]
    fn input_edit_multilinea_calcula_rows_correctamente() {
        let mut s = EditorState::new();
        s.set_text("ab");
        s.cursor = Cursor::at(0, 2);
        s.apply_key(&ev(NamedKey::Enter, false, false));
        let edits = s.pending_input_edits.borrow().clone();
        assert_eq!(edits.len(), 1);
        let e = &edits[0];
        // Insertó "\n" (auto-indent vacío porque no había indent) →
        // new_end_position debe estar en row=1, col=0.
        assert_eq!(e.start_byte, 2);
        assert_eq!(e.new_end_position.row, 1);
        assert_eq!(e.new_end_position.column, 0);
    }

    #[test]
    fn edit_seq_se_incrementa_solo_con_cambios() {
        let mut s = EditorState::new();
        let seq0 = s.edit_seq;
        s.apply_key(&ev(NamedKey::ArrowRight, false, false)); // CursorMoved
        assert_eq!(s.edit_seq, seq0, "movimiento no debería bumpear");
        s.apply_key(&evtext("a", false, false)); // Changed
        assert!(s.edit_seq > seq0);
    }

    #[test]
    fn highlight_cache_reuse_cuando_seq_no_cambia() {
        use crate::highlight::Language;
        let mut s = EditorState::new();
        s.set_text("fn main() {}");
        let _ = s.highlighted_spans(Language::Rust);
        let seq_before = s.edit_seq;
        let _ = s.highlighted_spans(Language::Rust);
        // Sin edición → seq igual → cache hit (no asserción directa
        // posible sin mock, pero al menos el seq no cambia).
        assert_eq!(s.edit_seq, seq_before);
    }

    #[test]
    fn multi_cursor_insert_aplica_a_todos() {
        let mut s = EditorState::new();
        s.set_text("ab\ncd\nef");
        // Cursor primary al final de "ab", extras al final de "cd" y "ef".
        s.cursor = Cursor::at(0, 2);
        s.add_cursor_at(1, 2);
        s.add_cursor_at(2, 2);
        s.apply_key(&evtext("!", false, false));
        assert_eq!(s.text(), "ab!\ncd!\nef!");
    }

    #[test]
    fn multi_cursor_backspace_aplica_a_todos() {
        let mut s = EditorState::new();
        s.set_text("ab\ncd\nef");
        s.cursor = Cursor::at(0, 2);
        s.add_cursor_at(1, 2);
        s.add_cursor_at(2, 2);
        s.apply_key(&ev(NamedKey::Backspace, false, false));
        assert_eq!(s.text(), "a\nc\ne");
    }

    #[test]
    fn dedupe_cursors_remueve_solapados() {
        let mut s = EditorState::new();
        s.set_text("abc");
        s.cursor = Cursor::at(0, 1);
        s.add_cursor_at(0, 1); // exacto primary → no se agrega
        s.add_cursor_at(0, 2);
        // El primer add no agregó nada; el segundo sí.
        assert_eq!(s.extra_cursors.len(), 1);
    }

    #[test]
    fn collapse_to_primary_descarta_extras() {
        let mut s = EditorState::new();
        s.set_text("abc");
        s.cursor = Cursor::at(0, 0);
        s.add_cursor_at(0, 1);
        s.add_cursor_at(0, 2);
        assert!(s.has_multi_cursor());
        s.collapse_to_primary();
        assert!(!s.has_multi_cursor());
    }

    #[test]
    fn highlight_cache_invalida_con_cambio_de_lenguaje() {
        use crate::highlight::Language;
        let mut s = EditorState::new();
        s.set_text("def f(): pass");
        let py = s.highlighted_spans(Language::Python);
        let rs = s.highlighted_spans(Language::Rust);
        // Distinto lenguaje → spans distintos (al menos el conteo o
        // las categorías difieren).
        assert!(py != rs || s.is_empty());
    }

    #[test]
    fn scroll_by_clampea_a_rango_valido() {
        let mut s = EditorState::new();
        let lines: String = (0..10).map(|n| format!("line {n}\n")).collect();
        s.set_text(&lines);
        s.scroll_by(-100);
        assert_eq!(s.scroll_offset, 0);
        s.scroll_by(1000);
        assert!(s.scroll_offset < 11);
    }

    fn estado_con_guardas(texto: &str, guards: Vec<usize>) -> EditorState {
        let mut s = EditorState::new();
        s.set_text(texto);
        s.set_guard_lines(guards);
        s
    }

    #[test]
    fn guarda_set_caret_at_en_linea_vacia_salta_hacia_abajo() {
        // "abc\n\ndef" → líneas: "abc", "", "def". La 1 es guarda.
        let mut s = estado_con_guardas("abc\n\ndef", vec![1]);
        s.set_caret_at(1, 0);
        // El caret no puede quedar en la línea 1 (guarda) — salta a 2.
        assert_eq!(s.cursor.caret, Pos::new(2, 0));
    }

    #[test]
    fn guarda_sin_linea_abajo_salta_arriba() {
        // Todas las líneas después de la 0 son guardas: el snap solo
        // puede ir hacia arriba.
        let mut s = estado_con_guardas("abc\n\n", vec![1, 2]);
        s.set_caret_at(1, 0);
        assert_eq!(s.cursor.caret.line, 0);
    }

    #[test]
    fn guarda_arrow_down_atraviesa_la_separacion() {
        let mut s = estado_con_guardas("abc\n\ndef", vec![1]);
        s.cursor = Cursor::at(0, 0);
        // Down debería terminar en línea 2, no en la 1 (guarda).
        s.apply_key(&ev(NamedKey::ArrowDown, false, false));
        assert_eq!(s.cursor.caret.line, 2);
    }

    #[test]
    fn guarda_arrow_up_atraviesa_la_separacion() {
        let mut s = estado_con_guardas("abc\n\ndef", vec![1]);
        s.cursor = Cursor::at(2, 1);
        s.apply_key(&ev(NamedKey::ArrowUp, false, false));
        assert_eq!(s.cursor.caret.line, 0);
    }

    #[test]
    fn set_text_limpia_guardas() {
        // Tras `set_text`, las guardas anteriores ya no son válidas:
        // el caller las repuebla. La función las limpia.
        let mut s = estado_con_guardas("abc\n\ndef", vec![1]);
        assert_eq!(s.guard_lines, vec![1]);
        s.set_text("nuevo");
        assert!(s.guard_lines.is_empty());
    }

    #[test]
    fn sin_guardas_set_caret_at_en_blank_se_queda() {
        // Con `guard_lines` vacío, comportamiento clásico: el caret
        // puede caer en cualquier línea sin snap.
        let mut s = EditorState::new();
        s.set_text("abc\n\ndef");
        s.set_caret_at(1, 0);
        assert_eq!(s.cursor.caret, Pos::new(1, 0));
    }

    #[test]
    fn guarda_shift_arrow_extiende_seleccion_a_traves() {
        // Con selección viva atravesando la guarda, NO snapear: el
        // usuario está seleccionando texto multi-zona.
        let mut s = estado_con_guardas("abc\n\ndef", vec![1]);
        s.cursor = Cursor::at(0, 3);
        s.apply_key(&ev(NamedKey::ArrowDown, true, false));
        // El caret puede quedar en la línea 1 mientras hay selección
        // viva — el snap se inhibe.
        assert!(s.cursor.has_selection());
        assert_eq!(s.cursor.caret.line, 1);
    }

    #[test]
    fn set_guard_lines_ordena_y_deduplica() {
        let mut s = EditorState::new();
        s.set_text("a\nb\nc\nd\ne");
        s.set_guard_lines(vec![3, 1, 1, 3]);
        assert_eq!(s.guard_lines, vec![1, 3]);
    }

    #[test]
    fn guarda_no_es_solo_blank_puede_ser_cualquiera() {
        // Una guarda no tiene que ser una línea vacía — el widget no
        // mira el contenido, sólo el índice. Una línea con texto
        // marcada como guarda igual repele al caret.
        let mut s = EditorState::new();
        s.set_text("aaa\nbbb\nccc");
        s.set_guard_lines(vec![1]); // línea "bbb" es guarda
        s.set_caret_at(1, 0);
        assert!(s.cursor.caret.line != 1);
    }

    #[test]
    fn ime_commit_inserta_como_tecleo() {
        // El caso español: el dead-key compone "á" y llega como Commit.
        let mut s = EditorState::new();
        s.apply_key(&evtext("c", false, false));
        let r = s.apply_ime_event(&ImeEvent::Commit("á".into()));
        assert_eq!(r, ApplyResult::Changed);
        assert_eq!(s.text(), "cá");
        assert!(s.preedit.is_none());
    }

    #[test]
    fn ime_preedit_no_toca_el_buffer() {
        let mut s = EditorState::new();
        s.set_text("ab");
        s.cursor = Cursor::at(0, 2);
        let r = s.apply_ime_event(&ImeEvent::Preedit { text: "´".into(), cursor: None });
        assert_eq!(r, ApplyResult::CursorMoved);
        // El buffer sigue intacto; el preedit vive aparte.
        assert_eq!(s.text(), "ab");
        assert_eq!(s.preedit.as_ref().map(|p| p.text.as_str()), Some("´"));
        // El Commit reemplaza la composición e inserta el char final.
        s.apply_ime_event(&ImeEvent::Commit("é".into()));
        assert_eq!(s.text(), "abé");
        assert!(s.preedit.is_none());
    }

    #[test]
    fn ime_preedit_vacio_y_disabled_cierran_la_composicion() {
        let mut s = EditorState::new();
        s.apply_ime_event(&ImeEvent::Preedit { text: "ㅎ".into(), cursor: None });
        assert!(s.preedit.is_some());
        // Un Preedit vacío cancela sin confirmar.
        let r = s.apply_ime_event(&ImeEvent::Preedit { text: String::new(), cursor: None });
        assert_eq!(r, ApplyResult::CursorMoved);
        assert!(s.preedit.is_none());
        // Disabled sobre algo en composición también limpia.
        s.apply_ime_event(&ImeEvent::Preedit { text: "ㅎ".into(), cursor: None });
        s.apply_ime_event(&ImeEvent::Disabled);
        assert!(s.preedit.is_none());
    }

    #[test]
    fn ime_commit_reemplaza_seleccion() {
        let mut s = EditorState::new();
        s.set_text("abc");
        s.cursor = Cursor {
            anchor: Some(Pos::new(0, 0)),
            caret: Pos::new(0, 2),
            desired_col: 2,
        };
        s.apply_ime_event(&ImeEvent::Commit("ñ".into()));
        assert_eq!(s.text(), "ñc");
    }

    #[test]
    fn ctrl_c_sin_seleccion_es_ignorado() {
        use crate::clipboard::MemClipboard;
        let mut s = EditorState::new();
        s.set_text("hola");
        s.cursor = Cursor::at(0, 4);
        let mut clip = MemClipboard::new();
        let r = s.apply_key_with_clipboard(&evtext("c", false, true), &mut clip);
        assert_eq!(r, ApplyResult::Ignored);
        assert!(clip.get().is_none());
    }
}
