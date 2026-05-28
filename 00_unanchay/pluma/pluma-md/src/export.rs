//! Exportador: `Cuerpo` + atoms → markdown.
//!
//! El contraparte de [`crate::parse_md`]. Concatena el contenido de cada
//! atom del cuerpo en el orden de `cuerpo.orden` separándolos con `\n\n`
//! (blank line, el separador GFM de bloque).
//!
//! Lossy en formato: si los atoms se importaron con `parse_md`, sus
//! prefijos de heading (`# `, `## `) ya están en el contenido — así que
//! `to_md` los preserva. Listas y otros bloques que pulldown aplanó NO
//! se reconstruyen: salen como párrafos. Para preservar formato fino,
//! exportá a otro formato o guardá el .md original junto al cuerpo.
//!
//! Pensado para ser igual de delgado que `parse_md`: un solo helper
//! reutilizable por la UI y los tests, sin depender de `pluma-store` ni
//! del runtime.

use std::collections::HashMap;

use pluma_core::NarrativeAtom;
use pluma_cuerpo::Cuerpo;
use uuid::Uuid;

/// Concatena `cuerpo.orden` → atom.content con `\n\n`. Atoms ausentes
/// del índice se saltan silenciosamente (puede pasar si el caller pasa
/// un subconjunto del grafo). Devuelve `""` cuando el cuerpo está
/// vacío o ninguno de sus atoms resolvió.
pub fn to_md(cuerpo: &Cuerpo, atoms: &HashMap<Uuid, NarrativeAtom>) -> String {
    let mut out = String::new();
    for atom_id in &cuerpo.orden {
        let Some(atom) = atoms.get(atom_id) else {
            continue;
        };
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&atom.content);
    }
    out
}

/// Variante que acepta el índice por `&NarrativeAtom` — útil cuando el
/// caller tiene los atoms prestados (típico del flujo de transform que
/// arma índices `HashMap<Uuid, &NarrativeAtom>`).
pub fn to_md_borrow(cuerpo: &Cuerpo, atoms: &HashMap<Uuid, &NarrativeAtom>) -> String {
    let mut out = String::new();
    for atom_id in &cuerpo.orden {
        let Some(atom) = atoms.get(atom_id) else {
            continue;
        };
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&atom.content);
    }
    out
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use pluma_cuerpo::Intencion;

    fn cuerpo_y_atoms(textos: &[&str]) -> (Cuerpo, HashMap<Uuid, NarrativeAtom>) {
        let mut c = Cuerpo::nuevo("es", "es", Intencion::Original, 0);
        let mut map = HashMap::new();
        for t in textos {
            let atom = NarrativeAtom::new(*t, "es");
            c.agregar(atom.id, 0);
            map.insert(atom.id, atom);
        }
        (c, map)
    }

    #[test]
    fn to_md_concatena_en_orden_con_doble_newline() {
        let (c, atoms) = cuerpo_y_atoms(&["Uno.", "Dos.", "Tres."]);
        assert_eq!(to_md(&c, &atoms), "Uno.\n\nDos.\n\nTres.");
    }

    #[test]
    fn to_md_devuelve_vacio_si_cuerpo_vacio() {
        let c = Cuerpo::nuevo("es", "es", Intencion::Original, 0);
        let atoms: HashMap<Uuid, NarrativeAtom> = HashMap::new();
        assert_eq!(to_md(&c, &atoms), "");
    }

    #[test]
    fn to_md_salta_atoms_ausentes_del_indice() {
        let (mut c, mut atoms) = cuerpo_y_atoms(&["A", "B", "C"]);
        // Sacar el del medio del índice (pero dejarlo en el orden).
        let id_b = c.orden[1];
        atoms.remove(&id_b);
        assert_eq!(to_md(&c, &atoms), "A\n\nC");
    }

    #[test]
    fn to_md_preserva_prefijos_de_heading() {
        let (mut c, atoms) = cuerpo_y_atoms(&[]);
        let h1 = NarrativeAtom::new("# Título", "es");
        let p = NarrativeAtom::new("Párrafo bajo el título.", "es");
        c.agregar(h1.id, 1);
        c.agregar(p.id, 1);
        let mut map = atoms;
        map.insert(h1.id, h1);
        map.insert(p.id, p);
        assert_eq!(to_md(&c, &map), "# Título\n\nPárrafo bajo el título.");
    }

    #[test]
    fn to_md_borrow_y_to_md_dan_el_mismo_resultado() {
        let (c, atoms) = cuerpo_y_atoms(&["uno", "dos"]);
        let borrow: HashMap<Uuid, &NarrativeAtom> =
            atoms.iter().map(|(k, v)| (*k, v)).collect();
        assert_eq!(to_md(&c, &atoms), to_md_borrow(&c, &borrow));
    }
}
