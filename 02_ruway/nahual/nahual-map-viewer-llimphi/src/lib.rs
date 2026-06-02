//! `nahual-map-viewer-llimphi` — visor de mapas GeoJSON y GPX.
//!
//! Duodécimo visor del shell meta-app. Un `.geojson` es JSON, así que sin
//! visor propio caía al árbol (`tree`) — que muestra la estructura cruda
//! de coordenadas, ilegible como mapa. Pero un GeoJSON es para *verlo*:
//! este visor lo parsea con `serde_json`, aplana las geometrías
//! (`Point`/`LineString`/`Polygon` y sus multi-/colecciones) a puntos,
//! polilíneas y anillos en `lon/lat`, los **proyecta** (equirectangular con
//! corrección por `cos(lat)` para que no se estire en latitudes altas),
//! los encaja en el panel respetando el aspecto y los pinta en la escena
//! vello vía `paint_with`: polígonos rellenos translúcidos con borde,
//! líneas trazadas, puntos como discos.
//!
//! Sin red ni tiles: el dato vectorial se dibuja tal cual, offline puro —
//! la ética soberana de la suite. Arranca encuadrando todo el contenido y
//! se navega con **arrastre (pan) y rueda (zoom)** vía [`MapView`], la
//! cámara que el host guarda y muta (zoom anclado al centro del panel).
//!
//! Patrón fino de los otros viewers: carga sync en [`load_map`], render en
//! [`map_viewer_view`]. No conoce el AppBus: el caller pasa el path. La
//! interacción la cablea el host (el shell mapea arrastre→pan y rueda→zoom
//! a su `Msg`); el visor sólo aplica la [`MapView`] al proyectar.
//!
//! MVP feo-primero: proyección plana (buena a escala ciudad/país, deforma
//! cerca de los polos), sin leyenda de propiedades, sin mapa-base. Capamos
//! por bytes y por vértices para que vello no se atragante.

#![forbid(unsafe_code)]

use std::collections::{BinaryHeap, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Circle, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_text::{draw_block, Alignment, TextBlock};
use llimphi_ui::View;

pub mod pmtiles;
pub mod vt;

/// Tope de bytes a leer (128 MiB). Holgado para extractos PMTiles de ciudad;
/// el caller puede subirlo. (Un planeta entero pide streaming, no leer todo.)
pub const DEFAULT_MAP_BYTES_MAX: u64 = 128 * 1024 * 1024;

/// Tope de vértices a retener. Cortar datasets enormes mantiene el panel
/// instantáneo (vello rebuild es barato hasta ~500 K primitivos/frame).
const MAX_VERTICES: usize = 200_000;

/// Una coordenada geográfica `[lon, lat]` en grados. La `z` (altitud) de
/// GeoJSON, si viene, se ignora.
pub type Coord = [f64; 2];

/// Un anillo o polilínea: secuencia de coordenadas.
pub type Ring = Vec<Coord>;

/// Caja envolvente en grados: `(min_lon, min_lat, max_lon, max_lat)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BBox {
    pub min_lon: f64,
    pub min_lat: f64,
    pub max_lon: f64,
    pub max_lat: f64,
}

impl BBox {
    /// Caja vacía/invertida: lista para acumular con [`expand`](Self::expand).
    fn empty() -> Self {
        BBox {
            min_lon: f64::INFINITY,
            min_lat: f64::INFINITY,
            max_lon: f64::NEG_INFINITY,
            max_lat: f64::NEG_INFINITY,
        }
    }

    fn expand(&mut self, [lon, lat]: Coord) {
        self.min_lon = self.min_lon.min(lon);
        self.min_lat = self.min_lat.min(lat);
        self.max_lon = self.max_lon.max(lon);
        self.max_lat = self.max_lat.max(lat);
    }

    /// `true` si nunca se expandió (no hubo coordenadas).
    fn is_empty(&self) -> bool {
        self.min_lon > self.max_lon || self.min_lat > self.max_lat
    }
}

/// Una etiqueta: el nombre de una feature anclado a un punto representativo
/// (el punto mismo, el medio de una línea, el centroide de un polígono).
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub at: Coord,
    pub text: String,
}

/// Tope de etiquetas a retener — más que esto satura el panel de texto.
const MAX_LABELS: usize = 200;

/// Tope de propiedades retenidas por feature (para inspección/choropleth).
const MAX_PROPS: usize = 80;

/// Propiedades de una feature, retenidas para inspección (clic) y estilo por
/// valor (choropleth). `props` son pares clave→valor ya stringificados (orden
/// de aparición); `numbers` son sólo las numéricas, para escalas de color.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FeatureProps {
    pub name: Option<String>,
    pub props: Vec<(String, String)>,
    pub numbers: Vec<(String, f64)>,
}

impl FeatureProps {
    /// Valor numérico de una propiedad por nombre, si existe.
    pub fn number(&self, key: &str) -> Option<f64> {
        self.numbers.iter().find(|(k, _)| k == key).map(|(_, v)| *v)
    }
}

/// Geometrías aplanadas listas para proyectar y pintar. Las geometrías
/// GeoJSON anidadas (multi-, colecciones) se desarman a estas tres listas.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MapData {
    /// Puntos sueltos (`Point`/`MultiPoint`).
    pub points: Vec<Coord>,
    /// Polilíneas (`LineString`/`MultiLineString`).
    pub lines: Vec<Ring>,
    /// Polígonos: cada uno es una lista de anillos; el primero es el
    /// contorno exterior y los siguientes, huecos. (`Polygon`/`MultiPolygon`.)
    pub polygons: Vec<Vec<Ring>>,
    /// Nombres de features (de `properties.nombre`/`name`/…) anclados a un
    /// punto representativo, para rotular el mapa.
    pub labels: Vec<Label>,
    /// Propiedades por feature. Los índices `*_feat` apuntan acá.
    pub features: Vec<FeatureProps>,
    /// Índice de feature de cada punto (paralelo a `points`).
    pub point_feat: Vec<usize>,
    /// Índice de feature de cada línea (paralelo a `lines`).
    pub line_feat: Vec<usize>,
    /// Índice de feature de cada polígono (paralelo a `polygons`).
    pub polygon_feat: Vec<usize>,
    /// Caja envolvente fija (basemap PMTiles): ancla la proyección a un marco
    /// estable para que el mapa no salte mientras llegan tiles. Si es `None`,
    /// la bbox se calcula del contenido.
    pub bbox_override: Option<BBox>,
}

impl MapData {
    /// Cantidad total de vértices retenidos.
    pub fn vertex_count(&self) -> usize {
        self.points.len()
            + self.lines.iter().map(Vec::len).sum::<usize>()
            + self
                .polygons
                .iter()
                .flat_map(|p| p.iter().map(Vec::len))
                .sum::<usize>()
    }

    /// Caja envolvente: el override fijo si está, o la de todo el contenido.
    pub fn bbox(&self) -> Option<BBox> {
        if self.bbox_override.is_some() {
            return self.bbox_override;
        }
        let mut bb = BBox::empty();
        for p in &self.points {
            bb.expand(*p);
        }
        for l in &self.lines {
            for c in l {
                bb.expand(*c);
            }
        }
        for poly in &self.polygons {
            for ring in poly {
                for c in ring {
                    bb.expand(*c);
                }
            }
        }
        if bb.is_empty() {
            None
        } else {
            Some(bb)
        }
    }

    fn total_features(&self) -> usize {
        self.points.len() + self.lines.len() + self.polygons.len()
    }

    /// Anexa otro `MapData`, reindexando sus features (para fusionar varios
    /// tiles en un solo mapa).
    fn append(&mut self, other: MapData) {
        let base = self.features.len();
        self.features.extend(other.features);
        self.labels.extend(other.labels);
        self.points.extend(other.points);
        self.point_feat.extend(other.point_feat.into_iter().map(|f| f + base));
        self.lines.extend(other.lines);
        self.line_feat.extend(other.line_feat.into_iter().map(|f| f + base));
        self.polygons.extend(other.polygons);
        self.polygon_feat.extend(other.polygon_feat.into_iter().map(|f| f + base));
    }
}

/// Estado del visor. Replica la forma de los otros para que el shell lo
/// trate igual.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum MapPreview {
    /// Sin archivo seleccionado.
    #[default]
    Empty,
    /// GeoJSON parseado a geometrías (posiblemente truncado por
    /// [`MAX_VERTICES`]).
    Map { data: MapData, truncated: bool },
    /// Parseó como JSON pero no contiene ninguna geometría reconocible.
    NoGeometry,
    /// Excede el tope de tamaño.
    TooBig(u64),
    /// E/S o parseo falló.
    Error(String),
}

/// Lee el archivo y lo parsea a geometrías, desambiguando el formato por
/// contenido: PMTiles (binario), GPX/KML (XML), GeoJSON (JSON).
pub fn load_map(path: &Path, max_bytes: u64) -> MapPreview {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() > max_bytes => return MapPreview::TooBig(meta.len()),
        Err(e) => return MapPreview::Error(e.to_string()),
        _ => {}
    }
    let raw = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return MapPreview::Error(e.to_string()),
    };
    // PMTiles: contenedor binario de vector tiles (magic "PMTiles").
    if raw.starts_with(PMTILES_MAGIC) {
        return load_pmtiles_overview(raw);
    }
    // El resto es texto.
    let src = match String::from_utf8(raw) {
        Ok(s) => s,
        Err(_) => return MapPreview::Error("archivo binario no reconocido".into()),
    };
    // GPX/KML son XML (arrancan con `<`); GeoJSON es JSON (`{`/`[`). El shell
    // rutea todos al lens `map`, así que el visor desambigua por contenido.
    if src.trim_start().starts_with('<') {
        let head = &src[..src.len().min(2048)];
        if head.contains("<kml") {
            parse_kml(&src)
        } else {
            parse_gpx(&src)
        }
    } else {
        parse_geojson(&src)
    }
}

/// Parsea una cadena GeoJSON a [`MapPreview`]. Tolerante: ignora geometrías
/// malformadas en vez de abortar, y recorta al llegar a [`MAX_VERTICES`].
pub fn parse_geojson(src: &str) -> MapPreview {
    match parse_into(src, MAX_VERTICES) {
        Err(e) => MapPreview::Error(e),
        Ok((data, truncated)) => {
            if data.total_features() == 0 {
                MapPreview::NoGeometry
            } else {
                MapPreview::Map { data, truncated }
            }
        }
    }
}

/// Núcleo del parseo con presupuesto de vértices explícito. Devuelve la
/// geometría aplanada y si se truncó. Separado para reusarlo con el
/// mapa-base (que necesita un tope mucho mayor que un documento a ojo).
fn parse_into(src: &str, cap: usize) -> Result<(MapData, bool), String> {
    let value: serde_json::Value = serde_json::from_str(src).map_err(|e| e.to_string())?;
    let mut data = MapData::default();
    let mut budget = cap;
    collect(&value, &mut data, &mut budget, None, None);
    Ok((data, budget == 0))
}

/// Decodifica un tile vectorial MVT (`bytes` en `z/x/y`) a un [`MapData`]
/// renderizable, reusando toda la maquinaria del visor: cada feature del tile
/// queda con su capa de origen como nombre (calle/agua/edificio…). Es la
/// costura entre el decoder soberano de [`vt`] y el render existente; sobre
/// esto se monta el basemap PMTiles cuando exista el lector del contenedor.
pub fn mvt_tile_to_mapdata(bytes: &[u8], z: u32, x: u32, y: u32) -> MapData {
    let mut data = MapData::default();
    let mut budget = MAX_VERTICES;
    for tf in vt::decode_mvt_tile(bytes, z, x, y) {
        if budget == 0 {
            break;
        }
        let fi = make_feature(&mut data, Some(&tf.layer));
        match tf.geom {
            vt::TileGeom::Point(c) => push_points(&mut data, std::slice::from_ref(&c), &mut budget, fi),
            vt::TileGeom::Line(l) => push_line(&mut data, l, &mut budget, fi),
            vt::TileGeom::Polygon(rings) => push_polygon(&mut data, rings, &mut budget, fi),
        }
    }
    data
}

/// Magic de un archivo PMTiles v3.
const PMTILES_MAGIC: &[u8] = b"PMTiles";

/// Carga una **vista general** de un `.pmtiles`: decodifica los tiles del zoom
/// más bajo que cubra el contenido (pocos tiles) y los funde en un [`MapData`].
/// Es el basemap soberano en su forma MVP: muestra el mapa completo a baja
/// resolución, reutilizando todo el render. El streaming por viewport (más
/// detalle al hacer zoom) es el paso siguiente.
pub fn load_pmtiles_overview(bytes: Vec<u8>) -> MapPreview {
    let pm = match pmtiles::PmTiles::from_bytes(bytes) {
        Ok(p) => p,
        Err(e) => return MapPreview::Error(e),
    };
    if pm.header.tile_type != 1 {
        return MapPreview::Error("pmtiles: sólo se soportan tiles MVT".into());
    }
    // Elegí el zoom más bajo cuyos tiles no superen un tope (vista general).
    const MAX_TILES: u32 = 64;
    let mut chosen = pm.header.min_zoom as u32;
    for z in pm.header.min_zoom as u32..=pm.header.max_zoom as u32 {
        let span = 1u32 << z;
        if span.saturating_mul(span) <= MAX_TILES {
            chosen = z;
        } else {
            break;
        }
    }
    let span = 1u32 << chosen;
    let mut data = MapData::default();
    // Ancla la proyección a los bounds del archivo (marco estable al streamear).
    data.bbox_override = pmtiles_bounds(&pm.header);
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
        MapPreview::Map { data, truncated: false }
    }
}

/// Bounds del header como [`BBox`], si son válidos (algunos archivos los dejan
/// en cero → caemos a "mundo entero").
fn pmtiles_bounds(h: &pmtiles::Header) -> Option<BBox> {
    let bb = BBox {
        min_lon: h.min_lon,
        min_lat: h.min_lat,
        max_lon: h.max_lon,
        max_lat: h.max_lat,
    };
    if bb.max_lon > bb.min_lon && bb.max_lat > bb.min_lat {
        Some(bb)
    } else {
        Some(BBox { min_lon: -180.0, min_lat: -85.05, max_lon: 180.0, max_lat: 85.05 })
    }
}

/// Basemap PMTiles **vivo**: mantiene el contenedor abierto y una caché de
/// tiles decodificados, y entrega el [`MapData`] visible para la cámara actual
/// (streaming por viewport). Sin red: todo sale del archivo local.
///
/// El host lo guarda mientras un `.pmtiles` esté abierto y llama a
/// [`Basemap::viewport`] cuando la cámara cambia.
pub struct Basemap {
    pm: pmtiles::PmTiles,
    bounds: BBox,
    /// Tiles ya decodificados (`(z,x,y)` → geometrías), con marca de uso para
    /// el desalojo LRU.
    cache: HashMap<(u32, u32, u32), CacheEntry>,
    /// Reloj lógico monótono: cada viewport lo incrementa y marca los tiles
    /// que toca, para saber cuáles son los menos usados.
    clock: u64,
}

/// Entrada de caché: tile decodificado + último reloj en que se usó.
struct CacheEntry {
    used: u64,
    data: MapData,
}

impl Basemap {
    /// Abre un `.pmtiles` ya en memoria como basemap vivo.
    pub fn open(bytes: Vec<u8>) -> Result<Self, String> {
        let pm = pmtiles::PmTiles::from_bytes(bytes)?;
        if pm.header.tile_type != 1 {
            return Err("pmtiles: sólo se soportan tiles MVT".into());
        }
        let bounds = pmtiles_bounds(&pm.header).unwrap();
        Ok(Basemap { pm, bounds, cache: HashMap::new(), clock: 0 })
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
        let mut out = MapData::default();
        out.bbox_override = Some(self.bounds);

        let Some((rx, ry, rw, rh)) = view.rect() else {
            return out;
        };
        let proj = Projection::fit(self.bounds, (rx as f64, ry as f64, rw as f64, rh as f64), view.zoom, view.pan);
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
fn evict_lru(cache: &mut HashMap<(u32, u32, u32), CacheEntry>, cap: usize) {
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

/// Nombres de campos numéricos presentes en las features, en orden de primera
/// aparición y sin repetir. Para que el host cicle el campo de choropleth.
pub fn numeric_fields(data: &MapData) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for f in &data.features {
        for (k, _) in &f.numbers {
            if !out.iter().any(|o| o == k) {
                out.push(k.clone());
            }
        }
    }
    out
}

/// Color de una posición `t ∈ [0,1]` en una escala secuencial azul→ámbar→rojo
/// (legible y con buen contraste sobre fondo oscuro o claro).
fn scale_color(t: f64) -> Color {
    let t = t.clamp(0.0, 1.0);
    // Tres paradas: azul (40,110,200) → ámbar (240,200,70) → rojo (210,60,50).
    let stops = [(40.0, 110.0, 200.0), (240.0, 200.0, 70.0), (210.0, 60.0, 50.0)];
    let (a, b, local) = if t < 0.5 {
        (stops[0], stops[1], t / 0.5)
    } else {
        (stops[1], stops[2], (t - 0.5) / 0.5)
    };
    let lerp = |x: f64, y: f64| (x + (y - x) * local).round().clamp(0.0, 255.0) as u8;
    Color::from_rgba8(lerp(a.0, b.0), lerp(a.1, b.1), lerp(a.2, b.2), 255)
}

/// Parsea GPX (XML de GPS): waypoints (`<wpt>`) → puntos, rutas (`<rte>`) y
/// segmentos de track (`<trkseg>`) → polilíneas. Los `<name>` de waypoints,
/// rutas y tracks se vuelven etiquetas. Tolerante: ignora lo que no entiende.
pub fn parse_gpx(src: &str) -> MapPreview {
    use quick_xml::events::Event;
    use quick_xml::reader::Reader;

    /// A quién se asigna el próximo `<name>` de texto.
    #[derive(Clone, Copy, PartialEq)]
    enum NameTarget {
        None,
        Seg,
        Wpt,
    }

    let mut reader = Reader::from_str(src);
    reader.trim_text(true);
    let mut buf = Vec::new();

    let mut data = MapData::default();
    let mut budget = MAX_VERTICES;

    // Línea (track-seg o ruta) en curso + su nombre heredado del trk/rte.
    let mut seg: Vec<Coord> = Vec::new();
    let mut seg_name: Option<String> = None;
    // Waypoint en curso (con hijos, p. ej. `<name>`).
    let mut wpt: Option<Coord> = None;
    let mut wpt_name: Option<String> = None;
    let mut target = NameTarget::None;

    // Cierra la línea en curso como polilínea con su etiqueta.
    let flush_seg =
        |data: &mut MapData, budget: &mut usize, seg: &mut Vec<Coord>, name: &mut Option<String>| {
            let rep = midpoint(seg);
            let fi = make_feature(data, name.as_deref());
            push_line(data, std::mem::take(seg), budget, fi);
            label_at(data, name.as_deref(), rep);
            *name = None;
        };

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) | Err(_) => break,
            Ok(Event::Start(e)) => match e.local_name().as_ref() {
                b"trk" | b"rte" => {
                    seg.clear();
                    seg_name = None;
                    target = NameTarget::Seg;
                }
                b"trkseg" => seg.clear(),
                b"trkpt" | b"rtept" => {
                    if let Some(c) = gpx_latlon(&e) {
                        seg.push(c);
                    }
                }
                b"wpt" => {
                    wpt = gpx_latlon(&e);
                    wpt_name = None;
                    target = NameTarget::Wpt;
                }
                b"name" => {} // el texto siguiente va al `target` vigente
                _ => {}
            },
            Ok(Event::Empty(e)) => match e.local_name().as_ref() {
                b"trkpt" | b"rtept" => {
                    if let Some(c) = gpx_latlon(&e) {
                        seg.push(c);
                    }
                }
                b"wpt" => {
                    if let Some(c) = gpx_latlon(&e) {
                        let fi = make_feature(&mut data, None);
                        push_points(&mut data, std::slice::from_ref(&c), &mut budget, fi);
                    }
                }
                _ => {}
            },
            Ok(Event::Text(t)) => {
                if target != NameTarget::None {
                    if let Ok(txt) = t.unescape() {
                        let txt = txt.trim().to_string();
                        if !txt.is_empty() {
                            match target {
                                NameTarget::Seg => seg_name.get_or_insert(txt),
                                NameTarget::Wpt => wpt_name.get_or_insert(txt),
                                NameTarget::None => unreachable!(),
                            };
                        }
                    }
                }
            }
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                b"trkseg" => flush_seg(&mut data, &mut budget, &mut seg, &mut seg_name),
                b"rte" => {
                    flush_seg(&mut data, &mut budget, &mut seg, &mut seg_name);
                    target = NameTarget::None;
                }
                b"trk" => target = NameTarget::None,
                b"wpt" => {
                    if let Some(c) = wpt.take() {
                        let fi = make_feature(&mut data, wpt_name.as_deref());
                        push_points(&mut data, std::slice::from_ref(&c), &mut budget, fi);
                        label_at(&mut data, wpt_name.as_deref(), Some(c));
                    }
                    wpt_name = None;
                    target = NameTarget::None;
                }
                _ => {}
            },
            _ => {}
        }
        buf.clear();
        if budget == 0 {
            break;
        }
    }

    if data.total_features() == 0 {
        MapPreview::NoGeometry
    } else {
        MapPreview::Map { data, truncated: budget == 0 }
    }
}

/// Parsea KML (XML de Google Earth): cada `<Placemark>` con su `<name>` y su
/// geometría (`<Point>`/`<LineString>`/`<Polygon>` con `<coordinates>`). Las
/// coordenadas KML son `lon,lat[,alt]` separadas por espacios. Tolerante.
pub fn parse_kml(src: &str) -> MapPreview {
    use quick_xml::events::Event;
    use quick_xml::reader::Reader;

    #[derive(Clone, Copy, PartialEq)]
    enum Geom {
        None,
        Point,
        Line,
        Ring,
    }

    let mut reader = Reader::from_str(src);
    reader.trim_text(true);
    let mut buf = Vec::new();

    let mut data = MapData::default();
    let mut budget = MAX_VERTICES;

    let mut placemark_name: Option<String> = None;
    let mut in_name = false; // dentro de <name> de un Placemark
    let mut geom = Geom::None;
    let mut in_polygon = false;
    let mut poly_rings: Vec<Ring> = Vec::new();
    let mut reading_coords = false;
    let mut coord_buf = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) | Err(_) => break,
            Ok(Event::Start(e)) => match e.local_name().as_ref() {
                b"Placemark" => {
                    placemark_name = None;
                    geom = Geom::None;
                }
                b"name" => in_name = true,
                b"Point" => geom = Geom::Point,
                b"LineString" => geom = Geom::Line,
                b"Polygon" => {
                    in_polygon = true;
                    poly_rings.clear();
                }
                b"LinearRing" => geom = Geom::Ring,
                b"coordinates" => {
                    reading_coords = true;
                    coord_buf.clear();
                }
                _ => {}
            },
            Ok(Event::Text(t)) => {
                if let Ok(txt) = t.unescape() {
                    if reading_coords {
                        coord_buf.push_str(&txt);
                    } else if in_name {
                        let txt = txt.trim();
                        if !txt.is_empty() {
                            placemark_name.get_or_insert_with(|| txt.to_string());
                        }
                    }
                }
            }
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                b"name" => in_name = false,
                b"coordinates" => {
                    reading_coords = false;
                    let coords = kml_coords(&coord_buf);
                    match geom {
                        Geom::Point => {
                            if let Some(c) = coords.first().copied() {
                                let fi = make_feature(&mut data, placemark_name.as_deref());
                                push_points(&mut data, std::slice::from_ref(&c), &mut budget, fi);
                                label_at(&mut data, placemark_name.as_deref(), Some(c));
                            }
                        }
                        Geom::Line => {
                            let rep = midpoint(&coords);
                            let fi = make_feature(&mut data, placemark_name.as_deref());
                            push_line(&mut data, coords, &mut budget, fi);
                            label_at(&mut data, placemark_name.as_deref(), rep);
                        }
                        Geom::Ring => {
                            if in_polygon {
                                poly_rings.push(coords);
                            } else {
                                // LinearRing suelto → polígono de un anillo.
                                let rep = centroid(&coords);
                                let fi = make_feature(&mut data, placemark_name.as_deref());
                                push_polygon(&mut data, vec![coords], &mut budget, fi);
                                label_at(&mut data, placemark_name.as_deref(), rep);
                            }
                        }
                        Geom::None => {}
                    }
                }
                b"Polygon" => {
                    let rep = poly_rings.first().and_then(|r| centroid(r));
                    let fi = make_feature(&mut data, placemark_name.as_deref());
                    push_polygon(&mut data, std::mem::take(&mut poly_rings), &mut budget, fi);
                    label_at(&mut data, placemark_name.as_deref(), rep);
                    in_polygon = false;
                }
                b"LinearRing" => geom = Geom::None,
                b"Point" | b"LineString" => geom = Geom::None,
                _ => {}
            },
            _ => {}
        }
        buf.clear();
        if budget == 0 {
            break;
        }
    }

    if data.total_features() == 0 {
        MapPreview::NoGeometry
    } else {
        MapPreview::Map { data, truncated: budget == 0 }
    }
}

/// Parsea un bloque de coordenadas KML (`lon,lat[,alt] lon,lat[,alt] …`).
fn kml_coords(s: &str) -> Vec<Coord> {
    s.split_whitespace()
        .filter_map(|tok| {
            let mut it = tok.split(',');
            let lon = it.next()?.trim().parse::<f64>().ok()?;
            let lat = it.next()?.trim().parse::<f64>().ok()?;
            (lon.is_finite() && lat.is_finite()).then_some([lon, lat])
        })
        .collect()
}

/// Lee los atributos `lat`/`lon` de un elemento GPX a una [`Coord`]
/// `[lon, lat]`. `None` si falta alguno o no son números finitos.
fn gpx_latlon(e: &quick_xml::events::BytesStart) -> Option<Coord> {
    let mut lat = None;
    let mut lon = None;
    for a in e.attributes().flatten() {
        match a.key.local_name().as_ref() {
            b"lat" => lat = std::str::from_utf8(&a.value).ok().and_then(|s| s.parse::<f64>().ok()),
            b"lon" => lon = std::str::from_utf8(&a.value).ok().and_then(|s| s.parse::<f64>().ok()),
            _ => {}
        }
    }
    match (lon, lat) {
        (Some(lon), Some(lat)) if lon.is_finite() && lat.is_finite() => Some([lon, lat]),
        _ => None,
    }
}

/// El mapa-base mundial (Natural Earth admin-0, 177 países) embebido en el
/// binario y parseado una sola vez. Da contexto geográfico a cualquier dato
/// — offline, sin red ni tiles. Si por algo no parseara, queda vacío y el
/// visor simplemente no pinta fondo.
fn world_base() -> &'static MapData {
    static WORLD: OnceLock<MapData> = OnceLock::new();
    WORLD.get_or_init(|| {
        const SRC: &str = include_str!("../assets/world-countries.geojson");
        // Tope amplio: el dataset tiene decenas de miles de vértices y no
        // queremos recortarlo como a un documento de usuario.
        parse_into(SRC, 4_000_000).map(|(d, _)| d).unwrap_or_default()
    })
}

/// `(polígonos, vértices, países)` del mapa-base embebido. Diagnóstico para
/// herramientas/ejemplos (verificar que el asset cargó sin abrir ventana).
pub fn world_base_stats() -> (usize, usize, usize) {
    let w = world_base();
    (w.polygons.len(), w.vertex_count(), w.labels.len())
}

/// Recorre recursivamente un valor GeoJSON (FeatureCollection / Feature /
/// geometría / GeometryCollection) acumulando geometrías en `data`. `budget`
/// es el presupuesto de vértices restante: al agotarse, deja de agregar.
/// `name` es el rótulo heredado de la `Feature` contenedora (si la hay), que
/// se ancla a un punto representativo de cada geometría hoja.
fn collect(
    v: &serde_json::Value,
    data: &mut MapData,
    budget: &mut usize,
    name: Option<&str>,
    feat: Option<usize>,
) {
    if *budget == 0 {
        return;
    }
    let Some(ty) = v.get("type").and_then(|t| t.as_str()) else {
        return;
    };
    // Índice de feature para las geometrías hoja: el heredado, o uno nuevo
    // (vacío) para geometría suelta sin Feature contenedora.
    let leaf_feat = |data: &mut MapData| match feat {
        Some(f) => f,
        None => make_feature(data, name),
    };
    match ty {
        "FeatureCollection" => {
            if let Some(arr) = v.get("features").and_then(|f| f.as_array()) {
                for f in arr {
                    collect(f, data, budget, None, None);
                }
            }
        }
        "Feature" => {
            // Una Feature crea su registro de propiedades una vez; toda su
            // geometría (incluso multi-) comparte ese índice.
            let mut fp = feature_props(v.get("properties"));
            let fname = feature_name(v.get("properties"));
            fp.name = fname.clone();
            data.features.push(fp);
            let fi = data.features.len() - 1;
            if let Some(g) = v.get("geometry") {
                collect(g, data, budget, fname.as_deref().or(name), Some(fi));
            }
        }
        "GeometryCollection" => {
            if let Some(arr) = v.get("geometries").and_then(|g| g.as_array()) {
                for g in arr {
                    collect(g, data, budget, name, feat);
                }
            }
        }
        "Point" => {
            if let Some(c) = coord(v.get("coordinates")) {
                let fi = leaf_feat(data);
                push_points(data, std::slice::from_ref(&c), budget, fi);
                label_at(data, name, Some(c));
            }
        }
        "MultiPoint" => {
            let cs = coord_list(v.get("coordinates"));
            let rep = cs.first().copied();
            let fi = leaf_feat(data);
            push_points(data, &cs, budget, fi);
            label_at(data, name, rep);
        }
        "LineString" => {
            let line = coord_list(v.get("coordinates"));
            let rep = midpoint(&line);
            let fi = leaf_feat(data);
            push_line(data, line, budget, fi);
            label_at(data, name, rep);
        }
        "MultiLineString" => {
            let lines = coord_rings(v.get("coordinates"));
            let rep = lines.first().and_then(|l| midpoint(l));
            let fi = leaf_feat(data);
            for line in lines {
                push_line(data, line, budget, fi);
            }
            label_at(data, name, rep);
        }
        "Polygon" => {
            let rings = coord_rings(v.get("coordinates"));
            let rep = rings.first().and_then(|r| centroid(r));
            let fi = leaf_feat(data);
            push_polygon(data, rings, budget, fi);
            label_at(data, name, rep);
        }
        "MultiPolygon" => {
            // coordinates: [ [ ring, ring... ], ... ]
            if let Some(arr) = v.get("coordinates").and_then(|c| c.as_array()) {
                let fi = leaf_feat(data);
                let mut rep = None;
                for poly in arr {
                    let rings = coord_rings(Some(poly));
                    if rep.is_none() {
                        rep = rings.first().and_then(|r| centroid(r));
                    }
                    push_polygon(data, rings, budget, fi);
                }
                label_at(data, name, rep);
            }
        }
        _ => {}
    }
}

/// Extrae un nombre legible de `properties`, probando claves usuales en
/// español/inglés. `None` si no hay propiedades o ninguna clave aplica.
fn feature_name(props: Option<&serde_json::Value>) -> Option<String> {
    let obj = props?.as_object()?;
    for key in ["nombre", "name", "título", "titulo", "title", "label", "Name", "NAME"] {
        if let Some(s) = obj.get(key).and_then(|v| v.as_str()) {
            let s = s.trim();
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// Ancla una etiqueta a `at` si hay nombre y punto, respetando [`MAX_LABELS`].
fn label_at(data: &mut MapData, name: Option<&str>, at: Option<Coord>) {
    if let (Some(text), Some(at)) = (name, at) {
        if data.labels.len() < MAX_LABELS {
            data.labels.push(Label { at, text: text.to_string() });
        }
    }
}

/// Vértice central de una polilínea (rótulo de líneas).
fn midpoint(line: &[Coord]) -> Option<Coord> {
    if line.is_empty() {
        None
    } else {
        Some(line[line.len() / 2])
    }
}

/// Centroide simple (promedio de vértices) de un anillo, ignorando el último
/// si repite el primero (anillos GeoJSON cerrados).
fn centroid(ring: &[Coord]) -> Option<Coord> {
    let pts: &[Coord] = match ring.split_last() {
        Some((last, head)) if !head.is_empty() && last == &head[0] => head,
        _ => ring,
    };
    if pts.is_empty() {
        return None;
    }
    let (mut sx, mut sy) = (0.0, 0.0);
    for [lon, lat] in pts {
        sx += lon;
        sy += lat;
    }
    let n = pts.len() as f64;
    Some([sx / n, sy / n])
}

fn push_points(data: &mut MapData, pts: &[Coord], budget: &mut usize, feat: usize) {
    for p in pts {
        if *budget == 0 {
            return;
        }
        data.points.push(*p);
        data.point_feat.push(feat);
        *budget -= 1;
    }
}

fn push_line(data: &mut MapData, mut line: Ring, budget: &mut usize, feat: usize) {
    if line.len() < 2 {
        return;
    }
    line.truncate(*budget);
    if line.len() < 2 {
        return;
    }
    *budget -= line.len();
    data.lines.push(line);
    data.line_feat.push(feat);
}

fn push_polygon(data: &mut MapData, rings: Vec<Ring>, budget: &mut usize, feat: usize) {
    let mut kept: Vec<Ring> = Vec::new();
    for mut ring in rings {
        if *budget == 0 {
            break;
        }
        ring.truncate(*budget);
        if ring.len() < 3 {
            continue;
        }
        *budget -= ring.len();
        kept.push(ring);
    }
    if !kept.is_empty() {
        data.polygons.push(kept);
        data.polygon_feat.push(feat);
    }
}

/// Crea una feature con un nombre opcional (para formatos sin propiedades
/// ricas: GPX/KML, o geometrías sueltas) y devuelve su índice.
fn make_feature(data: &mut MapData, name: Option<&str>) -> usize {
    let mut fp = FeatureProps::default();
    if let Some(n) = name {
        fp.name = Some(n.to_string());
        fp.props.push(("name".to_string(), n.to_string()));
    }
    data.features.push(fp);
    data.features.len() - 1
}

/// Construye [`FeatureProps`] desde el objeto `properties` de una Feature
/// GeoJSON: conserva escalares (número/string/bool) en orden, y los números
/// también en `numbers` para choropleth. Omite null/array/objeto.
fn feature_props(props: Option<&serde_json::Value>) -> FeatureProps {
    let mut fp = FeatureProps::default();
    let Some(obj) = props.and_then(|p| p.as_object()) else {
        return fp;
    };
    for (k, v) in obj {
        if fp.props.len() >= MAX_PROPS {
            break;
        }
        match v {
            serde_json::Value::Number(n) => {
                if let Some(f) = n.as_f64() {
                    fp.numbers.push((k.clone(), f));
                    fp.props.push((k.clone(), n.to_string()));
                }
            }
            serde_json::Value::String(s) => fp.props.push((k.clone(), s.clone())),
            serde_json::Value::Bool(b) => fp.props.push((k.clone(), b.to_string())),
            _ => {}
        }
    }
    fp
}

/// Lee una coordenada `[lon, lat(, z)]` de un valor JSON. `None` si no es un
/// array de al menos dos números finitos.
fn coord(v: Option<&serde_json::Value>) -> Option<Coord> {
    let arr = v?.as_array()?;
    let lon = arr.first()?.as_f64()?;
    let lat = arr.get(1)?.as_f64()?;
    if lon.is_finite() && lat.is_finite() {
        Some([lon, lat])
    } else {
        None
    }
}

/// Lee una lista de coordenadas `[[lon,lat], ...]`.
fn coord_list(v: Option<&serde_json::Value>) -> Vec<Coord> {
    let Some(arr) = v.and_then(|x| x.as_array()) else {
        return Vec::new();
    };
    arr.iter().filter_map(|c| coord(Some(c))).collect()
}

/// Lee una lista de anillos `[[[lon,lat], ...], ...]`.
fn coord_rings(v: Option<&serde_json::Value>) -> Vec<Ring> {
    let Some(arr) = v.and_then(|x| x.as_array()) else {
        return Vec::new();
    };
    arr.iter().map(|r| coord_list(Some(r))).collect()
}

/// Transformación de cámara del mapa: zoom (factor) + pan (desplazamiento en
/// píxeles físicos de pantalla). El host la guarda y la muta con la rueda y
/// el arrastre; el canvas la aplica al proyectar, anclando el zoom al centro
/// del panel.
///
/// La celda `rect` la **escribe el canvas** en cada paint con su rectángulo
/// físico, y la **lee el host** ([`MapView::contains`]) para acotar el
/// zoom-por-rueda al área del mapa (sin robarle el scroll a la lista).
#[derive(Clone)]
pub struct MapView {
    pub zoom: f64,
    pub pan: (f64, f64),
    /// Dibujar el mapa-base mundial de fondo.
    pub show_base: bool,
    /// Índice de la feature seleccionada (clic) en `MapData.features`, si la hay.
    pub selected: Option<usize>,
    /// Campo numérico por el que colorear los polígonos (choropleth). `None`
    /// = relleno uniforme.
    pub color_field: Option<String>,
    /// Modo búsqueda activo (captura el teclado para escribir la consulta).
    pub searching: bool,
    /// Consulta de búsqueda en curso.
    pub query: String,
    /// Modo ruteo activo (los clics fijan origen/destino).
    pub routing: bool,
    /// Puntos de ruta marcados por el usuario (0..2), en lon/lat.
    pub route_pins: Vec<Coord>,
    /// Ruta calculada (polilínea a dibujar), vacía si no hay.
    pub route_path: Vec<Coord>,
    /// Longitud de la ruta calculada, en metros.
    pub route_meters: f64,
    rect: Arc<Mutex<Option<(f32, f32, f32, f32)>>>,
}

impl Default for MapView {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: (0.0, 0.0),
            show_base: true,
            selected: None,
            color_field: None,
            searching: false,
            query: String::new(),
            routing: false,
            route_pins: Vec::new(),
            route_path: Vec::new(),
            route_meters: 0.0,
            rect: Arc::new(Mutex::new(None)),
        }
    }
}

impl MapView {
    /// Límites de zoom: ni tan lejos que desaparezca, ni tan cerca que se
    /// pierda en aritmética.
    pub const ZOOM_MIN: f64 = 0.2;
    pub const ZOOM_MAX: f64 = 64.0;

    /// Vuelve al encuadre inicial (zoom 1, sin pan) y limpia la selección.
    /// Conserva la celda del rect para no perder el gateo entre selecciones.
    pub fn reset(&mut self) {
        self.zoom = 1.0;
        self.pan = (0.0, 0.0);
        self.selected = None;
        self.searching = false;
        self.query.clear();
        self.routing = false;
        self.clear_route();
    }

    /// Limpia los puntos y la ruta calculada (no toca el modo).
    pub fn clear_route(&mut self) {
        self.route_pins.clear();
        self.route_path.clear();
        self.route_meters = 0.0;
    }

    /// Acumula un desplazamiento (de un arrastre), en píxeles físicos.
    pub fn pan_by(&mut self, dx: f64, dy: f64) {
        self.pan.0 += dx;
        self.pan.1 += dy;
    }

    /// Multiplica el zoom (acotado). El pan no se toca: el zoom queda
    /// anclado al centro del panel.
    pub fn zoom_by(&mut self, factor: f64) {
        if factor.is_finite() && factor > 0.0 {
            self.zoom = (self.zoom * factor).clamp(Self::ZOOM_MIN, Self::ZOOM_MAX);
        }
    }

    /// Zoom anclado a un punto de pantalla `(cx, cy)` (físicos): el lugar bajo
    /// el cursor queda fijo. Si todavía no se pintó (sin rect), cae a
    /// [`zoom_by`] (zoom al centro).
    pub fn zoom_at(&mut self, factor: f64, cx: f32, cy: f32) {
        if !(factor.is_finite() && factor > 0.0) {
            return;
        }
        let Some((rx, ry, rw, rh)) = self.rect.lock().ok().and_then(|g| *g) else {
            self.zoom_by(factor);
            return;
        };
        let pivot_x = rx as f64 + rw as f64 * 0.5;
        let pivot_y = ry as f64 + rh as f64 * 0.5;
        let z0 = self.zoom;
        let z1 = (z0 * factor).clamp(Self::ZOOM_MIN, Self::ZOOM_MAX);
        if (z1 - z0).abs() < f64::EPSILON {
            return;
        }
        // Mantener fijo el punto bajo el cursor:
        //   pan1 = pan0 - (c - pivot - pan0) * (z1 - z0) / z0
        let k = (z1 - z0) / z0;
        self.pan.0 -= (cx as f64 - pivot_x - self.pan.0) * k;
        self.pan.1 -= (cy as f64 - pivot_y - self.pan.1) * k;
        self.zoom = z1;
    }

    /// Alterna el mapa-base de fondo.
    pub fn toggle_base(&mut self) {
        self.show_base = !self.show_base;
    }

    /// `true` si `(x, y)` (físicos) cae dentro del último rect pintado por el
    /// canvas. `false` si todavía no se pintó.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        match self.rect.lock().ok().and_then(|g| *g) {
            Some((rx, ry, rw, rh)) => x >= rx && x <= rx + rw && y >= ry && y <= ry + rh,
            None => false,
        }
    }

    /// Último rect físico pintado por el canvas (si ya se pintó alguna vez).
    pub fn rect(&self) -> Option<(f32, f32, f32, f32)> {
        self.rect.lock().ok().and_then(|g| *g)
    }

    /// Registra el rect físico del canvas (lo llama el propio canvas).
    fn record_rect(&self, r: (f32, f32, f32, f32)) {
        if let Ok(mut g) = self.rect.lock() {
            *g = Some(r);
        }
    }
}

/// Paleta del viewer.
#[derive(Debug, Clone, Copy)]
pub struct MapViewerPalette {
    pub bg: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
    pub fg_error: Color,
    /// Trazo de líneas y bordes de polígono.
    pub stroke: Color,
    /// Relleno de polígonos (se aplica translúcido).
    pub fill: Color,
    /// Disco de los puntos.
    pub point: Color,
    /// Rejilla de coordenadas (se aplica muy tenue).
    pub grid: Color,
    /// Texto de etiquetas y rótulos de la rejilla.
    pub label: Color,
    /// Mapa-base mundial (tierra): se aplica muy tenue de fondo.
    pub land: Color,
}

impl Default for MapViewerPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl MapViewerPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg: t.bg_app,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
            fg_error: t.fg_destructive,
            stroke: t.accent,
            fill: t.accent,
            point: t.fg_text,
            grid: t.fg_muted,
            label: t.fg_text,
            land: t.fg_muted,
        }
    }
}

/// Multiplica el alfa de un color (sin reemplazarlo). Mismo patrón que los
/// widgets de llimphi.
fn with_alpha(c: Color, alpha: f32) -> Color {
    let rgba = c.to_rgba8();
    let a = (alpha.clamp(0.0, 1.0) * 255.0) as u8;
    Color::from_rgba8(rgba.r, rgba.g, rgba.b, a)
}

/// Pinta header (nombre + resumen) + body con el mapa proyectado.
pub fn map_viewer_view<Msg, F>(
    state: &MapPreview,
    path: Option<&Path>,
    palette: &MapViewerPalette,
    view: &MapView,
    on_pick: F,
) -> View<Msg>
where
    Msg: Clone + 'static,
    F: Fn(f32, f32, f32, f32) -> Option<Msg> + Send + Sync + 'static,
{
    let name = path
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().to_string());

    let header_text = match (name.as_deref(), state) {
        (Some(n), MapPreview::Map { data, truncated }) => {
            let bb = data.bbox();
            let bbox_txt = bb
                .map(|b| {
                    format!(
                        " · [{:.3},{:.3} → {:.3},{:.3}]",
                        b.min_lon, b.min_lat, b.max_lon, b.max_lat
                    )
                })
                .unwrap_or_default();
            format!(
                "mapa · {n} · {} pts · {} líneas · {} polígonos{}{}",
                data.points.len(),
                data.lines.len(),
                data.polygons.len(),
                bbox_txt,
                if *truncated { " · (truncado)" } else { "" },
            )
        }
        (Some(n), _) => format!("mapa · {n}"),
        (None, _) => "(seleccioná un .geojson)".to_string(),
    };
    // En modo búsqueda/ruteo, el header refleja el estado.
    let header_text = if view.searching {
        format!("buscar: {}▏", view.query)
    } else if view.routing {
        let dist = if view.route_meters > 0.0 {
            format!(" · {}", fmt_distance(view.route_meters / 1000.0))
        } else {
            String::new()
        };
        format!("ruta · {}/2 puntos{} · (clic origen y destino · r sale)", view.route_pins.len(), dist)
    } else {
        header_text
    };
    let header_color = if view.searching || view.routing {
        palette.fg_text
    } else {
        palette.fg_muted
    };

    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        padding: pad(12.0, 0.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(header_text, 10.0, header_color, Alignment::Start);

    let body = match state {
        MapPreview::Empty => simple_body("—", palette.fg_muted),
        MapPreview::NoGeometry => {
            simple_body("(JSON sin geometrías GeoJSON)", palette.fg_muted)
        }
        MapPreview::TooBig(n) => {
            simple_body(&format!("(archivo muy grande: {n} bytes — sin preview)"), palette.fg_muted)
        }
        MapPreview::Error(e) => simple_body(&format!("(no se pudo leer: {e})"), palette.fg_error),
        // El clic sobre el lienzo se reporta como fracción del rect (el host
        // la resuelve con `hit_test`); el resto de las variantes lo ignora.
        MapPreview::Map { data, .. } => {
            map_canvas(data.clone(), *palette, view.clone()).on_click_at(on_pick)
        }
    };

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(6.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg)
    .clip(true)
    .children(vec![header, body])
}

/// Proyección equirectangular fit-to-bounds + cámara (zoom/pan). Encapsula la
/// matemática para que el render (canvas) y el hit-test (clic) coincidan
/// exactamente — si difirieran, el clic seleccionaría la feature equivocada.
struct Projection {
    kx: f64,
    scale: f64,
    ox: f64,
    oy: f64,
    pmin_x: f64,
    max_lat: f64,
    pivot_x: f64,
    pivot_y: f64,
    zoom: f64,
    pan: (f64, f64),
}

impl Projection {
    /// Encaja `bb` en `rect` (`x, y, w, h`, físicos) con escala uniforme y la
    /// cámara dada.
    fn fit(bb: BBox, rect: (f64, f64, f64, f64), zoom: f64, pan: (f64, f64)) -> Self {
        let (rx, ry, rw, rh) = rect;
        let lat0 = (bb.min_lat + bb.max_lat) * 0.5;
        let kx = lat0.to_radians().cos().abs().max(0.05);
        let pmin_x = bb.min_lon * kx;
        let pw = (bb.max_lon * kx - pmin_x).max(0.0);
        let ph = (bb.max_lat - bb.min_lat).max(0.0);
        let inset = 6.0_f64;
        let aw = (rw - 2.0 * inset).max(1.0);
        let ah = (rh - 2.0 * inset).max(1.0);
        let sx = if pw > 1e-12 { aw / pw } else { f64::INFINITY };
        let sy = if ph > 1e-12 { ah / ph } else { f64::INFINITY };
        let scale = sx.min(sy).min(1.0e6);
        let scale = if scale.is_finite() { scale } else { 1.0 };
        Projection {
            kx,
            scale,
            ox: rx + inset + (aw - pw * scale) * 0.5,
            oy: ry + inset + (ah - ph * scale) * 0.5,
            pmin_x,
            max_lat: bb.max_lat,
            pivot_x: rx + rw * 0.5,
            pivot_y: ry + rh * 0.5,
            zoom,
            pan,
        }
    }

    /// lon/lat → coordenadas de pantalla **antes** de la cámara (fit puro).
    /// Independiente de zoom/pan, base para centrar/encuadrar.
    fn base(&self, [lon, lat]: Coord) -> (f64, f64) {
        (
            self.ox + (lon * self.kx - self.pmin_x) * self.scale,
            self.oy + (self.max_lat - lat) * self.scale,
        )
    }

    /// lon/lat → pantalla (Y invertida), pasando por la cámara.
    fn to_screen(&self, c: Coord) -> (f64, f64) {
        let (bx, by) = self.base(c);
        (
            self.pivot_x + (bx - self.pivot_x) * self.zoom + self.pan.0,
            self.pivot_y + (by - self.pivot_y) * self.zoom + self.pan.1,
        )
    }

    /// pantalla → lon/lat (inverso exacto de [`to_screen`]).
    fn inverse(&self, sx: f64, sy: f64) -> Coord {
        let bx = self.pivot_x + (sx - self.pivot_x - self.pan.0) / self.zoom;
        let by = self.pivot_y + (sy - self.pivot_y - self.pan.1) / self.zoom;
        let lon = ((bx - self.ox) / self.scale + self.pmin_x) / self.kx;
        let lat = self.max_lat - (by - self.oy) / self.scale;
        [lon, lat]
    }
}

/// Resuelve qué feature cae bajo un clic. `(fx, fy)` es la posición del clic
/// como fracción `[0, 1]` del rect del canvas (DPI-independiente). Devuelve el
/// índice en `data.features`, o `None` si el clic no toca ninguna geometría.
///
/// Prioridad: puntos > líneas > polígonos (lo más específico primero). Todo
/// en espacio de pantalla con la misma [`Projection`] que el render, así el
/// hit coincide con lo que se ve.
pub fn hit_test(data: &MapData, view: &MapView, fx: f64, fy: f64) -> Option<usize> {
    let (rx, ry, rw, rh) = view.rect.lock().ok().and_then(|g| *g)?;
    if rw <= 0.0 || rh <= 0.0 {
        return None;
    }
    let bb = data.bbox()?;
    let proj = Projection::fit(bb, (rx as f64, ry as f64, rw as f64, rh as f64), view.zoom, view.pan);
    let cx = rx as f64 + fx * rw as f64;
    let cy = ry as f64 + fy * rh as f64;
    let tol = 7.0_f64;

    for (i, p) in data.points.iter().enumerate() {
        let (sx, sy) = proj.to_screen(*p);
        if (sx - cx).hypot(sy - cy) <= tol + 3.0 {
            return data.point_feat.get(i).copied();
        }
    }
    for (li, line) in data.lines.iter().enumerate() {
        for w in line.windows(2) {
            if dist_point_seg(cx, cy, proj.to_screen(w[0]), proj.to_screen(w[1])) <= tol {
                return data.line_feat.get(li).copied();
            }
        }
    }
    for (pi, poly) in data.polygons.iter().enumerate() {
        if let Some(outer) = poly.first() {
            if point_in_ring_screen(cx, cy, outer, &proj) {
                return data.polygon_feat.get(pi).copied();
            }
        }
    }
    None
}

/// Busca features cuyo nombre o propiedades casen con `query` (sin distinción
/// de mayúsculas). Ranking: igualdad > prefijo > substring; el nombre pesa
/// sobre las propiedades. Devuelve hasta `limit` índices de `data.features`.
///
/// Geocodificación local y soberana: no consulta ningún servicio externo —
/// busca dentro de lo que ya cargaste. Para buscar direcciones de medio mundo
/// alcanza con cargar un dataset (un archivo), no una API.
pub fn search(data: &MapData, query: &str, limit: usize) -> Vec<usize> {
    let q = fold(&query.trim().to_lowercase());
    if q.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(u8, usize)> = Vec::new();
    for (fi, f) in data.features.iter().enumerate() {
        // El nombre cuenta doble (peso 2×); las propiedades, simple.
        let mut best = f.name.as_deref().map(|n| match_score(n, &q) * 2).unwrap_or(0);
        for (_, v) in &f.props {
            best = best.max(match_score(v, &q));
        }
        if best > 0 {
            scored.push((best, fi));
        }
    }
    // Mayor puntaje primero; a igual puntaje, orden estable por índice.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().take(limit).map(|(_, fi)| fi).collect()
}

/// Puntaje de coincidencia de `q` (ya en minúsculas y sin acentos) en `s`:
/// 3 igual, 2 prefijo, 1 substring, 0 nada. Plega acentos de `s` para que
/// "peru" encuentre "Perú".
fn match_score(s: &str, q: &str) -> u8 {
    let l = fold(&s.to_lowercase());
    if l == q {
        3
    } else if l.starts_with(q) {
        2
    } else if l.contains(q) {
        1
    } else {
        0
    }
}

/// Plega acentos latinos comunes (es/pt) a su vocal base, para búsqueda
/// tolerante a tildes. No es Unicode-completo, sólo lo usual en topónimos.
fn fold(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' | 'ã' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | 'õ' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            other => other,
        })
        .collect()
}

/// Caja envolvente de las geometrías de una feature (por su índice).
fn feature_bbox(data: &MapData, fi: usize) -> Option<BBox> {
    let mut bb = BBox::empty();
    for (i, p) in data.points.iter().enumerate() {
        if data.point_feat.get(i) == Some(&fi) {
            bb.expand(*p);
        }
    }
    for (i, l) in data.lines.iter().enumerate() {
        if data.line_feat.get(i) == Some(&fi) {
            for c in l {
                bb.expand(*c);
            }
        }
    }
    for (i, poly) in data.polygons.iter().enumerate() {
        if data.polygon_feat.get(i) == Some(&fi) {
            for ring in poly {
                for c in ring {
                    bb.expand(*c);
                }
            }
        }
    }
    (!bb.is_empty()).then_some(bb)
}

/// Centra y encuadra la cámara sobre una feature (vuelo a resultado de
/// búsqueda), y la deja seleccionada. La feature ocupa ~60% del panel; un
/// punto suelto usa un zoom fijo cómodo. No-op si no hay rect/datos.
pub fn focus_on(data: &MapData, view: &mut MapView, fi: usize) {
    let Some((rx, ry, rw, rh)) = view.rect.lock().ok().and_then(|g| *g) else {
        view.selected = Some(fi);
        return;
    };
    let (Some(bb), Some(fbb)) = (data.bbox(), feature_bbox(data, fi)) else {
        view.selected = Some(fi);
        return;
    };
    let proj = Projection::fit(bb, (rx as f64, ry as f64, rw as f64, rh as f64), 1.0, (0.0, 0.0));
    let (x0, y0) = proj.base([fbb.min_lon, fbb.max_lat]);
    let (x1, y1) = proj.base([fbb.max_lon, fbb.min_lat]);
    let fw = (x1 - x0).abs();
    let fh = (y1 - y0).abs();
    let degenerate = fw < 1e-6 && fh < 1e-6;
    let zoom = if degenerate {
        8.0
    } else {
        (0.6 * (rw as f64 / fw.max(1e-6)).min(rh as f64 / fh.max(1e-6)))
            .clamp(MapView::ZOOM_MIN, MapView::ZOOM_MAX)
    };
    let target = [(fbb.min_lon + fbb.max_lon) * 0.5, (fbb.min_lat + fbb.max_lat) * 0.5];
    let (bx, by) = proj.base(target);
    view.zoom = zoom;
    // pan que lleva el centro de la feature al centro del panel.
    view.pan = (
        -(bx - proj.pivot_x) * zoom,
        -(by - proj.pivot_y) * zoom,
    );
    view.selected = Some(fi);
}

/// Convierte un clic (fracción `[0,1]` del rect) a lon/lat, invirtiendo la
/// proyección actual. `None` si todavía no se pintó o no hay datos.
pub fn unproject(data: &MapData, view: &MapView, fx: f64, fy: f64) -> Option<Coord> {
    let (rx, ry, rw, rh) = view.rect.lock().ok().and_then(|g| *g)?;
    if rw <= 0.0 || rh <= 0.0 {
        return None;
    }
    let bb = data.bbox()?;
    let proj = Projection::fit(bb, (rx as f64, ry as f64, rw as f64, rh as f64), view.zoom, view.pan);
    Some(proj.inverse(rx as f64 + fx * rw as f64, ry as f64 + fy * rh as f64))
}

/// Distancia geodésica entre dos coordenadas (haversine), en metros.
fn haversine(a: Coord, b: Coord) -> f64 {
    const R: f64 = 6_371_000.0;
    let (lat1, lat2) = (a[1].to_radians(), b[1].to_radians());
    let dlat = (b[1] - a[1]).to_radians();
    let dlon = (b[0] - a[0]).to_radians();
    let h = (dlat * 0.5).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon * 0.5).sin().powi(2);
    2.0 * R * h.sqrt().clamp(-1.0, 1.0).asin()
}

/// Resultado de un ruteo: la polilínea seguida y su longitud en metros.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteResult {
    pub path: Vec<Coord>,
    pub meters: f64,
}

/// Calcula la ruta más corta entre `from` y `to` sobre la red de líneas
/// (`data.lines`), con A\* y heurística haversine. Soberano y offline: es
/// matemática de grafos sobre el dato cargado, sin OSRM ni servicio externo.
/// `None` si no hay red o los extremos quedan desconectados.
///
/// Los vértices se funden por proximidad (cuantización a ~0,1 m), así las
/// líneas que comparten un cruce quedan conectadas en el grafo.
pub fn route(data: &MapData, from: Coord, to: Coord) -> Option<RouteResult> {
    if data.lines.is_empty() {
        return None;
    }
    // Grafo no dirigido: nodos = vértices fundidos; aristas = tramos.
    let mut ids: HashMap<(i64, i64), usize> = HashMap::new();
    let mut coords: Vec<Coord> = Vec::new();
    let mut adj: Vec<Vec<(usize, f64)>> = Vec::new();
    for line in &data.lines {
        for w in line.windows(2) {
            let a = intern_node(w[0], &mut ids, &mut coords, &mut adj);
            let b = intern_node(w[1], &mut ids, &mut coords, &mut adj);
            if a == b {
                continue;
            }
            let d = haversine(w[0], w[1]);
            adj[a].push((b, d));
            adj[b].push((a, d));
        }
    }
    let src = nearest_node(&coords, from)?;
    let dst = nearest_node(&coords, to)?;

    // A* con heurística admisible (línea recta haversine al destino).
    let n = coords.len();
    let mut g = vec![f64::INFINITY; n];
    let mut came = vec![usize::MAX; n];
    g[src] = 0.0;
    let mut heap = BinaryHeap::new();
    heap.push(AStarNode { f: haversine(coords[src], coords[dst]), node: src });
    while let Some(AStarNode { node, .. }) = heap.pop() {
        if node == dst {
            break;
        }
        for &(nb, w) in &adj[node] {
            let tentative = g[node] + w;
            if tentative < g[nb] {
                g[nb] = tentative;
                came[nb] = node;
                heap.push(AStarNode { f: tentative + haversine(coords[nb], coords[dst]), node: nb });
            }
        }
    }
    if g[dst].is_infinite() {
        return None;
    }
    // Reconstruir el camino de destino a origen y darlo vuelta.
    let mut path = Vec::new();
    let mut cur = dst;
    while cur != usize::MAX {
        path.push(coords[cur]);
        if cur == src {
            break;
        }
        cur = came[cur];
    }
    path.reverse();
    Some(RouteResult { path, meters: g[dst] })
}

/// Inserta (o reusa) el nodo del grafo para una coordenada, fundiendo por
/// cuantización a ~1e-6° (~0,1 m) para unir cruces compartidos.
fn intern_node(
    c: Coord,
    ids: &mut HashMap<(i64, i64), usize>,
    coords: &mut Vec<Coord>,
    adj: &mut Vec<Vec<(usize, f64)>>,
) -> usize {
    let k = ((c[0] * 1e6).round() as i64, (c[1] * 1e6).round() as i64);
    if let Some(&i) = ids.get(&k) {
        return i;
    }
    let i = coords.len();
    ids.insert(k, i);
    coords.push(c);
    adj.push(Vec::new());
    i
}

/// Nodo del grafo más cercano a una coordenada (snap del clic a la red).
fn nearest_node(coords: &[Coord], c: Coord) -> Option<usize> {
    coords
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| haversine(**a, c).total_cmp(&haversine(**b, c)))
        .map(|(i, _)| i)
}

/// Entrada de la cola de prioridad de A\*: min-heap por `f` (total order vía
/// `total_cmp`, invertido para que el menor quede en la cima).
struct AStarNode {
    f: f64,
    node: usize,
}
impl PartialEq for AStarNode {
    fn eq(&self, o: &Self) -> bool {
        self.f == o.f
    }
}
impl Eq for AStarNode {}
impl Ord for AStarNode {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        o.f.total_cmp(&self.f)
    }
}
impl PartialOrd for AStarNode {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

/// Distancia de un punto `(px, py)` al segmento `a–b`, en pantalla.
fn dist_point_seg(px: f64, py: f64, a: (f64, f64), b: (f64, f64)) -> f64 {
    let (ax, ay) = a;
    let (bx, by) = b;
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    if len2 <= 1e-12 {
        return (px - ax).hypot(py - ay);
    }
    let t = (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0);
    let (qx, qy) = (ax + t * dx, ay + t * dy);
    (px - qx).hypot(py - qy)
}

/// Test punto-en-anillo (even-odd / ray casting) en espacio de pantalla.
fn point_in_ring_screen(px: f64, py: f64, ring: &[Coord], proj: &Projection) -> bool {
    let n = ring.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = proj.to_screen(ring[i]);
        let (xj, yj) = proj.to_screen(ring[j]);
        if (yi > py) != (yj > py) {
            let x_cross = (xj - xi) * (py - yi) / (yj - yi) + xi;
            if px < x_cross {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// Lienzo que proyecta y dibuja las geometrías encajadas en el panel,
/// aplicando la cámara (`zoom`/`pan`) y registrando su rect para el host.
fn map_canvas<Msg>(data: MapData, palette: MapViewerPalette, view: MapView) -> View<Msg>
where
    Msg: Clone + 'static,
{
    let zoom = view.zoom;
    let pan = view.pan;
    let show_base = view.show_base;
    let selected = view.selected;
    let color_field = view.color_field.clone();
    let route_pins = view.route_pins.clone();
    let route_path = view.route_path.clone();
    View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: pad(8.0, 6.0),
        ..Default::default()
    })
    .paint_with(move |scene, ts, rect| {
        // Registrar el rect físico para que el host acote el zoom-por-rueda.
        view.record_rect((rect.x, rect.y, rect.w, rect.h));
        let Some(bb) = data.bbox() else { return };
        if rect.w <= 8.0 || rect.h <= 8.0 {
            return;
        }

        // Proyección equirectangular (corrección por cos(lat)) + cámara
        // (zoom/pan), encapsulada para compartir la matemática exacta con el
        // hit-test del clic.
        let proj = Projection::fit(
            bb,
            (rect.x as f64, rect.y as f64, rect.w as f64, rect.h as f64),
            zoom,
            pan,
        );
        let to_screen = |c: Coord| proj.to_screen(c);
        let (pivot_x, pivot_y) = (proj.pivot_x, proj.pivot_y);
        let scale = proj.scale;

        let stroke_thin = Stroke::new(1.2);
        let stroke_edge = Stroke::new(1.0);
        let stroke_grid = Stroke::new(0.75);
        let fill_col = with_alpha(palette.fill, 0.18);
        let grid_col = with_alpha(palette.grid, 0.22);
        let grid_label_col = with_alpha(palette.label, 0.55);

        let in_panel = |x: f64, y: f64| {
            x >= rect.x as f64
                && x <= (rect.x + rect.w) as f64
                && y >= rect.y as f64
                && y <= (rect.y + rect.h) as f64
        };

        // --- Mapa-base mundial (detrás de todo) ----------------------
        // Países Natural Earth, proyectados con la misma cámara que el dato:
        // al hacer zoom a una región, sólo se ve su parte (el resto, clipeado).
        if show_base {
            let world = world_base();
            let land_fill = with_alpha(palette.land, 0.10);
            let land_stroke = with_alpha(palette.land, 0.32);
            let land_label = with_alpha(palette.land, 0.5);
            let stroke_coast = Stroke::new(0.6);
            for poly in &world.polygons {
                for (i, ring) in poly.iter().enumerate() {
                    let path = ring_path(ring, &to_screen, true);
                    if i == 0 {
                        scene.fill(Fill::NonZero, Affine::IDENTITY, land_fill, None, &path);
                    }
                    scene.stroke(&stroke_coast, Affine::IDENTITY, land_stroke, None, &path);
                }
            }
            // Nombres de país, sólo los que caen dentro del panel (el clip
            // recorta el resto, así que en una vista regional son pocos).
            for label in &world.labels {
                let (x, y) = to_screen(label.at);
                if in_panel(x, y) {
                    let block = TextBlock::simple(&label.text, 9.0, land_label, (x + 2.0, y - 6.0));
                    draw_block(scene, ts, &block);
                }
            }
        }

        // --- Rejilla de coordenadas (detrás del dato) ----------------
        // Líneas de lon/lat a un paso "redondo" con rótulo en grados, para
        // dar contexto geográfico aunque no haya mapa-base.
        let x0 = rect.x as f64;
        let y0 = rect.y as f64;
        let x1 = x0 + rect.w as f64;
        let y1 = y0 + rect.h as f64;
        let lon_step = nice_step(bb.max_lon - bb.min_lon);
        let lat_step = nice_step(bb.max_lat - bb.min_lat);
        for lon in ticks(bb.min_lon, bb.max_lon, lon_step) {
            let (gx, _) = to_screen([lon, bb.max_lat]);
            let mut path = BezPath::new();
            path.move_to((gx, y0));
            path.line_to((gx, y1));
            scene.stroke(&stroke_grid, Affine::IDENTITY, grid_col, None, &path);
            let txt = fmt_deg(lon, lon_step);
            let block = TextBlock::simple(&txt, 9.0, grid_label_col, (gx + 2.0, y1 - 12.0));
            draw_block(scene, ts, &block);
        }
        for lat in ticks(bb.min_lat, bb.max_lat, lat_step) {
            let (_, gy) = to_screen([bb.min_lon, lat]);
            let mut path = BezPath::new();
            path.move_to((x0, gy));
            path.line_to((x1, gy));
            scene.stroke(&stroke_grid, Affine::IDENTITY, grid_col, None, &path);
            let txt = fmt_deg(lat, lat_step);
            let block = TextBlock::simple(&txt, 9.0, grid_label_col, (x0 + 2.0, gy + 1.0));
            draw_block(scene, ts, &block);
        }

        // Choropleth: si hay campo de color activo, rango [min,max] del valor
        // a través de las features (para mapear cada polígono a un color).
        let choro = color_field.as_deref().and_then(|field| {
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            for f in &data.features {
                if let Some(v) = f.number(field) {
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
            }
            (hi > lo).then_some((field, lo, hi))
        });

        // Polígonos: relleno (choropleth o translúcido uniforme) + borde.
        for (pi, poly) in data.polygons.iter().enumerate() {
            // Color de relleno del polígono según el choropleth, si aplica.
            let fill = choro
                .and_then(|(field, lo, hi)| {
                    let fi = *data.polygon_feat.get(pi)?;
                    let v = data.features.get(fi)?.number(field)?;
                    Some(with_alpha(scale_color((v - lo) / (hi - lo)), 0.62))
                })
                .unwrap_or(fill_col);
            for (i, ring) in poly.iter().enumerate() {
                let path = ring_path(ring, &to_screen, true);
                if i == 0 {
                    scene.fill(Fill::NonZero, Affine::IDENTITY, fill, None, &path);
                }
                scene.stroke(&stroke_edge, Affine::IDENTITY, palette.stroke, None, &path);
            }
        }

        // Líneas.
        for line in &data.lines {
            let path = ring_path(line, &to_screen, false);
            scene.stroke(&stroke_thin, Affine::IDENTITY, palette.stroke, None, &path);
        }

        // Puntos: disco pequeño. Un radio levemente mayor si es el único
        // contenido (mapa de un solo punto), para que se vea.
        let r = if data.total_features() == 1 { 4.0 } else { 2.5 };
        for p in &data.points {
            let (x, y) = to_screen(*p);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palette.point,
                None,
                &Circle::new((x, y), r),
            );
        }

        // --- Etiquetas (encima de todo) ------------------------------
        for label in &data.labels {
            let (x, y) = to_screen(label.at);
            // Desplazada arriba-derecha del ancla para no taparla.
            let block = TextBlock::simple(&label.text, 11.0, palette.label, (x + 5.0, y - 14.0));
            draw_block(scene, ts, &block);
        }

        // --- Feature seleccionada (clic): resalte ---------------------
        if let Some(fi) = selected {
            let hl = Color::from_rgba8(255, 196, 64, 255); // ámbar, pop sobre cualquier tema
            let hl_stroke = Stroke::new(2.6);
            for (i, poly) in data.polygons.iter().enumerate() {
                if data.polygon_feat.get(i) == Some(&fi) {
                    for ring in poly {
                        let path = ring_path(ring, &to_screen, true);
                        scene.stroke(&hl_stroke, Affine::IDENTITY, hl, None, &path);
                    }
                }
            }
            for (i, line) in data.lines.iter().enumerate() {
                if data.line_feat.get(i) == Some(&fi) {
                    let path = ring_path(line, &to_screen, false);
                    scene.stroke(&hl_stroke, Affine::IDENTITY, hl, None, &path);
                }
            }
            for (i, p) in data.points.iter().enumerate() {
                if data.point_feat.get(i) == Some(&fi) {
                    let (x, y) = to_screen(*p);
                    scene.fill(Fill::NonZero, Affine::IDENTITY, hl, None, &Circle::new((x, y), 5.0));
                }
            }
        }

        // --- Ruta calculada + pines de origen/destino ----------------
        if !route_path.is_empty() {
            let route_col = Color::from_rgba8(64, 220, 140, 255); // verde ruta
            let path = ring_path(&route_path, &to_screen, false);
            scene.stroke(&Stroke::new(3.2), Affine::IDENTITY, route_col, None, &path);
        }
        for (i, pin) in route_pins.iter().enumerate() {
            let (x, y) = to_screen(*pin);
            // Origen verde, destino rojo.
            let col = if i == 0 {
                Color::from_rgba8(64, 220, 140, 255)
            } else {
                Color::from_rgba8(235, 90, 70, 255)
            };
            scene.fill(Fill::NonZero, Affine::IDENTITY, col, None, &Circle::new((x, y), 5.5));
            scene.stroke(
                &Stroke::new(1.4),
                Affine::IDENTITY,
                Color::from_rgba8(255, 255, 255, 230),
                None,
                &Circle::new((x, y), 5.5),
            );
        }

        // --- Mobiliario cartográfico (fijo a pantalla) ---------------
        let furn = with_alpha(palette.label, 0.7);
        let furn_line = Stroke::new(1.4);
        let rx = rect.x as f64;
        let ry = rect.y as f64;
        let rw = rect.w as f64;
        let rh = rect.h as f64;

        // Lectura del centro de la vista + zoom (arriba-izquierda):
        // invierte la proyección en el centro del panel.
        let [lon_c, lat_c] = proj.inverse(pivot_x, pivot_y);
        let read = format!("{}  {}   {:.1}×", fmt_lat(lat_c), fmt_lon(lon_c), zoom);
        draw_block(scene, ts, &TextBlock::simple(&read, 9.5, furn, (rx + 12.0, ry + 6.0)));

        // Flecha de norte (arriba-derecha): el norte siempre es arriba.
        let nx = rx + rw - 18.0;
        let ny = ry + 12.0;
        let mut arrow = BezPath::new();
        arrow.move_to((nx, ny + 15.0));
        arrow.line_to((nx, ny));
        arrow.move_to((nx - 4.0, ny + 5.0));
        arrow.line_to((nx, ny));
        arrow.line_to((nx + 4.0, ny + 5.0));
        scene.stroke(&furn_line, Affine::IDENTITY, furn, None, &arrow);
        draw_block(scene, ts, &TextBlock::simple("N", 9.0, furn, (nx - 3.5, ny + 15.0)));

        // Barra de escala (abajo-izquierda): un segmento de distancia
        // redonda, calculado de la proyección a la latitud de la vista.
        // En equirectangular el grado de latitud mide ~constante.
        let km_per_px = 110.574 / (scale * zoom).max(1e-9);
        let nice_km = nice_125(km_per_px * 110.0);
        let bar_px = (nice_km / km_per_px).clamp(20.0, rw * 0.45);
        let bx = rx + 14.0;
        let by = ry + rh - 22.0;
        let mut bar = BezPath::new();
        bar.move_to((bx, by - 5.0));
        bar.line_to((bx, by));
        bar.line_to((bx + bar_px, by));
        bar.line_to((bx + bar_px, by - 5.0));
        scene.stroke(&furn_line, Affine::IDENTITY, furn, None, &bar);
        draw_block(
            scene,
            ts,
            &TextBlock::simple(&fmt_distance(nice_km), 9.0, furn, (bx, by - 17.0)),
        );

        // --- Leyenda del choropleth (abajo-derecha) ------------------
        if let Some((field, lo, hi)) = choro {
            draw_legend(scene, ts, (rx, ry, rw, rh), furn, field, lo, hi);
        }

        // --- Panel de propiedades de la feature seleccionada ---------
        if let Some(fp) = selected.and_then(|fi| data.features.get(fi)) {
            draw_props_panel(scene, ts, (rx, ry, rw, rh), &palette, fp);
        }
    })
}

/// Dibuja la leyenda del choropleth (abajo-derecha): nombre del campo, barra
/// de gradiente azul→ámbar→rojo y el rango `lo – hi`.
fn draw_legend(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    ts: &mut llimphi_ui::llimphi_text::Typesetter,
    rect: (f64, f64, f64, f64),
    color: Color,
    field: &str,
    lo: f64,
    hi: f64,
) {
    use llimphi_ui::llimphi_raster::kurbo::Rect as KRect;
    let (rx, ry, rw, rh) = rect;
    let lw = 130.0_f64.min(rw - 24.0);
    if lw < 60.0 {
        return;
    }
    let lx = rx + rw - lw - 12.0;
    let ly = ry + rh - 34.0;
    let segs = 24;
    let seg_w = lw / segs as f64;
    for s in 0..segs {
        let t = (s as f64 + 0.5) / segs as f64;
        let x0 = lx + s as f64 * seg_w;
        let bar = KRect::new(x0, ly, x0 + seg_w + 0.6, ly + 8.0);
        scene.fill(Fill::NonZero, Affine::IDENTITY, scale_color(t), None, &bar);
    }
    scene.stroke(
        &Stroke::new(0.8),
        Affine::IDENTITY,
        color,
        None,
        &KRect::new(lx, ly, lx + lw, ly + 8.0),
    );
    draw_block(scene, ts, &TextBlock::simple(&clip_text(field, 26), 9.0, color, (lx, ly - 12.0)));
    let range = format!("{} – {}", fmt_num(lo), fmt_num(hi));
    draw_block(scene, ts, &TextBlock::simple(&range, 8.5, color, (lx, ly + 9.0)));
}

/// Formatea un número: entero si es exacto, dos decimales si no.
fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v:.2}")
    }
}

/// Dibuja un panel con las propiedades de la feature seleccionada en el
/// borde derecho del lienzo. Cabecera con el nombre + hasta [`PANEL_ROWS`]
/// pares clave→valor.
fn draw_props_panel(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    ts: &mut llimphi_ui::llimphi_text::Typesetter,
    rect: (f64, f64, f64, f64),
    palette: &MapViewerPalette,
    fp: &FeatureProps,
) {
    use llimphi_ui::llimphi_raster::kurbo::RoundedRect;

    const PANEL_ROWS: usize = 12;
    let (rx, ry, rw, rh) = rect;
    let pw = 220.0_f64.min(rw - 16.0);
    if pw < 80.0 {
        return;
    }
    let rows = fp.props.len().min(PANEL_ROWS);
    let header = fp.name.clone().unwrap_or_else(|| "(feature)".to_string());
    let ph = 14.0 + 16.0 + rows as f64 * 13.0 + 8.0;
    let px = rx + rw - pw - 8.0;
    let py = (ry + 30.0).min(ry + rh - ph - 8.0).max(ry + 8.0);

    let bg = with_alpha(palette.bg, 0.92);
    let border = with_alpha(palette.grid, 0.5);
    let panel = RoundedRect::new(px, py, px + pw, py + ph, 5.0);
    scene.fill(Fill::NonZero, Affine::IDENTITY, bg, None, &panel);
    scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, border, None, &panel);

    let pad = 8.0;
    draw_block(
        scene,
        ts,
        &TextBlock::simple(&clip_text(&header, 30), 11.5, palette.label, (px + pad, py + 6.0)),
    );
    let key_col = with_alpha(palette.fg_muted, 0.95);
    for (i, (k, v)) in fp.props.iter().take(PANEL_ROWS).enumerate() {
        let y = py + 24.0 + i as f64 * 13.0;
        let line = format!("{}: {}", clip_text(k, 16), clip_text(v, 22));
        draw_block(scene, ts, &TextBlock::simple(&line, 9.5, key_col, (px + pad, y)));
    }
    if fp.props.len() > PANEL_ROWS {
        let y = py + 24.0 + PANEL_ROWS as f64 * 13.0;
        let more = format!("… +{} más", fp.props.len() - PANEL_ROWS);
        draw_block(scene, ts, &TextBlock::simple(&more, 9.0, key_col, (px + pad, y)));
    }
}

/// Recorta un texto a `max` caracteres con elipsis.
fn clip_text(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// Paso "redondo" (1·2·5 × 10ⁿ) para una rejilla que cubra `span` con unas
/// ~4–8 divisiones. Devuelve un paso positivo aun para spans degenerados.
fn nice_step(span: f64) -> f64 {
    let span = span.abs();
    if span <= 1e-9 {
        return 1.0;
    }
    let target = span / 6.0;
    let mag = 10f64.powf(target.log10().floor());
    let norm = target / mag; // 1..10
    let step = if norm < 1.5 {
        1.0
    } else if norm < 3.5 {
        2.0
    } else if norm < 7.5 {
        5.0
    } else {
        10.0
    };
    step * mag
}

/// Redondea a un valor "lindo" (1·2·5·10 × 10ⁿ) cercano a `x`, para la barra
/// de escala. Siempre positivo.
fn nice_125(x: f64) -> f64 {
    if !(x > 0.0) {
        return 1.0;
    }
    let mag = 10f64.powf(x.log10().floor());
    let n = x / mag;
    let pick = if n < 1.5 {
        1.0
    } else if n < 3.0 {
        2.0
    } else if n < 7.0 {
        5.0
    } else {
        10.0
    };
    pick * mag
}

/// Formatea una distancia: km (entero o un decimal) o metros si < 1 km.
fn fmt_distance(km: f64) -> String {
    if km >= 1.0 {
        if (km - km.round()).abs() < 1e-9 {
            format!("{} km", km as i64)
        } else {
            format!("{km:.1} km")
        }
    } else {
        format!("{} m", (km * 1000.0).round() as i64)
    }
}

/// Latitud con hemisferio (`N`/`S`).
fn fmt_lat(lat: f64) -> String {
    let h = if lat >= 0.0 { 'N' } else { 'S' };
    format!("{:.2}°{h}", lat.abs())
}

/// Longitud con hemisferio (`E`/`O`).
fn fmt_lon(lon: f64) -> String {
    let h = if lon >= 0.0 { 'E' } else { 'O' };
    format!("{:.2}°{h}", lon.abs())
}

/// Múltiplos de `step` dentro de `[lo, hi]` (incluidos), redondeando el
/// primero hacia arriba. Capada por seguridad para no iterar de más.
fn ticks(lo: f64, hi: f64, step: f64) -> Vec<f64> {
    let mut out = Vec::new();
    if step <= 0.0 || !lo.is_finite() || !hi.is_finite() {
        return out;
    }
    let first = (lo / step).ceil() * step;
    let mut v = first;
    while v <= hi + step * 1e-6 && out.len() < 64 {
        out.push(v);
        v += step;
    }
    out
}

/// Formatea un grado con la cantidad de decimales que el paso amerita
/// (pasos chicos → más decimales), con sufijo `°`.
fn fmt_deg(value: f64, step: f64) -> String {
    let decimals = if step >= 1.0 {
        0
    } else {
        // -log10(step), acotado a [1, 4].
        (-step.log10().floor() as i32).clamp(1, 4) as usize
    };
    format!("{value:.decimals$}°")
}

/// Construye un `BezPath` en coordenadas de pantalla a partir de un anillo.
/// Si `close`, cierra el contorno (para relleno/borde de polígono).
fn ring_path(ring: &[Coord], to_screen: &impl Fn(Coord) -> (f64, f64), close: bool) -> BezPath {
    let mut path = BezPath::new();
    let mut it = ring.iter();
    if let Some(first) = it.next() {
        let (x, y) = to_screen(*first);
        path.move_to((x, y));
        for c in it {
            let (x, y) = to_screen(*c);
            path.line_to((x, y));
        }
        if close {
            path.close_path();
        }
    }
    path
}

/// Body de una sola línea (estados Empty/NoGeometry/TooBig/Error).
fn simple_body<Msg>(text: &str, color: Color) -> View<Msg>
where
    Msg: Clone + 'static,
{
    View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: pad(14.0, 8.0),
        ..Default::default()
    })
    .text_aligned(text.to_string(), 12.0, color, Alignment::Start)
}

/// Padding horizontal `h` + vertical `v`.
fn pad(h: f32, v: f32) -> Rect<llimphi_ui::llimphi_layout::taffy::LengthPercentage> {
    Rect {
        left: length(h),
        right: length(h),
        top: length(v),
        bottom: length(v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data_of(src: &str) -> MapData {
        match parse_geojson(src) {
            MapPreview::Map { data, .. } => data,
            other => panic!("esperaba Map, fue {other:?}"),
        }
    }

    #[test]
    fn punto_suelto() {
        let d = data_of(r#"{"type":"Point","coordinates":[10.0,20.0]}"#);
        assert_eq!(d.points, vec![[10.0, 20.0]]);
        assert!(d.lines.is_empty() && d.polygons.is_empty());
    }

    #[test]
    fn linestring() {
        let d = data_of(r#"{"type":"LineString","coordinates":[[0,0],[1,1],[2,0]]}"#);
        assert_eq!(d.lines, vec![vec![[0.0, 0.0], [1.0, 1.0], [2.0, 0.0]]]);
    }

    #[test]
    fn polygon_con_hueco() {
        let d = data_of(
            r#"{"type":"Polygon","coordinates":[
                [[0,0],[4,0],[4,4],[0,4],[0,0]],
                [[1,1],[2,1],[2,2],[1,2],[1,1]]
            ]}"#,
        );
        assert_eq!(d.polygons.len(), 1);
        assert_eq!(d.polygons[0].len(), 2, "exterior + hueco");
    }

    #[test]
    fn feature_collection_mixta() {
        let src = r#"{
            "type":"FeatureCollection",
            "features":[
                {"type":"Feature","geometry":{"type":"Point","coordinates":[1,2]},"properties":{}},
                {"type":"Feature","geometry":{"type":"LineString","coordinates":[[0,0],[1,1]]},"properties":{}}
            ]
        }"#;
        let d = data_of(src);
        assert_eq!(d.points.len(), 1);
        assert_eq!(d.lines.len(), 1);
    }

    #[test]
    fn multipolygon_y_multipoint() {
        let d = data_of(
            r#"{"type":"GeometryCollection","geometries":[
                {"type":"MultiPoint","coordinates":[[0,0],[1,1]]},
                {"type":"MultiPolygon","coordinates":[
                    [[[0,0],[1,0],[1,1],[0,0]]],
                    [[[2,2],[3,2],[3,3],[2,2]]]
                ]}
            ]}"#,
        );
        assert_eq!(d.points.len(), 2);
        assert_eq!(d.polygons.len(), 2);
    }

    #[test]
    fn bbox_correcta() {
        let d = data_of(r#"{"type":"MultiPoint","coordinates":[[-5,1],[3,-2],[0,7]]}"#);
        let bb = d.bbox().unwrap();
        assert_eq!(bb.min_lon, -5.0);
        assert_eq!(bb.max_lon, 3.0);
        assert_eq!(bb.min_lat, -2.0);
        assert_eq!(bb.max_lat, 7.0);
    }

    #[test]
    fn linea_de_un_punto_se_descarta() {
        // Una LineString degenerada (un solo vértice) no es una línea.
        let r = parse_geojson(r#"{"type":"LineString","coordinates":[[0,0]]}"#);
        assert_eq!(r, MapPreview::NoGeometry);
    }

    #[test]
    fn json_sin_geometria_es_no_geometry() {
        assert_eq!(parse_geojson(r#"{"type":"Topology","objects":{}}"#), MapPreview::NoGeometry);
        assert_eq!(parse_geojson(r#"{"foo":"bar"}"#), MapPreview::NoGeometry);
    }

    #[test]
    fn json_invalido_es_error() {
        assert!(matches!(parse_geojson("{ no es json "), MapPreview::Error(_)));
    }

    #[test]
    fn coordenadas_no_finitas_se_filtran() {
        // Un punto con NaN/inf textual no parsea como número JSON; uno fuera
        // de rango se conserva pero finito. Verificamos que basura se cae.
        let r = parse_geojson(r#"{"type":"Point","coordinates":["x","y"]}"#);
        assert_eq!(r, MapPreview::NoGeometry);
    }

    #[test]
    fn altitud_se_ignora() {
        let d = data_of(r#"{"type":"Point","coordinates":[10,20,500]}"#);
        assert_eq!(d.points, vec![[10.0, 20.0]]);
    }

    #[test]
    fn vertice_count_y_truncado() {
        let d = data_of(r#"{"type":"LineString","coordinates":[[0,0],[1,1],[2,2]]}"#);
        assert_eq!(d.vertex_count(), 3);
    }

    #[test]
    fn etiquetas_desde_properties() {
        let src = r#"{
            "type":"FeatureCollection",
            "features":[
                {"type":"Feature","properties":{"nombre":"La Paz"},
                 "geometry":{"type":"Point","coordinates":[-68.15,-16.5]}},
                {"type":"Feature","properties":{"name":"Ruta"},
                 "geometry":{"type":"LineString","coordinates":[[0,0],[2,2],[4,4]]}},
                {"type":"Feature","properties":{},
                 "geometry":{"type":"Point","coordinates":[1,1]}}
            ]
        }"#;
        let d = data_of(src);
        // Dos features con nombre → dos etiquetas; la de properties vacías no.
        assert_eq!(d.labels.len(), 2);
        assert_eq!(d.labels[0].text, "La Paz");
        assert_eq!(d.labels[0].at, [-68.15, -16.5]);
        // La etiqueta de la línea se ancla a su vértice medio.
        assert_eq!(d.labels[1].text, "Ruta");
        assert_eq!(d.labels[1].at, [2.0, 2.0]);
    }

    #[test]
    fn etiqueta_de_poligono_en_el_centroide() {
        let src = r#"{"type":"Feature","properties":{"nombre":"cuadra"},
            "geometry":{"type":"Polygon","coordinates":[[[0,0],[2,0],[2,2],[0,2],[0,0]]]}}"#;
        let d = data_of(src);
        assert_eq!(d.labels.len(), 1);
        // Centroide del cuadrado (ignorando el vértice de cierre repetido).
        assert_eq!(d.labels[0].at, [1.0, 1.0]);
    }

    #[test]
    fn nice_step_es_redondo() {
        assert_eq!(nice_step(60.0), 10.0);
        assert_eq!(nice_step(12.0), 2.0);
        assert_eq!(nice_step(3.0), 0.5);
        assert!(nice_step(0.0) > 0.0); // degenerado no rompe
    }

    #[test]
    fn ticks_dentro_del_rango() {
        let t = ticks(-3.0, 7.0, 2.0);
        assert_eq!(t, vec![-2.0, 0.0, 2.0, 4.0, 6.0]);
        assert!(ticks(0.0, 1.0, 0.0).is_empty()); // paso 0 no itera
    }

    #[test]
    fn fmt_deg_decimales_segun_paso() {
        assert_eq!(fmt_deg(10.0, 5.0), "10°");
        assert_eq!(fmt_deg(-16.5, 0.5), "-16.5°");
        assert_eq!(fmt_deg(0.25, 0.1), "0.2°");
    }

    #[test]
    fn nice_125_redondea() {
        assert_eq!(nice_125(1.0), 1.0);
        assert_eq!(nice_125(1.7), 2.0);
        assert_eq!(nice_125(4.0), 5.0);
        assert_eq!(nice_125(800.0), 1000.0);
        assert_eq!(nice_125(0.0), 1.0); // degenerado
    }

    #[test]
    fn fmt_distancia_km_y_m() {
        assert_eq!(fmt_distance(5.0), "5 km");
        assert_eq!(fmt_distance(2.5), "2.5 km");
        assert_eq!(fmt_distance(0.5), "500 m");
    }

    #[test]
    fn fmt_coordenadas_con_hemisferio() {
        assert_eq!(fmt_lat(-16.5), "16.50°S");
        assert_eq!(fmt_lat(40.0), "40.00°N");
        assert_eq!(fmt_lon(-70.65), "70.65°O");
        assert_eq!(fmt_lon(2.35), "2.35°E");
    }

    #[test]
    fn inexistente_es_error() {
        assert!(matches!(
            load_map(Path::new("/no/existe.geojson"), DEFAULT_MAP_BYTES_MAX),
            MapPreview::Error(_)
        ));
    }

    // --- GPX ---

    fn gpx_data(src: &str) -> MapData {
        match parse_gpx(src) {
            MapPreview::Map { data, .. } => data,
            other => panic!("esperaba Map, fue {other:?}"),
        }
    }

    #[test]
    fn gpx_waypoints_y_track() {
        let src = r#"<?xml version="1.0"?>
            <gpx version="1.1">
              <wpt lat="-13.51" lon="-71.97"><name>Cusco</name></wpt>
              <trk><name>Sendero</name><trkseg>
                <trkpt lat="-13.51" lon="-71.97"/>
                <trkpt lat="-13.50" lon="-71.98"/>
                <trkpt lat="-13.49" lon="-71.98"/>
              </trkseg></trk>
            </gpx>"#;
        let d = gpx_data(src);
        assert_eq!(d.points, vec![[-71.97, -13.51]]);
        assert_eq!(d.lines.len(), 1);
        assert_eq!(d.lines[0].len(), 3);
        // Etiquetas: el waypoint y el track.
        assert!(d.labels.iter().any(|l| l.text == "Cusco"));
        assert!(d.labels.iter().any(|l| l.text == "Sendero"));
    }

    #[test]
    fn gpx_ruta_es_linea() {
        let src = r#"<gpx><rte><name>R</name>
            <rtept lat="0" lon="0"/><rtept lat="1" lon="1"/><rtept lat="2" lon="0"/>
            </rte></gpx>"#;
        let d = gpx_data(src);
        assert_eq!(d.lines, vec![vec![[0.0, 0.0], [1.0, 1.0], [0.0, 2.0]]]);
    }

    #[test]
    fn gpx_waypoint_self_closing_sin_nombre() {
        let d = gpx_data(r#"<gpx><wpt lat="5" lon="-3"/></gpx>"#);
        assert_eq!(d.points, vec![[-3.0, 5.0]]);
        assert!(d.labels.is_empty());
    }

    #[test]
    fn gpx_vacio_es_no_geometry() {
        assert_eq!(parse_gpx("<gpx></gpx>"), MapPreview::NoGeometry);
    }

    #[test]
    fn load_map_desambigua_gpx_de_geojson() {
        let dir = std::env::temp_dir();
        let gpx_path = dir.join("nahual-map-test.gpx");
        std::fs::write(&gpx_path, r#"<gpx><wpt lat="1" lon="2"/></gpx>"#).unwrap();
        let r = load_map(&gpx_path, DEFAULT_MAP_BYTES_MAX);
        let _ = std::fs::remove_file(&gpx_path);
        match r {
            MapPreview::Map { data, .. } => assert_eq!(data.points, vec![[2.0, 1.0]]),
            other => panic!("GPX debió parsear como mapa, fue {other:?}"),
        }
    }

    // --- KML ---

    fn kml_data(src: &str) -> MapData {
        match parse_kml(src) {
            MapPreview::Map { data, .. } => data,
            other => panic!("esperaba Map, fue {other:?}"),
        }
    }

    #[test]
    fn kml_point_line_polygon() {
        let src = r#"<kml><Document>
            <Placemark><name>P</name><Point><coordinates>-77.03,-12.05,0</coordinates></Point></Placemark>
            <Placemark><name>L</name><LineString><coordinates>0,0,0 1,1,0 2,0,0</coordinates></LineString></Placemark>
            <Placemark><name>Poly</name><Polygon><outerBoundaryIs><LinearRing>
              <coordinates>0,0 2,0 2,2 0,2 0,0</coordinates>
            </LinearRing></outerBoundaryIs></Polygon></Placemark>
            </Document></kml>"#;
        let d = kml_data(src);
        assert_eq!(d.points, vec![[-77.03, -12.05]]);
        assert_eq!(d.lines, vec![vec![[0.0, 0.0], [1.0, 1.0], [2.0, 0.0]]]);
        assert_eq!(d.polygons.len(), 1);
        assert_eq!(d.polygons[0][0].len(), 5);
        assert!(d.labels.iter().any(|l| l.text == "P"));
        assert!(d.labels.iter().any(|l| l.text == "Poly"));
    }

    #[test]
    fn kml_coords_ignora_altitud_y_espacios() {
        let cs = kml_coords("  -71.9,-13.5,2400   -71.8,-13.4  ");
        assert_eq!(cs, vec![[-71.9, -13.5], [-71.8, -13.4]]);
    }

    #[test]
    fn load_map_desambigua_kml() {
        let dir = std::env::temp_dir();
        let p = dir.join("nahual-map-test.kml");
        std::fs::write(
            &p,
            r#"<kml><Placemark><Point><coordinates>2,1,0</coordinates></Point></Placemark></kml>"#,
        )
        .unwrap();
        let r = load_map(&p, DEFAULT_MAP_BYTES_MAX);
        let _ = std::fs::remove_file(&p);
        match r {
            MapPreview::Map { data, .. } => assert_eq!(data.points, vec![[2.0, 1.0]]),
            other => panic!("KML debió parsear como mapa, fue {other:?}"),
        }
    }

    #[test]
    fn mapa_base_mundial_carga_completo() {
        let w = world_base();
        // 177 features (148 Polygon + 29 MultiPolygon) → al aplanar, al menos
        // tantos polígonos como features.
        assert!(w.polygons.len() >= 177, "polígonos: {}", w.polygons.len());
        // Decenas de miles de vértices, sin truncar.
        assert!(w.vertex_count() > 5_000, "vértices: {}", w.vertex_count());
        // Trae nombres de país para rotular.
        assert!(w.labels.iter().any(|l| l.text == "Costa Rica"));
    }

    // --- Cámara (MapView) ---

    #[test]
    fn zoom_se_acota() {
        let mut v = MapView::default();
        v.zoom_by(1000.0);
        assert!((v.zoom - MapView::ZOOM_MAX).abs() < 1e-9);
        v.zoom_by(1e-6);
        assert!((v.zoom - MapView::ZOOM_MIN).abs() < 1e-9);
        // factor inválido no rompe.
        v.zoom_by(f64::NAN);
        assert!(v.zoom.is_finite());
    }

    #[test]
    fn pan_acumula_y_reset_vuelve_al_origen() {
        let mut v = MapView::default();
        v.pan_by(10.0, -5.0);
        v.pan_by(2.0, 3.0);
        assert_eq!(v.pan, (12.0, -2.0));
        v.zoom_by(2.0);
        v.reset();
        assert_eq!(v.pan, (0.0, 0.0));
        assert_eq!(v.zoom, 1.0);
    }

    #[test]
    fn zoom_at_ancla_el_punto_bajo_el_cursor() {
        let mut v = MapView::default();
        v.record_rect((0.0, 0.0, 200.0, 100.0)); // pivot (100, 50)
        let (pvx, pvy) = (100.0, 50.0);
        // Posición de pantalla de un punto base, con la cámara actual.
        let screen = |v: &MapView, bx: f64, by: f64| {
            (
                pvx + (bx - pvx) * v.zoom + v.pan.0,
                pvy + (by - pvy) * v.zoom + v.pan.1,
            )
        };
        // El punto base bajo el cursor (150, 50) a zoom 1 es (150, 50).
        let (bx, by) = (150.0, 50.0);
        assert_eq!(screen(&v, bx, by), (150.0, 50.0));
        v.zoom_at(2.0, 150.0, 50.0);
        assert_eq!(v.zoom, 2.0);
        // Tras el zoom, ese mismo punto base sigue bajo el cursor.
        let (sx, sy) = screen(&v, bx, by);
        assert!((sx - 150.0).abs() < 1e-9 && (sy - 50.0).abs() < 1e-9, "({sx}, {sy})");
    }

    #[test]
    fn zoom_at_sin_rect_cae_a_zoom_al_centro() {
        let mut v = MapView::default();
        v.zoom_at(2.0, 10.0, 10.0); // sin record_rect previo
        assert_eq!(v.zoom, 2.0);
        assert_eq!(v.pan, (0.0, 0.0)); // sin anclaje: pan intacto
    }

    #[test]
    fn hit_test_selecciona_feature_bajo_el_clic() {
        let d = data_of(
            r#"{"type":"FeatureCollection","features":[
                {"type":"Feature","properties":{"name":"sq"},
                 "geometry":{"type":"Polygon","coordinates":[[[0,0],[10,0],[10,10],[0,10],[0,0]]]}},
                {"type":"Feature","properties":{"name":"pt"},
                 "geometry":{"type":"Point","coordinates":[5,5]}}
            ]}"#,
        );
        let view = MapView::default();
        view.record_rect((0.0, 0.0, 100.0, 100.0));
        // Centro del panel = (5,5) = el punto → gana el punto (feature 1).
        assert_eq!(hit_test(&d, &view, 0.5, 0.5), Some(1));
        // Dentro del polígono pero lejos del punto → polígono (feature 0).
        assert_eq!(hit_test(&d, &view, 0.2, 0.2), Some(0));
        // Esquina del panel, fuera del bbox proyectado → nada.
        assert_eq!(hit_test(&d, &view, 0.99, 0.01), None);
    }

    #[test]
    fn feature_props_retiene_propiedades() {
        let d = data_of(
            r#"{"type":"Feature","properties":{"name":"X","pop":1234,"activo":true},
                "geometry":{"type":"Point","coordinates":[0,0]}}"#,
        );
        assert_eq!(d.features.len(), 1);
        let fp = &d.features[0];
        assert_eq!(fp.name.as_deref(), Some("X"));
        assert_eq!(fp.number("pop"), Some(1234.0));
        assert!(fp.props.iter().any(|(k, v)| k == "activo" && v == "true"));
        // El punto apunta a esa feature.
        assert_eq!(d.point_feat, vec![0]);
    }

    #[test]
    fn numeric_fields_distintos_en_orden() {
        let d = data_of(
            r#"{"type":"FeatureCollection","features":[
                {"type":"Feature","properties":{"pop":10,"gdp":5,"name":"a"},"geometry":{"type":"Point","coordinates":[0,0]}},
                {"type":"Feature","properties":{"pop":20,"area":3},"geometry":{"type":"Point","coordinates":[1,1]}}
            ]}"#,
        );
        // `name` no es numérico; los demás aparecen una vez (el orden lo fija
        // serde_json, que ordena claves).
        let fields = numeric_fields(&d);
        assert_eq!(fields.len(), 3);
        for f in ["pop", "gdp", "area"] {
            assert!(fields.iter().any(|x| x == f), "falta {f} en {fields:?}");
        }
        assert!(!fields.iter().any(|x| x == "name"));
    }

    #[test]
    fn scale_color_extremos_y_medio() {
        // Azul en 0, rojo en 1, ámbar al medio (sin pánico en bordes).
        let lo = scale_color(0.0).to_rgba8();
        let hi = scale_color(1.0).to_rgba8();
        assert!(lo.b > lo.r); // azulado
        assert!(hi.r > hi.b); // rojizo
        let _ = scale_color(0.5);
        // fuera de rango se acota.
        assert_eq!(scale_color(-1.0).to_rgba8().b, scale_color(0.0).to_rgba8().b);
    }

    #[test]
    fn search_rankea_nombre_sobre_props() {
        let d = data_of(
            r#"{"type":"FeatureCollection","features":[
                {"type":"Feature","properties":{"name":"Perú","capital":"Lima"},"geometry":{"type":"Point","coordinates":[-77,-12]}},
                {"type":"Feature","properties":{"name":"Bolivia","nota":"vecino de Perú"},"geometry":{"type":"Point","coordinates":[-68,-16]}},
                {"type":"Feature","properties":{"name":"Chile"},"geometry":{"type":"Point","coordinates":[-70,-33]}}
            ]}"#,
        );
        // "peru": feature 0 (nombre exacto, peso doble) gana a feature 1
        // (substring en una prop).
        let hits = search(&d, "peru", 10);
        assert_eq!(hits.first(), Some(&0));
        assert!(hits.contains(&1));
        assert!(!hits.contains(&2));
        // Case-insensitive y por prefijo.
        assert_eq!(search(&d, "CHI", 1), vec![2]);
        // Busca también en propiedades (capital).
        assert_eq!(search(&d, "lima", 1), vec![0]);
        // Vacío → nada.
        assert!(search(&d, "   ", 5).is_empty());
    }

    #[test]
    fn focus_on_centra_y_selecciona() {
        let d = data_of(
            r#"{"type":"Feature","properties":{"name":"sq"},
                "geometry":{"type":"Polygon","coordinates":[[[0,0],[10,0],[10,10],[0,10],[0,0]]]}}"#,
        );
        let mut view = MapView::default();
        view.record_rect((0.0, 0.0, 100.0, 100.0));
        focus_on(&d, &mut view, 0);
        assert_eq!(view.selected, Some(0));
        // El centro de la feature [5,5] debe caer en el centro del panel (50,50).
        let bb = d.bbox().unwrap();
        let proj = Projection::fit(bb, (0.0, 0.0, 100.0, 100.0), view.zoom, view.pan);
        let (sx, sy) = proj.to_screen([5.0, 5.0]);
        assert!((sx - 50.0).abs() < 0.5 && (sy - 50.0).abs() < 0.5, "({sx},{sy})");
    }

    #[test]
    fn route_sobre_cuadricula() {
        // Cuadrícula 2×2 de calles que comparten cruces exactos.
        let d = data_of(
            r#"{"type":"FeatureCollection","features":[
                {"type":"Feature","properties":{},"geometry":{"type":"LineString","coordinates":[[0,0],[1,0],[2,0]]}},
                {"type":"Feature","properties":{},"geometry":{"type":"LineString","coordinates":[[0,1],[1,1],[2,1]]}},
                {"type":"Feature","properties":{},"geometry":{"type":"LineString","coordinates":[[0,0],[0,1]]}},
                {"type":"Feature","properties":{},"geometry":{"type":"LineString","coordinates":[[1,0],[1,1]]}},
                {"type":"Feature","properties":{},"geometry":{"type":"LineString","coordinates":[[2,0],[2,1]]}}
            ]}"#,
        );
        // De la esquina (0,0) a la (2,1): A* encuentra un camino conectado.
        let r = route(&d, [0.0, 0.0], [2.0, 1.0]).expect("debe haber ruta");
        assert_eq!(r.path.first(), Some(&[0.0, 0.0]));
        assert_eq!(r.path.last(), Some(&[2.0, 1.0]));
        assert!(r.meters > 0.0);
        // El camino más corto en la grilla atraviesa 3 tramos unitarios.
        assert_eq!(r.path.len(), 4);
    }

    #[test]
    fn route_snapea_al_nodo_mas_cercano() {
        let d = data_of(
            r#"{"type":"Feature","properties":{},"geometry":{"type":"LineString","coordinates":[[0,0],[1,0],[2,0]]}}"#,
        );
        // Clics fuera de la línea snapean a los extremos.
        let r = route(&d, [0.1, 0.4], [1.9, -0.3]).expect("ruta");
        assert_eq!(r.path.first(), Some(&[0.0, 0.0]));
        assert_eq!(r.path.last(), Some(&[2.0, 0.0]));
    }

    #[test]
    fn mvt_tile_a_mapdata() {
        // Tile MVT con una LINESTRING (codificado a mano en el módulo vt).
        // Acá validamos la costura a MapData reutilizando el decoder.
        // MoveTo(0,0) LineTo +(100,0): una línea de dos vértices.
        fn varint(out: &mut Vec<u8>, mut v: u64) {
            loop {
                let mut b = (v & 0x7f) as u8;
                v >>= 7;
                if v != 0 {
                    b |= 0x80;
                }
                out.push(b);
                if v == 0 {
                    break;
                }
            }
        }
        let zz = |v: i32| ((v << 1) ^ (v >> 31)) as u64;
        let mut geom = Vec::new();
        varint(&mut geom, (1 << 3) | 1); // MoveTo count 1
        varint(&mut geom, zz(100));
        varint(&mut geom, zz(100));
        varint(&mut geom, (1 << 3) | 2); // LineTo count 1
        varint(&mut geom, zz(50));
        varint(&mut geom, zz(0));
        let mut feat = Vec::new();
        varint(&mut feat, (3 << 3) | 0);
        varint(&mut feat, 2); // LINESTRING
        varint(&mut feat, (4 << 3) | 2);
        varint(&mut feat, geom.len() as u64);
        feat.extend_from_slice(&geom);
        let mut layer = Vec::new();
        varint(&mut layer, (1 << 3) | 2);
        varint(&mut layer, 5);
        layer.extend_from_slice(b"roads");
        varint(&mut layer, (2 << 3) | 2);
        varint(&mut layer, feat.len() as u64);
        layer.extend_from_slice(&feat);
        let mut tile = Vec::new();
        varint(&mut tile, (3 << 3) | 2);
        varint(&mut tile, layer.len() as u64);
        tile.extend_from_slice(&layer);

        let d = mvt_tile_to_mapdata(&tile, 0, 0, 0);
        assert_eq!(d.lines.len(), 1);
        assert_eq!(d.lines[0].len(), 2);
        assert_eq!(d.features[0].name.as_deref(), Some("roads"));
    }

    #[test]
    fn pmtiles_overview_end_to_end() {
        // Construye un MVT (una LINESTRING), lo envuelve en un .pmtiles mínimo
        // y verifica que load_pmtiles_overview lo decodifica a MapData. Es el
        // camino completo decoder MVT + contenedor PMTiles, con datos sintéticos
        // (el archivo real validará compresión/Hilbert a escala).
        fn varint(out: &mut Vec<u8>, mut v: u64) {
            loop {
                let mut b = (v & 0x7f) as u8;
                v >>= 7;
                if v != 0 {
                    b |= 0x80;
                }
                out.push(b);
                if v == 0 {
                    break;
                }
            }
        }
        let zz = |v: i32| ((v << 1) ^ (v >> 31)) as u64;
        // MVT con una línea de dos vértices, capa "roads".
        let mut geom = Vec::new();
        varint(&mut geom, (1 << 3) | 1);
        varint(&mut geom, zz(100));
        varint(&mut geom, zz(100));
        varint(&mut geom, (1 << 3) | 2);
        varint(&mut geom, zz(50));
        varint(&mut geom, zz(0));
        let mut feat = Vec::new();
        varint(&mut feat, (3 << 3) | 0);
        varint(&mut feat, 2);
        varint(&mut feat, (4 << 3) | 2);
        varint(&mut feat, geom.len() as u64);
        feat.extend_from_slice(&geom);
        let mut layer = Vec::new();
        varint(&mut layer, (1 << 3) | 2);
        varint(&mut layer, 5);
        layer.extend_from_slice(b"roads");
        varint(&mut layer, (2 << 3) | 2);
        varint(&mut layer, feat.len() as u64);
        layer.extend_from_slice(&feat);
        let mut mvt = Vec::new();
        varint(&mut mvt, (3 << 3) | 2);
        varint(&mut mvt, layer.len() as u64);
        mvt.extend_from_slice(&layer);

        // .pmtiles mínimo (un tile en z0, sin compresión).
        let mut dir = Vec::new();
        varint(&mut dir, 1); // 1 entrada
        varint(&mut dir, 0); // tile_id 0
        varint(&mut dir, 1); // run_length
        varint(&mut dir, mvt.len() as u64); // length
        varint(&mut dir, 1); // offset+1
        let root_off = 127u64;
        let tile_off = root_off + dir.len() as u64;
        let mut file = vec![0u8; 127];
        file[0..7].copy_from_slice(b"PMTiles");
        file[7] = 3;
        file[8..16].copy_from_slice(&root_off.to_le_bytes());
        file[16..24].copy_from_slice(&(dir.len() as u64).to_le_bytes());
        file[40..48].copy_from_slice(&tile_off.to_le_bytes());
        file[56..64].copy_from_slice(&tile_off.to_le_bytes());
        file[64..72].copy_from_slice(&(mvt.len() as u64).to_le_bytes());
        file[97] = 1; // internal none
        file[98] = 1; // tile none
        file[99] = 1; // mvt
        file.extend_from_slice(&dir);
        file.extend_from_slice(&mvt);

        match load_pmtiles_overview(file) {
            MapPreview::Map { data, .. } => {
                assert_eq!(data.lines.len(), 1);
                assert_eq!(data.features[0].name.as_deref(), Some("roads"));
            }
            other => panic!("esperaba Map, fue {other:?}"),
        }
    }

    /// Construye un `.pmtiles` mínimo (z0, sin compresión) con una LINESTRING
    /// en la capa "roads". Reutilizable por los tests de streaming.
    fn tiny_pmtiles() -> Vec<u8> {
        fn varint(out: &mut Vec<u8>, mut v: u64) {
            loop {
                let mut b = (v & 0x7f) as u8;
                v >>= 7;
                if v != 0 {
                    b |= 0x80;
                }
                out.push(b);
                if v == 0 {
                    break;
                }
            }
        }
        let zz = |v: i32| ((v << 1) ^ (v >> 31)) as u64;
        let mut geom = Vec::new();
        varint(&mut geom, (1 << 3) | 1);
        varint(&mut geom, zz(100));
        varint(&mut geom, zz(100));
        varint(&mut geom, (1 << 3) | 2);
        varint(&mut geom, zz(50));
        varint(&mut geom, zz(0));
        let mut feat = Vec::new();
        varint(&mut feat, (3 << 3) | 0);
        varint(&mut feat, 2);
        varint(&mut feat, (4 << 3) | 2);
        varint(&mut feat, geom.len() as u64);
        feat.extend_from_slice(&geom);
        let mut layer = Vec::new();
        varint(&mut layer, (1 << 3) | 2);
        varint(&mut layer, 5);
        layer.extend_from_slice(b"roads");
        varint(&mut layer, (2 << 3) | 2);
        varint(&mut layer, feat.len() as u64);
        layer.extend_from_slice(&feat);
        let mut mvt = Vec::new();
        varint(&mut mvt, (3 << 3) | 2);
        varint(&mut mvt, layer.len() as u64);
        mvt.extend_from_slice(&layer);

        let mut dir = Vec::new();
        varint(&mut dir, 1);
        varint(&mut dir, 0);
        varint(&mut dir, 1);
        varint(&mut dir, mvt.len() as u64);
        varint(&mut dir, 1);
        let root_off = 127u64;
        let tile_off = root_off + dir.len() as u64;
        let mut file = vec![0u8; 127];
        file[0..7].copy_from_slice(b"PMTiles");
        file[7] = 3;
        file[8..16].copy_from_slice(&root_off.to_le_bytes());
        file[16..24].copy_from_slice(&(dir.len() as u64).to_le_bytes());
        file[40..48].copy_from_slice(&tile_off.to_le_bytes());
        file[56..64].copy_from_slice(&tile_off.to_le_bytes());
        file[64..72].copy_from_slice(&(mvt.len() as u64).to_le_bytes());
        file[97] = 1;
        file[98] = 1;
        file[99] = 1;
        file.extend_from_slice(&dir);
        file.extend_from_slice(&mvt);
        file
    }

    #[test]
    fn evict_lru_saca_los_mas_viejos() {
        let mut cache: HashMap<(u32, u32, u32), CacheEntry> = HashMap::new();
        for i in 0..10u64 {
            cache.insert(
                (0, i as u32, 0),
                CacheEntry { used: i, data: MapData::default() },
            );
        }
        // Capamos a 4: quedan los 4 de mayor `used` (6,7,8,9).
        evict_lru(&mut cache, 4);
        assert_eq!(cache.len(), 4);
        assert!(cache.contains_key(&(0, 9, 0)));
        assert!(cache.contains_key(&(0, 6, 0)));
        assert!(!cache.contains_key(&(0, 5, 0)));
        assert!(!cache.contains_key(&(0, 0, 0)));
    }

    #[test]
    fn basemap_viewport_funde_tiles_visibles() {
        let mut bm = Basemap::open(tiny_pmtiles()).expect("abre basemap");
        let view = MapView::default();
        // Sin rect pintado: viewport vacío pero con bbox anclada.
        let empty = bm.viewport(&view);
        assert!(empty.lines.is_empty());
        assert!(empty.bbox_override.is_some());
        // Con rect: a zoom 1 / span mundial elige z0 y funde el tile 0/0/0.
        view.record_rect((0.0, 0.0, 512.0, 512.0));
        let md = bm.viewport(&view);
        assert_eq!(md.lines.len(), 1);
        assert!(md.bbox_override.is_some(), "la bbox queda anclada");
        // Segunda llamada usa la caché (mismo resultado).
        assert_eq!(bm.viewport(&view).lines.len(), 1);
    }

    #[test]
    fn route_sin_lineas_no_hay_ruta() {
        let d = data_of(r#"{"type":"Point","coordinates":[0,0]}"#);
        assert!(route(&d, [0.0, 0.0], [1.0, 1.0]).is_none());
    }

    #[test]
    fn haversine_distancia_conocida() {
        // ~1 grado de latitud ≈ 111 km.
        let m = haversine([0.0, 0.0], [0.0, 1.0]);
        assert!((m - 111_195.0).abs() < 500.0, "{m}");
    }

    #[test]
    fn toggle_base_alterna() {
        let mut v = MapView::default();
        assert!(v.show_base);
        v.toggle_base();
        assert!(!v.show_base);
        // reset no toca la preferencia de base.
        v.reset();
        assert!(!v.show_base);
    }

    #[test]
    fn contains_usa_el_rect_registrado() {
        let v = MapView::default();
        // Sin paint todavía: nada contiene.
        assert!(!v.contains(10.0, 10.0));
        v.record_rect((100.0, 50.0, 200.0, 100.0));
        assert!(v.contains(150.0, 90.0));
        assert!(!v.contains(50.0, 90.0)); // a la izquierda del rect
        assert!(!v.contains(150.0, 200.0)); // debajo del rect
    }
}
