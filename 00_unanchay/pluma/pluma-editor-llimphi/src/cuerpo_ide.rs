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

use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::View;
use llimphi_widget_text_editor::{
    text_editor_view_highlighted, text_editor_view_styled, ApplyResult, Clipboard, EditorMetrics,
    EditorOptions, EditorPalette, EditorState, Language, PointerEvent, StyledSpan,
};
use pluma_core::NarrativeAtom;
use pluma_cuerpo::Cuerpo;
use pluma_editor_cuerpo::{CambioAtom, EditorCuerpo, SEPARADOR};
use pluma_estilo::{EstiloLienzo, EstiloTexto};
use uuid::Uuid;

// Re-exports — el caller importa todo desde `cuerpo_ide` sin tener que
// conocer la geografía interna de los dos crates que ensamblamos.
pub use llimphi_widget_text_editor::{
    ApplyResult as EditorApplyResult, EditorMetrics as IdeMetrics,
    EditorPalette as IdePalette, GutterStyle as IdeGutterStyle, Language as IdeLanguage,
    PointerEvent as IdePointerEvent,
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
    /// `state.edit_seq` cuando se computaron las guardas por última
    /// vez. Si difiere de `state.edit_seq`, la lista está stale y
    /// [`Self::recomputar_guard_lines_si_stale`] la reconstruye.
    seq_guardas: u64,
    /// Flag por **junction** entre átomos consecutivos. Longitud =
    /// `max(0, atom_ids.len() - 1)`. Índice *i* representa la
    /// separación entre `atom_ids[i]` y `atom_ids[i+1]`. `false` =
    /// separador (la línea vacía es guarda); `true` = fundida (la
    /// línea vacía pertenece a la zona, es contenido editable).
    ///
    /// Por convención TODA junction arranca como separador (`false`).
    /// La fusión es deliberada — un atajo del caller la togglea.
    pub fundido_junctions: Vec<bool>,
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
            seq_guardas: seq,
            fundido_junctions: Vec::new(),
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
    ///
    /// Tras cargar el texto, todas las junctions arrancan como
    /// separador (`fundido_junctions[i] = false`) y las guardas se
    /// computan en consecuencia.
    pub fn con_opciones(
        cuerpo: &Cuerpo,
        atoms: &HashMap<Uuid, &NarrativeAtom>,
        options: EditorOptions,
    ) -> Self {
        let editor_cuerpo = EditorCuerpo::from_cuerpo(cuerpo, atoms);
        let n_junctions = editor_cuerpo.atom_ids.len().saturating_sub(1);
        let mut state = EditorState::with_options(options);
        state.set_text(&editor_cuerpo.texto);
        let seq = state.edit_seq;
        let mut ide = Self {
            editor_cuerpo,
            state,
            seq_sincronizado: seq,
            seq_guardas: seq.wrapping_sub(1), // forzar recompute
            fundido_junctions: vec![false; n_junctions],
        };
        ide.recomputar_guard_lines();
        ide.state.snap_off_guards(-1);
        ide
    }

    /// Resetea el IDE a un nuevo cuerpo (útil cuando el caller cambia de
    /// pestaña / cuerpo activo). Limpia el undo del widget — semántica
    /// del `EditorState::set_text`. Conserva las opciones del editor.
    /// Todas las junctions vuelven a `false` (separador).
    pub fn recargar(&mut self, cuerpo: &Cuerpo, atoms: &HashMap<Uuid, &NarrativeAtom>) {
        self.editor_cuerpo = EditorCuerpo::from_cuerpo(cuerpo, atoms);
        self.state.set_text(&self.editor_cuerpo.texto);
        self.seq_sincronizado = self.state.edit_seq;
        let n_junctions = self.editor_cuerpo.atom_ids.len().saturating_sub(1);
        self.fundido_junctions = vec![false; n_junctions];
        self.recomputar_guard_lines();
        self.state.snap_off_guards(-1);
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
    /// `pendiente_sync` se mantiene automáticamente vía `edit_seq`. Si
    /// el buffer cambió, las guardas se recomputan tras el evento (y
    /// el caret vuelve a snapearse — el primer snap se hizo con la
    /// lista vieja).
    pub fn apply_key(&mut self, event: &llimphi_ui::KeyEvent) -> ApplyResult {
        let r = self.state.apply_key(event);
        self.refrescar_guardas_si_cambio(r);
        r
    }

    /// Como [`Self::apply_key`] con backend de clipboard — habilita
    /// `Ctrl+C/X/V`.
    pub fn apply_key_with_clipboard(
        &mut self,
        event: &llimphi_ui::KeyEvent,
        clipboard: &mut dyn Clipboard,
    ) -> ApplyResult {
        let r = self.state.apply_key_with_clipboard(event, clipboard);
        self.refrescar_guardas_si_cambio(r);
        r
    }

    /// Si la edición cambió el buffer, recomputa la lista de guardas y
    /// re-snappea el caret (el primer snap dentro del widget usó la
    /// lista anterior). Si sólo movió el cursor, no hace nada — las
    /// guardas no cambiaron.
    fn refrescar_guardas_si_cambio(&mut self, r: ApplyResult) {
        if r.changed() {
            self.recomputar_guard_lines();
            self.state.snap_off_guards(1);
        }
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
    /// `atom_ids` del editor queda alineado con el cuerpo nuevo. La
    /// lista de `fundido_junctions` se ajusta en consecuencia
    /// (junctions nuevas arrancan como separador `false`; junctions
    /// eliminadas se descartan preservando el flag de las que sobreviven).
    pub fn aplicar_cambios(&mut self, cambios: &[CambioAtom], nuevos_ids: &[Uuid]) {
        let n_antes = self.editor_cuerpo.atom_ids.len();
        self.editor_cuerpo.aplicar_cambios(cambios, nuevos_ids);
        let n_despues = self.editor_cuerpo.atom_ids.len();
        let target = n_despues.saturating_sub(1);
        // Preservamos el flag de las junctions que sobreviven (las
        // primeras `min(target, len_actual)`) y extendemos con `false`
        // (separador) para junctions nuevas. Si hay borrados al
        // final, simplemente truncamos.
        self.fundido_junctions.resize(target, false);
        // Reset de seq_guardas para forzar recompute en el próximo render.
        self.seq_guardas = self.state.edit_seq.wrapping_sub(1);
        let _ = n_antes;
        self.recomputar_guard_lines();
    }

    /// Togglea la junction *idx* (entre `atom_ids[idx]` y
    /// `atom_ids[idx+1]`) — si era separador, pasa a fundida; si era
    /// fundida, pasa a separador. Tras el toggle, las guardas y el
    /// caret se refrescan. `idx` fuera de rango → no-op silencioso.
    pub fn togglear_junction(&mut self, idx: usize) {
        if idx >= self.fundido_junctions.len() {
            return;
        }
        self.fundido_junctions[idx] = !self.fundido_junctions[idx];
        self.seq_guardas = self.state.edit_seq.wrapping_sub(1);
        self.recomputar_guard_lines();
        // El caret puede haber quedado sobre una nueva guarda o
        // liberado de una vieja — re-snap por las dudas.
        self.state.snap_off_guards(0);
    }

    /// Marca la junction *idx* como fundida (no es guarda). No-op si
    /// ya estaba fundida o si `idx` está fuera de rango.
    pub fn fundir_junction(&mut self, idx: usize) {
        if idx < self.fundido_junctions.len() && !self.fundido_junctions[idx] {
            self.togglear_junction(idx);
        }
    }

    /// Marca la junction *idx* como separador (es guarda). No-op si
    /// ya era separador o si `idx` está fuera de rango.
    pub fn separar_junction(&mut self, idx: usize) {
        if idx < self.fundido_junctions.len() && self.fundido_junctions[idx] {
            self.togglear_junction(idx);
        }
    }

    /// Recomputa `state.guard_lines` Y `state.line_tints` desde cero.
    /// 1. Enumera los índices de línea vacía del buffer (cada una es
    ///    candidata a guarda — aparece por un `\n\n` o trailing).
    /// 2. Las matchea por ordinal con `fundido_junctions`: la *i*-ésima
    ///    línea vacía corresponde a la *i*-ésima junction. Junctions
    ///    extra (más blanks que junctions registradas) se tratan como
    ///    separador (guarda) — eso pasa típicamente cuando el usuario
    ///    acaba de tipear `\n\n` y aún no llamó a `diff` para
    ///    materializar el atom nuevo.
    /// 3. Junctions `false` (separador) van a `guard_lines`; junctions
    ///    `true` (fundida) NO se agregan — la línea vacía pertenece a
    ///    la zona, es contenido editable.
    pub fn recomputar_guard_lines(&mut self) {
        let texto = self.state.text();
        let total_lineas = self.state.line_count();
        let mut guards: Vec<usize> = Vec::new();
        // Cada línea del buffer pertenece a un grupo (zona). Empezamos
        // en grupo 0; cada vez que cruzamos un separador (junction
        // que NO está fundida), incrementamos. Líneas guarda no
        // pertenecen a ningún grupo (color `None`).
        let mut tints: Vec<Option<Color>> = vec![None; total_lineas];
        let mut grupo_actual = 0usize;
        let mut junction_idx = 0usize;
        for (linea, contenido) in texto.lines().enumerate() {
            if contenido.is_empty() {
                // Línea candidata a junction.
                let fundida = self
                    .fundido_junctions
                    .get(junction_idx)
                    .copied()
                    .unwrap_or(false);
                if !fundida {
                    // Separador: guarda + corte de grupo.
                    guards.push(linea);
                    if linea < tints.len() {
                        tints[linea] = None;
                    }
                    grupo_actual += 1;
                } else {
                    // Fundida: la línea es contenido de la zona, hereda el tinte.
                    if linea < tints.len() {
                        tints[linea] = Some(color_de_grupo(grupo_actual));
                    }
                }
                junction_idx += 1;
            } else {
                // Línea de contenido — tinte del grupo actual.
                if linea < tints.len() {
                    tints[linea] = Some(color_de_grupo(grupo_actual));
                }
            }
        }
        self.state.set_guard_lines(guards);
        self.state.line_tints = tints;
        self.seq_guardas = self.state.edit_seq;
    }

    /// Devuelve la línea de la junction *idx* en el buffer actual,
    /// para permitir scroll/highlight dirigidos. `None` si el índice
    /// no corresponde a ninguna junction visible.
    pub fn linea_de_junction(&self, idx: usize) -> Option<usize> {
        let texto = self.state.text();
        let mut count = 0usize;
        for (linea, contenido) in texto.lines().enumerate() {
            if !contenido.is_empty() {
                continue;
            }
            if count == idx {
                return Some(linea);
            }
            count += 1;
        }
        None
    }

    /// Devuelve el índice de la junction que **precede** al atom en la
    /// línea actual del caret. Útil para "fundir el párrafo del caret
    /// con el anterior". Devuelve `None` si el caret está en el primer
    /// atom (no tiene junction anterior) o no se puede mapear.
    pub fn junction_antes_del_caret(&self) -> Option<usize> {
        let (linea, _) = self.caret();
        let texto = self.state.text();
        // Cuántas líneas vacías hay ANTES de `linea` — esa es la
        // cantidad de junctions que precede al atom actual; el índice
        // de la junction inmediatamente anterior es ese count - 1.
        let mut count = 0usize;
        for (i, contenido) in texto.lines().enumerate() {
            if i >= linea {
                break;
            }
            if contenido.is_empty() {
                count += 1;
            }
        }
        if count == 0 {
            None
        } else {
            Some(count - 1)
        }
    }

    /// Atajo retrocompatible — alias histórico de [`Self::aplicar_cambios`].
    #[inline]
    pub fn aplicar_cambios_locales(&mut self, cambios: &[CambioAtom], nuevos_ids: &[Uuid]) {
        self.aplicar_cambios(cambios, nuevos_ids);
    }

    /// Cuántas **zonas** distintas hay en el cuerpo actual. Una zona es
    /// un grupo de atoms consecutivos unidos por junctions fundidas; cada
    /// junction `false` (separador) marca el inicio de una zona nueva.
    ///
    /// Reglas:
    /// - `n_atoms == 0` → `0` zonas.
    /// - `n_atoms == 1` → `1` zona.
    /// - En general: `n_zonas = 1 + (cantidad de junctions separadoras)`.
    pub fn n_zonas(&self) -> usize {
        let n = self.editor_cuerpo.atom_ids.len();
        if n == 0 {
            return 0;
        }
        let separadoras = self.fundido_junctions.iter().filter(|f| !**f).count();
        // Cada separadora abre una zona nueva; la primera zona ya existe.
        // Si hay más junctions registradas que atoms-1 (estado transitorio
        // entre tipeo y diff), las extras se ignoran — el cap es real:
        // como mucho `n` zonas, una por atom.
        (1 + separadoras).min(n)
    }

    /// Devuelve la zona que contiene al atom `atom_idx` (0-based en
    /// `editor_cuerpo.atom_ids`). `None` si `atom_idx` está fuera de rango.
    pub fn zona_de_atom_idx(&self, atom_idx: usize) -> Option<usize> {
        if atom_idx >= self.editor_cuerpo.atom_ids.len() {
            return None;
        }
        // La zona del atom *i* es la cantidad de junctions separadoras en
        // `fundido_junctions[..i]`: cada separadora cierra una zona y abre
        // la siguiente.
        let zona = self
            .fundido_junctions
            .iter()
            .take(atom_idx)
            .filter(|f| !**f)
            .count();
        Some(zona)
    }

    /// Devuelve la zona a la que pertenece la línea `linea` del buffer.
    /// Líneas guarda (separadores no fundidos) no pertenecen a ninguna
    /// zona — devuelven `None`. Líneas vacías fundidas SÍ pertenecen a
    /// la zona del atom que las flanquea.
    pub fn zona_de_linea(&self, linea: usize) -> Option<usize> {
        if self.editor_cuerpo.atom_ids.is_empty() {
            return None;
        }
        if self.state.is_guard_line(linea) {
            return None;
        }
        let id = self.atom_id_en_linea(linea)?;
        let idx = self.editor_cuerpo.atom_ids.iter().position(|x| *x == id)?;
        self.zona_de_atom_idx(idx)
    }

    /// Zona actual del caret. Si el caret está sobre una guarda, busca la
    /// zona del atom de la línea inmediatamente anterior (consistente con
    /// [`Self::atom_id_en_linea`], que atribuye separadores al átomo
    /// anterior). Devuelve `0` si el cuerpo está vacío — los callers que
    /// quieran distinguir el caso usan [`Self::n_zonas`] previamente.
    pub fn zona_del_caret(&self) -> usize {
        if self.editor_cuerpo.atom_ids.is_empty() {
            return 0;
        }
        let (linea, _) = self.caret();
        if let Some(z) = self.zona_de_linea(linea) {
            return z;
        }
        // Caret sobre guarda: el snap del widget normalmente lo saca de
        // ahí, pero por las dudas tomamos la zona del atom anterior.
        let anterior = linea.saturating_sub(1);
        self.zona_de_linea(anterior).unwrap_or(0)
    }

    /// Rango inclusive de índices de atom (en `editor_cuerpo.atom_ids`)
    /// que forman la zona `zona`. `None` si `zona` está fuera de rango.
    pub fn atoms_de_zona(&self, zona: usize) -> Option<(usize, usize)> {
        if zona >= self.n_zonas() {
            return None;
        }
        // Primer atom de la zona: el primero cuyo zona_de_atom_idx == zona.
        // Caminamos las junctions contando separadoras: cuando la cuenta
        // alcanza `zona`, el atom siguiente es el inicio.
        let mut zona_actual = 0usize;
        let mut start: Option<usize> = None;
        let mut end: usize = 0;
        for atom_idx in 0..self.editor_cuerpo.atom_ids.len() {
            if atom_idx > 0 {
                // Junction inmediatamente anterior a este atom.
                let fundida = self
                    .fundido_junctions
                    .get(atom_idx - 1)
                    .copied()
                    .unwrap_or(false);
                if !fundida {
                    zona_actual += 1;
                }
            }
            if zona_actual == zona {
                if start.is_none() {
                    start = Some(atom_idx);
                }
                end = atom_idx;
            } else if zona_actual > zona {
                break;
            }
        }
        start.map(|s| (s, end))
    }

    /// Rango inclusive de líneas del buffer que cubren la zona `zona`.
    /// La línea de inicio es la del primer atom de la zona; la línea de
    /// fin es la última línea del último atom de la zona (cuenta los
    /// `\n` internos del atom). Las junctions fundidas internas a la zona
    /// quedan incluidas naturalmente porque no son guarda.
    pub fn lineas_de_zona(&self, zona: usize) -> Option<(usize, usize)> {
        let (start_atom, end_atom) = self.atoms_de_zona(zona)?;
        let start_id = *self.editor_cuerpo.atom_ids.get(start_atom)?;
        let end_id = *self.editor_cuerpo.atom_ids.get(end_atom)?;
        let (start_line, _) = self.posicion_de_atom(start_id)?;
        let (end_line_start, _) = self.posicion_de_atom(end_id)?;
        let end_parrafo = self.editor_cuerpo.texto.split(SEPARADOR).nth(end_atom)?;
        let end_line = end_line_start + end_parrafo.matches('\n').count();
        Some((start_line, end_line))
    }

    /// Devuelve los `Uuid` de los atoms que forman la zona `zona`, en
    /// orden. `None` si la zona está fuera de rango. Útil para construir
    /// un sub-`Cuerpo` que se pase a un ejecutor LLM y derivar SOLO esa
    /// zona, sin tocar el resto del documento.
    pub fn atom_ids_de_zona(&self, zona: usize) -> Option<Vec<Uuid>> {
        let (start, end) = self.atoms_de_zona(zona)?;
        Some(self.editor_cuerpo.atom_ids[start..=end].to_vec())
    }

    /// Mueve el caret al inicio de la zona `zona` (línea de la primera
    /// atom, columna 0) y se asegura de que sea visible. Si la zona está
    /// fuera de rango, no-op.
    pub fn ir_a_zona(&mut self, zona: usize) {
        let Some((start_line, _)) = self.lineas_de_zona(zona) else {
            return;
        };
        self.set_caret(start_line, 0);
    }

    /// Selecciona la zona `zona` entera: caret pasa al final de la última
    /// línea de la zona y el anchor queda en el inicio (línea de la
    /// primera atom, col 0). Si la zona está fuera de rango, no-op.
    pub fn seleccionar_zona(&mut self, zona: usize) {
        let Some((start_line, end_line)) = self.lineas_de_zona(zona) else {
            return;
        };
        self.set_caret(start_line, 0);
        let end_col = self.state.buffer.line_len_chars(end_line);
        self.state.extend_selection_to(end_line, end_col);
    }

    /// Zona siguiente (con wrap al inicio si estamos en la última). Si
    /// no hay zonas, no-op silencioso. Si hay solo una, recae sobre sí
    /// misma — mueve el caret al inicio.
    pub fn ir_a_zona_siguiente(&mut self) {
        let total = self.n_zonas();
        if total == 0 {
            return;
        }
        let actual = self.zona_del_caret();
        let siguiente = (actual + 1) % total;
        self.ir_a_zona(siguiente);
    }

    /// Zona anterior (con wrap al final si estamos en la primera).
    pub fn ir_a_zona_anterior(&mut self) {
        let total = self.n_zonas();
        if total == 0 {
            return;
        }
        let actual = self.zona_del_caret();
        let anterior = if actual == 0 { total - 1 } else { actual - 1 };
        self.ir_a_zona(anterior);
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
    // El IDE narrativo siempre quiere ver pista visual en las líneas
    // separadoras: encendemos `phantom_guard_lines` para que cada
    // guarda reciba el divisor fantasma. El estilo de gutter
    // (Numbers/Phantom) y el ancho los decide el caller — el omitido
    // del número en las líneas guarda ocurre automáticamente porque
    // `state.guard_lines` lo lleva (lo pobló `recomputar_guard_lines`).
    let mut metrics = metrics;
    metrics.phantom_guard_lines = true;
    text_editor_view_highlighted(
        &ide.state,
        palette,
        metrics,
        visible_lines,
        language,
        on_pointer,
    )
}

/// `Rgba` (`[u8;4]`) de `pluma-estilo` → `peniko::Color`.
#[inline]
fn rgba_a_color(c: [u8; 4]) -> Color {
    Color::from_rgba8(c[0], c[1], c[2], c[3])
}

/// Construye un [`StyledSpan`] de columnas `[ini, fin)` a partir de un
/// [`EstiloTexto`] efectivo (ya mergeado).
fn styled_span_de(ini: usize, fin: usize, e: &EstiloTexto) -> StyledSpan {
    StyledSpan {
        start_col: ini,
        end_col: fin,
        fg: e.color_fg.map(rgba_a_color),
        bg: e.color_bg.map(rgba_a_color),
        font_family: e.font_family.clone(),
        size_px: e.size_px,
        weight: e.weight,
        italic: e.italic,
        underline: e.underline,
        strikethrough: e.strikethrough,
    }
}

/// Resuelve un [`EstiloLienzo`] contra el layout actual de un [`CuerpoIde`]
/// y devuelve, por línea del buffer, los [`StyledSpan`] que el
/// `text-editor` consume. Capa por capa:
///
/// 1. **base + zona** — por cada línea de contenido, un span de la línea
///    entera con el estilo efectivo de su zona (`base` mergeado con el
///    override de zona). Las guardas/separadores reciben sólo `base`.
/// 2. **por span** — los overrides de selección guardados por átomo
///    (rangos de char dentro del contenido del átomo) se mapean a
///    sub-rangos por línea del buffer y se apilan ENCIMA de la capa de
///    zona — parley aplica en orden de inserción, así el span gana.
///
/// Si el estilo está vacío devuelve un vector de líneas vacías — el
/// caller puede entonces caer al render normal sin spans.
pub fn spans_estilo_por_linea(ide: &CuerpoIde, estilo: &EstiloLienzo) -> Vec<Vec<StyledSpan>> {
    let total = ide.state.line_count();
    let mut out: Vec<Vec<StyledSpan>> = vec![Vec::new(); total];
    if estilo.es_vacio() {
        return out;
    }

    // 1) Capa base + zona: un span de línea completa por cada línea con
    //    estilo efectivo no vacío.
    for line in 0..total {
        let efectivo = match ide.zona_de_linea(line) {
            Some(z) => estilo.estilo_de_zona(z),
            None => estilo.base.clone(),
        };
        if efectivo.es_vacio() {
            continue;
        }
        let len = ide.state.buffer.line_len_chars(line);
        if len == 0 {
            continue;
        }
        out[line].push(styled_span_de(0, len, &efectivo));
    }

    // 2) Capa por span (selección estilada), mapeando char-en-átomo →
    //    (línea, col) del buffer.
    for (atom_id, spans) in &estilo.por_span {
        let Some((start_line, _)) = ide.posicion_de_atom(*atom_id) else {
            continue;
        };
        let Some(idx) = ide
            .editor_cuerpo
            .atom_ids
            .iter()
            .position(|x| x == atom_id)
        else {
            continue;
        };
        let Some(contenido) = ide.editor_cuerpo.texto.split(SEPARADOR).nth(idx) else {
            continue;
        };
        // Mapa: posición de char (0..=nchars) → (línea, col) del buffer.
        let mut posmap: Vec<(usize, usize)> = Vec::with_capacity(contenido.chars().count() + 1);
        let mut ln = start_line;
        let mut col = 0usize;
        for ch in contenido.chars() {
            posmap.push((ln, col));
            if ch == '\n' {
                ln += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        posmap.push((ln, col));
        let nchars = posmap.len() - 1;

        for s in spans {
            let ini = s.ini.min(nchars);
            let fin = s.fin.min(nchars);
            if fin <= ini {
                continue;
            }
            // Trocea el rango por línea (un átomo multi-línea cae en varias).
            let mut i = ini;
            while i < fin {
                let (linea, c0) = posmap[i];
                let mut j = i;
                while j < fin && posmap[j].0 == linea {
                    j += 1;
                }
                let c_end = posmap[j - 1].1 + 1;
                if linea < out.len() {
                    out[linea].push(styled_span_de(c0, c_end, &s.estilo));
                }
                i = j;
            }
        }
    }

    out
}

/// Como [`cuerpo_ide_view`] pero aplica un [`EstiloLienzo`]: resuelve los
/// spans rich-text con [`spans_estilo_por_linea`] y los pinta vía
/// `text_editor_view_styled`. Si el estilo es vacío, cae al render normal
/// (highlight) — mismo resultado que `cuerpo_ide_view`.
#[allow(clippy::too_many_arguments)]
pub fn cuerpo_ide_view_estilado<Msg: Clone + 'static>(
    ide: &CuerpoIde,
    estilo: Option<&EstiloLienzo>,
    palette: &EditorPalette,
    metrics: EditorMetrics,
    visible_lines: usize,
    language: Language,
    on_pointer: impl Fn(PointerEvent) -> Option<Msg> + Send + Sync + Clone + 'static,
) -> View<Msg> {
    let mut metrics = metrics;
    metrics.phantom_guard_lines = true;
    match estilo.filter(|e| !e.es_vacio()) {
        Some(e) => {
            let spans = spans_estilo_por_linea(ide, e);
            text_editor_view_styled(&ide.state, palette, metrics, visible_lines, &spans, &[], on_pointer)
        }
        None => text_editor_view_highlighted(
            &ide.state,
            palette,
            metrics,
            visible_lines,
            language,
            on_pointer,
        ),
    }
}

/// Constructor para tests / herramientas: arma un `CuerpoIde` sin pasar
/// por un `Cuerpo` — recibe el texto plano y la lista de `atom_ids` en
/// orden. Útil cuando el caller quiere instrumentar un estado intermedio.
pub fn cuerpo_ide_desde_texto(texto: impl Into<String>, atom_ids: Vec<Uuid>) -> CuerpoIde {
    let texto = texto.into();
    let n_junctions = atom_ids.len().saturating_sub(1);
    let mut state = EditorState::new();
    state.set_text(&texto);
    let seq = state.edit_seq;
    let mut ide = CuerpoIde {
        editor_cuerpo: EditorCuerpo { texto, atom_ids },
        state,
        seq_sincronizado: seq,
        seq_guardas: seq.wrapping_sub(1),
        fundido_junctions: vec![false; n_junctions],
    };
    ide.recomputar_guard_lines();
    ide.state.snap_off_guards(-1);
    ide
}

/// Paleta circular de 8 tonalidades para colorear las zonas del IDE
/// narrativo. Cada índice de grupo `i` recibe `PALETA_ZONAS[i %
/// PALETA_ZONAS.len()]` — el alpha está calculado para sumar como
/// tinte sutil sobre el fondo del editor (≤16/255), sin afectar la
/// lectura del texto. Los matices están repartidos en el círculo
/// cromático para que dos grupos adyacentes se distingan al ojo aun
/// con baja saturación.
const PALETA_ZONAS: [Color; 8] = [
    // ámbar tibio
    Color::from_rgba8(238, 178, 53, 16),
    // verde salvia
    Color::from_rgba8(94, 184, 124, 16),
    // azul lavanda
    Color::from_rgba8(120, 150, 220, 16),
    // rosa palo
    Color::from_rgba8(220, 130, 160, 16),
    // turquesa
    Color::from_rgba8(80, 190, 200, 16),
    // violeta suave
    Color::from_rgba8(170, 130, 220, 16),
    // arena
    Color::from_rgba8(210, 190, 130, 16),
    // coral
    Color::from_rgba8(230, 140, 120, 16),
];

/// Devuelve el color asignado al grupo `idx` siguiendo la paleta
/// circular [`PALETA_ZONAS`].
pub fn color_de_grupo(idx: usize) -> Color {
    PALETA_ZONAS[idx % PALETA_ZONAS.len()]
}

/// Cuántos grupos distintos pueden colorearse antes de que se repita
/// la tonalidad. Útil para que la UI muestre "N grupos · ciclo cada
/// `paleta_zonas_len()`".
pub const fn paleta_zonas_len() -> usize {
    PALETA_ZONAS.len()
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
        // set_caret usa la API segura — con guardas, el caret no
        // puede caer en (0, 2) sólo si esa línea es guarda; "abc" no
        // lo es, así que el assert pasa.
        ide.set_caret(0, 2);
        assert_eq!(ide.caret(), (0, 2));
        // Set caret no marca pendiente_sync — sólo cambios del buffer
        // bumpean edit_seq.
        assert!(!ide.pendiente_sync());
    }

    #[test]
    fn from_cuerpo_arranca_con_todas_las_junctions_como_separador() {
        let (c, atoms) = cuerpo_con_atoms(&["A", "B", "C"]);
        let idx = indice(&atoms);
        let ide = CuerpoIde::from_cuerpo(&c, &idx);
        // 3 átomos → 2 junctions, ambas separador.
        assert_eq!(ide.fundido_junctions, vec![false, false]);
        // Y las dos líneas vacías (1 y 3) deberían ser guardas.
        assert_eq!(ide.state.guard_lines, vec![1, 3]);
    }

    #[test]
    fn fundir_junction_quita_la_guarda_de_esa_linea() {
        let (c, atoms) = cuerpo_con_atoms(&["A", "B", "C"]);
        let idx = indice(&atoms);
        let mut ide = CuerpoIde::from_cuerpo(&c, &idx);
        // Líneas: 0="A", 1="", 2="B", 3="", 4="C".
        // Fusionar la junction 0 (entre A y B): la línea 1 deja de ser guarda.
        ide.fundir_junction(0);
        assert_eq!(ide.fundido_junctions, vec![true, false]);
        assert_eq!(ide.state.guard_lines, vec![3]);
    }

    #[test]
    fn separar_junction_revierte_la_fusion() {
        let (c, atoms) = cuerpo_con_atoms(&["A", "B"]);
        let idx = indice(&atoms);
        let mut ide = CuerpoIde::from_cuerpo(&c, &idx);
        ide.fundir_junction(0);
        assert!(ide.state.guard_lines.is_empty());
        ide.separar_junction(0);
        assert_eq!(ide.state.guard_lines, vec![1]);
    }

    #[test]
    fn togglear_junction_es_idempotente_doble_aplica() {
        let (c, atoms) = cuerpo_con_atoms(&["A", "B"]);
        let idx = indice(&atoms);
        let mut ide = CuerpoIde::from_cuerpo(&c, &idx);
        ide.togglear_junction(0);
        ide.togglear_junction(0);
        assert_eq!(ide.fundido_junctions, vec![false]);
        assert_eq!(ide.state.guard_lines, vec![1]);
    }

    #[test]
    fn togglear_junction_fuera_de_rango_es_noop() {
        let (c, atoms) = cuerpo_con_atoms(&["A", "B"]);
        let idx = indice(&atoms);
        let mut ide = CuerpoIde::from_cuerpo(&c, &idx);
        ide.togglear_junction(99);
        // Sin cambios: 1 junction separador, 1 guarda.
        assert_eq!(ide.fundido_junctions, vec![false]);
        assert_eq!(ide.state.guard_lines, vec![1]);
    }

    #[test]
    fn caret_atraviesa_separador_pero_se_queda_en_linea_fundida() {
        let (c, atoms) = cuerpo_con_atoms(&["A", "B", "C"]);
        let idx = indice(&atoms);
        let mut ide = CuerpoIde::from_cuerpo(&c, &idx);
        // Fundir junction 0 → línea 1 deja de ser guarda.
        ide.fundir_junction(0);
        // Click en línea 1: el caret puede quedarse ahí porque es contenido.
        ide.set_caret(1, 0);
        assert_eq!(ide.caret(), (1, 0));
        // Click en línea 3 (sigue siendo guarda): salta.
        ide.set_caret(3, 0);
        assert!(ide.caret().0 != 3);
    }

    #[test]
    fn junction_antes_del_caret_apunta_a_la_correcta() {
        let (c, atoms) = cuerpo_con_atoms(&["A", "B", "C"]);
        let idx = indice(&atoms);
        let mut ide = CuerpoIde::from_cuerpo(&c, &idx);
        // En "A" (línea 0): no hay junction previa.
        ide.set_caret(0, 0);
        assert_eq!(ide.junction_antes_del_caret(), None);
        // En "B" (línea 2): la junction previa es la 0.
        ide.set_caret(2, 0);
        assert_eq!(ide.junction_antes_del_caret(), Some(0));
        // En "C" (línea 4): la junction previa es la 1.
        ide.set_caret(4, 0);
        assert_eq!(ide.junction_antes_del_caret(), Some(1));
    }

    #[test]
    fn linea_de_junction_devuelve_la_linea_vacia_correcta() {
        let (c, atoms) = cuerpo_con_atoms(&["A", "B", "C"]);
        let idx = indice(&atoms);
        let ide = CuerpoIde::from_cuerpo(&c, &idx);
        assert_eq!(ide.linea_de_junction(0), Some(1));
        assert_eq!(ide.linea_de_junction(1), Some(3));
        assert_eq!(ide.linea_de_junction(2), None);
    }

    #[test]
    fn line_tints_asigna_un_color_por_grupo() {
        let (c, atoms) = cuerpo_con_atoms(&["A", "B", "C"]);
        let idx = indice(&atoms);
        let ide = CuerpoIde::from_cuerpo(&c, &idx);
        // Líneas: 0="A", 1="", 2="B", 3="", 4="C". Tres grupos.
        let t = &ide.state.line_tints;
        assert_eq!(t.len(), 5);
        // Cada atom-line tiene tinte del grupo correspondiente.
        assert_eq!(t[0], Some(color_de_grupo(0)));
        assert_eq!(t[2], Some(color_de_grupo(1)));
        assert_eq!(t[4], Some(color_de_grupo(2)));
        // Guardas (líneas 1 y 3): sin tinte.
        assert_eq!(t[1], None);
        assert_eq!(t[3], None);
    }

    #[test]
    fn fundir_junction_unifica_el_color_del_grupo() {
        let (c, atoms) = cuerpo_con_atoms(&["A", "B", "C"]);
        let idx = indice(&atoms);
        let mut ide = CuerpoIde::from_cuerpo(&c, &idx);
        // Fundir junction 0 → atoms A+B forman un solo grupo (0).
        // Atom C sigue siendo grupo 1.
        ide.fundir_junction(0);
        let t = &ide.state.line_tints;
        assert_eq!(t[0], Some(color_de_grupo(0)));
        // Línea 1 deja de ser guarda y hereda el tinte del grupo 0.
        assert_eq!(t[1], Some(color_de_grupo(0)));
        assert_eq!(t[2], Some(color_de_grupo(0)));
        // Junction 1 sigue siendo separador → línea 3 es guarda sin tinte.
        assert_eq!(t[3], None);
        // Atom C es grupo 1 (no 2, porque se fusionó el primero).
        assert_eq!(t[4], Some(color_de_grupo(1)));
    }

    #[test]
    fn separar_revierte_color_a_grupos_originales() {
        let (c, atoms) = cuerpo_con_atoms(&["A", "B"]);
        let idx = indice(&atoms);
        let mut ide = CuerpoIde::from_cuerpo(&c, &idx);
        ide.fundir_junction(0);
        // Tras fundir, ambos atoms son grupo 0 (mismo color).
        assert_eq!(ide.state.line_tints[0], ide.state.line_tints[2]);
        ide.separar_junction(0);
        // Tras separar, colores distintos.
        assert_ne!(ide.state.line_tints[0], ide.state.line_tints[2]);
    }

    #[test]
    fn paleta_ciclica_repite_color_pasados_8_grupos() {
        // 9 atoms → grupo 0..8 → el último debe compartir color con el primero.
        let textos: Vec<&str> = vec!["a", "b", "c", "d", "e", "f", "g", "h", "i"];
        let (c, atoms) = cuerpo_con_atoms(&textos);
        let idx = indice(&atoms);
        let ide = CuerpoIde::from_cuerpo(&c, &idx);
        assert_eq!(ide.state.line_tints[0], ide.state.line_tints[16]);
    }

    #[test]
    fn n_zonas_arranca_igual_a_n_atoms() {
        let (c, atoms) = cuerpo_con_atoms(&["A", "B", "C", "D"]);
        let idx = indice(&atoms);
        let ide = CuerpoIde::from_cuerpo(&c, &idx);
        // Sin junctions fundidas: una zona por atom.
        assert_eq!(ide.n_zonas(), 4);
    }

    #[test]
    fn n_zonas_cae_cuando_fundimos_junctions() {
        let (c, atoms) = cuerpo_con_atoms(&["A", "B", "C", "D"]);
        let idx = indice(&atoms);
        let mut ide = CuerpoIde::from_cuerpo(&c, &idx);
        // Fundir junction 0 → A+B forman una zona, C y D solos: total 3.
        ide.fundir_junction(0);
        assert_eq!(ide.n_zonas(), 3);
        // Fundir también junction 2 → C+D se unen: total 2.
        ide.fundir_junction(2);
        assert_eq!(ide.n_zonas(), 2);
        // Fundir la del medio → todo una sola zona.
        ide.fundir_junction(1);
        assert_eq!(ide.n_zonas(), 1);
    }

    #[test]
    fn n_zonas_cuerpo_vacio() {
        let ide = CuerpoIde::nuevo_vacio();
        assert_eq!(ide.n_zonas(), 0);
    }

    #[test]
    fn zona_de_atom_idx_mapea_grupos() {
        let (c, atoms) = cuerpo_con_atoms(&["A", "B", "C", "D"]);
        let idx = indice(&atoms);
        let mut ide = CuerpoIde::from_cuerpo(&c, &idx);
        ide.fundir_junction(0); // A+B juntos
        assert_eq!(ide.zona_de_atom_idx(0), Some(0));
        assert_eq!(ide.zona_de_atom_idx(1), Some(0));
        assert_eq!(ide.zona_de_atom_idx(2), Some(1));
        assert_eq!(ide.zona_de_atom_idx(3), Some(2));
        assert_eq!(ide.zona_de_atom_idx(99), None);
    }

    #[test]
    fn lineas_de_zona_simple() {
        let (c, atoms) = cuerpo_con_atoms(&["A", "B", "C"]);
        let idx = indice(&atoms);
        let ide = CuerpoIde::from_cuerpo(&c, &idx);
        // Líneas: 0="A", 1="", 2="B", 3="", 4="C".
        assert_eq!(ide.lineas_de_zona(0), Some((0, 0)));
        assert_eq!(ide.lineas_de_zona(1), Some((2, 2)));
        assert_eq!(ide.lineas_de_zona(2), Some((4, 4)));
        assert_eq!(ide.lineas_de_zona(3), None);
    }

    #[test]
    fn lineas_de_zona_con_fusion_cubre_atoms_y_junctions() {
        let (c, atoms) = cuerpo_con_atoms(&["A", "B", "C"]);
        let idx = indice(&atoms);
        let mut ide = CuerpoIde::from_cuerpo(&c, &idx);
        // Fundir junction 0 → zona 0 = atoms A+B → líneas 0..=2 (incluye
        // la línea vacía fundida).
        ide.fundir_junction(0);
        assert_eq!(ide.lineas_de_zona(0), Some((0, 2)));
        // Zona 1 = solo C → línea 4.
        assert_eq!(ide.lineas_de_zona(1), Some((4, 4)));
    }

    #[test]
    fn zona_de_linea_devuelve_none_sobre_guarda() {
        let (c, atoms) = cuerpo_con_atoms(&["A", "B"]);
        let idx = indice(&atoms);
        let ide = CuerpoIde::from_cuerpo(&c, &idx);
        // Línea 1 es separador → sin zona.
        assert_eq!(ide.zona_de_linea(0), Some(0));
        assert_eq!(ide.zona_de_linea(1), None);
        assert_eq!(ide.zona_de_linea(2), Some(1));
    }

    #[test]
    fn ir_a_zona_mueve_el_caret() {
        let (c, atoms) = cuerpo_con_atoms(&["A", "B", "C"]);
        let idx = indice(&atoms);
        let mut ide = CuerpoIde::from_cuerpo(&c, &idx);
        ide.ir_a_zona(2);
        assert_eq!(ide.caret(), (4, 0));
        ide.ir_a_zona(0);
        assert_eq!(ide.caret(), (0, 0));
        // Fuera de rango: no-op (caret se queda donde estaba).
        ide.ir_a_zona(99);
        assert_eq!(ide.caret(), (0, 0));
    }

    #[test]
    fn ir_a_zona_siguiente_y_anterior_con_wrap() {
        let (c, atoms) = cuerpo_con_atoms(&["A", "B", "C"]);
        let idx = indice(&atoms);
        let mut ide = CuerpoIde::from_cuerpo(&c, &idx);
        // `from_cuerpo` deja el caret al final del texto (convención de
        // set_text); plantamos explícitamente en zona 0 para arrancar.
        ide.set_caret(0, 0);
        ide.ir_a_zona_siguiente();
        assert_eq!(ide.caret(), (2, 0));
        ide.ir_a_zona_siguiente();
        assert_eq!(ide.caret(), (4, 0));
        // Wrap desde zona 2 → zona 0.
        ide.ir_a_zona_siguiente();
        assert_eq!(ide.caret(), (0, 0));
        // Anterior desde zona 0 → wrap a la última (línea 4).
        ide.ir_a_zona_anterior();
        assert_eq!(ide.caret(), (4, 0));
    }

    #[test]
    fn seleccionar_zona_planta_anchor_y_extiende_al_final() {
        let (c, atoms) = cuerpo_con_atoms(&["Uno", "Dos", "Tres"]);
        let idx = indice(&atoms);
        let mut ide = CuerpoIde::from_cuerpo(&c, &idx);
        ide.seleccionar_zona(1);
        // Anchor en (2, 0), caret en (2, 3) — "Dos" tiene 3 chars.
        assert_eq!(ide.caret(), (2, 3));
        assert_eq!(ide.state.selected_text().as_deref(), Some("Dos"));
    }

    #[test]
    fn seleccionar_zona_fundida_abarca_lineas_intermedias() {
        let (c, atoms) = cuerpo_con_atoms(&["Uno", "Dos", "Tres"]);
        let idx = indice(&atoms);
        let mut ide = CuerpoIde::from_cuerpo(&c, &idx);
        // Fundimos 0 → zona 0 = "Uno\n\nDos" (línea vacía es contenido).
        ide.fundir_junction(0);
        ide.seleccionar_zona(0);
        let sel = ide.state.selected_text().unwrap_or_default();
        assert!(sel.starts_with("Uno"), "selección debería empezar con 'Uno': {sel:?}");
        assert!(sel.ends_with("Dos"), "selección debería terminar con 'Dos': {sel:?}");
    }

    #[test]
    fn atom_ids_de_zona_devuelve_orden_correcto() {
        let (c, atoms) = cuerpo_con_atoms(&["A", "B", "C", "D"]);
        let idx = indice(&atoms);
        let mut ide = CuerpoIde::from_cuerpo(&c, &idx);
        // Sin fusiones: zona 1 = solo atom B (idx 1).
        assert_eq!(ide.atom_ids_de_zona(1), Some(vec![atoms[1].id]));
        // Tras fundir 0: zona 0 = A+B; zona 1 = C; zona 2 = D.
        ide.fundir_junction(0);
        assert_eq!(
            ide.atom_ids_de_zona(0),
            Some(vec![atoms[0].id, atoms[1].id])
        );
        assert_eq!(ide.atom_ids_de_zona(1), Some(vec![atoms[2].id]));
        assert_eq!(ide.atom_ids_de_zona(2), Some(vec![atoms[3].id]));
        assert_eq!(ide.atom_ids_de_zona(99), None);
    }

    #[test]
    fn zona_del_caret_sigue_al_caret() {
        let (c, atoms) = cuerpo_con_atoms(&["A", "B", "C"]);
        let idx = indice(&atoms);
        let mut ide = CuerpoIde::from_cuerpo(&c, &idx);
        ide.set_caret(0, 0);
        assert_eq!(ide.zona_del_caret(), 0);
        ide.set_caret(2, 0);
        assert_eq!(ide.zona_del_caret(), 1);
        ide.set_caret(4, 0);
        assert_eq!(ide.zona_del_caret(), 2);
    }

    #[test]
    fn recargar_resetea_junctions_a_separador() {
        let (c1, atoms1) = cuerpo_con_atoms(&["A", "B"]);
        let idx1 = indice(&atoms1);
        let mut ide = CuerpoIde::from_cuerpo(&c1, &idx1);
        ide.fundir_junction(0);
        assert_eq!(ide.fundido_junctions, vec![true]);

        let (c2, atoms2) = cuerpo_con_atoms(&["X", "Y", "Z"]);
        let idx2 = indice(&atoms2);
        ide.recargar(&c2, &idx2);
        // El cuerpo nuevo arranca todo como separador.
        assert_eq!(ide.fundido_junctions, vec![false, false]);
        assert_eq!(ide.state.guard_lines, vec![1, 3]);
    }

    // ----- Resolver de estilo --------------------------------------------

    #[test]
    fn spans_estilo_vacio_devuelve_lineas_vacias() {
        let (c, atoms) = cuerpo_con_atoms(&["Hola", "Mundo"]);
        let idx = indice(&atoms);
        let ide = CuerpoIde::from_cuerpo(&c, &idx);
        let spans = spans_estilo_por_linea(&ide, &EstiloLienzo::nuevo());
        assert_eq!(spans.len(), ide.state.line_count());
        assert!(spans.iter().all(|l| l.is_empty()));
    }

    #[test]
    fn spans_estilo_base_pinta_linea_completa_por_zona() {
        let (c, atoms) = cuerpo_con_atoms(&["Hola mundo", "Segundo"]);
        let idx = indice(&atoms);
        let ide = CuerpoIde::from_cuerpo(&c, &idx);
        // Líneas: 0="Hola mundo", 1="" (guarda), 2="Segundo".
        let mut estilo = EstiloLienzo::nuevo();
        estilo.set_base(&EstiloTexto {
            color_fg: Some([255, 0, 0, 255]),
            ..Default::default()
        });
        let spans = spans_estilo_por_linea(&ide, &estilo);
        // Línea de contenido: un span de la línea entera.
        assert_eq!(spans[0].len(), 1);
        assert_eq!(spans[0][0].start_col, 0);
        assert_eq!(spans[0][0].end_col, 10);
        assert!(spans[0][0].fg.is_some());
        // Guarda (línea vacía): sin span (len 0 → skip).
        assert!(spans[1].is_empty());
        // Segunda zona: línea entera también.
        assert_eq!(spans[2].len(), 1);
        assert_eq!(spans[2][0].end_col, 7);
    }

    #[test]
    fn spans_estilo_zona_override_aplica_solo_a_su_zona() {
        let (c, atoms) = cuerpo_con_atoms(&["Hola mundo", "Segundo"]);
        let idx = indice(&atoms);
        let ide = CuerpoIde::from_cuerpo(&c, &idx);
        let mut estilo = EstiloLienzo::nuevo();
        // Sólo la zona 1 tiene estilo (negrita); la 0 queda sin estilo.
        estilo.set_zona(1, &EstiloTexto { weight: Some(700.0), ..Default::default() });
        let spans = spans_estilo_por_linea(&ide, &estilo);
        // Zona 0 sin override y base vacía → sin span.
        assert!(spans[0].is_empty());
        // Zona 1 → span con weight.
        assert_eq!(spans[2].len(), 1);
        assert_eq!(spans[2][0].weight, Some(700.0));
    }

    #[test]
    fn spans_estilo_por_span_se_apila_sobre_la_zona() {
        let (c, atoms) = cuerpo_con_atoms(&["Hola mundo", "Segundo"]);
        let idx = indice(&atoms);
        let ide = CuerpoIde::from_cuerpo(&c, &idx);
        let mut estilo = EstiloLienzo::nuevo();
        estilo.set_base(&EstiloTexto {
            color_fg: Some([10, 20, 30, 255]),
            ..Default::default()
        });
        // Subrayar "Hola" (chars 0..4) del primer átomo.
        estilo.set_span(
            atoms[0].id,
            0,
            4,
            EstiloTexto { underline: Some(true), ..Default::default() },
        );
        let spans = spans_estilo_por_linea(&ide, &estilo);
        // Línea 0: span de zona (entero) + span de selección [0,4).
        assert_eq!(spans[0].len(), 2);
        let sel = &spans[0][1];
        assert_eq!((sel.start_col, sel.end_col), (0, 4));
        assert_eq!(sel.underline, Some(true));
    }
}
