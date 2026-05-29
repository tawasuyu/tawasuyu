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
//! ## Grupos / folders (aplanados)
//!
//! `tullpu-core::Lienzo` es una lista plana de capas. PSD permite grupos
//! anidados con sus propias propiedades (visibilidad, opacidad, blend).
//! Para no inventar un modelo de árbol en `tullpu-core` para este puente, el
//! bridge **aplana** la jerarquía y propaga las propiedades del grupo a sus
//! hijas:
//!
//! - El nombre de la capa importada lleva la ruta completa, separada por `/`:
//!   `"raíz/sub/hoja"`.
//! - La **visibilidad efectiva** es el AND lógico de `visible` de la capa y
//!   de todos los grupos por encima (un grupo invisible esconde todas sus
//!   hojas).
//! - La **opacidad efectiva** es el producto de la opacidad de la capa por
//!   la de cada grupo (todas como `[0,1]`).
//! - El **blend de la capa** se preserva. Si algún grupo en la cadena tiene
//!   blend distinto de `Normal`/`PassThrough`, el bridge lo registra en
//!   `informe.grupos_con_blend_propio` — modelarlo fielmente requeriría
//!   rasterizar el grupo a una capa intermedia (no este puente).
//!
//! Esta aproximación es **exacta** cuando todos los grupos están en
//! `Normal`/`PassThrough` (el caso por defecto en Photoshop). Para los demás
//! casos el render visual diverge, pero queda anotado en el informe para que
//! la UI lo señale al usuario.
//!
//! ## Qué NO se porta (todavía)
//!
//! - Máscaras de capa (PSD las codifica por canal separado; el modelo de
//!   tullpu las soporta como hash aparte — el bridge no las extrae aún).
//! - Blend de grupo distinto de `Normal`/`PassThrough` (se aplana y se
//!   anota; ver arriba).
//! - Clipping masks, layer styles, smart objects, ajustes (curvas, niveles…).
//!
//! Catálogo de blend modes Photoshop completo (28 discriminantes upstream):
//! Normal/PassThrough, Dissolve, Multiply, Screen, Overlay, Darken/Lighten,
//! Difference, LinearDodge (=Aditivo), familia Burn/Dodge, familia Light
//! (Soft/Hard/Vivid/Linear/Pin), HardMix, Exclusion, Subtract, Divide,
//! familia HSL (Hue/Saturation/Color/Luminosity) y comparativos
//! (DarkerColor/LighterColor). Todos mapean directo a [`ModoFusion`] sin
//! degradado.
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

use psd::{Psd, PsdError, PsdGroup};
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
    /// Cantidad de grupos / folders detectados en el PSD. El bridge los
    /// aplana; este número es informativo para que la UI lo muestre.
    pub grupos_detectados: usize,
    /// Grupos cuyo blend mode efectivo no es Normal ni PassThrough — el
    /// bridge no rasteriza grupos, así que ese blend no se respeta. Pares
    /// `(ruta_grupo, blend_original_debug)`.
    pub grupos_con_blend_propio: Vec<(String, String)>,
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
///
/// Si el PSD tiene grupos, se aplanan: el nombre lleva la ruta jerárquica
/// (`"raíz/sub/hoja"`), la opacidad multiplica las de los grupos por encima,
/// y la visibilidad se anula si cualquier grupo de la cadena está oculto.
/// Ver el doc del módulo para los casos límite (blend de grupo distinto de
/// Normal/PassThrough).
pub fn importar_psd(bytes: &[u8]) -> Result<DocumentoPsdImportado, ImportPsdError> {
    let psd = Psd::from_bytes(bytes)?;
    let width = psd.width();
    let height = psd.height();
    let n_capas = psd.layers().len();
    if n_capas == 0 {
        return Err(ImportPsdError::SinCapas);
    }

    let grupos = psd.groups();
    let mut lienzo = Lienzo::nuevo(width, height);
    let mut buffers: HashMap<Hash, Vec<u8>> = HashMap::new();
    let mut informe = InformeImportacion::default();
    informe.grupos_detectados = grupos.len();

    // Pre-computamos la cadena efectiva (path + propiedades) por grupo,
    // memoizando: cada grupo se resuelve a lo sumo una vez aunque tenga
    // varias capas adentro.
    let mut cache_cadenas: HashMap<u32, CadenaGrupo> = HashMap::new();
    let mut grupos_blend_reportados: std::collections::HashSet<u32> =
        std::collections::HashSet::new();

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

        // Resolvemos la cadena de grupos por encima de esta capa (si está
        // adentro de alguno) — memoizada por id de grupo.
        let cadena = match layer.parent_id() {
            Some(gid) => cadena_de_grupo(
                gid,
                grupos,
                &mut cache_cadenas,
                &mut informe,
                &mut grupos_blend_reportados,
            ),
            None => CadenaGrupo::raiz(),
        };

        let nombre_efectivo = if cadena.ruta.is_empty() {
            layer.name().to_string()
        } else {
            format!("{}/{}", cadena.ruta, layer.name())
        };
        let opacidad_capa = layer.opacity() as f32 / 255.0;
        let visible_capa = layer.visible();

        if degradado {
            informe.caidas_a_normal.push((
                nombre_efectivo.clone(),
                nombre_blend(blend_disc).to_string(),
            ));
        }

        let mut capa = Capa::raster(&nombre_efectivo, hash);
        capa.blend = blend;
        capa.opacidad = (opacidad_capa * cadena.opacidad).clamp(0.0, 1.0);
        capa.visible = visible_capa && cadena.visible;
        lienzo.apilar(capa);
    }

    informe.capas_importadas = lienzo.capas.len();

    Ok(DocumentoPsdImportado {
        lienzo,
        buffers,
        informe,
    })
}

/// Propiedades acumuladas de una cadena de grupos (los ancestros de una
/// capa, de raíz a hoja). Calculada una vez por grupo y memoizada.
#[derive(Debug, Clone)]
struct CadenaGrupo {
    /// Ruta separada por `/`, vacía si la capa está en la raíz.
    ruta: String,
    /// Producto de opacidades de todos los grupos en la cadena. `1.0` en raíz.
    opacidad: f32,
    /// AND lógico de `visible` de los grupos. `true` en raíz.
    visible: bool,
}

impl CadenaGrupo {
    fn raiz() -> Self {
        Self {
            ruta: String::new(),
            opacidad: 1.0,
            visible: true,
        }
    }
}

/// Resuelve recursivamente la cadena de un grupo, memoizando. Si encuentra
/// un grupo con blend distinto de Normal/PassThrough lo anota en el informe
/// (una sola vez por grupo).
fn cadena_de_grupo(
    gid: u32,
    grupos: &HashMap<u32, PsdGroup>,
    cache: &mut HashMap<u32, CadenaGrupo>,
    informe: &mut InformeImportacion,
    reportados: &mut std::collections::HashSet<u32>,
) -> CadenaGrupo {
    if let Some(c) = cache.get(&gid) {
        return c.clone();
    }
    let Some(grupo) = grupos.get(&gid) else {
        // El PSD referenció un grupo que no existe en el mapa — defensa.
        return CadenaGrupo::raiz();
    };
    // Cadena del padre (recursión memoizada).
    let padre = match grupo.parent_id() {
        Some(pid) => cadena_de_grupo(pid, grupos, cache, informe, reportados),
        None => CadenaGrupo::raiz(),
    };

    let nombre = grupo.name();
    let ruta = if padre.ruta.is_empty() {
        nombre.to_string()
    } else {
        format!("{}/{}", padre.ruta, nombre)
    };
    let opacidad = padre.opacidad * (grupo.opacity() as f32 / 255.0);
    let visible = padre.visible && grupo.visible();

    // El blend del grupo PSD sólo se respeta para Normal/PassThrough — el
    // resto requeriría rasterizar el grupo y se reporta como divergencia.
    let blend_disc = grupo.blend_mode() as u32;
    if blend_disc != NORMAL && blend_disc != PASS_THROUGH && reportados.insert(gid) {
        informe
            .grupos_con_blend_propio
            .push((ruta.clone(), nombre_blend(blend_disc).to_string()));
    }

    let cadena = CadenaGrupo {
        ruta,
        opacidad: opacidad.clamp(0.0, 1.0),
        visible,
    };
    cache.insert(gid, cadena.clone());
    cadena
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
        DARKER_COLOR => (ModoFusion::ColorMasOscuro, false),
        LIGHTER_COLOR => (ModoFusion::ColorMasClaro, false),
        DISSOLVE => (ModoFusion::Disolver, false),
        // Catálogo Photoshop cerrado: cualquier discriminante futuro que
        // agregue Adobe cae acá como degradado.
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

    /// 1×1, un grupo con una capa adentro.
    const FIXTURE_GRUPO_UNA_CAPA: &[u8] =
        include_bytes!("../tests/fixtures/group-one-layer.psd");

    /// 1×1, grupo anidado en otro grupo, una capa en el subgrupo.
    const FIXTURE_GRUPOS_ANIDADOS: &[u8] =
        include_bytes!("../tests/fixtures/groups-nested.psd");

    /// 1×1, dos grupos hermanos, cada uno con una capa adentro.
    const FIXTURE_GRUPOS_HERMANOS: &[u8] =
        include_bytes!("../tests/fixtures/groups-siblings.psd");

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

        // Comparativos por luminosidad: ahora directos al triple per-píxel.
        assert_eq!(
            mapear_blend(DARKER_COLOR),
            (ModoFusion::ColorMasOscuro, false)
        );
        assert_eq!(
            mapear_blend(LIGHTER_COLOR),
            (ModoFusion::ColorMasClaro, false)
        );

        // Dissolve: PRNG estable, rama propia en el compositor.
        assert_eq!(mapear_blend(DISSOLVE), (ModoFusion::Disolver, false));

        // Tras Fase 10, todo el catálogo Photoshop mapea directo —
        // cualquier disc desconocido cae como degradado a Normal.
        let (modo, degradado) = mapear_blend(99);
        assert_eq!(modo, ModoFusion::Normal);
        assert!(degradado);
        assert_eq!(nombre_blend(SOFT_LIGHT), "SoftLight");
        assert_eq!(nombre_blend(LUMINOSITY), "Luminosity");
        assert_eq!(nombre_blend(DARKER_COLOR), "DarkerColor");
        assert_eq!(nombre_blend(DISSOLVE), "Dissolve");
    }

    #[test]
    fn grupo_una_capa_aplana_con_path_jerarquico() {
        // Un solo grupo con una capa adentro: el nombre debe quedar como
        // "Grupo/Capa" y `grupos_detectados == 1`.
        let imp = importar_psd(FIXTURE_GRUPO_UNA_CAPA).unwrap();
        assert_eq!(imp.lienzo.capas.len(), 1, "1 capa hoja");
        assert_eq!(imp.informe.grupos_detectados, 1);
        assert!(imp.informe.grupos_con_blend_propio.is_empty());

        let nombre = &imp.lienzo.capas[0].nombre;
        assert!(
            nombre.contains('/'),
            "el path debe ser jerárquico, fue '{}'",
            nombre
        );
        // No queremos doble-slash ni "/" inicial.
        assert!(!nombre.starts_with('/'));
        assert!(!nombre.contains("//"));
    }

    #[test]
    fn grupos_anidados_concatenan_path() {
        // Grupo dentro de grupo, una capa en el subgrupo. El path debe
        // contener al menos dos `/` (raíz/sub/hoja → al menos 2 separadores).
        let imp = importar_psd(FIXTURE_GRUPOS_ANIDADOS).unwrap();
        assert_eq!(imp.lienzo.capas.len(), 1, "1 capa hoja");
        // `psd.groups()` cuenta nodos; para este fixture upstream son 2.
        assert_eq!(imp.informe.grupos_detectados, 2);

        let nombre = &imp.lienzo.capas[0].nombre;
        let segmentos: Vec<&str> = nombre.split('/').collect();
        assert!(
            segmentos.len() >= 3,
            "path con al menos raíz/sub/hoja, fue '{}'",
            nombre
        );
    }

    #[test]
    fn grupos_hermanos_no_se_mezclan_en_paths() {
        // Dos grupos hermanos, una capa en cada uno. Los paths deben tener
        // raíces distintas (no anidación entre ellos).
        let imp = importar_psd(FIXTURE_GRUPOS_HERMANOS).unwrap();
        assert_eq!(imp.lienzo.capas.len(), 2, "2 capas hoja");
        assert_eq!(imp.informe.grupos_detectados, 2);

        let raices: std::collections::HashSet<&str> = imp
            .lienzo
            .capas
            .iter()
            .map(|c| c.nombre.split('/').next().unwrap())
            .collect();
        assert_eq!(
            raices.len(),
            2,
            "los hermanos deben tener raíces distintas, fueron {:?}",
            raices
        );
    }

    #[test]
    fn propiedades_efectivas_quedan_en_rango() {
        // Sanity: opacidad ∈ [0,1] aún tras propagación; visible es boolean
        // limpio. Aplica al fixture anidado (más capas de multiplicación).
        let imp = importar_psd(FIXTURE_GRUPOS_ANIDADOS).unwrap();
        for capa in &imp.lienzo.capas {
            assert!(
                (0.0..=1.0).contains(&capa.opacidad),
                "opacidad fuera de rango: {}",
                capa.opacidad
            );
        }
    }

    #[test]
    fn psd_sin_grupos_no_introduce_path() {
        // Regresión: si no hay grupos, los nombres deben quedar pelados.
        let imp = importar_psd(FIXTURE_DOS_CAPAS).unwrap();
        assert_eq!(imp.informe.grupos_detectados, 0);
        for capa in &imp.lienzo.capas {
            assert!(
                !capa.nombre.contains('/'),
                "capa sin grupo no debe tener '/' en el nombre, fue '{}'",
                capa.nombre
            );
        }
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
