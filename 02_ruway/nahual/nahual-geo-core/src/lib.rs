//! `nahual-geo-core` — núcleo geoespacial del visor de mapas de nahual,
//! **agnóstico de GUI**.
//!
//! Parsers (GeoJSON/GPX/KML, PMTiles v3, MVT), modelo del mundo
//! (`MapData`/`BBox`/`FeatureProps`/`MapView`), proyección equirectangular
//! con cámara, hit-test, búsqueda, ruteo A* y basemap por viewport. Sin
//! render: el frontend (`nahual-map-viewer-llimphi` u otro) sólo lo pinta.
//!
//! Antes vivía dentro del crate `*-llimphi`; extraído para cumplir la
//! regla #2 del repo (UIs intercambiables sobre cores agnósticos).

// ─── Submódulos ───────────────────────────────────────────────────────────────

pub mod pmtiles;
pub mod vt;

mod tipos;
mod geom;
mod camara;
mod parsers;
mod basemap;
mod busqueda;
mod ruteo;

// ─── Re-exportaciones de la API pública ──────────────────────────────────────

// Tipos base
pub use tipos::{
    BBox, Coord, FeatureProps, Label, MapData, MapPreview, MapView, Ring,
    MAX_LABELS, MAX_PROPS, MAX_VERTICES,
};

// Proyección y operaciones de cámara
pub use camara::{focus_on, hit_test, unproject, Projection};

// Parsers y carga de formatos
pub use parsers::{
    kml_coords, load_map, numeric_fields, parse_geojson, parse_into, parse_gpx, parse_kml,
    DEFAULT_MAP_BYTES_MAX,
};

// Basemap PMTiles y mapa mundial
pub use basemap::{
    evict_lru, load_pmtiles_overview, mvt_tile_to_mapdata, pmtiles_extent, world_base,
    world_base_stats, Basemap, CacheEntry, PMTILES_MAGIC,
};

// Búsqueda soberana
pub use busqueda::search;

// Ruteo A*
pub use ruteo::{haversine, route, RouteResult};

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

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
        assert_eq!(
            parse_geojson(r#"{"type":"Topology","objects":{}}"#),
            MapPreview::NoGeometry
        );
        assert_eq!(parse_geojson(r#"{"foo":"bar"}"#), MapPreview::NoGeometry);
    }

    #[test]
    fn json_invalido_es_error() {
        assert!(matches!(
            parse_geojson("{ no es json "),
            MapPreview::Error(_)
        ));
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
        assert!(
            (sx - 150.0).abs() < 1e-9 && (sy - 50.0).abs() < 1e-9,
            "({sx}, {sy})"
        );
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
        assert!(
            (sx - 50.0).abs() < 0.5 && (sy - 50.0).abs() < 0.5,
            "({sx},{sy})"
        );
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
        use std::collections::HashMap;
        let mut cache: HashMap<(u32, u32, u32), CacheEntry> = HashMap::new();
        for i in 0..10u64 {
            cache.insert(
                (0, i as u32, 0),
                CacheEntry {
                    used: i,
                    data: MapData::default(),
                },
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
    fn pmtiles_extent_deriva_de_geometria_con_header_roto() {
        // tiny_pmtiles deja los bounds del header en cero (header roto, como el
        // export real de bbbike/tilemaker con max_lon=0). pmtiles_extent debe
        // ignorarlos y derivar la extensión de la geometría del tile.
        let pm = pmtiles::PmTiles::from_bytes(tiny_pmtiles()).unwrap();
        let bb = pmtiles_extent(&pm);
        assert!(bb.max_lon >= bb.min_lon && bb.max_lat >= bb.min_lat);
        // No es el mundo entero, y cae donde está la geometría (oeste lejano).
        assert!(bb.min_lon > -180.0 && bb.max_lon < -100.0, "{bb:?}");
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
