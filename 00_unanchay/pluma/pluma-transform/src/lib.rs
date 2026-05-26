//! `pluma-transform` — transformaciones declarativas que derivan un cuerpo de
//! otro dentro del haz.
//!
//! Una `Transformacion` describe la *receta*: qué tipo de transformación
//! (traducir, cambiar de tono, resumir, reescribir, etc.), con qué parámetros,
//! desde qué cuerpo madre, hacia qué cuerpo hija. Un `Ejecutor` la aplica:
//! lee la madre, produce los `NarrativeAtom`s de la hija (si los crea) y la
//! `CartaHebras` que enlaza madre con hija.
//!
//! Este crate define:
//!   - los tipos `Transformacion` y `TipoTransformacion`;
//!   - el rasgo `Ejecutor`;
//!   - una implementación concreta, `EjecutorIdentidad`, que no necesita
//!     LLM ni embeddings: la hija comparte los `Uuid` de la madre y la carta
//!     es 1↔1 con `Derivado`. Sirve para validar el flujo end-to-end sin
//!     conectar todavía rimay/iniy.
//!
//! Los ejecutores reales (`pluma-transform-rimay` para Traducir,
//! `pluma-transform-iniy` para Tono/Resumir/Reescribir) viven en crates
//! aparte para no acoplar este modelo a primitivas de LLM o de embeddings.

#![forbid(unsafe_code)]

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use pluma_align::{alinear_uno_a_uno, CartaHebras, OrigenAlineamiento};
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion, Lengua};

/// Qué tipo de transformación es. Los parámetros viven dentro de cada variante
/// — un `Resumir` siempre lleva su `palabras_objetivo`; un `Reescribir`
/// siempre lleva su `prompt`. Eso permite serializar la transformación entera
/// como un solo objeto y re-ejecutarla idéntica más adelante.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TipoTransformacion {
    /// Copia 1:1 — la hija comparte los Uuid de la madre. Útil cuando una
    /// hija arranca como bifurcación humana editable a mano sin transformar.
    Identidad,
    /// Traducir a otra lengua. Lo aplica un backend que conoce el par
    /// (lengua_madre, lengua_destino).
    Traducir { lengua_destino: Lengua },
    /// Reescribir con otro tono. La etiqueta es libre — "formal", "casual",
    /// "técnico", "infantil", "poetico-andino".
    Tono { etiqueta: String },
    /// Resumir, opcionalmente con un objetivo de palabras. `None` deja al
    /// backend decidir.
    Resumir { palabras_objetivo: Option<u32> },
    /// Reescritura libre dictada por un prompt humano.
    Reescribir { prompt: String },
    /// Cualquier transformación custom — `kind` la nombra, `rhai_script` la
    /// implementa cuando se quiera evaluar localmente (la integración con Rhai
    /// queda fuera del crate; aquí solo se guarda el texto).
    Custom { kind: String, rhai_script: String },
}

impl TipoTransformacion {
    /// La `Intencion` natural del cuerpo hija que resulta de este tipo.
    /// Centraliza la traducción "qué etiqueta llevará el cuerpo hija". Sirve
    /// para que el constructor de la hija no la inventa.
    pub fn intencion_natural(&self) -> Intencion {
        match self {
            TipoTransformacion::Identidad => Intencion::Original, // Identidad bifurca; la hija puede luego derivar a otra cosa.
            TipoTransformacion::Traducir { .. } => Intencion::Traduccion,
            TipoTransformacion::Tono { etiqueta } => {
                Intencion::Tono { etiqueta: etiqueta.clone() }
            }
            TipoTransformacion::Resumir { palabras_objetivo } => {
                Intencion::Resumen { palabras_objetivo: *palabras_objetivo }
            }
            TipoTransformacion::Reescribir { prompt } => {
                Intencion::Reescritura { prompt: prompt.clone() }
            }
            TipoTransformacion::Custom { kind, .. } => {
                Intencion::Custom { kind: kind.clone() }
            }
        }
    }
}

/// Una transformación en concreto: la receta + el par madre/hija + autoría +
/// timestamps. Se persiste en el documento para poder regenerar la hija
/// cuando la madre cambie (caso *stale*).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transformacion {
    /// Identidad estable de la transformación. Se referencia desde
    /// `OrigenAlineamiento::Derivado { transformacion, .. }`.
    pub id: Uuid,
    /// Cuerpo madre (lo que se transforma).
    pub madre: Uuid,
    /// Cuerpo hija (el resultado). Se fija al ejecutarla por primera vez.
    pub hija: Uuid,
    /// Tipo + parámetros.
    pub tipo: TipoTransformacion,
    /// Quién creó esta transformación. Libre — el editor decide cómo
    /// representarlo (usuario local, identidad agora, "auto").
    pub autor: String,
    /// Instante de creación.
    pub creada_en: u64,
    /// Instante de la última regeneración aplicada. `None` mientras la
    /// transformación nunca se haya ejecutado.
    pub regenerada_en: Option<u64>,
}

impl Transformacion {
    /// Construye una transformación nueva con id aleatorio.
    pub fn nueva(
        madre: Uuid,
        hija: Uuid,
        tipo: TipoTransformacion,
        autor: impl Into<String>,
        ahora: u64,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            madre,
            hija,
            tipo,
            autor: autor.into(),
            creada_en: ahora,
            regenerada_en: None,
        }
    }

    /// Marca esta transformación como recién regenerada. Lo llama el ejecutor
    /// cuando termina la aplicación con éxito.
    pub fn marcar_regenerada(&mut self, ahora: u64) {
        self.regenerada_en = Some(ahora);
    }
}

/// Producto de aplicar una transformación: el cuerpo hija (ya poblado), los
/// `NarrativeAtom`s que el ejecutor *creó* nuevos (vacío si reusa los de la
/// madre, como Identidad) y la `CartaHebras` que enlaza madre y hija.
#[derive(Debug, Clone)]
pub struct ProductoTransformacion {
    pub hija: Cuerpo,
    pub atoms_nuevos: Vec<NarrativeAtom>,
    pub carta: CartaHebras,
}

/// Error que devuelve un ejecutor cuando no puede aplicar la transformación.
/// Es deliberadamente delgado — los errores específicos de un backend (un
/// HTTP timeout de un servicio de traducción remoto, por ejemplo) los anota
/// el propio backend en su mensaje.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorEjecutor {
    /// El ejecutor no sabe aplicar este tipo de transformación.
    TipoNoSoportado,
    /// La madre referenciada no se encontró (faltan átomos, índice corrupto).
    MadreInvalida(&'static str),
    /// El backend falló al producir el resultado.
    Backend(String),
}

/// Un ejecutor sabe aplicar una `Transformacion` sobre una madre y producir
/// la hija. Los backends reales (rimay, iniy, Rhai, LLM remoto) implementan
/// este rasgo en crates aparte.
///
/// **Async**: los ejecutores triviales (Identidad, Tabla) hacen `async fn`
/// sin awaits; los que llaman a servicios externos (LLM) sí. Mantener el
/// rasgo async desde el principio evita que el primer adapter remoto
/// fuerce una migración del API.
#[async_trait]
pub trait Ejecutor: Send + Sync {
    /// Aplica `t` sobre `madre`. Recibe `ahora` para sellar los timestamps de
    /// los productos (frescura del cuerpo hija, timestamps de los
    /// alineamientos derivados). El ejecutor NO debe leer el reloj del
    /// sistema directamente: eso queda en el caller para mantener el flujo
    /// determinista y testeable.
    async fn aplicar(
        &self,
        t: &Transformacion,
        madre: &Cuerpo,
        ahora: u64,
    ) -> Result<ProductoTransformacion, ErrorEjecutor>;
}

// =============================================================================
//  EjecutorIdentidad — el ejecutor más simple, sin backend externo
// =============================================================================

/// Aplica `TipoTransformacion::Identidad`: el cuerpo hija comparte los `Uuid`
/// de párrafo con la madre (no se clonan átomos: el caller mantiene UNA
/// instancia en el `NarrativeGraph`, y dos cuerpos la ordenan). La carta de
/// hebras es 1↔1, todas con `OrigenAlineamiento::Derivado` y fuerza 1.0.
///
/// Usos:
///   - bifurcar una hija que luego el humano editará a mano (perfecta para
///     "duplicar el borrador" antes de retocarlo);
///   - probar end-to-end el flujo madre/hija sin depender todavía de
///     rimay/iniy/Rhai.
///
/// Solo acepta `TipoTransformacion::Identidad`; cualquier otro tipo
/// devuelve `ErrorEjecutor::TipoNoSoportado` — la primera vez que el editor
/// llama "Identidad" sobre una madre, este es el ejecutor que toma el
/// trabajo, sin tirar de un backend pesado.
pub struct EjecutorIdentidad;

#[async_trait]
impl Ejecutor for EjecutorIdentidad {
    async fn aplicar(
        &self,
        t: &Transformacion,
        madre: &Cuerpo,
        ahora: u64,
    ) -> Result<ProductoTransformacion, ErrorEjecutor> {
        if !matches!(t.tipo, TipoTransformacion::Identidad) {
            return Err(ErrorEjecutor::TipoNoSoportado);
        }
        // El cuerpo hija nace con la `Intencion::Original` que `Identidad`
        // produce — un bifurcado puro. Su `derivado_de` apunta a la madre,
        // pero su intencion sigue siendo Original; eso es deliberado: la hija
        // queda libre para luego pasar por otra transformación que le dé
        // intencion final. Mientras tanto, `derivado_de` registra la deuda.
        let mut hija = Cuerpo::nuevo(
            // Heredamos branch_id pero le añadimos sufijo para que sea
            // distinto del branch de la madre. El caller puede luego
            // sobrescribirlo si quiere un nombre semantico.
            format!("{}-bifurcado", madre.branch_id),
            format!("{} (copia)", madre.metadatos.nombre_legible),
            TipoTransformacion::Identidad.intencion_natural(),
            ahora,
        );
        // `Cuerpo::deriva_de` espera la intencion ya derivada para que
        // `valida_consistencia` cuadre. Como Identidad produce Original
        // semánticamente, tocamos los campos a mano — esto es el caso especial
        // de Identidad. Las otras transformaciones usan deriva_de() limpio.
        hija.metadatos.derivado_de = Some(madre.id);
        hija.metadatos.fresco_hasta = Some(ahora);
        hija.orden = madre.orden.clone();

        // Carta uno-a-uno; el origen apunta a esta transformacion.
        let carta = alinear_uno_a_uno(
            madre,
            &hija,
            OrigenAlineamiento::Derivado { transformacion: t.id, timestamp: ahora },
        );

        Ok(ProductoTransformacion {
            hija,
            atoms_nuevos: Vec::new(),
            carta,
        })
    }
}

// =============================================================================
//  Pruebas
// =============================================================================

#[cfg(test)]
mod pruebas {
    use super::*;

    fn madre_de_3_atomos() -> (Cuerpo, Vec<Uuid>) {
        let mut madre = Cuerpo::nuevo("es", "es (original)", Intencion::Original, 100);
        let ids: Vec<Uuid> = (0..3).map(|_| Uuid::new_v4()).collect();
        for &id in &ids {
            madre.agregar(id, 101);
        }
        (madre, ids)
    }

    #[test]
    fn intencion_natural_de_cada_tipo() {
        assert_eq!(
            TipoTransformacion::Identidad.intencion_natural(),
            Intencion::Original
        );
        assert_eq!(
            TipoTransformacion::Traducir { lengua_destino: "qu".into() }.intencion_natural(),
            Intencion::Traduccion
        );
        assert_eq!(
            TipoTransformacion::Tono { etiqueta: "formal".into() }.intencion_natural(),
            Intencion::Tono { etiqueta: "formal".into() }
        );
        assert_eq!(
            TipoTransformacion::Resumir { palabras_objetivo: Some(120) }.intencion_natural(),
            Intencion::Resumen { palabras_objetivo: Some(120) }
        );
    }

    #[test]
    fn transformacion_se_marca_regenerada() {
        let mut t = Transformacion::nueva(
            Uuid::new_v4(), Uuid::new_v4(),
            TipoTransformacion::Identidad,
            "tester", 1000,
        );
        assert_eq!(t.regenerada_en, None);
        t.marcar_regenerada(1500);
        assert_eq!(t.regenerada_en, Some(1500));
    }

    #[tokio::test]
    async fn identidad_produce_hija_con_mismos_uuids_y_carta_uno_a_uno() {
        let (madre, ids) = madre_de_3_atomos();
        let t = Transformacion::nueva(
            madre.id, Uuid::new_v4(),
            TipoTransformacion::Identidad,
            "tester", 200,
        );
        let prod = EjecutorIdentidad.aplicar(&t, &madre, 200).await.unwrap();

        // La hija comparte UUIDs con la madre.
        assert_eq!(prod.hija.orden, ids);

        // No se crean atoms nuevos — Identidad reusa los de la madre.
        assert!(prod.atoms_nuevos.is_empty());

        // Carta 1↔1, fuerza 1.0, origen Derivado apuntando a t.id.
        assert_eq!(prod.carta.hebras.len(), 3);
        for (i, h) in prod.carta.hebras.iter().enumerate() {
            assert_eq!(h.atom_a, ids[i]);
            assert_eq!(h.atom_b, ids[i]);
            assert_eq!(h.fuerza, 1.0);
            match &h.origen {
                OrigenAlineamiento::Derivado { transformacion, timestamp } => {
                    assert_eq!(*transformacion, t.id);
                    assert_eq!(*timestamp, 200);
                }
                otro => panic!("origen inesperado: {otro:?}"),
            }
        }

        // La hija nace fresca y deriva de la madre.
        assert_eq!(prod.hija.metadatos.derivado_de, Some(madre.id));
        assert_eq!(prod.hija.metadatos.fresco_hasta, Some(200));
    }

    #[tokio::test]
    async fn identidad_rechaza_otros_tipos() {
        let (madre, _) = madre_de_3_atomos();
        let t = Transformacion::nueva(
            madre.id, Uuid::new_v4(),
            TipoTransformacion::Traducir { lengua_destino: "qu".into() },
            "tester", 200,
        );
        assert!(matches!(
            EjecutorIdentidad.aplicar(&t, &madre, 200).await,
            Err(ErrorEjecutor::TipoNoSoportado)
        ));
    }

    #[tokio::test]
    async fn hija_de_identidad_marca_stale_si_madre_cambia_despues() {
        let (madre, _) = madre_de_3_atomos();
        let t = Transformacion::nueva(
            madre.id, Uuid::new_v4(),
            TipoTransformacion::Identidad,
            "tester", 200,
        );
        let prod = EjecutorIdentidad.aplicar(&t, &madre, 200).await.unwrap();
        // La madre se modifica después de la regeneración.
        assert!(prod.hija.es_stale(300));
        assert!(!prod.hija.es_stale(150));
    }
}
