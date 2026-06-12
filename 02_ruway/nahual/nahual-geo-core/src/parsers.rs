//! Parsers de formatos geoespaciales: GeoJSON, GPX (XML) y KML (XML).
//! Entrada → [`MapData`] plano, tolerante a errores, con presupuesto de
//! vértices para mantener el panel instantáneo.

use std::path::Path;

use crate::geom::{
    centroid, coord, coord_list, coord_rings, feature_props, label_at, make_feature, midpoint,
    push_line, push_points, push_polygon,
};
use crate::tipos::{MapData, MapPreview, MAX_VERTICES};

// ─── Topes ───────────────────────────────────────────────────────────────────

/// Tope de bytes a leer (128 MiB). Holgado para extractos PMTiles de ciudad;
/// el caller puede subirlo. (Un planeta entero pide streaming, no leer todo.)
pub const DEFAULT_MAP_BYTES_MAX: u64 = 128 * 1024 * 1024;

/// Magic de un archivo PMTiles v3.
const PMTILES_MAGIC: &[u8] = b"PMTiles";

// ─── Despacho principal ──────────────────────────────────────────────────────

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
        return crate::basemap::load_pmtiles_overview(raw);
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

// ─── GeoJSON ─────────────────────────────────────────────────────────────────

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
pub fn parse_into(src: &str, cap: usize) -> Result<(MapData, bool), String> {
    let value: serde_json::Value = serde_json::from_str(src).map_err(|e| e.to_string())?;
    let mut data = MapData::default();
    let mut budget = cap;
    collect(&value, &mut data, &mut budget, None, None);
    Ok((data, budget == 0))
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

/// Extrae un nombre legible de `properties`, probando claves usuales en
/// español/inglés. `None` si no hay propiedades o ninguna clave aplica.
fn feature_name(props: Option<&serde_json::Value>) -> Option<String> {
    let obj = props?.as_object()?;
    for key in [
        "nombre", "name", "título", "titulo", "title", "label", "Name", "NAME",
    ] {
        if let Some(s) = obj.get(key).and_then(|v| v.as_str()) {
            let s = s.trim();
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
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

// ─── GPX ─────────────────────────────────────────────────────────────────────

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
    let mut seg: Vec<crate::tipos::Coord> = Vec::new();
    let mut seg_name: Option<String> = None;
    // Waypoint en curso (con hijos, p. ej. `<name>`).
    let mut wpt: Option<crate::tipos::Coord> = None;
    let mut wpt_name: Option<String> = None;
    let mut target = NameTarget::None;

    // Cierra la línea en curso como polilínea con su etiqueta.
    let flush_seg = |data: &mut MapData,
                     budget: &mut usize,
                     seg: &mut Vec<crate::tipos::Coord>,
                     name: &mut Option<String>| {
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
        MapPreview::Map {
            data,
            truncated: budget == 0,
        }
    }
}

/// Lee los atributos `lat`/`lon` de un elemento GPX a una [`Coord`]
/// `[lon, lat]`. `None` si falta alguno o no son números finitos.
fn gpx_latlon(e: &quick_xml::events::BytesStart) -> Option<crate::tipos::Coord> {
    let mut lat = None;
    let mut lon = None;
    for a in e.attributes().flatten() {
        match a.key.local_name().as_ref() {
            b"lat" => {
                lat = std::str::from_utf8(&a.value)
                    .ok()
                    .and_then(|s| s.parse::<f64>().ok())
            }
            b"lon" => {
                lon = std::str::from_utf8(&a.value)
                    .ok()
                    .and_then(|s| s.parse::<f64>().ok())
            }
            _ => {}
        }
    }
    match (lon, lat) {
        (Some(lon), Some(lat)) if lon.is_finite() && lat.is_finite() => Some([lon, lat]),
        _ => None,
    }
}

// ─── KML ─────────────────────────────────────────────────────────────────────

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
    let mut poly_rings: Vec<crate::tipos::Ring> = Vec::new();
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
        MapPreview::Map {
            data,
            truncated: budget == 0,
        }
    }
}

/// Parsea un bloque de coordenadas KML (`lon,lat[,alt] lon,lat[,alt] …`).
pub fn kml_coords(s: &str) -> Vec<crate::tipos::Coord> {
    s.split_whitespace()
        .filter_map(|tok| {
            let mut it = tok.split(',');
            let lon = it.next()?.trim().parse::<f64>().ok()?;
            let lat = it.next()?.trim().parse::<f64>().ok()?;
            (lon.is_finite() && lat.is_finite()).then_some([lon, lat])
        })
        .collect()
}
