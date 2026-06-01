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

/// Tope de bytes a leer (16 MiB). Un GeoJSON más grande que eso es un
/// dataset, no un documento a ojo; el caller puede subirlo si hace falta.
pub const DEFAULT_MAP_BYTES_MAX: u64 = 16 * 1024 * 1024;

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

    /// Caja envolvente de todo el contenido, o `None` si no hay coordenadas.
    pub fn bbox(&self) -> Option<BBox> {
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

/// Lee el archivo y lo parsea a geometrías. La detección de tipo ya la hizo
/// el shell (lens `map`); acá leemos UTF-8 y desarmamos el GeoJSON.
pub fn load_map(path: &Path, max_bytes: u64) -> MapPreview {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() > max_bytes => return MapPreview::TooBig(meta.len()),
        Err(e) => return MapPreview::Error(e.to_string()),
        _ => {}
    }
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return MapPreview::Error(e.to_string()),
    };
    // GPX/KML son XML (arrancan con `<`); GeoJSON es JSON (`{`/`[`). El shell
    // rutea los tres al lens `map`, así que el visor desambigua por contenido.
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
    collect(&value, &mut data, &mut budget, None);
    Ok((data, budget == 0))
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
            push_line(data, std::mem::take(seg), budget);
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
                        push_points(&mut data, std::slice::from_ref(&c), &mut budget);
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
                        push_points(&mut data, std::slice::from_ref(&c), &mut budget);
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
                                push_points(&mut data, std::slice::from_ref(&c), &mut budget);
                                label_at(&mut data, placemark_name.as_deref(), Some(c));
                            }
                        }
                        Geom::Line => {
                            let rep = midpoint(&coords);
                            push_line(&mut data, coords, &mut budget);
                            label_at(&mut data, placemark_name.as_deref(), rep);
                        }
                        Geom::Ring => {
                            if in_polygon {
                                poly_rings.push(coords);
                            } else {
                                // LinearRing suelto → polígono de un anillo.
                                let rep = centroid(&coords);
                                push_polygon(&mut data, vec![coords], &mut budget);
                                label_at(&mut data, placemark_name.as_deref(), rep);
                            }
                        }
                        Geom::None => {}
                    }
                }
                b"Polygon" => {
                    let rep = poly_rings.first().and_then(|r| centroid(r));
                    push_polygon(&mut data, std::mem::take(&mut poly_rings), &mut budget);
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
fn collect(v: &serde_json::Value, data: &mut MapData, budget: &mut usize, name: Option<&str>) {
    if *budget == 0 {
        return;
    }
    let Some(ty) = v.get("type").and_then(|t| t.as_str()) else {
        return;
    };
    match ty {
        "FeatureCollection" => {
            if let Some(arr) = v.get("features").and_then(|f| f.as_array()) {
                for f in arr {
                    collect(f, data, budget, None);
                }
            }
        }
        "Feature" => {
            // El nombre de la feature manda sobre uno heredado.
            let fname = feature_name(v.get("properties"));
            if let Some(g) = v.get("geometry") {
                collect(g, data, budget, fname.as_deref().or(name));
            }
        }
        "GeometryCollection" => {
            if let Some(arr) = v.get("geometries").and_then(|g| g.as_array()) {
                for g in arr {
                    collect(g, data, budget, name);
                }
            }
        }
        "Point" => {
            if let Some(c) = coord(v.get("coordinates")) {
                push_points(data, std::slice::from_ref(&c), budget);
                label_at(data, name, Some(c));
            }
        }
        "MultiPoint" => {
            let cs = coord_list(v.get("coordinates"));
            let rep = cs.first().copied();
            push_points(data, &cs, budget);
            label_at(data, name, rep);
        }
        "LineString" => {
            let line = coord_list(v.get("coordinates"));
            let rep = midpoint(&line);
            push_line(data, line, budget);
            label_at(data, name, rep);
        }
        "MultiLineString" => {
            let lines = coord_rings(v.get("coordinates"));
            let rep = lines.first().and_then(|l| midpoint(l));
            for line in lines {
                push_line(data, line, budget);
            }
            label_at(data, name, rep);
        }
        "Polygon" => {
            let rings = coord_rings(v.get("coordinates"));
            let rep = rings.first().and_then(|r| centroid(r));
            push_polygon(data, rings, budget);
            label_at(data, name, rep);
        }
        "MultiPolygon" => {
            // coordinates: [ [ ring, ring... ], ... ]
            if let Some(arr) = v.get("coordinates").and_then(|c| c.as_array()) {
                let mut rep = None;
                for poly in arr {
                    let rings = coord_rings(Some(poly));
                    if rep.is_none() {
                        rep = rings.first().and_then(|r| centroid(r));
                    }
                    push_polygon(data, rings, budget);
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

fn push_points(data: &mut MapData, pts: &[Coord], budget: &mut usize) {
    for p in pts {
        if *budget == 0 {
            return;
        }
        data.points.push(*p);
        *budget -= 1;
    }
}

fn push_line(data: &mut MapData, mut line: Ring, budget: &mut usize) {
    if line.len() < 2 {
        return;
    }
    line.truncate(*budget);
    if line.len() < 2 {
        return;
    }
    *budget -= line.len();
    data.lines.push(line);
}

fn push_polygon(data: &mut MapData, rings: Vec<Ring>, budget: &mut usize) {
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
    }
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
    rect: Arc<Mutex<Option<(f32, f32, f32, f32)>>>,
}

impl Default for MapView {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: (0.0, 0.0),
            show_base: true,
            rect: Arc::new(Mutex::new(None)),
        }
    }
}

impl MapView {
    /// Límites de zoom: ni tan lejos que desaparezca, ni tan cerca que se
    /// pierda en aritmética.
    pub const ZOOM_MIN: f64 = 0.2;
    pub const ZOOM_MAX: f64 = 64.0;

    /// Vuelve al encuadre inicial (zoom 1, sin pan). Conserva la celda del
    /// rect para no perder el gateo entre selecciones.
    pub fn reset(&mut self) {
        self.zoom = 1.0;
        self.pan = (0.0, 0.0);
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
pub fn map_viewer_view<Msg>(
    state: &MapPreview,
    path: Option<&Path>,
    palette: &MapViewerPalette,
    view: &MapView,
) -> View<Msg>
where
    Msg: Clone + 'static,
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

    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        padding: pad(12.0, 0.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(header_text, 10.0, palette.fg_muted, Alignment::Start);

    let body = match state {
        MapPreview::Empty => simple_body("—", palette.fg_muted),
        MapPreview::NoGeometry => {
            simple_body("(JSON sin geometrías GeoJSON)", palette.fg_muted)
        }
        MapPreview::TooBig(n) => {
            simple_body(&format!("(archivo muy grande: {n} bytes — sin preview)"), palette.fg_muted)
        }
        MapPreview::Error(e) => simple_body(&format!("(no se pudo leer: {e})"), palette.fg_error),
        MapPreview::Map { data, .. } => map_canvas(data.clone(), *palette, view.clone()),
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

/// Lienzo que proyecta y dibuja las geometrías encajadas en el panel,
/// aplicando la cámara (`zoom`/`pan`) y registrando su rect para el host.
fn map_canvas<Msg>(data: MapData, palette: MapViewerPalette, view: MapView) -> View<Msg>
where
    Msg: Clone + 'static,
{
    let zoom = view.zoom;
    let pan = view.pan;
    let show_base = view.show_base;
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

        // Cámara: zoom anclado al centro del panel + pan en px físicos.
        let pivot_x = rect.x as f64 + rect.w as f64 * 0.5;
        let pivot_y = rect.y as f64 + rect.h as f64 * 0.5;
        let camera = |x: f64, y: f64| -> (f64, f64) {
            (
                pivot_x + (x - pivot_x) * zoom + pan.0,
                pivot_y + (y - pivot_y) * zoom + pan.1,
            )
        };

        // Proyección equirectangular con corrección por coseno de la
        // latitud media: comprime el eje X para que un grado de longitud y
        // uno de latitud midan parecido en pantalla (sin esto, los mapas se
        // estiran a lo ancho lejos del ecuador).
        let lat0 = (bb.min_lat + bb.max_lat) * 0.5;
        let kx = lat0.to_radians().cos().abs().max(0.05);

        let pmin_x = bb.min_lon * kx;
        let pmax_x = bb.max_lon * kx;
        let pw = (pmax_x - pmin_x).max(0.0);
        let ph = (bb.max_lat - bb.min_lat).max(0.0);

        let inset = 6.0_f64;
        let aw = (rect.w as f64 - 2.0 * inset).max(1.0);
        let ah = (rect.h as f64 - 2.0 * inset).max(1.0);

        // Escala uniforme que encaja la caja entera. Si una dimensión es
        // degenerada (punto único, línea vertical/horizontal), se apoya en
        // la otra; con tope para que un único punto no explote.
        let sx = if pw > 1e-12 { aw / pw } else { f64::INFINITY };
        let sy = if ph > 1e-12 { ah / ph } else { f64::INFINITY };
        let scale = sx.min(sy).min(1.0e6);
        let scale = if scale.is_finite() { scale } else { 1.0 };

        let ox = rect.x as f64 + inset + (aw - pw * scale) * 0.5;
        let oy = rect.y as f64 + inset + (ah - ph * scale) * 0.5;

        // lon/lat → pantalla (Y invertida: lat arriba, pantalla abajo),
        // pasando por la cámara (zoom/pan).
        let to_screen = |[lon, lat]: Coord| -> (f64, f64) {
            let x = ox + (lon * kx - pmin_x) * scale;
            let y = oy + (bb.max_lat - lat) * scale;
            camera(x, y)
        };

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

        // Polígonos: relleno translúcido del contorno exterior + borde de
        // cada anillo.
        for poly in &data.polygons {
            for (i, ring) in poly.iter().enumerate() {
                let path = ring_path(ring, &to_screen, true);
                if i == 0 {
                    scene.fill(Fill::NonZero, Affine::IDENTITY, fill_col, None, &path);
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

        // --- Mobiliario cartográfico (fijo a pantalla) ---------------
        let furn = with_alpha(palette.label, 0.7);
        let furn_line = Stroke::new(1.4);
        let rx = rect.x as f64;
        let ry = rect.y as f64;
        let rw = rect.w as f64;
        let rh = rect.h as f64;

        // Lectura del centro de la vista + zoom (arriba-izquierda).
        // Invierte la cámara y la proyección en el centro del panel.
        let cbx = pivot_x - pan.0 / zoom;
        let cby = pivot_y - pan.1 / zoom;
        let lon_c = ((cbx - ox) / scale + pmin_x) / kx;
        let lat_c = bb.max_lat - (cby - oy) / scale;
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
    })
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
