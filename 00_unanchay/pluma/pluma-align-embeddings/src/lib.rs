//! `pluma-align-embeddings` — alineador de cuerpos por embeddings.
//!
//! Toma dos cuerpos, calcula embeddings de cada párrafo vía un
//! `rimay_verbo_core::Provider` (mock determinista, BGE local, Cohere
//! remoto — quien implemente el rasgo) y produce una [`CartaHebras`]
//! cuya fuerza es la similitud coseno entre los embeddings. La política
//! de selección de pares la elige el caller.
//!
//! Este crate vive APARTE de `pluma-align` porque introduce async +
//! dependencia a `rimay-verbo-core`. Mantener `pluma-align` sincrónico y
//! agnóstico permite usarlo en contextos sin runtime async (UI thread,
//! tests rápidos, wawa userspace).

#![forbid(unsafe_code)]

use std::collections::HashMap;

use anyhow::{Context, Result};
use uuid::Uuid;

use pluma_align::{Alineamiento, CartaHebras, OrigenAlineamiento};
use pluma_core::NarrativeAtom;
use pluma_cuerpo::Cuerpo;
use rimay_verbo_core::Provider;

/// Política con la que se eligen pares ganadores en la matriz NxM de
/// similitudes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModoAlineacion {
    /// Para cada átomo del cuerpo A, registrar UNA hebra con el átomo de
    /// B de mayor similitud (siempre que pase el umbral). Un átomo de B
    /// puede recibir varias hebras — caso típico cuando la traducción
    /// fusiona dos párrafos del original en uno.
    MejorParaCadaA,
    /// Greedy mutuo: registrar el par `(a, b)` solo si `a` es el mejor
    /// candidato de `b` Y `b` es el mejor de `a`. Hebras 1↔1, más limpias.
    /// Reduce las hebras y deja sin emparejar lo que es genuinamente
    /// ambiguo — la UI muestra los huérfanos como "sin contraparte".
    MutuoMejor,
}

/// Parámetros de la alineación. Defaults pensados para que un MockProvider
/// (vectores aleatorios) NO produzca hebras espurias: umbral elevado.
#[derive(Debug, Clone, Copy)]
pub struct ParamsAlineacion {
    /// Umbral mínimo de similitud coseno para registrar una hebra. Por
    /// debajo, el par se descarta. Rango razonable con modelos reales:
    /// 0.45–0.7. Para mock random, conviene >= 0.95 (efectivamente nada).
    pub umbral_minimo: f32,
    /// Política de selección.
    pub modo: ModoAlineacion,
}

impl Default for ParamsAlineacion {
    fn default() -> Self {
        Self {
            umbral_minimo: 0.55,
            modo: ModoAlineacion::MutuoMejor,
        }
    }
}

/// Alinea dos cuerpos calculando embeddings de cada párrafo y emparejando
/// según `params`. La carta resultante tiene `OrigenAlineamiento::Embeddings
/// { modelo: provider.model_id().name, timestamp: ahora }`.
///
/// `ahora` es el timestamp (segundos UNIX) que el caller decide — el crate
/// no lee el reloj, para mantener el flujo testeable.
///
/// Errores propagados: del provider, si falla la inferencia. Átomos
/// referenciados que no existan en `atoms` se omiten en silencio (no
/// son alineables si no hay texto).
pub async fn alinear_por_embeddings(
    cuerpo_a: &Cuerpo,
    cuerpo_b: &Cuerpo,
    atoms: &HashMap<Uuid, &NarrativeAtom>,
    provider: &dyn Provider,
    params: &ParamsAlineacion,
    ahora: u64,
) -> Result<CartaHebras> {
    let (ids_a, textos_a) = recolectar_textos(cuerpo_a, atoms);
    let (ids_b, textos_b) = recolectar_textos(cuerpo_b, atoms);

    if ids_a.is_empty() || ids_b.is_empty() {
        return Ok(CartaHebras::nueva().con_par(cuerpo_a.id, cuerpo_b.id));
    }

    let emb_a = provider
        .embed_batch(&textos_a)
        .await
        .context("embedding lote del cuerpo A falló")?;
    let emb_b = provider
        .embed_batch(&textos_b)
        .await
        .context("embedding lote del cuerpo B falló")?;

    // Matriz NxM de similitudes coseno.
    let n = emb_a.len();
    let m = emb_b.len();
    let mut sim = vec![0.0f32; n * m];
    for i in 0..n {
        for j in 0..m {
            let s = emb_a[i].cosine(&emb_b[j]).context(
                "similitud coseno cruzando modelos distintos — provider inconsistente",
            )?;
            sim[i * m + j] = s;
        }
    }

    let modelo = provider.model_id().name.clone();
    let origen = OrigenAlineamiento::Embeddings {
        modelo,
        timestamp: ahora,
    };

    let pares = match params.modo {
        ModoAlineacion::MejorParaCadaA => mejores_por_fila(&sim, n, m, params.umbral_minimo),
        ModoAlineacion::MutuoMejor => mutuos_mejor(&sim, n, m, params.umbral_minimo),
    };

    let mut carta = CartaHebras::nueva().con_par(cuerpo_a.id, cuerpo_b.id);
    for (i, j, s) in pares {
        carta.agregar(Alineamiento::nuevo(ids_a[i], ids_b[j], s, origen.clone()));
    }
    Ok(carta)
}

/// Recolecta (uuids_en_orden, textos_en_orden) de un cuerpo, omitiendo
/// átomos no presentes en el índice.
fn recolectar_textos(
    cuerpo: &Cuerpo,
    atoms: &HashMap<Uuid, &NarrativeAtom>,
) -> (Vec<Uuid>, Vec<String>) {
    let mut ids = Vec::with_capacity(cuerpo.orden.len());
    let mut textos = Vec::with_capacity(cuerpo.orden.len());
    for &id in &cuerpo.orden {
        if let Some(atom) = atoms.get(&id) {
            ids.push(id);
            textos.push(atom.content.as_str().to_string());
        }
    }
    (ids, textos)
}

/// Para cada fila `i` de la matriz, encuentra el `j` con mayor sim y lo
/// emite si supera el umbral. Genera hasta N pares (uno por átomo de A).
fn mejores_por_fila(sim: &[f32], n: usize, m: usize, umbral: f32) -> Vec<(usize, usize, f32)> {
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let mut mejor = (usize::MAX, f32::NEG_INFINITY);
        for j in 0..m {
            let s = sim[i * m + j];
            if s > mejor.1 {
                mejor = (j, s);
            }
        }
        if mejor.0 != usize::MAX && mejor.1 >= umbral {
            out.push((i, mejor.0, mejor.1));
        }
    }
    out
}

/// Greedy mutuo: i↔j solo si i es el mejor candidato de j y j es el mejor
/// de i. Garantiza 1↔1 y descarta pares ambiguos.
fn mutuos_mejor(sim: &[f32], n: usize, m: usize, umbral: f32) -> Vec<(usize, usize, f32)> {
    // Mejor j por cada i.
    let mut mejor_de_a = vec![usize::MAX; n];
    let mut mejor_de_a_s = vec![f32::NEG_INFINITY; n];
    for i in 0..n {
        for j in 0..m {
            let s = sim[i * m + j];
            if s > mejor_de_a_s[i] {
                mejor_de_a_s[i] = s;
                mejor_de_a[i] = j;
            }
        }
    }
    // Mejor i por cada j.
    let mut mejor_de_b = vec![usize::MAX; m];
    let mut mejor_de_b_s = vec![f32::NEG_INFINITY; m];
    for j in 0..m {
        for i in 0..n {
            let s = sim[i * m + j];
            if s > mejor_de_b_s[j] {
                mejor_de_b_s[j] = s;
                mejor_de_b[j] = i;
            }
        }
    }
    // Emisión: solo pares mutuos sobre umbral.
    let mut out = Vec::new();
    for i in 0..n {
        let j = mejor_de_a[i];
        if j == usize::MAX {
            continue;
        }
        if mejor_de_b[j] == i && mejor_de_a_s[i] >= umbral {
            out.push((i, j, mejor_de_a_s[i]));
        }
    }
    out
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use pluma_align::OrigenAlineamiento;
    use pluma_cuerpo::Intencion;
    use rimay_verbo_mock::MockProvider;

    fn cuerpo_con_atomos(
        branch: &str,
        intencion: Intencion,
        textos: &[&str],
    ) -> (Cuerpo, Vec<NarrativeAtom>) {
        let mut c = Cuerpo::nuevo(branch, branch, intencion, 100);
        let atoms: Vec<NarrativeAtom> =
            textos.iter().map(|t| NarrativeAtom::new(*t, branch)).collect();
        for a in &atoms {
            c.agregar(a.id, 101);
        }
        (c, atoms)
    }

    fn indice<'a>(atoms: &'a [NarrativeAtom]) -> HashMap<Uuid, &'a NarrativeAtom> {
        atoms.iter().map(|a| (a.id, a)).collect()
    }

    #[tokio::test]
    async fn textos_identicos_dan_fuerza_uno() {
        let (a, atoms_a) = cuerpo_con_atomos(
            "a",
            Intencion::Original,
            &["alpha", "beta", "gamma"],
        );
        let (b, atoms_b) = cuerpo_con_atomos(
            "b",
            Intencion::Traduccion,
            &["alpha", "beta", "gamma"],
        );
        let mut atoms_all = atoms_a;
        atoms_all.extend(atoms_b);
        let idx = indice(&atoms_all);

        let provider = MockProvider::default();
        // Mock es determinista por texto: misma cadena → mismo vector →
        // coseno = 1. Subimos el umbral para asegurar que solo registra
        // pares de coseno ≈ 1.
        let params = ParamsAlineacion {
            umbral_minimo: 0.99,
            modo: ModoAlineacion::MutuoMejor,
        };
        let carta = alinear_por_embeddings(&a, &b, &idx, &provider, &params, 7).await.unwrap();
        assert_eq!(carta.hebras.len(), 3);
        for h in &carta.hebras {
            assert!(h.fuerza > 0.99);
            match &h.origen {
                OrigenAlineamiento::Embeddings { timestamp, .. } => assert_eq!(*timestamp, 7),
                _ => panic!("origen debería ser Embeddings"),
            }
        }
    }

    #[tokio::test]
    async fn textos_random_no_pasan_umbral_alto() {
        let (a, atoms_a) = cuerpo_con_atomos(
            "a",
            Intencion::Original,
            &["alfa beta gamma", "delta epsilon"],
        );
        let (b, atoms_b) = cuerpo_con_atomos(
            "b",
            Intencion::Traduccion,
            &["lorem ipsum dolor", "consectetur adipiscing"],
        );
        let mut atoms_all = atoms_a;
        atoms_all.extend(atoms_b);
        let idx = indice(&atoms_all);

        let provider = MockProvider::default();
        // Umbral alto: el mock (vectores aleatorios entre textos no
        // idénticos) no debe producir hebras.
        let params = ParamsAlineacion {
            umbral_minimo: 0.95,
            modo: ModoAlineacion::MutuoMejor,
        };
        let carta = alinear_por_embeddings(&a, &b, &idx, &provider, &params, 1).await.unwrap();
        assert!(carta.hebras.is_empty());
    }

    #[tokio::test]
    async fn cuerpo_vacio_devuelve_carta_vacia_sin_llamar_al_provider() {
        // Si uno de los cuerpos está vacío, no hay nada que embeddear.
        let (a, atoms_a) = cuerpo_con_atomos("a", Intencion::Original, &["solo a"]);
        let b = Cuerpo::nuevo("b", "vacio", Intencion::Traduccion, 0);
        let idx = indice(&atoms_a);
        let provider = MockProvider::default();
        let carta = alinear_por_embeddings(
            &a,
            &b,
            &idx,
            &provider,
            &ParamsAlineacion::default(),
            1,
        )
        .await
        .unwrap();
        assert!(carta.hebras.is_empty());
        assert_eq!(carta.cuerpo_a, Some(a.id));
        assert_eq!(carta.cuerpo_b, Some(b.id));
    }

    #[tokio::test]
    async fn mejor_para_cada_a_puede_apuntar_dos_a_uno() {
        // Cuerpo A con 2 textos idénticos a UN texto de B.
        let (a, atoms_a) = cuerpo_con_atomos(
            "a",
            Intencion::Original,
            &["compartido", "compartido"],
        );
        let (b, atoms_b) = cuerpo_con_atomos(
            "b",
            Intencion::Traduccion,
            &["compartido", "diferente"],
        );
        let mut atoms_all = atoms_a;
        atoms_all.extend(atoms_b);
        let idx = indice(&atoms_all);

        let provider = MockProvider::default();
        let params = ParamsAlineacion {
            umbral_minimo: 0.99,
            modo: ModoAlineacion::MejorParaCadaA,
        };
        let carta = alinear_por_embeddings(&a, &b, &idx, &provider, &params, 1).await.unwrap();
        // Las dos filas de A apuntan al mismo j=0 (texto "compartido" de B).
        assert_eq!(carta.hebras.len(), 2);
        let target = carta.hebras[0].atom_b;
        assert_eq!(carta.hebras[1].atom_b, target);
    }

    #[tokio::test]
    async fn mutuo_mejor_descarta_pares_ambiguos() {
        // Dos textos de A idénticos a UN texto de B: con MutuoMejor, solo
        // UN par mutuo gana (B prefiere su mejor único; la otra fila de A
        // queda huérfana).
        let (a, atoms_a) = cuerpo_con_atomos(
            "a",
            Intencion::Original,
            &["compartido", "compartido"],
        );
        let (b, atoms_b) = cuerpo_con_atomos(
            "b",
            Intencion::Traduccion,
            &["compartido", "diferente"],
        );
        let mut atoms_all = atoms_a;
        atoms_all.extend(atoms_b);
        let idx = indice(&atoms_all);

        let provider = MockProvider::default();
        let params = ParamsAlineacion {
            umbral_minimo: 0.99,
            modo: ModoAlineacion::MutuoMejor,
        };
        let carta = alinear_por_embeddings(&a, &b, &idx, &provider, &params, 1).await.unwrap();
        // Solo una hebra: el primer "compartido" de A con "compartido" de B
        // (los dos j=0 candidatos empatan; el algoritmo se queda con el primer i).
        assert_eq!(carta.hebras.len(), 1);
    }
}
