//! Basemap PMTiles: carga de vista general, basemap vivo con streaming por
//! viewport y caché LRU, mapa-base mundial embebido y estadísticas.

use std::collections::HashMap;

use crate::parsers::parse_into;
use crate::tipos::{BBox, MapData, MapPreview, MapView};
use crate::vt;

// ─── Magic PMTiles ────────────────────────────────────────────────────────────

/// Magic de un archivo PMTiles v3.
pub const PMTILES_MAGIC: &[u8] = b"PMTiles";

// ─── Decodificación MVT ───────────────────────────────────────────────────────

/// Decodifica un tile vectorial MVT (`bytes` en `z/x/y`) a un [`MapData`]
/// renderizable, reusando toda la maquinaria del visor: cada feature del tile
/// queda con su capa de origen como nombre (calle/agua/edificio…). Es la
/// costura entre el decoder soberano de [`vt`] y el render existente; sobre
/// esto se monta el basemap PMTiles cuando exista el lector del contenedor.
pub fn mvt_tile_to_mapdata(bytes: &[u8], z: u32, x: u32, y: u32) -> MapData {
    use crate::geom::{push_line, push_points, push_polygon};
    use crate::geom::make_feature;
    use crate::tipos::MAX_VERTICES;

    let mut data = MapData::default();
    let mut budget = MAX_VERTICES;
    for tf in vt::decode_mvt_tile(bytes, z, x, y) {
        if budget == 0 {
            break;
        }
        let fi = make_feature(&mut data, Some(&tf.layer));
        match tf.geom {
            vt::TileGeom::Point(c) => {
                push_points(&mut data, std::slice::from_ref(&c), &mut budget, fi)
            }
            vt::TileGeom::Line(l) => push_line(&mut data, l, &mut budget, fi),
            vt::TileGeom::Polygon(rings) => push_polygon(&mut data, rings, &mut budget, fi),
        }
    }
    data
}

// ─── Vista general ────────────────────────────────────────────────────────────

/// Zoom de vista general: el más alto cuyos tiles no superen ~64 (pocos tiles
/// que igual cubren el contenido). Lo comparten el overview y el cálculo de
/// extensión, para que coincidan.
fn overview_zoom(h: &crate::pmtiles::Header) -> u32 {
    let mut chosen = h.min_zoom as u32;
    for z in h.min_zoom as u32..=h.max_zoom as u32 {
        let span = 1u32 << z;
        if span.saturating_mul(span) <= 64 {
            chosen = z;
        } else {
            break;
        }
    }
    chosen
}

/// Extensión geográfica del basemap, para anclar la proyección. Usa los bounds
/// del header si son **sanos**; si están rotos (algunos generadores dejan
/// `max_lon=0` u otros campos en cero — visto en exportes bbbike/tilemaker),
/// los deriva de la geometría real del zoom mínimo; en último caso, mundo.
pub fn pmtiles_extent(pm: &crate::pmtiles::PmTiles) -> BBox {
    const WORLD: BBox = BBox {
        min_lon: -180.0,
        min_lat: -85.05,
        max_lon: 180.0,
        max_lat: 85.05,
    };
    let h = &pm.header;
    let header_ok = h.max_lon > h.min_lon
        && h.max_lat > h.min_lat
        && h.min_lon >= -180.5
        && h.max_lon <= 180.5
        && h.min_lat >= -85.5
        && h.max_lat <= 85.5
        // Campo de longitud/latitud faltante (queda en 0 mientras el otro no).
        && !(h.min_lon != 0.0 && h.max_lon == 0.0)
        && !(h.min_lat != 0.0 && h.max_lat == 0.0);
    if header_ok {
        return BBox {
            min_lon: h.min_lon,
            min_lat: h.min_lat,
            max_lon: h.max_lon,
            max_lat: h.max_lat,
        };
    }
    // Header roto: derivar de la geometría a la vista general (mismo zoom que
    // el overview, para que la bbox cubra todo lo que se muestra).
    let z = overview_zoom(h);
    let span = 1u32 << z;
    let mut bb = BBox::empty();
    let mut n = 0;
    'outer: for x in 0..span {
        for y in 0..span {
            if n >= 64 {
                break 'outer;
            }
            n += 1;
            if let Some(bytes) = pm.tile(z, x, y) {
                if let Some(b) = mvt_tile_to_mapdata(&bytes, z, x, y).bbox() {
                    bb.expand([b.min_lon, b.min_lat]);
                    bb.expand([b.max_lon, b.max_lat]);
                }
            }
        }
    }
    if bb.is_empty() {
        WORLD
    } else {
        bb
    }
}

/// Carga una **vista general** de un `.pmtiles`: decodifica los tiles del zoom
/// más bajo que cubra el contenido (pocos tiles) y los funde en un [`MapData`].
/// Es el basemap soberano en su forma MVP: muestra el mapa completo a baja
/// resolución, reutilizando todo el render. El streaming por viewport (más
/// detalle al hacer zoom) es el paso siguiente.
pub fn load_pmtiles_overview(bytes: Vec<u8>) -> MapPreview {
    let pm = match crate::pmtiles::PmTiles::from_bytes(bytes) {
        Ok(p) => p,
        Err(e) => return MapPreview::Error(e),
    };
    if pm.header.tile_type != 1 {
        return MapPreview::Error("pmtiles: sólo se soportan tiles MVT".into());
    }
    let chosen = overview_zoom(&pm.header);
    let span = 1u32 << chosen;
    let mut data = MapData::default();
    // Ancla la proyección a los bounds del archivo (marco estable al streamear).
    data.bbox_override = Some(pmtiles_extent(&pm));
    for x in 0..span {
        for y in 0..span {
            if let Some(tile) = pm.tile(chosen, x, y) {
                data.append(mvt_tile_to_mapdata(&tile, chosen, x, y));
            }
        }
    }
    if data.total_features() == 0 {
        MapPreview::NoGeometry
    } else {
        MapPreview::Map {
            data,
            truncated: false,
        }
    }
}

// ─── Mapa-base mundial embebido ───────────────────────────────────────────────

/// El mapa-base mundial (Natural Earth admin-0, 177 países) embebido en el
/// binario y parseado una sola vez. Da contexto geográfico a cualquier dato
/// — offline, sin red ni tiles. Si por algo no parseara, queda vacío y el
/// visor simplemente no pinta fondo.
pub fn world_base() -> &'static MapData {
    use std::sync::OnceLock;
    static WORLD: OnceLock<MapData> = OnceLock::new();
    WORLD.get_or_init(|| {
        const SRC: &str = include_str!("../assets/world-countries.geojson");
        // Tope amplio: el dataset tiene decenas de miles de vértices y no
        // queremos recortarlo como a un documento de usuario.
        parse_into(SRC, 4_000_000)
            .map(|(d, _)| d)
            .unwrap_or_default()
    })
}

/// `(polígonos, vértices, países)` del mapa-base embebido. Diagnóstico para
/// herramientas/ejemplos (verificar que el asset cargó sin abrir ventana).
pub fn world_base_stats() -> (usize, usize, usize) {
    let w = world_base();
    (w.polygons.len(), w.vertex_count(), w.labels.len())
}

// ─── Basemap vivo ─────────────────────────────────────────────────────────────

/// Entrada de caché: tile decodificado + último reloj en que se usó.
pub struct CacheEntry {
    pub used: u64,
    pub data: MapData,
}

/// Basemap PMTiles **vivo**: mantiene el contenedor abierto y una caché de
/// tiles decodificados, y entrega el [`MapData`] visible para la cámara actual
/// (streaming por viewport). Sin red: todo sale del archivo local.
///
/// El host lo guarda mientras un `.pmtiles` esté abierto y llama a
/// [`Basemap::viewport`] cuando la cámara cambia.
pub struct Basemap {
    pm: crate::pmtiles::PmTiles,
    bounds: BBox,
    /// Tiles ya decodificados (`(z,x,y)` → geometrías), con marca de uso para
    /// el desalojo LRU.
    cache: HashMap<(u32, u32, u32), CacheEntry>,
    /// Reloj lógico monótono: cada viewport lo incrementa y marca los tiles
    /// que toca, para saber cuáles son los menos usados.
    clock: u64,
}

impl Basemap {
    /// Abre un `.pmtiles` ya en memoria como basemap vivo.
    pub fn open(bytes: Vec<u8>) -> Result<Self, String> {
        let pm = crate::pmtiles::PmTiles::from_bytes(bytes)?;
        if pm.header.tile_type != 1 {
            return Err("pmtiles: sólo se soportan tiles MVT".into());
        }
        let bounds = pmtiles_extent(&pm);
        Ok(Basemap {
            pm,
            bounds,
            cache: HashMap::new(),
            clock: 0,
        })
    }

    /// Tope de tiles a fundir por viewport (evita explosiones de memoria).
    const MAX_TILES: usize = 48;
    /// Tope de tiles decodificados en caché (desalojo LRU al excederlo).
    const CACHE_CAP: usize = 256;

    /// Tiles actualmente en caché (diagnóstico/tests).
    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }

    /// Devuelve el [`MapData`] visible para `view`: elige el zoom de tiles
    /// según el span visible y el ancho del panel, enumera los tiles que
    /// tocan el viewport, los decodifica (cacheando) y los funde. La bbox
    /// queda anclada a los bounds del archivo.
    pub fn viewport(&mut self, view: &MapView) -> MapData {
        use crate::camara::Projection;

        let mut out = MapData::default();
        out.bbox_override = Some(self.bounds);

        let Some((rx, ry, rw, rh)) = view.rect() else {
            return out;
        };
        let proj = Projection::fit(
            self.bounds,
            (rx as f64, ry as f64, rw as f64, rh as f64),
            view.zoom,
            view.pan,
        );
        // Esquinas del panel → lon/lat (región visible).
        let a = proj.inverse(rx as f64, ry as f64);
        let b = proj.inverse((rx + rw) as f64, (ry + rh) as f64);
        let west = a[0].min(b[0]).max(-180.0);
        let east = a[0].max(b[0]).min(180.0);
        let south = a[1].min(b[1]).max(-85.05);
        let north = a[1].max(b[1]).min(85.05);

        let zmin = self.pm.header.min_zoom as u32;
        let zmax = self.pm.header.max_zoom as u32;
        let z = vt::zoom_for_span(west, east, rw as f64).clamp(zmin, zmax);

        // Rango de tiles visibles (Y crece hacia el sur).
        let (x0, y0) = vt::lonlat_to_tile(z, west, north);
        let (x1, y1) = vt::lonlat_to_tile(z, east, south);
        let (x0, x1) = (x0.min(x1), x0.max(x1));
        let (y0, y1) = (y0.min(y1), y0.max(y1));

        // Reloj nuevo para este viewport: los tiles que toquemos quedan como
        // los más recientes (a salvo del desalojo de este frame).
        self.clock += 1;
        let now = self.clock;

        // Asegura los tiles en caché (decodificando los nuevos), tocando su
        // marca de uso, respetando el tope por viewport.
        let mut count = 0usize;
        'outer: for x in x0..=x1 {
            for y in y0..=y1 {
                if count >= Self::MAX_TILES {
                    break 'outer;
                }
                count += 1;
                let key = (z, x, y);
                match self.cache.get_mut(&key) {
                    Some(entry) => entry.used = now,
                    None => {
                        let data = self
                            .pm
                            .tile(z, x, y)
                            .map(|bytes| mvt_tile_to_mapdata(&bytes, z, x, y))
                            .unwrap_or_default();
                        self.cache.insert(key, CacheEntry { used: now, data });
                    }
                }
            }
        }
        evict_lru(&mut self.cache, Self::CACHE_CAP);

        // Funde lo cacheado en el viewport.
        let mut merged = 0usize;
        for x in x0..=x1 {
            for y in y0..=y1 {
                if merged >= Self::MAX_TILES {
                    break;
                }
                merged += 1;
                if let Some(entry) = self.cache.get(&(z, x, y)) {
                    out.append(entry.data.clone());
                }
            }
        }
        out
    }
}

/// Desaloja las entradas menos usadas hasta que la caché entre en `cap`.
/// Las tocadas en el viewport actual tienen el reloj más alto, así que el
/// desalojo nunca pisa lo que se está por usar.
pub fn evict_lru(cache: &mut HashMap<(u32, u32, u32), CacheEntry>, cap: usize) {
    while cache.len() > cap {
        // Encuentra la entrada de menor `used` (la más vieja).
        let oldest = cache.iter().min_by_key(|(_, e)| e.used).map(|(k, _)| *k);
        match oldest {
            Some(k) => {
                cache.remove(&k);
            }
            None => break,
        }
    }
}
