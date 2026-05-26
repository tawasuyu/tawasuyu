//! `cuerpo_ide` — el text-editor IDE de Llimphi montado sobre un
//! [`pluma_editor_cuerpo::EditorCuerpo`].
//!
//! La página del editor multilienzo es **un solo control**: el usuario ve
//! todos los párrafos del cuerpo concatenados en un buffer plano editado
//! con el `text-editor` widget de Llimphi (cursor libre, multi-cursor,
//! undo, find/replace, highlight si lo activa el caller). Por debajo
//! seguimos teniendo un grafo de `NarrativeAtom`s con hebras vivas.
//!
//! Esta capa cose las dos cosas:
//!
//!   1. `from_cuerpo` toma un `Cuerpo` + el índice de atoms y arma un
//!      [`EditorCuerpo`] (texto plano + Uuids en orden) y un
//!      [`EditorState`] del widget cargado con ese texto.
//!   2. El caller mete eventos de teclado vía [`CuerpoIde::apply_key`]
//!      — los eventos van directo al `EditorState`. El buffer queda
//!      desincronizado del `EditorCuerpo` (que sigue mostrando el texto
//!      original) hasta que el caller llama a [`CuerpoIde::diff`].
//!   3. [`CuerpoIde::diff`] mete `state.text()` en
//!      `editor_cuerpo.texto` y devuelve la lista mínima de
//!      [`CambioAtom`] que el caller debe aplicar al grafo
//!      (mutar contenido / crear atom nuevo / eliminar uno que ya no
//!      aparece).
//!   4. Tras persistir en el grafo (creando `NarrativeAtom`s reales para
//!      los `Crear`), el caller pasa los Uuids resultantes a
//!      [`CuerpoIde::aplicar_cambios_locales`] para que el `atom_ids`
//!      del editor refleje el nuevo orden.
//!
//! El widget no sabe ni necesita saber que el texto está particionado en
//! átomos: lo trata como un buffer único, con `\n\n` separando párrafos.

use llimphi_ui::View;
use llimphi_widget_text_editor::{
    text_editor_view_highlighted, EditorMetrics, EditorPalette, EditorState, Language,
    PointerEvent,
};
use pluma_core::NarrativeAtom;
use pluma_cuerpo::Cuerpo;
use pluma_editor_cuerpo::{CambioAtom, EditorCuerpo};
use std::collections::HashMap;
use uuid::Uuid;

/// Una página de edición: cuerpo plano + estado del text-editor.
///
/// Es `Clone` porque `EditorState` lo es; útil para snapshots de undo a
/// nivel de aplicación (el undo del widget cubre ediciones de buffer,
/// pero no operaciones de alto nivel sobre el grafo).
#[derive(Debug, Clone)]
pub struct CuerpoIde {
    /// Vista plana del cuerpo. `texto` se actualiza cuando el caller
    /// llama a [`Self::diff`]; mientras tanto la fuente de verdad
    /// editable es `state.buffer`.
    pub editor_cuerpo: EditorCuerpo,
    /// Buffer + cursor + undo + viewport del widget.
    pub state: EditorState,
    /// Marca interna: el buffer fue tocado desde la última sincronización
    /// con `editor_cuerpo.texto`. La fija [`Self::apply_key`] cuando el
    /// widget reporta `Changed`.
    pendiente_sync: bool,
}

impl CuerpoIde {
    /// Construye una página del IDE a partir de un `Cuerpo` + atoms del
    /// grafo. El `EditorState` queda cargado con el texto plano del
    /// cuerpo y el caret al final (convención de `EditorState::set_text`).
    pub fn from_cuerpo(
        cuerpo: &Cuerpo,
        atoms: &HashMap<Uuid, &NarrativeAtom>,
    ) -> Self {
        let editor_cuerpo = EditorCuerpo::from_cuerpo(cuerpo, atoms);
        let mut state = EditorState::new();
        state.set_text(&editor_cuerpo.texto);
        Self {
            editor_cuerpo,
            state,
            pendiente_sync: false,
        }
    }

    /// Resetea el IDE a un nuevo cuerpo (útil cuando el caller cambia de
    /// pestaña / cuerpo activo). Limpia el undo del widget — semántica
    /// del `EditorState::set_text`.
    pub fn recargar(
        &mut self,
        cuerpo: &Cuerpo,
        atoms: &HashMap<Uuid, &NarrativeAtom>,
    ) {
        self.editor_cuerpo = EditorCuerpo::from_cuerpo(cuerpo, atoms);
        self.state.set_text(&self.editor_cuerpo.texto);
        self.pendiente_sync = false;
    }

    /// `true` si el buffer del widget difiere del `editor_cuerpo.texto`
    /// memorizado (al menos una tecla tocó el contenido desde la última
    /// llamada a [`Self::diff`]).
    pub fn pendiente_sync(&self) -> bool {
        self.pendiente_sync
    }

    /// Envuelve [`EditorState::apply_key`]; si la tecla modificó el
    /// buffer, marca `pendiente_sync`.
    pub fn apply_key(
        &mut self,
        event: &llimphi_ui::KeyEvent,
    ) -> llimphi_widget_text_editor::ApplyResult {
        let r = self.state.apply_key(event);
        if r.changed() {
            self.pendiente_sync = true;
        }
        r
    }

    /// Vuelca el texto del buffer en `editor_cuerpo.texto` y devuelve el
    /// diff mínimo contra los atoms originales pasados por el caller.
    /// El caller suele recolectar `atoms_originales` del grafo justo
    /// antes — el editor no consulta el grafo por sí mismo.
    ///
    /// Limpia `pendiente_sync`: el `editor_cuerpo.texto` ya refleja el
    /// buffer del widget.
    pub fn diff(
        &mut self,
        atoms_originales: &HashMap<Uuid, &NarrativeAtom>,
    ) -> Vec<CambioAtom> {
        self.editor_cuerpo.set_texto(self.state.text());
        self.pendiente_sync = false;
        self.editor_cuerpo.diff(atoms_originales)
    }

    /// Tras persistir los cambios en el grafo (creando `NarrativeAtom`s
    /// nuevos para los `Crear` y removiendo los `Eliminar`), pasá acá los
    /// Uuids generados para los `Crear`, en orden, y el `atom_ids` del
    /// editor queda alineado con el cuerpo nuevo.
    pub fn aplicar_cambios_locales(&mut self, cambios: &[CambioAtom], nuevos_ids: &[Uuid]) {
        self.editor_cuerpo.aplicar_cambios(cambios, nuevos_ids);
    }

    /// Atajo: `editor_cuerpo.atom_ids.len()` — cuántos átomos cubre el
    /// cuerpo plano que el editor está mostrando ahora mismo (puede
    /// diferir de los párrafos del buffer hasta que el caller llame a
    /// [`Self::diff`]).
    pub fn n_atoms(&self) -> usize {
        self.editor_cuerpo.atom_ids.len()
    }

    /// Texto crudo del buffer del widget (no del `editor_cuerpo.texto`).
    /// Equivalente a `state.text()` — atajo para callers que solo
    /// necesitan leer.
    pub fn texto_buffer(&self) -> String {
        self.state.text()
    }
}

/// Render del IDE: arma el `text-editor` widget con el texto del cuerpo.
///
/// `language` es típicamente [`Language::Plain`] para prosa narrativa
/// (sin syntax highlight); el caller puede pasar otro si su contenido
/// es código embebido. `visible_lines` cumple el rol habitual del
/// widget — cuántas líneas dibujamos como máximo por frame.
///
/// `on_pointer` propaga el `PointerEvent` del widget (Click / Drag dentro
/// del área de texto) al `Msg` del caller; el caller convierte (x, y) en
/// (line, col) con `EditorMetrics::screen_to_pos` y aplica
/// `state.set_caret_at` o `state.extend_selection_to`.
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

/// Constructor accesorio para tests / herramientas: arma un `CuerpoIde`
/// sin pasar por un `Cuerpo` — recibe el texto plano y la lista de
/// `atom_ids` en orden. Útil cuando el caller quiere instrumentar un
/// estado intermedio.
pub fn cuerpo_ide_desde_texto(texto: impl Into<String>, atom_ids: Vec<Uuid>) -> CuerpoIde {
    let texto = texto.into();
    let mut state = EditorState::new();
    state.set_text(&texto);
    CuerpoIde {
        editor_cuerpo: EditorCuerpo { texto, atom_ids },
        state,
        pendiente_sync: false,
    }
}

/// Constante re-exportada — el caller puede usarla para construir texto
/// manualmente sabiendo que el split se va a hacer por `"\n\n"`.
pub use pluma_editor_cuerpo::SEPARADOR as SEPARADOR_PARRAFO;

#[cfg(test)]
mod pruebas {
    use super::*;
    use pluma_cuerpo::Intencion;
    use pluma_editor_cuerpo::SEPARADOR;

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
    fn from_cuerpo_carga_texto_concatenado_en_el_buffer() {
        let (c, atoms) = cuerpo_con_atoms(&["Uno.", "Dos.", "Tres."]);
        let idx = indice(&atoms);
        let ide = CuerpoIde::from_cuerpo(&c, &idx);
        assert_eq!(ide.state.text(), format!("Uno.{s}Dos.{s}Tres.", s = SEPARADOR));
        assert_eq!(ide.editor_cuerpo.atom_ids.len(), 3);
        assert!(!ide.pendiente_sync());
    }

    #[test]
    fn diff_sin_cambios_es_vacio_y_limpia_pendiente() {
        let (c, atoms) = cuerpo_con_atoms(&["uno", "dos"]);
        let idx = indice(&atoms);
        let mut ide = CuerpoIde::from_cuerpo(&c, &idx);
        let d = ide.diff(&idx);
        assert!(d.is_empty());
        assert!(!ide.pendiente_sync());
    }

    #[test]
    fn editar_buffer_y_diff_emite_mutar_con_uuid_preservado() {
        let (c, atoms) = cuerpo_con_atoms(&["uno", "dos", "tres"]);
        let idx = indice(&atoms);
        let mut ide = CuerpoIde::from_cuerpo(&c, &idx);
        // Simula que el usuario tipeó "uno\n\nDOS!\n\ntres" — vamos
        // directo al buffer para no depender de KeyEvents acá.
        ide.state.set_text(&format!("uno{s}DOS!{s}tres", s = SEPARADOR));
        let d = ide.diff(&idx);
        assert_eq!(d.len(), 1);
        match &d[0] {
            CambioAtom::Mutar { id, texto_nuevo } => {
                assert_eq!(*id, atoms[1].id);
                assert_eq!(texto_nuevo, "DOS!");
            }
            otro => panic!("esperaba Mutar, fue {otro:?}"),
        }
    }

    #[test]
    fn aplicar_cambios_locales_alinea_atom_ids_con_los_nuevos_uuids() {
        let (c, atoms) = cuerpo_con_atoms(&["uno", "dos"]);
        let idx = indice(&atoms);
        let mut ide = CuerpoIde::from_cuerpo(&c, &idx);
        ide.state.set_text(&format!("uno{s}tres{s}cuatro", s = SEPARADOR));
        let cambios = ide.diff(&idx);
        // 1 Mutar(dos→tres) + 1 Crear(cuatro).
        let nuevo_id = Uuid::new_v4();
        ide.aplicar_cambios_locales(&cambios, &[nuevo_id]);
        assert_eq!(ide.editor_cuerpo.atom_ids.len(), 3);
        assert_eq!(ide.editor_cuerpo.atom_ids[2], nuevo_id);
    }

    #[test]
    fn recargar_resetea_estado_a_cuerpo_nuevo() {
        let (c1, atoms1) = cuerpo_con_atoms(&["uno", "dos"]);
        let idx1 = indice(&atoms1);
        let mut ide = CuerpoIde::from_cuerpo(&c1, &idx1);
        // Ensucia el buffer.
        ide.state.set_text("editado");
        let (c2, atoms2) = cuerpo_con_atoms(&["A", "B", "C"]);
        let idx2 = indice(&atoms2);
        ide.recargar(&c2, &idx2);
        assert_eq!(ide.state.text(), format!("A{s}B{s}C", s = SEPARADOR));
        assert_eq!(ide.editor_cuerpo.atom_ids.len(), 3);
        assert!(!ide.pendiente_sync());
    }

    #[test]
    fn cuerpo_ide_desde_texto_construye_sin_grafo() {
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let ide = cuerpo_ide_desde_texto(
            format!("A{s}B", s = SEPARADOR),
            vec![id_a, id_b],
        );
        assert_eq!(ide.editor_cuerpo.atom_ids, vec![id_a, id_b]);
        assert_eq!(ide.state.text(), format!("A{s}B", s = SEPARADOR));
    }
}
