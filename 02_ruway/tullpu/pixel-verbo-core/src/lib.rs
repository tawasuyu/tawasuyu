//! `pixel-verbo-core` — el contrato model-agnostic de modelos de píxel.
//!
//! Calco directo del patrón de `rimay-verbo-core` (embeddings): un trait
//! [`Proveedor`] que las impls concretas cumplen (mock, ONNX
//! segment-anything, modelos de upscale, generadores…). El consumidor
//! natural es `tullpu-ops`, que delega `TransformacionPixel::Ia` a un
//! proveedor; pero cualquier crate del workspace puede usar el contrato
//! sin saber qué modelo hay del otro lado del socket.
//!
//! ## Diferencia con `rimay-verbo`
//!
//! El proveedor de embeddings es asíncrono (los modelos textuales tienden
//! a vivir detrás de HTTP). El de píxeles es **sincrónico**: tullpu corre
//! en un bucle Llimphi sync, y los modelos de imagen mock o vía
//! `pixel-verbo-daemon` por socket Unix se consumen con `std::io`
//! bloqueante sin meter tokio en la app. La sincronía es decisión de
//! frontera, no propiedad del modelo: una impl tokio puede vivir aparte y
//! ofrecer el mismo trait detrás de un `block_on`.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::fmt;

// =============================================================================
//  Identidad del modelo
// =============================================================================

/// Identidad de un modelo de píxeles. El daemon expone su `ModelId` en el
/// handshake; los consumidores guardan ese `ModelId` para marcar las
/// capas IA con la procedencia del buffer derivado.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelId {
    pub name: String,
}

impl ModelId {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
        }
    }
}

impl fmt::Display for ModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

// =============================================================================
//  Imagen — el formato de cable entre la app, el daemon y los modelos
// =============================================================================

/// Imagen Rgba8 planar, fila por fila, no premultiplicada — el mismo
/// formato que ya viaja por todo tullpu. La serialización postcard la
/// hace transportable por socket sin codec extra.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Imagen {
    pub ancho: u32,
    pub alto: u32,
    /// `ancho * alto * 4` bytes Rgba8.
    pub bytes: Vec<u8>,
}

impl Imagen {
    /// Construye validando que el largo del buffer coincida con
    /// `ancho * alto * 4`. La inconsistencia es el bug más típico cuando
    /// un proveedor olvida actualizar la cabecera tras una op que
    /// reescala — preferimos error explícito a corrupción silenciosa.
    pub fn nueva(ancho: u32, alto: u32, bytes: Vec<u8>) -> Result<Self, Error> {
        let esperado = (ancho as usize) * (alto as usize) * 4;
        if bytes.len() != esperado {
            return Err(Error::Dimension {
                ancho,
                alto,
                esperado,
                encontrado: bytes.len(),
            });
        }
        Ok(Self { ancho, alto, bytes })
    }

    /// Cantidad de píxeles (sin canal). Útil para budgets de op.
    pub fn pixeles(&self) -> usize {
        (self.ancho as usize) * (self.alto as usize)
    }
}

// =============================================================================
//  Catálogo declarado de ops IA
// =============================================================================

/// El catálogo de operaciones que un proveedor puede ofrecer. Es un enum
/// cerrado para que el cable sea estable; ops experimentales pueden viajar
/// como `Restyle { prompt: "…" }` con convenciones en el prompt antes de
/// promoverse a variante propia.
///
/// **Invariante de dimensión** del MVP: el output mide igual que el input
/// (o que la `Generar { ancho, alto }` declarada). Ops que cambian tamaño
/// (upscale, padding) quedan para una variante futura cuando tullpu sepa
/// recortar/reescalar el lienzo. Cada proveedor debe respetar el
/// invariante o devolver [`Error::Dimension`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpPixel {
    /// Devuelve una máscara como capa Rgba8: alfa en el área detectada,
    /// transparente fuera. Las impls reales (SAM, U2Net) ven el prompt
    /// como descripción de qué segmentar; el mock se inventa una región.
    Segmentar { prompt: Option<String> },
    /// Rellena el área transparente (alfa < 255) de la imagen entrante.
    /// Las impls reales corren un modelo de inpaint; el mock interpola
    /// del borde más cercano. El prompt orienta el contenido a sintetizar.
    Inpaint { prompt: Option<String> },
    /// Reestiliza preservando estructura. Cambia paleta/tono guiado por
    /// el prompt. El mock aplica un shift HSL deterministico por hash del
    /// prompt — visualmente verifica que el wiring sirvió.
    Restyle { prompt: String },
    /// Genera una imagen del tamaño pedido sin imagen de entrada. El mock
    /// produce un gradiente determinista sembrado por hash(prompt).
    Generar {
        prompt: String,
        ancho: u32,
        alto: u32,
    },
}

impl OpPixel {
    /// `true` si la op consume una imagen de entrada. `Generar` es la
    /// única que no.
    pub fn requiere_entrada(&self) -> bool {
        !matches!(self, OpPixel::Generar { .. })
    }

    /// Etiqueta corta para UI/log.
    pub fn etiqueta(&self) -> &'static str {
        match self {
            OpPixel::Segmentar { .. } => "segmentar",
            OpPixel::Inpaint { .. } => "inpaint",
            OpPixel::Restyle { .. } => "restyle",
            OpPixel::Generar { .. } => "generar",
        }
    }
}

// =============================================================================
//  Errores
// =============================================================================

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "dimensión inválida: {ancho}×{alto} debe ser {esperado} bytes, vinieron {encontrado}"
    )]
    Dimension {
        ancho: u32,
        alto: u32,
        esperado: usize,
        encontrado: usize,
    },
    #[error("op '{op}' no soportada por el modelo {modelo}")]
    OpNoSoportada { op: &'static str, modelo: String },
    #[error("op requiere imagen de entrada y no se proporcionó")]
    EntradaFaltante,
    #[error("backend de pixel-verbo: {0}")]
    Backend(String),
    #[error("postcard: {0}")]
    Postcard(String),
}

// =============================================================================
//  El trait
// =============================================================================

/// Un proveedor de modelos de píxel. Cada backend (`pixel-verbo-mock`, un
/// futuro `pixel-verbo-onnx`, el `DaemonClient`) implementa este trait.
/// La sincronía la decide la frontera (ver doc del crate).
pub trait Proveedor: Send + Sync {
    /// El modelo que este proveedor sirve.
    fn model_id(&self) -> &ModelId;

    /// Aplica una op. `entrada` es `Some` salvo para [`OpPixel::Generar`].
    /// El proveedor debe respetar el invariante de dimensión: para ops
    /// con entrada, `salida.ancho/alto == entrada.ancho/alto`; para
    /// `Generar { ancho, alto }`, el output tiene exactamente esas
    /// dimensiones.
    fn aplicar(&self, op: &OpPixel, entrada: Option<Imagen>) -> Result<Imagen, Error>;
}

// =============================================================================
//  Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imagen_valida_dimension() {
        assert!(Imagen::nueva(2, 2, vec![0u8; 16]).is_ok());
        let err = Imagen::nueva(2, 2, vec![0u8; 4]).unwrap_err();
        assert!(matches!(err, Error::Dimension { .. }));
    }

    #[test]
    fn op_etiqueta_y_requiere_entrada() {
        assert_eq!(OpPixel::Segmentar { prompt: None }.etiqueta(), "segmentar");
        assert!(OpPixel::Segmentar { prompt: None }.requiere_entrada());
        assert!(!OpPixel::Generar {
            prompt: "x".into(),
            ancho: 4,
            alto: 4,
        }
        .requiere_entrada());
    }

    #[test]
    fn model_id_display() {
        let id = ModelId::new("mock");
        assert_eq!(format!("{id}"), "mock");
    }
}
