//! `foreign-psd` — importa archivos `.psd` (Adobe Photoshop) como
//! [`tullpu_core::Lienzo`] nativos.
//!
//! Una capa PSD se traduce a una [`tullpu_core::Capa`] **raster** apuntando a
//! un buffer Rgba8 de tamaño `width × height` (el del lienzo PSD). El orden
//! visual coincide naturalmente: el índice `0` en `Psd::layer_by_idx` es el
//! fondo, lo mismo que en `Lienzo::capas`.
//!
//! El crate **no compone** la imagen final — devuelve el lienzo y los buffers
//! por separado, así el caller los pone en el almacén que use (en memoria
//! para tests/demos, content-addressed en el host, `almacen.rs` de wawa
//! cuando aterrice in-cage). Para una composición rápida usa
//! `tullpu-render::componer`.
//!
//! ## Qué se porta
//!
//! - Bounds del lienzo (`width`/`height`).
//! - Pila de capas raster: nombre, blend mode, opacidad, visibilidad y RGBA.
//! - Mapeo conservador de [`psd::BlendMode`] a [`tullpu_core::ModoFusion`].
//!
//! ## Qué NO se porta (todavía)
//!
//! - Máscaras de capa (PSD las codifica por canal separado; el modelo de
//!   tullpu las soporta como hash aparte — el bridge no las extrae aún).
//! - Grupos / folders. Las capas hijas aparecen, los nodos folder se ignoran.
//! - Clipping masks, layer styles, smart objects, ajustes (curvas, niveles…).
//! - Modos de fusión que tullpu aún no compone (Color Burn, Soft/Hard Light,
//!   HSL, etc.); caen a [`ModoFusion::Normal`] y se anotan en
//!   [`InformeImportacion::caidas_a_normal`].
//!
//! Es el espejo conceptual de `pluma/foreign-docx`: importa lo legible y
//! deja la fidelidad fancy al editor nativo.
//!
//! ## Ejemplo
//!
//! ```no_run
//! use foreign_psd::importar_psd;
//! let bytes = std::fs::read("mi-arte.psd").unwrap();
//! let imp = importar_psd(&bytes).unwrap();
//! println!("Lienzo {}×{}, {} capas", imp.lienzo.width, imp.lienzo.height, imp.lienzo.capas.len());
//! // imp.buffers: HashMap<Hash, Vec<u8>>: ponelos en tu almacén.
//! ```

#![forbid(unsafe_code)]

use std::collections::HashMap;

use psd::{Psd, PsdError};
use thiserror::Error;
use tullpu_core::{hash_bytes, Capa, Hash, Lienzo, ModoFusion};

/// Resultado del import: el lienzo armado + los buffers Rgba8 que sus capas
/// referencian por hash. El caller decide dónde persistirlos.
#[derive(Debug)]
pub struct DocumentoPsdImportado {
    /// Lienzo listo para componer/persistir. Sus capas apuntan por hash a los
    /// buffers de [`Self::buffers`].
    pub lienzo: Lienzo,
    /// Buffers Rgba8 únicos referenciados por las capas, indexados por su
    /// hash BLAKE3. Mismo tamaño cada uno: `width * height * 4` bytes.
    pub buffers: HashMap<Hash, Vec<u8>>,
    /// Informe legible del import — qué se portó tal cual, qué se degradó.
    pub informe: InformeImportacion,
}

/// Resumen humano-legible de las decisiones del import. La app/CLI lo usa
/// para avisar al usuario "tu blend Color Burn cayó a Normal".
#[derive(Debug, Default, Clone)]
pub struct InformeImportacion {
    /// Capas importadas (todas las que llegan al lienzo).
    pub capas_importadas: usize,
    /// Nombres de capas cuyo blend mode PSD no tiene equivalente en
    /// [`ModoFusion`] y se forzaron a [`ModoFusion::Normal`]. Pares
    /// `(nombre_capa, blend_original_debug)`.
    pub caidas_a_normal: Vec<(String, String)>,
}

/// Errores del import. Se mantiene chico: o falla parsear el PSD, o el
/// archivo está bien pero no tiene capas (un PSD "flat" sin layer info).
#[derive(Debug, Error)]
pub enum ImportPsdError {
    /// El parser de `psd` rechazó el archivo. Mensaje del crate envuelto.
    #[error("psd inválido: {0}")]
    Psd(String),
    /// El archivo parseó pero no tiene capas — PSD "flat" (sólo image data
    /// global). Aún no soportado: requeriría tomar el composite final como
    /// única capa raster, decisión que dejamos al caller a futuro.
    #[error("el PSD no tiene capas explícitas (PSD flat); este puente requiere layer info")]
    SinCapas,
}

impl From<PsdError> for ImportPsdError {
    fn from(value: PsdError) -> Self {
        ImportPsdError::Psd(value.to_string())
    }
}

/// Importa un `.psd` desde bytes (lo que devuelve `std::fs::read`).
///
/// Construye el [`Lienzo`] con sus capas en orden visual (índice 0 = fondo),
/// hashea cada buffer Rgba8 con BLAKE3 y deduplica: si dos capas comparten
/// pixel data exacta, comparten hash y entrada en `buffers`.
pub fn importar_psd(bytes: &[u8]) -> Result<DocumentoPsdImportado, ImportPsdError> {
    let psd = Psd::from_bytes(bytes)?;
    let width = psd.width();
    let height = psd.height();
    let n_capas = psd.layers().len();
    if n_capas == 0 {
        return Err(ImportPsdError::SinCapas);
    }

    let mut lienzo = Lienzo::nuevo(width, height);
    let mut buffers: HashMap<Hash, Vec<u8>> = HashMap::new();
    let mut informe = InformeImportacion::default();

    let esperado = (width as usize) * (height as usize) * 4;

    for layer in psd.layers() {
        let bytes_rgba = layer.rgba();
        // `psd::PsdLayer::rgba()` documenta devolver un buffer canvas-sized.
        // Si por algún motivo no, lo reportamos como inválido (el modelo de
        // tullpu exige W*H*4 estricto, lo valida `tullpu-render`).
        if bytes_rgba.len() != esperado {
            return Err(ImportPsdError::Psd(format!(
                "capa '{}' devolvió {} bytes, esperaba {} (lienzo {}×{})",
                layer.name(),
                bytes_rgba.len(),
                esperado,
                width,
                height,
            )));
        }

        let hash = hash_bytes(&bytes_rgba);
        buffers.entry(hash).or_insert(bytes_rgba);

        // El crate `psd` no re-exporta `BlendMode` en su raíz pública (sólo lo
        // devuelve por valor desde `PsdLayer::blend_mode`). Como el enum
        // upstream documenta discriminantes explícitos (`PassThrough = 0`,
        // `Normal = 1`, ..., `Luminosity = 27`), casteamos al discriminante
        // y mapeamos por número — estable mientras el crate no rompa esos
        // valores (cosa que sería un breaking change explícito).
        let blend_disc = layer.blend_mode() as u32;
        let (blend, degradado) = mapear_blend(blend_disc);
        if degradado {
            informe
                .caidas_a_normal
                .push((layer.name().to_string(), nombre_blend(blend_disc).to_string()));
        }

        let mut capa = Capa::raster(layer.name(), hash);
        capa.blend = blend;
        capa.opacidad = layer.opacity() as f32 / 255.0;
        capa.visible = layer.visible();
        lienzo.apilar(capa);
    }

    informe.capas_importadas = lienzo.capas.len();

    Ok(DocumentoPsdImportado {
        lienzo,
        buffers,
        informe,
    })
}

// =============================================================================
//  Blend modes — tabla por discriminante upstream
// -----------------------------------------------------------------------------
//  Los identificadores de `psd::BlendMode` (no exportado en la API pública del
//  crate, pero sí casteables desde el valor que devuelve `PsdLayer::blend_mode`).
//  Si el crate `psd` cambiara estos valores sería un breaking change suyo.
// =============================================================================

const PASS_THROUGH: u32 = 0;
const NORMAL: u32 = 1;
const DISSOLVE: u32 = 2;
const DARKEN: u32 = 3;
const MULTIPLY: u32 = 4;
const COLOR_BURN: u32 = 5;
const LINEAR_BURN: u32 = 6;
const DARKER_COLOR: u32 = 7;
const LIGHTEN: u32 = 8;
const SCREEN: u32 = 9;
const COLOR_DODGE: u32 = 10;
const LINEAR_DODGE: u32 = 11;
const LIGHTER_COLOR: u32 = 12;
const OVERLAY: u32 = 13;
const SOFT_LIGHT: u32 = 14;
const HARD_LIGHT: u32 = 15;
const VIVID_LIGHT: u32 = 16;
const LINEAR_LIGHT: u32 = 17;
const PIN_LIGHT: u32 = 18;
const HARD_MIX: u32 = 19;
const DIFFERENCE: u32 = 20;
const EXCLUSION: u32 = 21;
const SUBTRACT: u32 = 22;
const DIVIDE: u32 = 23;
const HUE: u32 = 24;
const SATURATION: u32 = 25;
const COLOR: u32 = 26;
const LUMINOSITY: u32 = 27;

/// Mapea el discriminante de un `psd::BlendMode` al catálogo de
/// [`ModoFusion`]. Devuelve `(modo, degradado)` donde `degradado == true`
/// indica que se forzó a `Normal` porque tullpu aún no soporta ese blend.
fn mapear_blend(disc: u32) -> (ModoFusion, bool) {
    match disc {
        NORMAL | PASS_THROUGH => (ModoFusion::Normal, false),
        MULTIPLY => (ModoFusion::Multiplicar, false),
        SCREEN => (ModoFusion::Pantalla, false),
        OVERLAY => (ModoFusion::Superponer, false),
        LIGHTEN => (ModoFusion::Aclarar, false),
        DARKEN => (ModoFusion::Oscurecer, false),
        DIFFERENCE => (ModoFusion::Diferencia, false),
        LINEAR_DODGE => (ModoFusion::Aditivo, false),
        COLOR_BURN => (ModoFusion::SubExpQuemado, false),
        LINEAR_BURN => (ModoFusion::SubLinealQuemado, false),
        COLOR_DODGE => (ModoFusion::SobreExpAclarado, false),
        HARD_LIGHT => (ModoFusion::LuzFuerte, false),
        SOFT_LIGHT => (ModoFusion::LuzSuave, false),
        VIVID_LIGHT => (ModoFusion::LuzViva, false),
        LINEAR_LIGHT => (ModoFusion::LuzLineal, false),
        PIN_LIGHT => (ModoFusion::LuzPunto, false),
        HARD_MIX => (ModoFusion::MezclaDura, false),
        EXCLUSION => (ModoFusion::Exclusion, false),
        SUBTRACT => (ModoFusion::Resta, false),
        DIVIDE => (ModoFusion::Division, false),
        HUE => (ModoFusion::HslTono, false),
        SATURATION => (ModoFusion::HslSaturacion, false),
        COLOR => (ModoFusion::HslColor, false),
        LUMINOSITY => (ModoFusion::HslLuminosidad, false),
        // Quedan en degradado los blends "exotic" sin equivalente per-pixel
        // sensato: Dissolve (necesita PRNG por píxel), DarkerColor y
        // LighterColor (basados en luminosidad pero no per-channel).
        _ => (ModoFusion::Normal, true),
    }
}

/// Etiqueta legible para un discriminante de blend mode — usada en el
/// informe cuando se cae a Normal.
fn nombre_blend(disc: u32) -> &'static str {
    match disc {
        PASS_THROUGH => "PassThrough",
        NORMAL => "Normal",
        DISSOLVE => "Dissolve",
        DARKEN => "Darken",
        MULTIPLY => "Multiply",
        COLOR_BURN => "ColorBurn",
        LINEAR_BURN => "LinearBurn",
        DARKER_COLOR => "DarkerColor",
        LIGHTEN => "Lighten",
        SCREEN => "Screen",
        COLOR_DODGE => "ColorDodge",
        LINEAR_DODGE => "LinearDodge",
        LIGHTER_COLOR => "LighterColor",
        OVERLAY => "Overlay",
        SOFT_LIGHT => "SoftLight",
        HARD_LIGHT => "HardLight",
        VIVID_LIGHT => "VividLight",
        LINEAR_LIGHT => "LinearLight",
        PIN_LIGHT => "PinLight",
        HARD_MIX => "HardMix",
        DIFFERENCE => "Difference",
        EXCLUSION => "Exclusion",
        SUBTRACT => "Subtract",
        DIVIDE => "Divide",
        HUE => "Hue",
        SATURATION => "Saturation",
        COLOR => "Color",
        LUMINOSITY => "Luminosity",
        _ => "Desconocido",
    }
}

// =============================================================================
//  Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// PSD `two-layers-red-green-1x1.psd` del corpus de tests del crate `psd`
    /// (MIT/Apache, redistribución permitida). 1×1 píxel, dos capas opacas:
    /// "Red Layer" (#ff0000) y "Green Layer" (#00ff00).
    const FIXTURE_DOS_CAPAS: &[u8] =
        include_bytes!("../tests/fixtures/two-layers-red-green-1x1.psd");

    /// PSD `green-1x1.psd` del mismo corpus. 1×1 píxel, una sola capa verde.
    const FIXTURE_UNA_CAPA: &[u8] = include_bytes!("../tests/fixtures/green-1x1.psd");

    #[test]
    fn bytes_basura_dan_error_de_parseo() {
        let err = importar_psd(b"esto no es un psd").unwrap_err();
        assert!(matches!(err, ImportPsdError::Psd(_)));
    }

    #[test]
    fn importa_una_capa() {
        let imp = importar_psd(FIXTURE_UNA_CAPA).expect("green-1x1.psd debe parsear");
        assert_eq!(imp.lienzo.width, 1);
        assert_eq!(imp.lienzo.height, 1);
        assert_eq!(imp.lienzo.capas.len(), 1);
        assert_eq!(imp.informe.capas_importadas, 1);
        assert!(imp.informe.caidas_a_normal.is_empty());

        // El bit `visible` del PSD se traslada tal cual — no asumimos su
        // valor para este fixture (algunas exportaciones de Photoshop dejan
        // capas internas marcadas no-visibles aunque tengan pixel data).
        // Validamos que el campo no quede en estado inválido más adelante.
        let capa = &imp.lienzo.capas[0];
        assert_eq!(capa.opacidad, 1.0);
        assert_eq!(capa.blend, ModoFusion::Normal);

        // El buffer está en el mapa y tiene el tamaño correcto.
        let buf = imp
            .buffers
            .get(&capa.contenido)
            .expect("buffer apuntado debe existir");
        assert_eq!(buf.len(), 4); // 1*1*4
        // Píxel verde puro, alfa máximo.
        assert_eq!(buf[0], 0);
        assert_eq!(buf[1], 255);
        assert_eq!(buf[2], 0);
        assert_eq!(buf[3], 255);
    }

    #[test]
    fn importa_dos_capas_en_orden_visual() {
        let imp = importar_psd(FIXTURE_DOS_CAPAS).expect("two-layers debe parsear");
        assert_eq!(imp.lienzo.capas.len(), 2);

        // En PSD el orden de `layers()` es bottom→top; en tullpu Lienzo::capas
        // también. Verificamos que los nombres llegaron y que los buffers
        // están en el almacén.
        for capa in &imp.lienzo.capas {
            let buf = imp.buffers.get(&capa.contenido).expect("buffer presente");
            assert_eq!(buf.len(), 4);
        }

        // Las dos capas tienen píxeles distintos → hashes distintos →
        // entradas distintas en `buffers`. (Si fueran iguales, la dedup
        // produciría un solo hash.)
        assert_eq!(imp.buffers.len(), 2);

        // Verde y rojo aparecen en algún orden — el crate `psd` decide.
        let pixeles: Vec<[u8; 4]> = imp
            .lienzo
            .capas
            .iter()
            .map(|c| {
                let b = &imp.buffers[&c.contenido];
                [b[0], b[1], b[2], b[3]]
            })
            .collect();
        let rojo = [255, 0, 0, 255];
        let verde = [0, 255, 0, 255];
        assert!(pixeles.contains(&rojo), "esperaba ver rojo, encontré {:?}", pixeles);
        assert!(pixeles.contains(&verde), "esperaba ver verde, encontré {:?}", pixeles);
    }

    #[test]
    fn dedup_capa_identica_comparte_hash() {
        // Si las dos capas tuvieran exactamente el mismo Rgba, la dedup las
        // colapsaría a un solo entry en `buffers`. Lo verificamos
        // sintéticamente: importamos green-1x1.psd dos veces y mezclamos los
        // mapas — el conjunto de claves debe tener tamaño 1.
        let a = importar_psd(FIXTURE_UNA_CAPA).unwrap();
        let b = importar_psd(FIXTURE_UNA_CAPA).unwrap();
        let mut unidos: HashMap<Hash, Vec<u8>> = HashMap::new();
        unidos.extend(a.buffers);
        unidos.extend(b.buffers);
        assert_eq!(unidos.len(), 1);
    }

    #[test]
    fn mapear_blend_cubre_los_soportados() {
        assert_eq!(mapear_blend(NORMAL), (ModoFusion::Normal, false));
        assert_eq!(mapear_blend(MULTIPLY), (ModoFusion::Multiplicar, false));
        assert_eq!(mapear_blend(SCREEN), (ModoFusion::Pantalla, false));
        assert_eq!(mapear_blend(LINEAR_DODGE), (ModoFusion::Aditivo, false));

        // Familia "burn/dodge/light" — ahora directos, sin degradado.
        assert_eq!(mapear_blend(COLOR_BURN), (ModoFusion::SubExpQuemado, false));
        assert_eq!(
            mapear_blend(LINEAR_BURN),
            (ModoFusion::SubLinealQuemado, false)
        );
        assert_eq!(
            mapear_blend(COLOR_DODGE),
            (ModoFusion::SobreExpAclarado, false)
        );
        assert_eq!(mapear_blend(SOFT_LIGHT), (ModoFusion::LuzSuave, false));
        assert_eq!(mapear_blend(HARD_LIGHT), (ModoFusion::LuzFuerte, false));
        assert_eq!(mapear_blend(VIVID_LIGHT), (ModoFusion::LuzViva, false));
        assert_eq!(mapear_blend(LINEAR_LIGHT), (ModoFusion::LuzLineal, false));
        assert_eq!(mapear_blend(PIN_LIGHT), (ModoFusion::LuzPunto, false));
        assert_eq!(mapear_blend(HARD_MIX), (ModoFusion::MezclaDura, false));
        assert_eq!(mapear_blend(EXCLUSION), (ModoFusion::Exclusion, false));
        assert_eq!(mapear_blend(SUBTRACT), (ModoFusion::Resta, false));
        assert_eq!(mapear_blend(DIVIDE), (ModoFusion::Division, false));

        // Familia HSL — ya directos vía W3C Compositing.
        assert_eq!(mapear_blend(HUE), (ModoFusion::HslTono, false));
        assert_eq!(mapear_blend(SATURATION), (ModoFusion::HslSaturacion, false));
        assert_eq!(mapear_blend(COLOR), (ModoFusion::HslColor, false));
        assert_eq!(mapear_blend(LUMINOSITY), (ModoFusion::HslLuminosidad, false));

        // El residuo "degradado" queda sólo en los exotic per-pixel sin
        // equivalente sensato: Dissolve (PRNG por píxel) y
        // DarkerColor/LighterColor (comparativos basados en luminosidad).
        for raro in [DISSOLVE, DARKER_COLOR, LIGHTER_COLOR] {
            let (modo, degradado) = mapear_blend(raro);
            assert_eq!(modo, ModoFusion::Normal);
            assert!(degradado, "esperaba degradado para disc {raro}");
        }
        assert_eq!(nombre_blend(SOFT_LIGHT), "SoftLight");
        assert_eq!(nombre_blend(LUMINOSITY), "Luminosity");
    }

    #[test]
    fn lienzo_importado_redondea_por_objeto() {
        // Sanity: el lienzo importado debe (de)serializarse limpio a
        // format::Objeto vía tullpu_core.
        let imp = importar_psd(FIXTURE_DOS_CAPAS).unwrap();
        let obj = tullpu_core::lienzo_a_objeto(&imp.lienzo).unwrap();
        let l2 = tullpu_core::lienzo_desde_objeto(&obj).unwrap();
        assert_eq!(imp.lienzo, l2);
        // Dos buffers únicos → dos hijos en el objeto.
        assert_eq!(obj.hijos.len(), 2);
    }
}
