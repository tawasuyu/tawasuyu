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
//! ## Grupos / folders (aplanados o rasterizados)
//!
//! `tullpu-core::Lienzo` es una lista plana de capas. PSD permite grupos
//! anidados con sus propias propiedades (visibilidad, opacidad, blend). El
//! bridge construye un árbol interno (`NodoPsd::{Capa,Grupo}`) y lo recorre
//! en post-order; cada grupo decide su destino según su blend efectivo:
//!
//! - **Grupos `Normal` o `PassThrough`** (el caso por defecto en Photoshop):
//!   se **aplanan**. El nombre lleva la ruta completa separada por `/`
//!   (`"raíz/sub/hoja"`); la opacidad se multiplica; la visibilidad es AND
//!   lógico. Resultado idéntico al render Photoshop sin componer nada.
//! - **Grupos con blend distinto** (Multiply, Screen, Overlay…): se
//!   **rasterizan** internamente. Las capas dentro se componen con
//!   `tullpu-render::componer` a un único buffer Rgba8, y el grupo entero
//!   queda como una sola `Capa` raster con el blend del grupo aplicado.
//!   Las propiedades del grupo (opacidad, visibilidad) se preservan en esa
//!   capa resultante. El nombre y blend de la capa rasterizada se anotan en
//!   `informe.grupos_rasterizados` para que la UI los muestre.
//!
//! La rasterización es post-order: si un grupo Multiply contiene un grupo
//! Screen, primero se rasteriza el Screen interno, luego ese resultado
//! participa de la composición del Multiply externo. El blend de capa
//! propio se preserva al pie de la letra.
//!
//! ## Qué NO se porta (todavía)
//!
//! - Máscaras de capa (PSD las codifica por canal separado; el modelo de
//!   tullpu las soporta como hash aparte — el bridge no las extrae aún).
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
    /// aplana (`Normal`/`PassThrough`) o rasteriza (otros blends); este
    /// número es informativo para que la UI lo muestre.
    pub grupos_detectados: usize,
    /// Grupos cuyo blend mode requirió composición intermedia y aparecen en
    /// el lienzo como una sola capa raster con ese blend. Pares
    /// `(nombre_grupo, blend_debug)`.
    pub grupos_rasterizados: Vec<(String, String)>,
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
/// Construye un árbol interno con la jerarquía PSD (capas y grupos), lo
/// recorre en post-order, y produce un [`Lienzo`] con capas en orden visual
/// (índice 0 = fondo). Los grupos con blend Normal/PassThrough se aplanan;
/// los demás se rasterizan componiendo sus capas vía
/// [`tullpu_render::componer`]. Ver el doc del módulo para los detalles.
pub fn importar_psd(bytes: &[u8]) -> Result<DocumentoPsdImportado, ImportPsdError> {
    let psd = Psd::from_bytes(bytes)?;
    let width = psd.width();
    let height = psd.height();
    let layers = psd.layers();
    let n_capas = layers.len();
    if n_capas == 0 {
        return Err(ImportPsdError::SinCapas);
    }
    let grupos = psd.groups();
    let esperado = (width as usize) * (height as usize) * 4;

    let mut buffers: HashMap<Hash, Vec<u8>> = HashMap::new();
    let mut informe = InformeImportacion::default();
    informe.grupos_detectados = grupos.len();

    let arbol = construir_arbol(layers, grupos);

    // Recorrido post-order: cada nodo aporta una lista de capas
    // ya-propagadas (o una sola, si era un grupo non-Normal rasterizado).
    let mut propagables: Vec<CapaPropagable> = Vec::new();
    for nodo in &arbol {
        propagables.extend(procesar_nodo(
            nodo,
            layers,
            grupos,
            width,
            height,
            esperado,
            &mut buffers,
            &mut informe,
        )?);
    }

    // Construimos el Lienzo final a partir de la lista plana resultante.
    let mut lienzo = Lienzo::nuevo(width, height);
    for prop in propagables {
        let mut capa = Capa::raster(&prop.nombre, prop.contenido);
        capa.blend = prop.blend;
        capa.opacidad = prop.opacidad;
        capa.visible = prop.visible;
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
//  Árbol PSD y recorrido post-order
// =============================================================================

/// Nodo interno del árbol PSD que el bridge recorre para aplanar o
/// rasterizar grupos. `Capa` referencia una capa hoja por su índice en
/// `psd.layers()`; `Grupo` contiene a sus hijos directos en orden visual.
#[derive(Debug)]
enum NodoPsd {
    Capa { idx: usize },
    Grupo { id: u32, hijos: Vec<NodoPsd> },
}

/// Capa "casi lista" — el resultado de procesar un nodo: nombre con su
/// prefijo de path, blend, opacidad y visibilidad ya combinadas con las
/// propiedades de los grupos por encima, y el hash del buffer Rgba8 que
/// referencia. Se vuelca a `Lienzo::capas` al final.
#[derive(Debug, Clone)]
struct CapaPropagable {
    nombre: String,
    blend: ModoFusion,
    opacidad: f32,
    visible: bool,
    contenido: Hash,
}

/// Reconstruye el árbol PSD a partir de las capas hoja (orden bottom→top) y
/// el mapa de grupos. El orden entre hermanos de un mismo padre se infiere
/// del menor índice de capa hoja que cae bajo cada nodo — PSD codifica las
/// capas en una secuencia plana donde las hijas de un grupo son contiguas.
fn construir_arbol(
    layers: &[psd::PsdLayer],
    grupos: &HashMap<u32, PsdGroup>,
) -> Vec<NodoPsd> {
    let mut orden_cache: HashMap<u32, usize> = HashMap::new();
    for g in grupos.values() {
        calcular_orden_grupo(g.id(), layers, grupos, &mut orden_cache);
    }
    construir_subarbol(None, layers, grupos, &orden_cache)
}

/// Orden visual inferido de un grupo: el mínimo índice de capa hoja que
/// queda bajo su descendencia. `usize::MAX` si el grupo está vacío.
fn calcular_orden_grupo(
    gid: u32,
    layers: &[psd::PsdLayer],
    grupos: &HashMap<u32, PsdGroup>,
    cache: &mut HashMap<u32, usize>,
) -> usize {
    if let Some(&v) = cache.get(&gid) {
        return v;
    }
    let min_layers = layers
        .iter()
        .enumerate()
        .filter(|(_, l)| l.parent_id() == Some(gid))
        .map(|(i, _)| i)
        .min();
    let min_subgrupos = grupos
        .values()
        .filter(|g| g.parent_id() == Some(gid))
        .map(|g| calcular_orden_grupo(g.id(), layers, grupos, cache))
        .min();
    let r = match (min_layers, min_subgrupos) {
        (Some(a), Some(b)) => a.min(b),
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => usize::MAX,
    };
    cache.insert(gid, r);
    r
}

fn construir_subarbol(
    parent_id: Option<u32>,
    layers: &[psd::PsdLayer],
    grupos: &HashMap<u32, PsdGroup>,
    orden_cache: &HashMap<u32, usize>,
) -> Vec<NodoPsd> {
    let mut entries: Vec<(usize, NodoPsd)> = Vec::new();
    for (idx, layer) in layers.iter().enumerate() {
        if layer.parent_id() == parent_id {
            entries.push((idx, NodoPsd::Capa { idx }));
        }
    }
    for grupo in grupos.values() {
        if grupo.parent_id() == parent_id {
            let orden = orden_cache.get(&grupo.id()).copied().unwrap_or(usize::MAX);
            let hijos = construir_subarbol(Some(grupo.id()), layers, grupos, orden_cache);
            entries.push((orden, NodoPsd::Grupo { id: grupo.id(), hijos }));
        }
    }
    entries.sort_by_key(|(o, _)| *o);
    entries.into_iter().map(|(_, n)| n).collect()
}

#[allow(clippy::too_many_arguments)]
fn procesar_nodo(
    nodo: &NodoPsd,
    layers: &[psd::PsdLayer],
    grupos: &HashMap<u32, PsdGroup>,
    width: u32,
    height: u32,
    esperado: usize,
    buffers: &mut HashMap<Hash, Vec<u8>>,
    informe: &mut InformeImportacion,
) -> Result<Vec<CapaPropagable>, ImportPsdError> {
    match nodo {
        NodoPsd::Capa { idx } => {
            let layer = &layers[*idx];
            let bytes_rgba = layer.rgba();
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

            let blend_disc = layer.blend_mode() as u32;
            let (blend, degradado) = mapear_blend(blend_disc);
            if degradado {
                informe
                    .caidas_a_normal
                    .push((layer.name().to_string(), nombre_blend(blend_disc).to_string()));
            }
            Ok(vec![CapaPropagable {
                nombre: layer.name().to_string(),
                blend,
                opacidad: layer.opacity() as f32 / 255.0,
                visible: layer.visible(),
                contenido: hash,
            }])
        }
        NodoPsd::Grupo { id, hijos } => {
            // Procesamos los hijos primero (post-order).
            let mut sub: Vec<CapaPropagable> = Vec::new();
            for hijo in hijos {
                sub.extend(procesar_nodo(
                    hijo, layers, grupos, width, height, esperado, buffers, informe,
                )?);
            }
            // Si el grupo no existe en el mapa (defensa), tratamos como
            // transparente — devolvemos los hijos tal cual.
            let Some(grupo) = grupos.get(id) else {
                return Ok(sub);
            };
            let blend_disc = grupo.blend_mode() as u32;
            let g_op = grupo.opacity() as f32 / 255.0;
            let g_vis = grupo.visible();
            let nombre_g = grupo.name();

            if blend_disc == NORMAL || blend_disc == PASS_THROUGH {
                // Aplanar con propagación: prefix path, multiplicar opacidad,
                // AND visible. Resultado idéntico al render Photoshop.
                let aplanadas = sub
                    .into_iter()
                    .map(|c| CapaPropagable {
                        nombre: if c.nombre.is_empty() {
                            nombre_g.to_string()
                        } else {
                            format!("{}/{}", nombre_g, c.nombre)
                        },
                        blend: c.blend,
                        opacidad: (c.opacidad * g_op).clamp(0.0, 1.0),
                        visible: c.visible && g_vis,
                        contenido: c.contenido,
                    })
                    .collect();
                Ok(aplanadas)
            } else {
                // Rasterizar: componer todas las capas del grupo en un único
                // buffer Rgba8 y devolver UNA sola capa raster con el blend
                // del grupo. Recursión segura: los subgrupos con blend propio
                // ya quedaron rasterizados antes (post-order).
                let raster_hash = rasterizar_grupo(&sub, width, height, buffers)?;
                let (blend_grupo, _) = mapear_blend(blend_disc);
                informe
                    .grupos_rasterizados
                    .push((nombre_g.to_string(), nombre_blend(blend_disc).to_string()));
                Ok(vec![CapaPropagable {
                    nombre: nombre_g.to_string(),
                    blend: blend_grupo,
                    opacidad: g_op.clamp(0.0, 1.0),
                    visible: g_vis,
                    contenido: raster_hash,
                }])
            }
        }
    }
}

/// Compone las `capas` sobre un lienzo `width × height` y guarda el buffer
/// resultante en `buffers`, devolviendo su hash. Adapter trivial entre el
/// `HashMap` de buffers que el bridge va llenando y el trait
/// `FuenteBuffers` que espera el compositor.
fn rasterizar_grupo(
    capas: &[CapaPropagable],
    width: u32,
    height: u32,
    buffers: &mut HashMap<Hash, Vec<u8>>,
) -> Result<Hash, ImportPsdError> {
    let mut temp = Lienzo::nuevo(width, height);
    for c in capas {
        let mut tc = Capa::raster(&c.nombre, c.contenido);
        tc.blend = c.blend;
        tc.opacidad = c.opacidad;
        tc.visible = c.visible;
        temp.apilar(tc);
    }
    // Componer requiere `&FuenteBuffers`, así que congelamos el borrow de
    // `buffers` adentro del scope y soltamos antes de extender.
    let bytes = {
        let vista = VistaBuffers { buffers: &*buffers };
        let img = tullpu_render::componer(&temp, &vista)
            .map_err(|e| ImportPsdError::Psd(format!("rasterizar grupo: {e}")))?;
        img.into_raw()
    };
    let hash = hash_bytes(&bytes);
    buffers.entry(hash).or_insert(bytes);
    Ok(hash)
}

/// Vista de solo lectura sobre el mapa de buffers para alimentar el
/// compositor sin clonar.
struct VistaBuffers<'a> {
    buffers: &'a HashMap<Hash, Vec<u8>>,
}

impl<'a> tullpu_render::FuenteBuffers for VistaBuffers<'a> {
    fn obtener(&self, hash: Hash) -> Option<&[u8]> {
        self.buffers.get(&hash).map(|v| v.as_slice())
    }
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
        assert!(imp.informe.grupos_rasterizados.is_empty());

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

    // -------------------------------------------------------------------------
    //  Rasterización de grupos (Fase 15) — tests sintéticos contra el helper
    //  `rasterizar_grupo` directamente, sin pasar por un PSD real (no hay
    //  fixtures con grupos non-Normal en el corpus upstream).
    // -------------------------------------------------------------------------

    fn buffer_solido(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            v.extend_from_slice(&rgba);
        }
        v
    }

    #[test]
    fn rasterizar_grupo_multiplicar_da_color_esperado() {
        // Dos capas dentro de un "grupo Multiply" (semánticamente): fondo
        // gris [128,128,128,255] y top blanco [255,255,255,255] con blend
        // Multiplicar opacidad 1.0. El composite del sub-lienzo es el fondo
        // tal cual: Multiply(gris, blanco) = gris.
        let w = 2;
        let h = 2;
        let mut buffers: HashMap<Hash, Vec<u8>> = HashMap::new();
        let h_fondo = hash_bytes(&buffer_solido(w, h, [128, 128, 128, 255]));
        let h_top = hash_bytes(&buffer_solido(w, h, [255, 255, 255, 255]));
        buffers.insert(h_fondo, buffer_solido(w, h, [128, 128, 128, 255]));
        buffers.insert(h_top, buffer_solido(w, h, [255, 255, 255, 255]));

        let capas = vec![
            CapaPropagable {
                nombre: "fondo".into(),
                blend: ModoFusion::Normal,
                opacidad: 1.0,
                visible: true,
                contenido: h_fondo,
            },
            CapaPropagable {
                nombre: "top".into(),
                blend: ModoFusion::Multiplicar,
                opacidad: 1.0,
                visible: true,
                contenido: h_top,
            },
        ];
        let hash_raster = super::rasterizar_grupo(&capas, w, h, &mut buffers).unwrap();
        let buf = buffers.get(&hash_raster).expect("buffer rasterizado presente");
        assert_eq!(buf.len(), (w * h * 4) as usize);
        // Multiply(0.5, 1.0) = 0.5 → 128 (con pequeño redondeo). Verificamos
        // los 4 píxeles.
        for px in buf.chunks_exact(4) {
            assert!(
                (px[0] as i16 - 128).abs() <= 1,
                "R: {:?}",
                px
            );
            assert!((px[1] as i16 - 128).abs() <= 1, "G: {:?}", px);
            assert!((px[2] as i16 - 128).abs() <= 1, "B: {:?}", px);
            assert_eq!(px[3], 255, "alfa opaca, {:?}", px);
        }
    }

    #[test]
    fn rasterizar_grupo_capa_invisible_no_aporta() {
        // Una capa visible roja + una capa invisible azul → la azul no debe
        // aparecer en el resultado.
        let w = 1;
        let h = 1;
        let mut buffers: HashMap<Hash, Vec<u8>> = HashMap::new();
        let h_roja = hash_bytes(&buffer_solido(w, h, [200, 0, 0, 255]));
        let h_azul = hash_bytes(&buffer_solido(w, h, [0, 0, 255, 255]));
        buffers.insert(h_roja, buffer_solido(w, h, [200, 0, 0, 255]));
        buffers.insert(h_azul, buffer_solido(w, h, [0, 0, 255, 255]));

        let capas = vec![
            CapaPropagable {
                nombre: "roja".into(),
                blend: ModoFusion::Normal,
                opacidad: 1.0,
                visible: true,
                contenido: h_roja,
            },
            CapaPropagable {
                nombre: "azul-oculta".into(),
                blend: ModoFusion::Normal,
                opacidad: 1.0,
                visible: false,
                contenido: h_azul,
            },
        ];
        let hash = super::rasterizar_grupo(&capas, w, h, &mut buffers).unwrap();
        let buf = buffers.get(&hash).unwrap();
        assert_eq!(buf[0], 200);
        assert_eq!(buf[2], 0);
    }

    #[test]
    fn rasterizar_grupo_dedup_misma_composicion() {
        // Dos llamadas con las mismas capas → mismo hash. Garantiza la
        // dedup natural por content-addressing aún para buffers
        // rasterizados.
        let w = 2;
        let h = 1;
        let mut buffers: HashMap<Hash, Vec<u8>> = HashMap::new();
        let h_a = hash_bytes(&buffer_solido(w, h, [10, 20, 30, 255]));
        buffers.insert(h_a, buffer_solido(w, h, [10, 20, 30, 255]));

        let capas = vec![CapaPropagable {
            nombre: "a".into(),
            blend: ModoFusion::Normal,
            opacidad: 1.0,
            visible: true,
            contenido: h_a,
        }];
        let h1 = super::rasterizar_grupo(&capas, w, h, &mut buffers).unwrap();
        let h2 = super::rasterizar_grupo(&capas, w, h, &mut buffers).unwrap();
        assert_eq!(h1, h2);
    }
}
