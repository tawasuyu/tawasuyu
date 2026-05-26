//! `pluma-align` — los alineamientos entre cuerpos paralelos.
//!
//! Un *alineamiento* enlaza un átomo de un cuerpo madre con un átomo de un
//! cuerpo hija (o de cualquier cuerpo del haz: la noción no impone dirección).
//! Llevan asociados una *fuerza* (qué tan correspondientes son los párrafos)
//! y un *origen* (cómo se calculó: a mano, por embeddings o como subproducto
//! de una transformación). La UI los pinta como *hebras*: barras de color
//! verticales entre columnas, con saturación = fuerza y tipo de trazo = origen.
//!
//! Un átomo puede tener múltiples alineamientos:
//!   - 1↔1 — caso traducción típica;
//!   - 1↔N o N↔1 — un párrafo del original se traduce en dos del destino;
//!   - 0↔1 — un párrafo del destino sin contraparte en el original
//!           (texto añadido por el traductor);
//!   - 1↔0 — un párrafo del original que el destino eliminó.
//!
//! El crate no decide la política — solo modela. Las hebras 0↔X se representan
//! como *ausencia* de alineamientos para ese átomo, no como un alineamiento
//! degenerado: que no tenga hebra ES la información.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use pluma_cuerpo::Cuerpo;

/// Cómo se produjo un alineamiento. La UI lo usa para distinguir hebras:
/// continuas (Derivado), punteadas (Embeddings de baja confianza),
/// trazos manuales con marca.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OrigenAlineamiento {
    /// Lo trazó un humano. La autoría queda anotada para auditoría.
    Manual {
        autor: String,
        timestamp: u64,
    },
    /// Lo dedujo un calculador de embeddings (rimay/iniy). `modelo` identifica
    /// la versión del calculador — si cambia, los alineamientos viejos pueden
    /// requerir recálculo.
    Embeddings {
        modelo: String,
        timestamp: u64,
    },
    /// Cae como subproducto de una transformación de `pluma-transform`. Cuando
    /// `Identidad` deriva un cuerpo hija desde una madre, cada átomo
    /// arrastra su contraparte 1:1 — esos son los alineamientos `Derivado`.
    Derivado {
        /// `Uuid` de la `Transformacion` (en `pluma-transform`) que lo emitió.
        transformacion: Uuid,
        timestamp: u64,
    },
}

impl OrigenAlineamiento {
    /// El instante en que este origen fue establecido. Útil para invalidar
    /// alineamientos viejos en bloque ("todos los que tengan modelo X
    /// anterior a hace una semana").
    pub fn timestamp(&self) -> u64 {
        match self {
            OrigenAlineamiento::Manual { timestamp, .. } => *timestamp,
            OrigenAlineamiento::Embeddings { timestamp, .. } => *timestamp,
            OrigenAlineamiento::Derivado { timestamp, .. } => *timestamp,
        }
    }
}

/// Un alineamiento entre dos átomos de dos cuerpos. La dirección
/// (`atom_a → atom_b`) no implica jerarquía — el grafo de cuerpos decide
/// quién deriva de quién; aquí solo se anota el par. La UI pinta una sola
/// hebra por par independientemente del orden.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alineamiento {
    /// Identidad estable del alineamiento — sobrevive a recálculos del par
    /// (si el caller mantiene el mismo `Uuid`).
    pub id: Uuid,
    /// Átomo del cuerpo de la izquierda (el primer término del par).
    pub atom_a: Uuid,
    /// Átomo del cuerpo de la derecha.
    pub atom_b: Uuid,
    /// `[0.0, 1.0]` — qué tan correspondientes son los dos párrafos. `1.0` es
    /// "el mismo párrafo" (o derivación directa); `0.0` no se almacena
    /// (mejor no almacenar nada). La UI mapea esto a saturación de color.
    pub fuerza: f32,
    /// Cómo se calculó este alineamiento.
    pub origen: OrigenAlineamiento,
    /// `true` si el alineamiento sigue siendo válido frente al estado actual
    /// de los cuerpos; `false` si el algoritmo lo marcó como stale tras una
    /// edición. La UI lo usa para desaturar la hebra.
    pub fresco: bool,
}

impl Alineamiento {
    /// Crea un alineamiento nuevo con id aleatorio. La fuerza se sujeta al
    /// intervalo `[0, 1]` — un input fuera de rango se acepta como saturado.
    pub fn nuevo(
        atom_a: Uuid,
        atom_b: Uuid,
        fuerza: f32,
        origen: OrigenAlineamiento,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            atom_a,
            atom_b,
            fuerza: fuerza.clamp(0.0, 1.0),
            origen,
            fresco: true,
        }
    }

    /// `true` si este alineamiento toca el átomo dado, sea por `atom_a` o por
    /// `atom_b`. Útil para listar todas las hebras incidentes a un párrafo.
    pub fn toca(&self, atom_id: Uuid) -> bool {
        self.atom_a == atom_id || self.atom_b == atom_id
    }

    /// Dado uno de los dos átomos del par, devuelve el otro. `None` si el
    /// átomo no participa.
    pub fn contraparte(&self, atom_id: Uuid) -> Option<Uuid> {
        if self.atom_a == atom_id {
            Some(self.atom_b)
        } else if self.atom_b == atom_id {
            Some(self.atom_a)
        } else {
            None
        }
    }
}

/// Una *carta de hebras* entre dos cuerpos: la colección de alineamientos
/// que viven en ese par. Es la unidad que la UI consume: pasa por las hebras
/// en orden y pinta una barra por cada una.
///
/// `cuerpo_a` y `cuerpo_b` se anotan para detectar al vuelo que la carta
/// corresponde al par que la UI espera (evita confundir hebras de pares
/// distintos). El crate no fuerza qué par es a/b vs b/a — el caller decide
/// y se mantiene consistente.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CartaHebras {
    pub cuerpo_a: Option<Uuid>,
    pub cuerpo_b: Option<Uuid>,
    pub hebras: Vec<Alineamiento>,
}

impl CartaHebras {
    /// Una carta vacía, sin par anotado.
    pub fn nueva() -> Self {
        Self::default()
    }

    /// Anota el par al que pertenece esta carta. Útil al ingestarla.
    pub fn con_par(mut self, cuerpo_a: Uuid, cuerpo_b: Uuid) -> Self {
        self.cuerpo_a = Some(cuerpo_a);
        self.cuerpo_b = Some(cuerpo_b);
        self
    }

    /// Agrega una hebra. La carta no deduplica; eso es del caller (las hebras
    /// son por id, no por pares — un par puede tener varias hebras de orígenes
    /// distintos coexistiendo, p. ej. una Manual y una Embeddings de respaldo).
    pub fn agregar(&mut self, hebra: Alineamiento) {
        self.hebras.push(hebra);
    }

    /// Itera las hebras que tocan el átomo dado.
    pub fn hebras_de(&self, atom_id: Uuid) -> impl Iterator<Item = &Alineamiento> {
        self.hebras.iter().filter(move |h| h.toca(atom_id))
    }

    /// Marca como stale todas las hebras cuyo origen sea anterior a
    /// `umbral_ts` — útil tras una edición sustancial de un cuerpo. Devuelve
    /// cuántas se marcaron.
    pub fn marcar_stale_anteriores_a(&mut self, umbral_ts: u64) -> usize {
        let mut n = 0;
        for h in self.hebras.iter_mut() {
            if h.fresco && h.origen.timestamp() < umbral_ts {
                h.fresco = false;
                n += 1;
            }
        }
        n
    }

    /// Serializa la carta a postcard.
    pub fn serializar(&self) -> Result<Vec<u8>, &'static str> {
        postcard::to_allocvec(self).map_err(|_| "carta :: serializacion fallida")
    }

    /// Reconstruye la carta desde postcard.
    pub fn deserializar(bytes: &[u8]) -> Result<CartaHebras, &'static str> {
        postcard::from_bytes::<CartaHebras>(bytes)
            .map_err(|_| "carta :: deserializacion fallida")
    }
}

// =============================================================================
//  Alineadores — estrategias concretas para producir CartaHebras
// =============================================================================

/// Alinea dos cuerpos posición-a-posición, hasta agotar el más corto. Es el
/// alineador trivial: útil como baseline, como salida natural de una
/// transformación `Identidad`, o cuando el usuario importa textos que sabe
/// estructurados con la misma cantidad de párrafos. La `fuerza` es siempre
/// 1.0 — el alineador no juzga similitud semántica, asume que la posición
/// es verdad. Los párrafos sobrantes del cuerpo más largo quedan sin hebra
/// (no se inventan alineamientos huérfanos).
///
/// `origen` se aplica idéntico a cada hebra producida.
pub fn alinear_uno_a_uno(
    cuerpo_a: &Cuerpo,
    cuerpo_b: &Cuerpo,
    origen: OrigenAlineamiento,
) -> CartaHebras {
    let mut carta = CartaHebras::nueva().con_par(cuerpo_a.id, cuerpo_b.id);
    for (a, b) in cuerpo_a.orden.iter().zip(cuerpo_b.orden.iter()) {
        carta.agregar(Alineamiento::nuevo(*a, *b, 1.0, origen.clone()));
    }
    carta
}

/// Alinea dos cuerpos a partir de una *tabla* explícita de pares
/// `(atom_a, atom_b, fuerza)`. Es el camino para alineamientos manuales —
/// el editor recibe los pares del usuario y los confirma de un golpe—.
/// Los pares cuyos átomos no aparezcan en los respectivos cuerpos se
/// descartan en silencio: alinear lo que no existe no tiene sentido.
pub fn alinear_explicito(
    cuerpo_a: &Cuerpo,
    cuerpo_b: &Cuerpo,
    pares: &[(Uuid, Uuid, f32)],
    origen: OrigenAlineamiento,
) -> CartaHebras {
    // Indexar los cuerpos para chequeo de pertenencia O(1).
    let en_a: HashMap<Uuid, ()> = cuerpo_a.orden.iter().map(|&id| (id, ())).collect();
    let en_b: HashMap<Uuid, ()> = cuerpo_b.orden.iter().map(|&id| (id, ())).collect();
    let mut carta = CartaHebras::nueva().con_par(cuerpo_a.id, cuerpo_b.id);
    for &(a, b, fuerza) in pares {
        if en_a.contains_key(&a) && en_b.contains_key(&b) {
            carta.agregar(Alineamiento::nuevo(a, b, fuerza, origen.clone()));
        }
    }
    carta
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use pluma_cuerpo::Intencion;

    fn cuerpos_paralelos(n: usize) -> (Cuerpo, Cuerpo, Vec<Uuid>, Vec<Uuid>) {
        let mut a = Cuerpo::nuevo("es", "es", Intencion::Original, 100);
        let mut b = Cuerpo::nuevo("qu", "qu", Intencion::Traduccion, 100);
        // En este test no validamos consistencia: nos importa la alineación.
        let ids_a: Vec<Uuid> = (0..n).map(|_| Uuid::new_v4()).collect();
        let ids_b: Vec<Uuid> = (0..n).map(|_| Uuid::new_v4()).collect();
        for &id in &ids_a { a.agregar(id, 101); }
        for &id in &ids_b { b.agregar(id, 101); }
        (a, b, ids_a, ids_b)
    }

    #[test]
    fn nuevo_alineamiento_clampea_fuerza() {
        let h_alto = Alineamiento::nuevo(
            Uuid::new_v4(), Uuid::new_v4(), 5.0,
            OrigenAlineamiento::Manual { autor: "yo".into(), timestamp: 1 },
        );
        let h_bajo = Alineamiento::nuevo(
            Uuid::new_v4(), Uuid::new_v4(), -0.3,
            OrigenAlineamiento::Manual { autor: "yo".into(), timestamp: 1 },
        );
        assert_eq!(h_alto.fuerza, 1.0);
        assert_eq!(h_bajo.fuerza, 0.0);
        assert!(h_alto.fresco);
    }

    #[test]
    fn contraparte_y_toca_funcionan_en_ambos_sentidos() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let h = Alineamiento::nuevo(
            a, b, 0.8,
            OrigenAlineamiento::Manual { autor: "x".into(), timestamp: 0 },
        );
        assert!(h.toca(a));
        assert!(h.toca(b));
        assert!(!h.toca(Uuid::new_v4()));
        assert_eq!(h.contraparte(a), Some(b));
        assert_eq!(h.contraparte(b), Some(a));
        assert_eq!(h.contraparte(Uuid::new_v4()), None);
    }

    #[test]
    fn alinear_uno_a_uno_empareja_hasta_el_mas_corto() {
        let (a, b, ids_a, ids_b) = cuerpos_paralelos(3);
        // Acortar b a 2 párrafos.
        let mut b = b;
        b.remover(ids_b[2], 102);
        let carta = alinear_uno_a_uno(
            &a, &b,
            OrigenAlineamiento::Manual { autor: "x".into(), timestamp: 200 },
        );
        assert_eq!(carta.hebras.len(), 2);
        assert_eq!(carta.cuerpo_a, Some(a.id));
        assert_eq!(carta.cuerpo_b, Some(b.id));
        // Los pares son (ids_a[0], ids_b[0]) y (ids_a[1], ids_b[1]).
        assert_eq!(carta.hebras[0].atom_a, ids_a[0]);
        assert_eq!(carta.hebras[0].atom_b, ids_b[0]);
        assert_eq!(carta.hebras[1].atom_a, ids_a[1]);
        assert_eq!(carta.hebras[1].atom_b, ids_b[1]);
        // Fuerza al máximo — es alineamiento posicional, no semántico.
        assert!(carta.hebras.iter().all(|h| h.fuerza == 1.0));
        // ids_a[2] queda huérfano: ninguna hebra lo toca.
        assert!(carta.hebras_de(ids_a[2]).next().is_none());
    }

    #[test]
    fn alinear_explicito_descarta_pares_ajenos() {
        let (a, b, ids_a, ids_b) = cuerpos_paralelos(2);
        let huerfano = Uuid::new_v4();
        let pares = vec![
            (ids_a[0], ids_b[0], 0.95),
            (ids_a[1], ids_b[1], 0.80),
            (huerfano, ids_b[0], 0.99), // este se descarta
            (ids_a[0], huerfano, 0.50), // y este también
        ];
        let carta = alinear_explicito(
            &a, &b, &pares,
            OrigenAlineamiento::Manual { autor: "ana".into(), timestamp: 300 },
        );
        assert_eq!(carta.hebras.len(), 2);
        assert!((carta.hebras[0].fuerza - 0.95).abs() < 1e-6);
        assert!((carta.hebras[1].fuerza - 0.80).abs() < 1e-6);
    }

    #[test]
    fn hebras_de_filtra_por_atomo_en_ambos_lados() {
        let (a, b, ids_a, ids_b) = cuerpos_paralelos(3);
        let carta = alinear_uno_a_uno(
            &a, &b,
            OrigenAlineamiento::Manual { autor: "x".into(), timestamp: 1 },
        );
        // Cada átomo de a participa en exactamente UNA hebra.
        for &id in &ids_a {
            assert_eq!(carta.hebras_de(id).count(), 1);
        }
        for &id in &ids_b {
            assert_eq!(carta.hebras_de(id).count(), 1);
        }
    }

    #[test]
    fn marcar_stale_solo_toca_anteriores_al_umbral() {
        let (a, b, ids_a, ids_b) = cuerpos_paralelos(2);
        let mut carta = CartaHebras::nueva().con_par(a.id, b.id);
        carta.agregar(Alineamiento::nuevo(
            ids_a[0], ids_b[0], 1.0,
            OrigenAlineamiento::Embeddings { modelo: "m1".into(), timestamp: 100 },
        ));
        carta.agregar(Alineamiento::nuevo(
            ids_a[1], ids_b[1], 1.0,
            OrigenAlineamiento::Embeddings { modelo: "m1".into(), timestamp: 500 },
        ));
        let n = carta.marcar_stale_anteriores_a(300);
        assert_eq!(n, 1);
        assert!(!carta.hebras[0].fresco);
        assert!(carta.hebras[1].fresco);

        // Llamarlo de nuevo no vuelve a contar el ya stale.
        let n2 = carta.marcar_stale_anteriores_a(300);
        assert_eq!(n2, 0);
    }

    #[test]
    fn roundtrip_postcard_de_carta() {
        let (a, b, _, _) = cuerpos_paralelos(2);
        let carta = alinear_uno_a_uno(
            &a, &b,
            OrigenAlineamiento::Derivado { transformacion: Uuid::new_v4(), timestamp: 7 },
        );
        let bytes = carta.serializar().unwrap();
        let recuperada = CartaHebras::deserializar(&bytes).unwrap();
        assert_eq!(recuperada, carta);
    }
}
