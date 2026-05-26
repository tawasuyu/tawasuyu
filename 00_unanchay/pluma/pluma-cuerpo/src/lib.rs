//! `pluma-cuerpo` — el *cuerpo* (lienzo) de un documento multivista.
//!
//! Un documento pluma deja de ser una sola secuencia lineal de párrafos: es un
//! *haz* de cuerpos. Cada cuerpo recorre el mismo material desde una mirada
//! distinta — el original en español, su traducción al quechua, el resumen
//! en inglés, el comentario crítico, la versión "tono infantil". Todos viven
//! en paralelo, sincronizados por alineamientos párrafo-a-párrafo
//! (ver `pluma-align`).
//!
//! Este crate define solo el cuerpo: una colección ordenada de [`Uuid`]s de
//! `NarrativeAtom`s, con metadatos que explican qué *intención* tiene este
//! cuerpo dentro del haz, y de qué cuerpo madre deriva si es derivado.
//!
//! No define la UI ni la alineación entre cuerpos. Esos viven en
//! `pluma-editor-llimphi` y `pluma-align` respectivamente.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// La identidad de una lengua dentro de un cuerpo: un código corto estilo
/// ISO 639 (`"es"`, `"qu"`, `"en"`). No se restringe la enumeracion para no
/// encerrarnos en un catalogo fijo; el campo es un `String`, y `pluma-localize`
/// (o `rimay-localize`) decide qué significa cada código.
pub type Lengua = String;

/// La razón por la que existe un cuerpo dentro del haz. Si es `Original`, el
/// cuerpo no deriva de ningún otro — es la madre. Cualquier otra variante
/// implica que existe un `derivado_de` apuntando a un cuerpo madre.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Intencion {
    /// Cuerpo "madre" del haz — no deriva de ningún otro.
    Original,
    /// Traducción de la madre a otra lengua.
    Traduccion,
    /// Reescritura con un tono distinto (formal, casual, técnico, infantil…).
    /// La `etiqueta` es libre — la UI la presenta como label de la columna.
    Tono { etiqueta: String },
    /// Resumen de la madre, con un objetivo opcional de palabras.
    Resumen { palabras_objetivo: Option<u32> },
    /// Reescritura libre dictada por un prompt humano.
    Reescritura { prompt: String },
    /// Comentario crítico, glosa o anotación marginal — cada átomo del cuerpo
    /// anotación se alinea al átomo del cuerpo madre que comenta.
    Anotacion,
    /// Cualquier otra cosa que la app quiera categorizar — la etiqueta es libre.
    Custom { kind: String },
}

impl Intencion {
    /// `true` si esta intención implica que el cuerpo deriva de otro. Solo
    /// `Original` es la excepción: vive solo.
    pub fn es_derivada(&self) -> bool {
        !matches!(self, Intencion::Original)
    }
}

/// Metadatos descriptivos de un cuerpo: lo que le explica al usuario y al
/// sistema qué representa este cuerpo dentro del haz.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaCuerpo {
    /// Nombre legible — el rótulo que la UI muestra en la cabecera de la
    /// columna. Libre. Ejemplos: `"es"`, `"qu (Cuzco)"`, `"borrador 2"`,
    /// `"comentario de Ana"`.
    pub nombre_legible: String,
    /// Lengua del cuerpo, si aplica. Útil para encadenar con `rimay-localize`,
    /// motores de embeddings y exportadores. `None` para cuerpos sin lengua
    /// específica (notas formales, listas de tareas, etc.).
    pub lengua: Option<Lengua>,
    /// Por qué existe este cuerpo dentro del haz.
    pub intencion: Intencion,
    /// Cuerpo madre del que deriva, si lo hay. Si `intencion` no es
    /// `Original`, este campo debería estar `Some`; el constructor lo respeta,
    /// y `valida_consistencia` lo verifica.
    pub derivado_de: Option<Uuid>,
    /// Marca temporal (segundos UNIX) del estado de la madre cuando este
    /// cuerpo se regeneró por última vez. Si la madre cambia después de este
    /// timestamp, el cuerpo queda *stale* — la UI lo pinta con la hebra
    /// desaturada. `None` mientras el cuerpo nunca se haya regenerado o sea
    /// `Original`.
    pub fresco_hasta: Option<u64>,
    /// Instante de creación (segundos UNIX). Se fija en el constructor.
    pub creado_en: u64,
    /// Instante de la última modificación de la estructura del cuerpo
    /// (inserciones, removidos, reordenamientos). NO se actualiza cuando
    /// cambia el contenido de un átomo — eso lo gestiona el átomo.
    pub modificado_en: u64,
}

/// El cuerpo (lienzo) en sí: una secuencia ORDENADA de identidades de
/// `NarrativeAtom`s. La identidad estable de un párrafo es su `Uuid`; los
/// alineamientos entre cuerpos hablan en términos de esos `Uuid`s, no de
/// posiciones — así un párrafo movido dentro del cuerpo no rompe sus hebras.
///
/// El cuerpo NO posee los átomos: solo los referencia. La posesión vive en el
/// `NarrativeGraph` (en `pluma-graph`). Esa separación permite que distintos
/// cuerpos compartan átomos (un párrafo idéntico en madre e hija — caso
/// común tras `Intencion::Identidad`) sin duplicación.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cuerpo {
    /// Identidad del cuerpo. Estable: ni una traducción ni un renombre lo
    /// cambian. Es el ancla a la que los alineamientos apuntan.
    pub id: Uuid,
    /// Identificador de rama, en el sentido que ya entiende `pluma-core`. Por
    /// convención: `"es"`, `"qu"`, `"borrador-2"`, `"resumen-en"`. La rama da
    /// también la identidad del `branch_id` que los `NarrativeAtom`s
    /// referencian.
    pub branch_id: String,
    /// Metadatos del cuerpo.
    pub metadatos: MetaCuerpo,
    /// Orden de presentación de los párrafos del cuerpo. Cada `Uuid` debe
    /// existir en el `NarrativeGraph` que aloja al documento; el cuerpo no
    /// lo valida (no conoce el grafo), eso queda para el caller.
    pub orden: Vec<Uuid>,
}

impl Cuerpo {
    /// Crea un cuerpo vacío con la identidad y los metadatos esenciales.
    /// `ahora` es el timestamp (segundos UNIX) que el caller decide — el
    /// crate no toca el reloj para mantenerse `no_std`-amigable y testable.
    pub fn nuevo(
        branch_id: impl Into<String>,
        nombre_legible: impl Into<String>,
        intencion: Intencion,
        ahora: u64,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            branch_id: branch_id.into(),
            metadatos: MetaCuerpo {
                nombre_legible: nombre_legible.into(),
                lengua: None,
                intencion,
                derivado_de: None,
                fresco_hasta: None,
                creado_en: ahora,
                modificado_en: ahora,
            },
            orden: Vec::new(),
        }
    }

    /// Marca este cuerpo como derivado de `madre`, fijando también el
    /// timestamp de frescura con el que se regeneró por primera vez. Útil al
    /// crear cuerpos via `pluma-transform`.
    pub fn deriva_de(mut self, madre: Uuid, fresco_hasta: u64) -> Self {
        self.metadatos.derivado_de = Some(madre);
        self.metadatos.fresco_hasta = Some(fresco_hasta);
        self
    }

    /// Anota la lengua del cuerpo. Útil al encadenar — `Cuerpo::nuevo(...).con_lengua("qu")`.
    pub fn con_lengua(mut self, lengua: impl Into<Lengua>) -> Self {
        self.metadatos.lengua = Some(lengua.into());
        self
    }

    /// Agrega un átomo al final del cuerpo. Actualiza `modificado_en`. No
    /// verifica duplicados: un mismo átomo PUEDE aparecer dos veces si el
    /// caller lo necesita (caso raro, pero no se prohíbe — el cuerpo es
    /// agnóstico).
    pub fn agregar(&mut self, atom_id: Uuid, ahora: u64) {
        self.orden.push(atom_id);
        self.metadatos.modificado_en = ahora;
    }

    /// Inserta un átomo en `indice`. Si `indice > len`, se inserta al final.
    pub fn insertar(&mut self, indice: usize, atom_id: Uuid, ahora: u64) {
        let pos = indice.min(self.orden.len());
        self.orden.insert(pos, atom_id);
        self.metadatos.modificado_en = ahora;
    }

    /// Remueve el primer átomo con `atom_id`. Devuelve `true` si removió.
    pub fn remover(&mut self, atom_id: Uuid, ahora: u64) -> bool {
        if let Some(pos) = self.orden.iter().position(|&id| id == atom_id) {
            self.orden.remove(pos);
            self.metadatos.modificado_en = ahora;
            true
        } else {
            false
        }
    }

    /// Mueve la primera ocurrencia de `atom_id` al nuevo índice. Devuelve
    /// `true` si efectuó el cambio. Un movimiento al mismo índice es no-op
    /// y devuelve `true`. Si `nuevo_indice > len-1`, se mueve al final.
    pub fn mover(&mut self, atom_id: Uuid, nuevo_indice: usize, ahora: u64) -> bool {
        let Some(actual) = self.orden.iter().position(|&id| id == atom_id) else {
            return false;
        };
        let destino = nuevo_indice.min(self.orden.len().saturating_sub(1));
        if actual == destino {
            return true;
        }
        let id = self.orden.remove(actual);
        self.orden.insert(destino, id);
        self.metadatos.modificado_en = ahora;
        true
    }

    /// Posición del primer átomo con `atom_id`, si existe en el cuerpo.
    pub fn posicion(&self, atom_id: Uuid) -> Option<usize> {
        self.orden.iter().position(|&id| id == atom_id)
    }

    /// `true` si el cuerpo deriva de otro (su intención no es `Original`).
    pub fn es_derivado(&self) -> bool {
        self.metadatos.intencion.es_derivada()
    }

    /// `true` si la madre cambió después de `fresco_hasta`. Si no es derivado
    /// o nunca se regeneró, devuelve `false` (no aplica el concepto).
    pub fn es_stale(&self, modificado_madre_en: u64) -> bool {
        match self.metadatos.fresco_hasta {
            Some(ts) => modificado_madre_en > ts,
            None => false,
        }
    }

    /// Marca el cuerpo como recién regenerado: `fresco_hasta = ahora`.
    pub fn marcar_fresco(&mut self, ahora: u64) {
        self.metadatos.fresco_hasta = Some(ahora);
        self.metadatos.modificado_en = ahora;
    }

    /// Verifica la consistencia interna del cuerpo. Útil al cargar de disco o
    /// tras transformaciones que prometen no romper invariantes. Reglas:
    ///
    /// - Si `intencion.es_derivada()`, `derivado_de` debe ser `Some`.
    /// - Si `intencion == Original`, `derivado_de` debe ser `None`.
    /// - `modificado_en >= creado_en`.
    pub fn valida_consistencia(&self) -> Result<(), &'static str> {
        let m = &self.metadatos;
        if m.intencion.es_derivada() && m.derivado_de.is_none() {
            return Err("cuerpo :: intencion derivada sin `derivado_de`");
        }
        if !m.intencion.es_derivada() && m.derivado_de.is_some() {
            return Err("cuerpo :: intencion Original con `derivado_de` no nulo");
        }
        if m.modificado_en < m.creado_en {
            return Err("cuerpo :: `modificado_en` anterior a `creado_en`");
        }
        Ok(())
    }

    /// Serializa el cuerpo a su forma binaria `postcard` — el codec ya
    /// canónico del workspace (lo usa `format`/`akasha` en wawa).
    pub fn serializar(&self) -> Result<Vec<u8>, &'static str> {
        postcard::to_allocvec(self).map_err(|_| "cuerpo :: serializacion fallida")
    }

    /// Reconstruye un cuerpo desde su forma binaria.
    pub fn deserializar(bytes: &[u8]) -> Result<Cuerpo, &'static str> {
        postcard::from_bytes::<Cuerpo>(bytes)
            .map_err(|_| "cuerpo :: deserializacion fallida")
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn ahora_test() -> u64 {
        1_716_724_800
    }

    #[test]
    fn nuevo_es_consistente_y_vacio() {
        let c = Cuerpo::nuevo("es", "es (original)", Intencion::Original, ahora_test());
        assert!(c.orden.is_empty());
        assert!(!c.es_derivado());
        assert!(c.metadatos.derivado_de.is_none());
        assert_eq!(c.metadatos.creado_en, c.metadatos.modificado_en);
        c.valida_consistencia().unwrap();
    }

    #[test]
    fn derivado_exige_madre() {
        // Construir un cuerpo derivado sin madre — debe detectarse.
        let mut c = Cuerpo::nuevo("qu", "qu", Intencion::Traduccion, ahora_test());
        assert!(c.es_derivado());
        assert!(c.valida_consistencia().is_err());

        c.metadatos.derivado_de = Some(Uuid::new_v4());
        c.valida_consistencia().unwrap();
    }

    #[test]
    fn original_no_puede_tener_madre() {
        let mut c = Cuerpo::nuevo("es", "es", Intencion::Original, ahora_test());
        c.metadatos.derivado_de = Some(Uuid::new_v4());
        assert!(c.valida_consistencia().is_err());
    }

    #[test]
    fn agregar_insertar_remover_mover_actualiza_modificado() {
        let mut c = Cuerpo::nuevo("es", "es", Intencion::Original, 100);
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let d = Uuid::new_v4();
        c.agregar(a, 101);
        c.agregar(b, 102);
        c.insertar(1, d, 103);
        // orden: [a, d, b]
        assert_eq!(c.orden, vec![a, d, b]);
        assert_eq!(c.metadatos.modificado_en, 103);
        assert_eq!(c.posicion(d), Some(1));

        // Mover b al inicio.
        assert!(c.mover(b, 0, 104));
        assert_eq!(c.orden, vec![b, a, d]);
        assert_eq!(c.metadatos.modificado_en, 104);

        // Remover a.
        assert!(c.remover(a, 105));
        assert_eq!(c.orden, vec![b, d]);

        // Remover algo que no está.
        assert!(!c.remover(Uuid::new_v4(), 106));
        // El timestamp NO se mueve si no removió.
        assert_eq!(c.metadatos.modificado_en, 105);
    }

    #[test]
    fn insertar_mas_alla_del_final_aterriza_al_final() {
        let mut c = Cuerpo::nuevo("es", "es", Intencion::Original, 0);
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        c.agregar(a, 1);
        c.insertar(99, b, 2);
        assert_eq!(c.orden, vec![a, b]);
    }

    #[test]
    fn stale_solo_aplica_si_madre_se_modifico_despues() {
        let c = Cuerpo::nuevo("qu", "qu", Intencion::Traduccion, 100)
            .deriva_de(Uuid::new_v4(), 200);
        // La madre no se ha tocado desde la regeneración.
        assert!(!c.es_stale(150));
        assert!(!c.es_stale(200));
        // La madre cambió DESPUES.
        assert!(c.es_stale(201));
    }

    #[test]
    fn marcar_fresco_actualiza_ambos_timestamps() {
        let mut c = Cuerpo::nuevo("qu", "qu", Intencion::Traduccion, 100)
            .deriva_de(Uuid::new_v4(), 100);
        c.marcar_fresco(500);
        assert_eq!(c.metadatos.fresco_hasta, Some(500));
        assert_eq!(c.metadatos.modificado_en, 500);
    }

    #[test]
    fn roundtrip_postcard_es_simetrico() {
        let mut c = Cuerpo::nuevo("qu", "quechua del Cuzco", Intencion::Traduccion, 1000)
            .deriva_de(Uuid::new_v4(), 1000)
            .con_lengua("qu");
        c.agregar(Uuid::new_v4(), 1001);
        c.agregar(Uuid::new_v4(), 1002);
        let bytes = c.serializar().unwrap();
        let recuperado = Cuerpo::deserializar(&bytes).unwrap();
        assert_eq!(recuperado, c);
        recuperado.valida_consistencia().unwrap();
    }

    #[test]
    fn intencion_es_derivada_distingue_original_de_lo_demas() {
        assert!(!Intencion::Original.es_derivada());
        assert!(Intencion::Traduccion.es_derivada());
        assert!(Intencion::Tono { etiqueta: "formal".into() }.es_derivada());
        assert!(Intencion::Resumen { palabras_objetivo: Some(200) }.es_derivada());
        assert!(Intencion::Reescritura { prompt: "p".into() }.es_derivada());
        assert!(Intencion::Anotacion.es_derivada());
        assert!(Intencion::Custom { kind: "x".into() }.es_derivada());
    }
}
