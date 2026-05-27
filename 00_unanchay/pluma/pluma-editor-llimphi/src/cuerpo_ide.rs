//! `cuerpo_ide` — el text-editor IDE de Llimphi montado sobre un
//! [`pluma_editor_cuerpo::EditorCuerpo`].
//!
//! La página del editor multilienzo es **un solo control**: el usuario ve
//! todos los párrafos del cuerpo concatenados en un buffer plano editado
//! con el `text-editor` widget de Llimphi (cursor libre, multi-cursor,
//! undo/redo, find/replace, clipboard, highlight si lo activa el caller,
//! viewport scroll). Por debajo seguimos teniendo un grafo de
//! `NarrativeAtom`s con hebras vivas.
//!
//! Esta capa cose las dos cosas. Flujo típico:
//!
//!   1. [`CuerpoIde::from_cuerpo`] toma un `Cuerpo` + el índice de atoms
//!      y arma un [`EditorCuerpo`] (texto plano + Uuids en orden) y un
//!      [`EditorState`] del widget cargado con ese texto.
//!   2. El caller mete eventos de teclado vía [`CuerpoIde::apply_key`].
//!      El buffer queda desincronizado del `EditorCuerpo` (que sigue
//!      mostrando el texto original) hasta el próximo `diff`. La
//!      desincronía se detecta exactamente vía
//!      [`EditorState::edit_seq`] — ningún flag manual que se pueda
//!      perder si el caller toca el `state` por su cuenta.
//!   3. [`CuerpoIde::diff`] mete `state.text()` en
//!      `editor_cuerpo.texto` y devuelve la lista mínima de
//!      [`CambioAtom`] que el caller debe aplicar al grafo
//!      (mutar contenido / crear atom nuevo / eliminar uno que ya no
//!      aparece). Si el buffer no se tocó desde el último diff, retorna
//!      `vec![]` sin escanear nada.
//!   4. Tras persistir en el grafo (creando `NarrativeAtom`s reales para
//!      los `Crear`), el caller pasa los Uuids resultantes a
//!      [`CuerpoIde::aplicar_cambios`] para que el `atom_ids` del editor
//!      refleje el nuevo orden y la sincronía quede sellada.
//!
//! El widget no sabe ni necesita saber que el texto está particionado
//! en átomos: lo trata como un buffer único, con `\n\n` separando
//! párrafos. Esta capa SÍ lo sabe y expone helpers
//! [`CuerpoIde::posicion_de_atom`] / [`CuerpoIde::atom_id_en_linea`] que
//! traducen entre coordenadas del buffer y Uuids.

use std::collections::HashMap;

use llimphi_ui::View;
use llimphi_widget_text_editor::{
    text_editor_view_highlighted, ApplyResult, Clipboard, EditorMetrics, EditorOptions,
    EditorPalette, EditorState, Language, PointerEvent,
};
use pluma_core::NarrativeAtom;
use pluma_cuerpo::Cuerpo;
use pluma_editor_cuerpo::{CambioAtom, EditorCuerpo, SEPARADOR};
use uuid::Uuid;

// Re-exports — el caller importa todo desde `cuerpo_ide` sin tener que
// conocer la geografía interna de los dos crates que ensamblamos.
pub use llimphi_widget_text_editor::{
    ApplyResult as EditorApplyResult, EditorMetrics as IdeMetrics,
    EditorPalette as IdePalette, Language as IdeLanguage, PointerEvent as IdePointerEvent,
};
pub use pluma_editor_cuerpo::{CambioAtom as IdeCambio, SEPARADOR as SEPARADOR_PARRAFO};

/// Una página de edición: cuerpo plano + estado del text-editor.
///
/// Es `Clone` porque `EditorState` lo es; útil para snapshots a nivel
/// de aplicación (p.ej. "guardar como" sobre una copia, o un undo de
/// alto nivel que cubre operaciones sobre el grafo, no solo el buffer).
#[derive(Debug, Clone)]
pub struct CuerpoIde {
    /// Vista plana del cuerpo. `texto` se actualiza cuando el caller
    /// llama a [`Self::diff`]; mientras tanto la fuente de verdad
    /// editable es `state.buffer`.
    pub editor_cuerpo: EditorCuerpo,
    /// Buffer + cursor + undo + viewport del widget.
    pub state: EditorState,
    /// `state.edit_seq` cuando el `editor_cuerpo.texto` fue sincronizado
    /// por última vez con el buffer. El widget bumpea `edit_seq` con
    /// **toda** mutación del buffer (set_text, apply_key, etc.) — usar
    /// ese contador como marca evita el bug clásico de "flag bool que
    /// se olvida de bajarse" cuando el caller mete cambios por fuera de
    /// `apply_key`.
    seq_sincronizado: u64,
}

impl CuerpoIde {
    /// Construye una página vacía. Útil para callers que quieren cargar
    /// el cuerpo después con [`Self::recargar`] (p.ej. UI con `Option<…>`
    /// que arranca sin documento abierto).
    pub fn nuevo_vacio() -> Self {
        let state = EditorState::new();
        let seq = state.edit_seq;
        Self {
            editor_cuerpo: EditorCuerpo {
                texto: String::new(),
                atom_ids: Vec::new(),
            },
            state,
            seq_sincronizado: seq,
        }
    }

    /// Construye una página del IDE a partir de un `Cuerpo` + atoms del
    /// grafo. El `EditorState` queda cargado con el texto plano del
    /// cuerpo y el caret al final (convención de `EditorState::set_text`).
    pub fn from_cuerpo(cuerpo: &Cuerpo, atoms: &HashMap<Uuid, &NarrativeAtom>) -> Self {
        Self::con_opciones(cuerpo, atoms, EditorOptions::default())
    }

    /// Como [`Self::from_cuerpo`] pero permite pasar opciones del editor
    /// (tab → spaces, indent size, page size, single-line).
    pub fn con_opciones(
        cuerpo: &Cuerpo,
        atoms: &HashMap<Uuid, &NarrativeAtom>,
        options: EditorOptions,
    ) -> Self {
        let editor_cuerpo = EditorCuerpo::from_cuerpo(cuerpo, atoms);
        let mut state = EditorState::with_options(options);
        state.set_text(&editor_cuerpo.texto);
        let seq = state.edit_seq;
        Self {
            editor_cuerpo,
            state,
            seq_sincronizado: seq,
        }
    }

    /// Resetea el IDE a un nuevo cuerpo (útil cuando el caller cambia de
    /// pestaña / cuerpo activo). Limpia el undo del widget — semántica
    /// del `EditorState::set_text`. Conserva las opciones del editor.
    pub fn recargar(&mut self, cuerpo: &Cuerpo, atoms: &HashMap<Uuid, &NarrativeAtom>) {
        self.editor_cuerpo = EditorCuerpo::from_cuerpo(cuerpo, atoms);
        self.state.set_text(&self.editor_cuerpo.texto);
        self.seq_sincronizado = self.state.edit_seq;
    }

    /// `true` si el buffer del widget difiere del `editor_cuerpo.texto`
    /// memorizado — al menos una mutación tocó el contenido desde la
    /// última llamada a [`Self::diff`] o [`Self::recargar`].
    ///
    /// Derivado de `state.edit_seq`, así que es resistente a mutaciones
    /// del state por fuera de `apply_key` (p.ej. el caller llamó
    /// `state.set_text` por su cuenta).
    pub fn pendiente_sync(&self) -> bool {
        self.state.edit_seq != self.seq_sincronizado
    }

    /// Reenvía el evento a [`EditorState::apply_key`]. El tracking de
    /// `pendiente_sync` se mantiene automáticamente vía `edit_seq`.
    pub fn apply_key(&mut self, event: &llimphi_ui::KeyEvent) -> ApplyResult {
        self.state.apply_key(event)
    }

    /// Como [`Self::apply_key`] con backend de clipboard — habilita
    /// `Ctrl+C/X/V`.
    pub fn apply_key_with_clipboard(
        &mut self,
        event: &llimphi_ui::KeyEvent,
        clipboard: &mut dyn Clipboard,
    ) -> ApplyResult {
        self.state.apply_key_with_clipboard(event, clipboard)
    }

    /// Vuelca el texto del buffer en `editor_cuerpo.texto` (si hubo
    /// cambios) y devuelve el diff mínimo contra los atoms originales
    /// pasados por el caller. Si el buffer no se tocó desde el último
    /// `diff` / `recargar`, retorna `vec![]` sin escanear nada — es el
    /// path caliente de un `Ctrl+S` sobre un documento sin cambios.
    ///
    /// El caller suele recolectar `atoms_originales` del grafo justo
    /// antes — el editor no consulta el grafo por sí mismo.
    pub fn diff(&mut self, atoms_originales: &HashMap<Uuid, &NarrativeAtom>) -> Vec<CambioAtom> {
        if !self.pendiente_sync() {
            return Vec::new();
        }
        self.editor_cuerpo.set_texto(self.state.text());
        self.seq_sincronizado = self.state.edit_seq;
        self.editor_cuerpo.diff(atoms_originales)
    }

    /// Tras persistir los cambios en el grafo (creando `NarrativeAtom`s
    /// nuevos para los `Crear` y removiendo los `Eliminar`), pasá acá
    /// los Uuids generados para los `Crear`, **en orden**, y el
    /// `atom_ids` del editor queda alineado con el cuerpo nuevo.
    pub fn aplicar_cambios(&mut self, cambios: &[CambioAtom], nuevos_ids: &[Uuid]) {
        self.editor_cuerpo.aplicar_cambios(cambios, nuevos_ids);
    }

    /// Atajo retrocompatible — alias histórico de [`Self::aplicar_cambios`].
    #[inline]
    pub fn aplicar_cambios_locales(&mut self, cambios: &[CambioAtom], nuevos_ids: &[Uuid]) {
        self.aplicar_cambios(cambios, nuevos_ids);
    }

    /// Cuántos átomos cubre el cuerpo plano que el editor está mostrando
    /// (estado del último sync — puede diferir de los párrafos del
    /// buffer hasta el próximo `diff`).
    #[inline]
    pub fn n_atoms(&self) -> usize {
        self.editor_cuerpo.atom_ids.len()
    }

    /// Cantidad de párrafos en el buffer **ahora mismo** (puede diferir
    /// de [`Self::n_atoms`] si el usuario insertó/eliminó separadores
    /// desde el último sync). Útil para feedback en vivo del header.
    pub fn n_parrafos_buffer(&self) -> usize {
        // No clonamos el string — iteramos directo sobre el rope.
        let texto = self.state.text();
        texto
            .split(SEPARADOR)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .count()
    }

    /// Texto crudo del buffer del widget. Atajo de `state.text()` para
    /// callers que solo leen.
    #[inline]
    pub fn texto_buffer(&self) -> String {
        self.state.text()
    }

    /// Línea inicial (0-based) del átomo `id` en `editor_cuerpo.texto`.
    /// Camina los párrafos sumando `\n`s reales — robusto a átomos
    /// multi-línea (si el cuerpo guarda contenido con `\n` interno) y al
    /// número de newlines del [`SEPARADOR`].
    ///
    /// Devuelve `None` si el id no pertenece al cuerpo. La posición es
    /// exacta para el texto memorizado del último sync; si el caller
    /// editó el buffer y no llamó [`Self::diff`] aún, la posición es la
    /// del *cuerpo sincronizado*, no la del buffer vivo.
    pub fn posicion_de_atom(&self, id: Uuid) -> Option<(usize, usize)> {
        let idx = self.editor_cuerpo.atom_ids.iter().position(|x| *x == id)?;
        if idx == 0 {
            return Some((0, 0));
        }
        // Líneas vacías que aporta el separador entre dos párrafos: para
        // `\n\n` (2 newlines) hay 1 línea vacía entre los párrafos.
        let lineas_vacias_sep = SEPARADOR.matches('\n').count().saturating_sub(1);
        let mut linea = 0usize;
        for (i, parrafo) in self.editor_cuerpo.texto.split(SEPARADOR).enumerate() {
            if i == idx {
                return Some((linea, 0));
            }
            // Líneas que ocupa este párrafo (N `\n`s ⇒ N+1 líneas).
            linea += parrafo.matches('\n').count() + 1;
            linea += lineas_vacias_sep;
        }
        // No debería pasar — `idx` está en rango por `position`.
        None
    }

    /// Inversa de [`Self::posicion_de_atom`]: dado una línea del buffer
    /// (0-based), devuelve el Uuid del átomo al que pertenece esa línea.
    /// Si la línea cae sobre el separador (la línea en blanco entre dos
    /// párrafos), la atribuye al átomo **anterior** — así un click justo
    /// debajo del último renglón sigue seleccionando el párrafo que
    /// estabas leyendo.
    ///
    /// Camina los párrafos reales del texto sincronizado, igual que
    /// [`Self::posicion_de_atom`].
    pub fn atom_id_en_linea(&self, linea: usize) -> Option<Uuid> {
        if self.editor_cuerpo.atom_ids.is_empty() {
            return None;
        }
        let lineas_vacias_sep = SEPARADOR.matches('\n').count().saturating_sub(1);
        let mut cursor_linea = 0usize;
        for (i, parrafo) in self.editor_cuerpo.texto.split(SEPARADOR).enumerate() {
            let content_lines = parrafo.matches('\n').count() + 1;
            let fin_parrafo = cursor_linea + content_lines;
            // Dentro del contenido del párrafo i.
            if linea < fin_parrafo {
                return self.editor_cuerpo.atom_ids.get(i).copied();
            }
            // En el separador posterior al párrafo i.
            if linea < fin_parrafo + lineas_vacias_sep {
                return self.editor_cuerpo.atom_ids.get(i).copied();
            }
            cursor_linea = fin_parrafo + lineas_vacias_sep;
        }
        None
    }

    /// Caret actual `(line, col)` del cursor primario. Atajo de
    /// `state.cursor.caret`.
    #[inline]
    pub fn caret(&self) -> (usize, usize) {
        let p = self.state.cursor.caret;
        (p.line, p.col)
    }

    /// Posiciona el caret `(line, col)`, clampeando al rango válido.
    /// Atajo de `state.set_caret_at` para callers que no quieren tocar
    /// el state directamente.
    #[inline]
    pub fn set_caret(&mut self, line: usize, col: usize) {
        self.state.set_caret_at(line, col);
    }
}

impl Default for CuerpoIde {
    fn default() -> Self {
        Self::nuevo_vacio()
    }
}

/// Render del IDE: arma el `text-editor` widget con el texto del cuerpo.
///
/// `language` es típicamente [`Language::Plain`] para prosa narrativa
/// (sin syntax highlight); el caller puede pasar otro si su contenido
/// es código embebido. `visible_lines` cumple el rol habitual del
/// widget — cuántas líneas dibujamos como máximo por frame (el widget
/// cappea internamente a 200; pasar un número alto cuando se desconoce
/// el viewport real es seguro).
///
/// `on_pointer` propaga el `PointerEvent` del widget (Click / Drag
/// dentro del área de texto) al `Msg` del caller; el caller convierte
/// (x, y) en (line, col) con [`EditorMetrics::screen_to_pos`] y aplica
/// [`CuerpoIde::set_caret`] o `state.extend_selection_to`.
pub fn cuerpo_ide_view<Msg: Clone + 'static>(
    ide: &CuerpoIde,
    palette: &EditorPalette,
    metrics: EditorMetrics,
    visible_lines: usize,
    language: Language,
    on_pointer: impl Fn(PointerEvent) -> Option<Msg> + Send + Sync + Clone + 'static,
) -> View<Msg> {
    text_editor_view_highlighted(
        &ide.state,
        palette,
        metrics,
        visible_lines,
        language,
        on_pointer,
    )
}

/// Constructor para tests / herramientas: arma un `CuerpoIde` sin pasar
/// por un `Cuerpo` — recibe el texto plano y la lista de `atom_ids` en
/// orden. Útil cuando el caller quiere instrumentar un estado intermedio.
pub fn cuerpo_ide_desde_texto(texto: impl Into<String>, atom_ids: Vec<Uuid>) -> CuerpoIde {
    let texto = texto.into();
    let mut state = EditorState::new();
    state.set_text(&texto);
    let seq = state.edit_seq;
    CuerpoIde {
        editor_cuerpo: EditorCuerpo { texto, atom_ids },
        state,
        seq_sincronizado: seq,
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use pluma_cuerpo::Intencion;

    fn cuerpo_con_atoms(textos: &[&str]) -> (Cuerpo, Vec<NarrativeAtom>) {
        let mut c = Cuerpo::nuevo("es", "es", Intencion::Original, 0);
        let atoms: Vec<NarrativeAtom> =
            textos.iter().map(|t| NarrativeAtom::new(*t, "es")).collect();
        for a in &atoms {
            c.agregar(a.id, 0);
        }
        (c, atoms)
    }

    fn indice(atoms: &[NarrativeAtom]) -> HashMap<Uuid, &NarrativeAtom> {
        atoms.iter().map(|a| (a.id, a)).collect()
    }

    #[test]
    fn nuevo_vacio_arranca_sincronizado() {
        let ide = CuerpoIde::nuevo_vacio();
        assert!(!ide.pendiente_sync());
        assert_eq!(ide.n_atoms(), 0);
        assert_eq!(ide.n_parrafos_buffer(), 0);
        assert_eq!(ide.texto_buffer(), "");
    }

    #[test]
    fn default_es_equivalente_a_nuevo_vacio() {
        let a = CuerpoIde::default();
        let b = CuerpoIde::nuevo_vacio();
        assert_eq!(a.editor_cuerpo, b.editor_cuerpo);
        assert_eq!(a.texto_buffer(), b.texto_buffer());
    }

    #[test]
    fn from_cuerpo_carga_texto_concatenado_y_arranca_sincronizado() {
        let (c, atoms) = cuerpo_con_atoms(&["Uno.", "Dos.", "Tres."]);
        let idx = indice(&atoms);
        let ide = CuerpoIde::from_cuerpo(&c, &idx);
        assert_eq!(
            ide.state.text(),
            format!("Uno.{s}Dos.{s}Tres.", s = SEPARADOR)
        );
        assert_eq!(ide.editor_cuerpo.atom_ids.len(), 3);
        assert!(!ide.pendiente_sync());
    }

    #[test]
    fn diff_sin_cambios_corta_temprano_y_no_toca_texto() {
        let (c, atoms) = cuerpo_con_atoms(&["uno", "dos"]);
        let idx = indice(&atoms);
        let mut ide = CuerpoIde::from_cuerpo(&c, &idx);
        let seq_antes = ide.state.edit_seq;
        let texto_antes = ide.editor_cuerpo.texto.clone();

        let d = ide.diff(&idx);
        assert!(d.is_empty());
        // El edit_seq no debe avanzar — diff no toca el state.
        assert_eq!(ide.state.edit_seq, seq_antes);
        // Y el texto memorizado tampoco.
        assert_eq!(ide.editor_cuerpo.texto, texto_antes);
        assert!(!ide.pendiente_sync());
    }

    #[test]
    fn editar_buffer_y_diff_emite_mutar_con_uuid_preservado() {
        let (c, atoms) = cuerpo_con_atoms(&["uno", "dos", "tres"]);
        let idx = indice(&atoms);
        let mut ide = CuerpoIde::from_cuerpo(&c, &idx);
        ide.state
            .set_text(&format!("uno{s}DOS!{s}tres", s = SEPARADOR));
        assert!(
            ide.pendiente_sync(),
            "set_text debe disparar pendiente_sync vía edit_seq"
        );
        let d = ide.diff(&idx);
        assert_eq!(d.len(), 1);
        match &d[0] {
            CambioAtom::Mutar { id, texto_nuevo } => {
                assert_eq!(*id, atoms[1].id);
                assert_eq!(texto_nuevo, "DOS!");
            }
            otro => panic!("esperaba Mutar, fue {otro:?}"),
        }
        assert!(!ide.pendiente_sync(), "tras diff el editor queda sincronizado");
    }

    #[test]
    fn aplicar_cambios_alinea_atom_ids_con_los_nuevos_uuids() {
        let (c, atoms) = cuerpo_con_atoms(&["uno", "dos"]);
        let idx = indice(&atoms);
        let mut ide = CuerpoIde::from_cuerpo(&c, &idx);
        ide.state
            .set_text(&format!("uno{s}tres{s}cuatro", s = SEPARADOR));
        let cambios = ide.diff(&idx);
        let nuevo_id = Uuid::new_v4();
        ide.aplicar_cambios(&cambios, &[nuevo_id]);
        assert_eq!(ide.editor_cuerpo.atom_ids.len(), 3);
        assert_eq!(ide.editor_cuerpo.atom_ids[2], nuevo_id);
    }

    #[test]
    fn alias_legacy_aplicar_cambios_locales_sigue_funcionando() {
        let (c, atoms) = cuerpo_con_atoms(&["uno"]);
        let idx = indice(&atoms);
        let mut ide = CuerpoIde::from_cuerpo(&c, &idx);
        ide.state.set_text(&format!("uno{s}dos", s = SEPARADOR));
        let cambios = ide.diff(&idx);
        let nuevo = Uuid::new_v4();
        ide.aplicar_cambios_locales(&cambios, &[nuevo]);
        assert_eq!(ide.editor_cuerpo.atom_ids, vec![atoms[0].id, nuevo]);
    }

    #[test]
    fn recargar_resetea_estado_a_cuerpo_nuevo() {
        let (c1, atoms1) = cuerpo_con_atoms(&["uno", "dos"]);
        let idx1 = indice(&atoms1);
        let mut ide = CuerpoIde::from_cuerpo(&c1, &idx1);
        ide.state.set_text("editado a mano");
        assert!(ide.pendiente_sync());

        let (c2, atoms2) = cuerpo_con_atoms(&["A", "B", "C"]);
        let idx2 = indice(&atoms2);
        ide.recargar(&c2, &idx2);
        assert_eq!(ide.state.text(), format!("A{s}B{s}C", s = SEPARADOR));
        assert_eq!(ide.editor_cuerpo.atom_ids.len(), 3);
        assert!(!ide.pendiente_sync());
    }

    #[test]
    fn cuerpo_ide_desde_texto_construye_sin_grafo_y_sincronizado() {
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let ide = cuerpo_ide_desde_texto(format!("A{s}B", s = SEPARADOR), vec![id_a, id_b]);
        assert_eq!(ide.editor_cuerpo.atom_ids, vec![id_a, id_b]);
        assert_eq!(ide.state.text(), format!("A{s}B", s = SEPARADOR));
        assert!(!ide.pendiente_sync());
    }

    #[test]
    fn n_parrafos_buffer_cuenta_split_actual_no_atom_ids_memorizados() {
        let (c, atoms) = cuerpo_con_atoms(&["uno", "dos"]);
        let idx = indice(&atoms);
        let mut ide = CuerpoIde::from_cuerpo(&c, &idx);
        // n_atoms refleja lo memorizado; n_parrafos_buffer el buffer vivo.
        assert_eq!(ide.n_atoms(), 2);
        assert_eq!(ide.n_parrafos_buffer(), 2);
        ide.state
            .set_text(&format!("uno{s}dos{s}tres{s}cuatro", s = SEPARADOR));
        assert_eq!(ide.n_atoms(), 2, "atom_ids viejos hasta el próximo diff");
        assert_eq!(ide.n_parrafos_buffer(), 4, "el buffer ya tiene 4");
    }

    #[test]
    fn posicion_de_atom_devuelve_linea_inicial_correcta() {
        let (c, atoms) = cuerpo_con_atoms(&["primero", "segundo", "tercero"]);
        let idx = indice(&atoms);
        let ide = CuerpoIde::from_cuerpo(&c, &idx);
        // Texto: "primero\n\nsegundo\n\ntercero"
        // Líneas:   0       1    2       3    4
        assert_eq!(ide.posicion_de_atom(atoms[0].id), Some((0, 0)));
        assert_eq!(ide.posicion_de_atom(atoms[1].id), Some((2, 0)));
        assert_eq!(ide.posicion_de_atom(atoms[2].id), Some((4, 0)));
        assert_eq!(ide.posicion_de_atom(Uuid::new_v4()), None);
    }

    #[test]
    fn atom_id_en_linea_atribuye_separador_al_atom_previo() {
        let (c, atoms) = cuerpo_con_atoms(&["primero", "segundo", "tercero"]);
        let idx = indice(&atoms);
        let ide = CuerpoIde::from_cuerpo(&c, &idx);
        // Texto: "primero\n\nsegundo\n\ntercero"
        // Líneas:    0     1     2     3     4
        assert_eq!(ide.atom_id_en_linea(0), Some(atoms[0].id));
        // Línea 1 = "" (separador): se atribuye al atom previo.
        assert_eq!(ide.atom_id_en_linea(1), Some(atoms[0].id));
        assert_eq!(ide.atom_id_en_linea(2), Some(atoms[1].id));
        assert_eq!(ide.atom_id_en_linea(3), Some(atoms[1].id));
        assert_eq!(ide.atom_id_en_linea(4), Some(atoms[2].id));
        // Fuera de rango → None.
        assert_eq!(ide.atom_id_en_linea(99), None);
        // IDE vacío → None siempre.
        let vacio = CuerpoIde::nuevo_vacio();
        assert_eq!(vacio.atom_id_en_linea(0), None);
    }

    #[test]
    fn posicion_y_atom_id_son_inversas_para_atomos_single_line() {
        let (c, atoms) = cuerpo_con_atoms(&["a", "b", "c", "d"]);
        let idx = indice(&atoms);
        let ide = CuerpoIde::from_cuerpo(&c, &idx);
        for a in &atoms {
            let (line, _) = ide.posicion_de_atom(a.id).expect("atom existe");
            assert_eq!(ide.atom_id_en_linea(line), Some(a.id));
        }
    }

    #[test]
    fn caret_helpers_son_passthrough_consistente() {
        let (c, atoms) = cuerpo_con_atoms(&["abc", "def"]);
        let idx = indice(&atoms);
        let mut ide = CuerpoIde::from_cuerpo(&c, &idx);
        ide.set_caret(0, 2);
        assert_eq!(ide.caret(), (0, 2));
        // Set caret no marca pendiente_sync — sólo cambios del buffer
        // bumpean edit_seq.
        assert!(!ide.pendiente_sync());
    }
}
