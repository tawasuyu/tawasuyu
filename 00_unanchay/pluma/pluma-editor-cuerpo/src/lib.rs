//! `pluma-editor-cuerpo` — sincronía entre un cuerpo (lista ordenada de
//! `NarrativeAtom`s) y un único buffer de texto plano editable.
//!
//! La idea del editor multilienzo es **un solo control para la página
//! entera**: el usuario ve todos los párrafos concatenados en un
//! `text-editor` IDE estándar (cursor libre, selección entre párrafos,
//! undo global) — pero por debajo cada párrafo sigue siendo un
//! `NarrativeAtom` con `Uuid` propio. Hebras, alineamientos,
//! transformaciones LLM, persistencia: todo lo que pluma ya hace, sigue
//! funcionando sin saber que la UI muestra un buffer único.
//!
//! Este crate cubre la traducción en los dos sentidos:
//!
//! - **Cuerpo → texto**: concatena los atoms con `\n\n` entre cada uno.
//!   Doble salto es el separador natural de párrafo en markdown y un
//!   usuario no escribiría doble enter dentro de un mismo párrafo.
//!
//! - **Texto → cuerpo**: dado el texto editado + el orden previo de
//!   atoms + un índice de los contenidos viejos, computa el mínimo
//!   diff (mutar / crear / eliminar atoms) para que el cuerpo refleje
//!   el texto. Greedy con anclas por contenido: si un párrafo del texto
//!   coincide byte-a-byte con un atom viejo, lo reusamos (UUID
//!   preservado — las hebras siguen vigentes).
//!
//! El crate NO renderiza nada. La UI que reuse el `text-editor` IDE
//! (con sus EditorState/cursor/undo/highlight) vive arriba y consume
//! estas estructuras para sincronizar el `Buffer` de ropey con el grafo.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use uuid::Uuid;

use pluma_core::NarrativeAtom;
use pluma_cuerpo::Cuerpo;

/// Separador canónico entre párrafos en el buffer plano.
/// Doble salto: lo entiende cualquier markdown reader y nadie lo usa
/// dentro de un párrafo normal.
pub const SEPARADOR: &str = "\n\n";

/// Diferencia entre dos estados del cuerpo. `EditorCuerpo::diff`
/// produce esta lista en orden de aplicación; el caller la consume
/// secuencialmente.
#[derive(Debug, Clone, PartialEq)]
pub enum CambioAtom {
    /// El atom `id` existe en el cuerpo viejo Y en el nuevo, pero su
    /// contenido cambió. Aplicar: `graph.get_mut(id).set_content(...)`
    /// + `propagate_mutation(id)`.
    Mutar { id: Uuid, texto_nuevo: String },
    /// Un párrafo nuevo apareció. Crear un `NarrativeAtom` con el texto
    /// y agregarlo al cuerpo en la posición indicada (0-based).
    Crear { texto: String, posicion: usize },
    /// El atom `id` ya no aparece en el texto. Removerlo del cuerpo
    /// (el atom mismo puede quedar en el grafo — el usuario decide).
    Eliminar { id: Uuid },
}

/// Vista plana de un cuerpo como texto editable + mapeo a los Uuids
/// originales.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorCuerpo {
    /// Texto plano del cuerpo entero: párrafos concatenados con
    /// `SEPARADOR` entre cada uno. El text-editor IDE edita esto.
    pub texto: String,
    /// Uuids de los atoms del cuerpo en el orden en que aparecen en
    /// `texto`. `atom_ids.len()` siempre coincide con la cantidad de
    /// párrafos no-vacíos en `texto` AL CONSTRUIRSE (después de un
    /// edit del usuario puede divergir — la función `diff` resuelve).
    pub atom_ids: Vec<Uuid>,
}

impl EditorCuerpo {
    /// Construye un editor a partir de un cuerpo + el índice de atoms
    /// del grafo. Los atoms cuyo Uuid no esté en el índice se omiten
    /// (cuerpo huérfano — un caso de datos corruptos).
    pub fn from_cuerpo(cuerpo: &Cuerpo, atoms: &HashMap<Uuid, &NarrativeAtom>) -> Self {
        let mut chunks: Vec<&str> = Vec::with_capacity(cuerpo.orden.len());
        let mut ids: Vec<Uuid> = Vec::with_capacity(cuerpo.orden.len());
        for id in &cuerpo.orden {
            if let Some(atom) = atoms.get(id) {
                chunks.push(atom.content.as_str());
                ids.push(*id);
            }
        }
        let texto = chunks.join(SEPARADOR);
        Self {
            texto,
            atom_ids: ids,
        }
    }

    /// Lista los párrafos actuales del texto (split por `SEPARADOR`,
    /// trim de espacios alrededor, vacíos descartados). El resultado
    /// puede tener tamaño distinto a `atom_ids` después de una edición
    /// que insertó/eliminó separadores.
    pub fn parrafos(&self) -> Vec<&str> {
        self.texto
            .split(SEPARADOR)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect()
    }

    /// Reemplaza el texto entero — equivalente a aplicar la salida del
    /// `text-editor` IDE de un golpe. No toca `atom_ids`; el caller
    /// debe llamar `diff` después para sincronizar el cuerpo.
    pub fn set_texto(&mut self, nuevo: impl Into<String>) {
        self.texto = nuevo.into();
    }

    /// Calcula el diff mínimo para que el cuerpo (referenciado por
    /// `atom_ids`) refleje el `texto` actual. Estrategia greedy con
    /// anclas por contenido:
    ///
    /// 1. Recorre los párrafos del texto y los atoms originales en
    ///    paralelo.
    /// 2. Si el párrafo i coincide byte-a-byte con el atom i del
    ///    `atoms_originales`, no hay cambio para esa posición.
    /// 3. Si difiere, emite `Mutar` reusando el Uuid del atom i (las
    ///    hebras siguen apuntando al mismo Uuid).
    /// 4. Si el texto tiene MÁS párrafos que los atoms originales, los
    ///    sobrantes son `Crear` al final.
    /// 5. Si el texto tiene MENOS, los atoms sobrantes son `Eliminar`.
    ///
    /// `atoms_originales` debe contener los Uuids de `self.atom_ids`
    /// (los del cuerpo cuando se construyó el editor). El caller suele
    /// recolectarlos del grafo justo antes de llamar a `diff`.
    pub fn diff(
        &self,
        atoms_originales: &HashMap<Uuid, &NarrativeAtom>,
    ) -> Vec<CambioAtom> {
        let parrafos_nuevos = self.parrafos();
        let mut cambios = Vec::new();
        let n = parrafos_nuevos.len();
        let m = self.atom_ids.len();
        let comun = n.min(m);

        for i in 0..comun {
            let id_viejo = self.atom_ids[i];
            let texto_nuevo = parrafos_nuevos[i].to_string();
            let texto_viejo = atoms_originales
                .get(&id_viejo)
                .map(|a| a.content.as_str())
                .unwrap_or("");
            if texto_viejo != texto_nuevo {
                cambios.push(CambioAtom::Mutar {
                    id: id_viejo,
                    texto_nuevo,
                });
            }
        }
        // Sobrantes del texto: párrafos nuevos.
        for (offset, p) in parrafos_nuevos.iter().enumerate().skip(comun) {
            cambios.push(CambioAtom::Crear {
                texto: p.to_string(),
                posicion: offset,
            });
        }
        // Sobrantes del cuerpo: atoms a eliminar.
        for &id in self.atom_ids.iter().skip(comun) {
            cambios.push(CambioAtom::Eliminar { id });
        }
        cambios
    }

    /// Aplica una lista de cambios al `atom_ids` del editor — útil tras
    /// que el caller persistió los cambios en el grafo. Devuelve los
    /// Uuids de los atoms recién creados, en orden, para que el caller
    /// los asigne a los `Crear` reales (el editor no sabe generar
    /// `Uuid`s — eso lo hace `NarrativeAtom::new` arriba).
    ///
    /// `nuevos_ids` debe tener al menos tantos elementos como cambios
    /// `Crear` haya. Si tiene menos, los `Crear` sobrantes se descartan;
    /// si más, se ignoran los extras.
    pub fn aplicar_cambios(&mut self, cambios: &[CambioAtom], nuevos_ids: &[Uuid]) {
        let mut idx_nuevo = 0;
        let mut a_eliminar: Vec<Uuid> = Vec::new();
        for c in cambios {
            match c {
                CambioAtom::Mutar { .. } => {} // el atom_id viejo se reusa
                CambioAtom::Crear { .. } => {
                    if let Some(&id) = nuevos_ids.get(idx_nuevo) {
                        self.atom_ids.push(id);
                        idx_nuevo += 1;
                    }
                }
                CambioAtom::Eliminar { id } => {
                    a_eliminar.push(*id);
                }
            }
        }
        self.atom_ids.retain(|id| !a_eliminar.contains(id));
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
    fn from_cuerpo_concatena_con_separador_y_mantiene_orden() {
        let (c, atoms) = cuerpo_con_atoms(&["Uno.", "Dos.", "Tres."]);
        let idx = indice(&atoms);
        let ed = EditorCuerpo::from_cuerpo(&c, &idx);
        assert_eq!(ed.texto, "Uno.\n\nDos.\n\nTres.");
        assert_eq!(ed.atom_ids, vec![atoms[0].id, atoms[1].id, atoms[2].id]);
    }

    #[test]
    fn parrafos_split_y_trim_de_vacios() {
        let (c, atoms) = cuerpo_con_atoms(&["Uno", "Dos"]);
        let idx = indice(&atoms);
        let mut ed = EditorCuerpo::from_cuerpo(&c, &idx);
        // El usuario agrega espacios y un párrafo vacío entre medio:
        ed.set_texto("  Uno  \n\n\n\n  Dos  ");
        let p = ed.parrafos();
        assert_eq!(p, vec!["Uno", "Dos"]);
    }

    #[test]
    fn sin_cambios_diff_vacio() {
        let (c, atoms) = cuerpo_con_atoms(&["uno", "dos"]);
        let idx = indice(&atoms);
        let ed = EditorCuerpo::from_cuerpo(&c, &idx);
        assert!(ed.diff(&idx).is_empty());
    }

    #[test]
    fn mutar_un_parrafo_emite_mutar_con_uuid_preservado() {
        let (c, atoms) = cuerpo_con_atoms(&["uno", "dos", "tres"]);
        let idx = indice(&atoms);
        let mut ed = EditorCuerpo::from_cuerpo(&c, &idx);
        // El usuario edita el segundo párrafo.
        ed.set_texto("uno\n\nDOS!\n\ntres");
        let d = ed.diff(&idx);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0], CambioAtom::Mutar {
            id: atoms[1].id,
            texto_nuevo: "DOS!".to_string(),
        });
    }

    #[test]
    fn agregar_parrafos_emite_crear() {
        let (c, atoms) = cuerpo_con_atoms(&["uno", "dos"]);
        let idx = indice(&atoms);
        let mut ed = EditorCuerpo::from_cuerpo(&c, &idx);
        ed.set_texto("uno\n\ndos\n\ntres\n\ncuatro");
        let d = ed.diff(&idx);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0], CambioAtom::Crear {
            texto: "tres".to_string(),
            posicion: 2,
        });
        assert_eq!(d[1], CambioAtom::Crear {
            texto: "cuatro".to_string(),
            posicion: 3,
        });
    }

    #[test]
    fn eliminar_parrafos_emite_eliminar_para_los_ids_sobrantes() {
        let (c, atoms) = cuerpo_con_atoms(&["uno", "dos", "tres", "cuatro"]);
        let idx = indice(&atoms);
        let mut ed = EditorCuerpo::from_cuerpo(&c, &idx);
        // El usuario borra los dos últimos.
        ed.set_texto("uno\n\ndos");
        let d = ed.diff(&idx);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0], CambioAtom::Eliminar { id: atoms[2].id });
        assert_eq!(d[1], CambioAtom::Eliminar { id: atoms[3].id });
    }

    #[test]
    fn cambios_mixtos_solo_emiten_lo_que_cambia() {
        let (c, atoms) = cuerpo_con_atoms(&["uno", "dos", "tres"]);
        let idx = indice(&atoms);
        let mut ed = EditorCuerpo::from_cuerpo(&c, &idx);
        // Cambia el primero ("uno" → "UNO!"), conserva el segundo,
        // cambia el tercero ("tres" → "nuevo"). 3 párrafos, 3 atoms.
        ed.set_texto("UNO!\n\ndos\n\nnuevo");
        let d = ed.diff(&idx);
        // Solo dos Mutar — el segundo párrafo coincide byte-a-byte y
        // se omite sin emitir cambio.
        assert_eq!(d.len(), 2);
        match &d[0] {
            CambioAtom::Mutar { id, texto_nuevo } => {
                assert_eq!(*id, atoms[0].id);
                assert_eq!(texto_nuevo, "UNO!");
            }
            otro => panic!("esperaba Mutar(0), fue {otro:?}"),
        }
        match &d[1] {
            CambioAtom::Mutar { id, texto_nuevo } => {
                assert_eq!(*id, atoms[2].id);
                assert_eq!(texto_nuevo, "nuevo");
            }
            otro => panic!("esperaba Mutar(2), fue {otro:?}"),
        }
    }

    #[test]
    fn aplicar_cambios_extiende_atom_ids_para_crear_y_remueve_eliminar() {
        let (c, atoms) = cuerpo_con_atoms(&["uno", "dos"]);
        let idx = indice(&atoms);
        let mut ed = EditorCuerpo::from_cuerpo(&c, &idx);
        // Texto nuevo: dos crear + un eliminar.
        ed.set_texto("uno\n\ntres\n\ncuatro\n\ncinco");
        let cambios = ed.diff(&idx);
        // 1 Mutar(dos → tres) + 2 Crear(cuatro, cinco) + 0 Eliminar.
        // ATOM ORIGINAL: ["uno", "dos"] (2 items)
        // PARRAFOS NUEVOS: ["uno", "tres", "cuatro", "cinco"] (4 items)
        // comun = 2 → Mutar(atoms[1].id, "tres") (uno está igual).
        // Crear "cuatro" pos 2, Crear "cinco" pos 3.
        let nuevos_ids: Vec<Uuid> = vec![Uuid::new_v4(), Uuid::new_v4()];
        ed.aplicar_cambios(&cambios, &nuevos_ids);
        assert_eq!(ed.atom_ids.len(), 4);
        assert_eq!(ed.atom_ids[0], atoms[0].id);
        assert_eq!(ed.atom_ids[1], atoms[1].id); // reusado en Mutar
        assert_eq!(ed.atom_ids[2], nuevos_ids[0]);
        assert_eq!(ed.atom_ids[3], nuevos_ids[1]);
    }

    #[test]
    fn aplicar_cambios_remueve_atom_ids_eliminados() {
        let (c, atoms) = cuerpo_con_atoms(&["uno", "dos", "tres"]);
        let idx = indice(&atoms);
        let mut ed = EditorCuerpo::from_cuerpo(&c, &idx);
        ed.set_texto("uno");
        let cambios = ed.diff(&idx);
        // 0 Mutar + 0 Crear + 2 Eliminar (dos, tres).
        ed.aplicar_cambios(&cambios, &[]);
        assert_eq!(ed.atom_ids, vec![atoms[0].id]);
    }
}
