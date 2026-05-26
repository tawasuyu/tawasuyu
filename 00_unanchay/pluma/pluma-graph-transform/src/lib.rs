//! `pluma-graph-transform` — pegamento entre `pluma-graph` y `pluma-transform`.
//!
//! Dos helpers que cierran el flujo end-to-end:
//!
//! 1. [`indice_atoms`]: construye el `HashMap<Uuid, &NarrativeAtom>` que los
//!    ejecutores LLM esperan en su `aplicar_con_atoms`. Lo arma a partir
//!    del `NarrativeGraph` actual con cero copias (referencias).
//!
//! 2. [`persistir_producto`]: dado un [`ProductoTransformacion`] devuelto
//!    por un ejecutor, mete los `atoms_nuevos` en el grafo y devuelve el
//!    par `(hija, carta)` listo para mostrar/persistir aparte. La hija
//!    es solo metadatos + orden de Uuids; la carta es la `CartaHebras`
//!    para que la UI la pinte.
//!
//! Por qué un crate separado y no un método en `NarrativeGraph`:
//!
//! - `pluma-graph` debe quedar agnóstico de la idea de transformación —
//!   si mañana hay otra forma de producir átomos, no queremos contaminar
//!   el grafo con ese acoplamiento.
//! - `pluma-transform` no depende de `pluma-graph` por la misma razón en
//!   reverso: el modelo de transformación funciona con cualquier
//!   resolver de átomos (un grafo en disco, una colección en memoria,
//!   un mock). Mantenerlo limpio facilita reusarlo desde wawa o desde
//!   un test.
//!
//! Este crate vive en el intersticio. Lo trae solo quien quiera el
//! pegamento; quien no, sigue con los dos crates por separado.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use uuid::Uuid;

use pluma_align::CartaHebras;
use pluma_core::NarrativeAtom;
use pluma_cuerpo::Cuerpo;
use pluma_graph::NarrativeGraph;
use pluma_transform::ProductoTransformacion;

/// Construye un índice `Uuid → &NarrativeAtom` desde el grafo. Es lo
/// que los ejecutores LLM esperan en `aplicar_con_atoms`. Vive el tiempo
/// del préstamo `&graph`, sin copiar átomos.
pub fn indice_atoms(graph: &NarrativeGraph) -> HashMap<Uuid, &NarrativeAtom> {
    graph.atoms().map(|a| (a.id, a)).collect()
}

/// Persiste el resultado de un ejecutor: mete los `atoms_nuevos` en el
/// grafo (los toma por valor, sin clonar) y devuelve `(hija, carta)`
/// listos para que el caller los inserte en su catálogo de cuerpos +
/// los pase a la vista multilienzo.
///
/// La hija NO se inserta en `NarrativeGraph` — el grafo es de átomos,
/// no de cuerpos. La gestión de cuerpos vive en otro nivel (ver
/// `pluma-store` cuando se cierre).
pub fn persistir_producto(
    graph: &mut NarrativeGraph,
    producto: ProductoTransformacion,
) -> (Cuerpo, CartaHebras) {
    let ProductoTransformacion {
        hija,
        atoms_nuevos,
        carta,
    } = producto;
    for atom in atoms_nuevos {
        graph.insert(atom);
    }
    (hija, carta)
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use pluma_cuerpo::Intencion;
    use pluma_llm_mock::MockChatClient;
    use pluma_transform::{TipoTransformacion, Transformacion};
    use pluma_transform_llm::EjecutorTraducirLlm;
    use pluma_transform_tabla::EjecutorTraducirTabla;
    use pluma_transform::Ejecutor;

    #[test]
    fn indice_atoms_resuelve_los_atomos_del_grafo() {
        let mut g = NarrativeGraph::new();
        let a = NarrativeAtom::new("uno", "es");
        let b = NarrativeAtom::new("dos", "es");
        let (ia, ib) = (a.id, b.id);
        g.insert(a);
        g.insert(b);
        let idx = indice_atoms(&g);
        assert_eq!(idx.len(), 2);
        assert_eq!(idx[&ia].content.as_str(), "uno");
        assert_eq!(idx[&ib].content.as_str(), "dos");
    }

    #[tokio::test]
    async fn persistir_producto_de_tabla_pone_atoms_nuevos_en_grafo() {
        // Sembrar la madre.
        let atom_a = NarrativeAtom::new("uno", "es");
        let atom_b = NarrativeAtom::new("dos", "es");
        let (id_a, id_b) = (atom_a.id, atom_b.id);
        let mut madre = Cuerpo::nuevo("es", "es", Intencion::Original, 0);
        madre.agregar(id_a, 1);
        madre.agregar(id_b, 1);

        let mut g = NarrativeGraph::new();
        g.insert(atom_a);
        g.insert(atom_b);
        assert_eq!(g.len(), 2);

        // Tabla traduce las dos.
        let mut tabla = HashMap::new();
        tabla.insert(id_a, "huk".to_string());
        tabla.insert(id_b, "iskay".to_string());
        let ej = EjecutorTraducirTabla::new(tabla, "qu");
        let t = Transformacion::nueva(
            madre.id,
            Uuid::new_v4(),
            TipoTransformacion::Traducir { lengua_destino: "qu".into() },
            "tester",
            1,
        );
        let producto = ej.aplicar(&t, &madre, 1).await.unwrap();
        assert_eq!(producto.atoms_nuevos.len(), 2);

        let (hija, carta) = persistir_producto(&mut g, producto);

        // El grafo ahora tiene 4 átomos: 2 madre + 2 hija.
        assert_eq!(g.len(), 4);
        // La hija tiene Uuids nuevos (distintos de los de la madre).
        for &id in &hija.orden {
            assert_ne!(id, id_a);
            assert_ne!(id, id_b);
            assert!(g.contains(id));
        }
        // La carta enlaza madre con hija.
        assert_eq!(carta.hebras.len(), 2);
    }

    #[tokio::test]
    async fn ciclo_completo_con_ejecutor_llm_persiste_atoms_nuevos() {
        // Sembrar la madre.
        let atom_a = NarrativeAtom::new("uno", "es");
        let atom_b = NarrativeAtom::new("dos", "es");
        let (id_a, id_b) = (atom_a.id, atom_b.id);
        let mut madre = Cuerpo::nuevo("es", "es", Intencion::Original, 0);
        madre.agregar(id_a, 1);
        madre.agregar(id_b, 1);

        let mut g = NarrativeGraph::new();
        g.insert(atom_a);
        g.insert(atom_b);

        // Mock chat con respuestas indexadas.
        let chat = MockChatClient::default()
            .con_respuesta("uno", "huk")
            .con_respuesta("dos", "iskay");
        let ej = EjecutorTraducirLlm::new(chat, "qu");
        let t = Transformacion::nueva(
            madre.id,
            Uuid::new_v4(),
            TipoTransformacion::Traducir { lengua_destino: "qu".into() },
            "tester",
            1,
        );

        // Flujo que la app real escribe:
        //   1. Construir índice del grafo.
        //   2. ejecutor.aplicar_con_atoms.
        //   3. persistir_producto.
        let idx = indice_atoms(&g);
        let producto = ej.aplicar_con_atoms(&t, &madre, &idx, 1).await.unwrap();
        // `producto` referencia atoms en `idx` que prestan de `g`; tras
        // este `await`, ya tenemos producto OWNED — drop idx y persistir.
        drop(idx);
        let (hija, carta) = persistir_producto(&mut g, producto);

        assert_eq!(g.len(), 4);
        assert_eq!(hija.orden.len(), 2);
        assert_eq!(carta.hebras.len(), 2);
        // Los textos de la hija están en el grafo.
        let textos: Vec<String> = hija
            .orden
            .iter()
            .map(|id| g.get(*id).unwrap().content.as_str().to_string())
            .collect();
        assert_eq!(textos, vec!["huk".to_string(), "iskay".to_string()]);
    }
}
