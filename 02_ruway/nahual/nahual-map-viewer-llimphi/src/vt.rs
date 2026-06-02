//! Decodificador de **vector tiles** (Mapbox Vector Tile / MVT) — el corazón
//! soberano del "mapa de calles vivo": traduce un tile vectorial (protobuf en
//! coordenadas locales del tile) a geometrías en lon/lat, **sin Mapbox ni
//! librería ajena**. El protobuf se parsea a mano (no hay dependencia de
//! `prost`/`protobuf`), fiel a la ética de dependencias mínimas.
//!
//! Esquema MVT (relevante):
//! ```text
//! Tile  { repeated Layer layers = 3; }
//! Layer { string name = 1; repeated Feature features = 2; uint32 extent = 5; }
//! Feature { GeomType type = 3; repeated uint32 geometry = 4 [packed]; }
//! GeomType: POINT=1, LINESTRING=2, POLYGON=3
//! ```
//! La geometría es un flujo de comandos (`MoveTo`/`LineTo`/`ClosePath`) con
//! deltas en zigzag, en el espacio `0..extent` del tile (Y hacia abajo).
//!
//! Lo que falta para el basemap vivo end-to-end (y necesita un `.pmtiles` real
//! para validarse): el **contenedor PMTiles** (IDs Hilbert + directorios
//! gzip) que ubica los bytes de cada tile, y el **streaming por viewport**
//! (pedir los tiles visibles al hacer zoom). Este módulo es la pieza dura y
//! reusable, ya verificable.

use super::Coord;

/// Geometría de un tile, ya en lon/lat.
#[derive(Debug, Clone, PartialEq)]
pub enum TileGeom {
    Point(Coord),
    Line(Vec<Coord>),
    Polygon(Vec<Vec<Coord>>),
}

/// Una feature decodificada con su capa de origen (calle, agua, edificio…).
#[derive(Debug, Clone, PartialEq)]
pub struct TileFeature {
    pub layer: String,
    pub geom: TileGeom,
}

/// Convierte un punto en coordenadas locales del tile (`px, py` en `0..extent`,
/// Y hacia abajo) a lon/lat, vía Web Mercator esférico.
pub fn tile_to_lonlat(z: u32, x: u32, y: u32, extent: f64, px: f64, py: f64) -> Coord {
    let n = (1u64 << z) as f64;
    let fx = x as f64 + px / extent;
    let fy = y as f64 + py / extent;
    let lon = fx / n * 360.0 - 180.0;
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * fy / n)).sinh().atan().to_degrees();
    [lon, lat]
}

/// lon/lat → índice de tile `(x, y)` en el zoom `z` (esquema slippy/XYZ de
/// Web Mercator). Acota a `[0, 2^z-1]`.
pub fn lonlat_to_tile(z: u32, lon: f64, lat: f64) -> (u32, u32) {
    let n = (1u64 << z) as f64;
    let x = ((lon + 180.0) / 360.0 * n).floor();
    let lat = lat.clamp(-85.05112878, 85.05112878).to_radians();
    let y = ((1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / std::f64::consts::PI) / 2.0 * n).floor();
    let m = (n - 1.0).max(0.0);
    (x.clamp(0.0, m) as u32, y.clamp(0.0, m) as u32)
}

/// Zoom de tiles para que un span de `west..east` grados ocupe el ancho del
/// panel a ~512 px por tile. Sin acotar al rango del dataset (eso lo hace el
/// llamador).
pub fn zoom_for_span(west: f64, east: f64, panel_w: f64) -> u32 {
    let span = (east - west).abs().max(1e-9);
    let twoz = (360.0 / span) * (panel_w.max(1.0) / 512.0);
    twoz.max(1.0).log2().floor().clamp(0.0, 22.0) as u32
}

// ---------------------------------------------------------------------------
// Lector protobuf mínimo (wire format)
// ---------------------------------------------------------------------------

/// Cursor sobre un buffer protobuf. Sólo lo necesario para MVT.
struct Pbf<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Pbf<'a> {
    fn new(b: &'a [u8]) -> Self {
        Pbf { b, i: 0 }
    }

    fn eof(&self) -> bool {
        self.i >= self.b.len()
    }

    /// Lee un varint LEB128. Devuelve 0 si el buffer se acaba (tolerante).
    fn varint(&mut self) -> u64 {
        let mut shift = 0u32;
        let mut out = 0u64;
        while self.i < self.b.len() && shift < 64 {
            let byte = self.b[self.i];
            self.i += 1;
            out |= ((byte & 0x7f) as u64) << shift;
            if byte & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        out
    }

    /// Lee `(field_number, wire_type)` del próximo tag, o `None` en EOF.
    fn tag(&mut self) -> Option<(u64, u8)> {
        if self.eof() {
            return None;
        }
        let key = self.varint();
        Some((key >> 3, (key & 0x7) as u8))
    }

    /// Lee un bloque length-delimited (wire type 2) y devuelve su slice.
    fn len_delim(&mut self) -> &'a [u8] {
        let len = self.varint() as usize;
        let end = (self.i + len).min(self.b.len());
        let s = &self.b[self.i..end];
        self.i = end;
        s
    }

    /// Salta un campo del wire type dado.
    fn skip(&mut self, wire: u8) {
        match wire {
            0 => {
                self.varint();
            }
            1 => self.i = (self.i + 8).min(self.b.len()),
            2 => {
                let len = self.varint() as usize;
                self.i = (self.i + len).min(self.b.len());
            }
            5 => self.i = (self.i + 4).min(self.b.len()),
            _ => self.i = self.b.len(),
        }
    }
}

// ---------------------------------------------------------------------------
// Decodificación MVT
// ---------------------------------------------------------------------------

/// Decodifica un tile MVT a features en lon/lat. `(z, x, y)` ubican el tile en
/// la pirámide Web Mercator. Tolerante: ignora lo que no entiende.
pub fn decode_mvt_tile(bytes: &[u8], z: u32, x: u32, y: u32) -> Vec<TileFeature> {
    let mut out = Vec::new();
    let mut p = Pbf::new(bytes);
    while let Some((field, wire)) = p.tag() {
        if field == 3 && wire == 2 {
            decode_layer(p.len_delim(), z, x, y, &mut out);
        } else {
            p.skip(wire);
        }
    }
    out
}

fn decode_layer(bytes: &[u8], z: u32, x: u32, y: u32, out: &mut Vec<TileFeature>) {
    let mut name = String::new();
    let mut extent = 4096u64; // default del spec
    let mut features: Vec<&[u8]> = Vec::new();
    let mut p = Pbf::new(bytes);
    while let Some((field, wire)) = p.tag() {
        match (field, wire) {
            (1, 2) => name = String::from_utf8_lossy(p.len_delim()).into_owned(),
            (2, 2) => features.push(p.len_delim()),
            (5, 0) => extent = p.varint(),
            _ => p.skip(wire),
        }
    }
    let extent = extent.max(1) as f64;
    for fb in features {
        decode_feature(fb, &name, z, x, y, extent, out);
    }
}

fn decode_feature(
    bytes: &[u8],
    layer: &str,
    z: u32,
    x: u32,
    y: u32,
    extent: f64,
    out: &mut Vec<TileFeature>,
) {
    let mut geom_type = 0u64;
    let mut geom: Vec<u32> = Vec::new();
    let mut p = Pbf::new(bytes);
    while let Some((field, wire)) = p.tag() {
        match (field, wire) {
            (3, 0) => geom_type = p.varint(),
            // `geometry` es packed: un bloque length-delimited de varints.
            (4, 2) => {
                let mut gp = Pbf::new(p.len_delim());
                while !gp.eof() {
                    geom.push(gp.varint() as u32);
                }
            }
            _ => p.skip(wire),
        }
    }
    decode_geometry(&geom, geom_type, layer, z, x, y, extent, out);
}

/// Comandos MVT.
const CMD_MOVE_TO: u32 = 1;
const CMD_LINE_TO: u32 = 2;
const CMD_CLOSE_PATH: u32 = 7;

fn zigzag(v: u32) -> i32 {
    ((v >> 1) as i32) ^ -((v & 1) as i32)
}

#[allow(clippy::too_many_arguments)]
fn decode_geometry(
    g: &[u32],
    geom_type: u64,
    layer: &str,
    z: u32,
    x: u32,
    y: u32,
    extent: f64,
    out: &mut Vec<TileFeature>,
) {
    let mut i = 0usize;
    let (mut cx, mut cy) = (0i32, 0i32);
    // Sub-geometrías acumuladas (una por MoveTo en líneas; anillos en polígono).
    let mut current: Vec<Coord> = Vec::new();
    let mut rings: Vec<Vec<Coord>> = Vec::new();

    let to_ll = |cx: i32, cy: i32| tile_to_lonlat(z, x, y, extent, cx as f64, cy as f64);

    while i < g.len() {
        let cmd = g[i] & 0x7;
        let count = (g[i] >> 3) as usize;
        i += 1;
        match cmd {
            CMD_MOVE_TO => {
                for _ in 0..count {
                    if i + 1 >= g.len() {
                        break;
                    }
                    cx += zigzag(g[i]);
                    cy += zigzag(g[i + 1]);
                    i += 2;
                    // Un MoveTo arranca una nueva sub-geometría.
                    if geom_type == 1 {
                        // POINT (o MultiPoint): cada MoveTo es un punto.
                        out.push(TileFeature {
                            layer: layer.to_string(),
                            geom: TileGeom::Point(to_ll(cx, cy)),
                        });
                    } else {
                        if !current.is_empty() {
                            flush(geom_type, &mut current, &mut rings, layer, out);
                        }
                        current = vec![to_ll(cx, cy)];
                    }
                }
            }
            CMD_LINE_TO => {
                for _ in 0..count {
                    if i + 1 >= g.len() {
                        break;
                    }
                    cx += zigzag(g[i]);
                    cy += zigzag(g[i + 1]);
                    i += 2;
                    current.push(to_ll(cx, cy));
                }
            }
            CMD_CLOSE_PATH => {
                // Cierra el anillo de polígono actual.
                if geom_type == 3 && current.len() >= 3 {
                    if current.first() != current.last() {
                        let first = current[0];
                        current.push(first);
                    }
                    rings.push(std::mem::take(&mut current));
                }
            }
            _ => break,
        }
    }
    if !current.is_empty() {
        flush(geom_type, &mut current, &mut rings, layer, out);
    }
    if geom_type == 3 && !rings.is_empty() {
        out.push(TileFeature {
            layer: layer.to_string(),
            geom: TileGeom::Polygon(std::mem::take(&mut rings)),
        });
    }
}

fn flush(
    geom_type: u64,
    current: &mut Vec<Coord>,
    rings: &mut Vec<Vec<Coord>>,
    layer: &str,
    out: &mut Vec<TileFeature>,
) {
    let geom = std::mem::take(current);
    match geom_type {
        2 if geom.len() >= 2 => out.push(TileFeature {
            layer: layer.to_string(),
            geom: TileGeom::Line(geom),
        }),
        3 if geom.len() >= 3 => rings.push(geom),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mercator_esquinas_conocidas() {
        // Tile 0/0/0: esquina NW = (-180, +85.051…).
        let nw = tile_to_lonlat(0, 0, 0, 4096.0, 0.0, 0.0);
        assert!((nw[0] - -180.0).abs() < 1e-9);
        assert!((nw[1] - 85.0511).abs() < 1e-3);
        // Centro del mundo (z1, esquina compartida) = (0,0).
        let c = tile_to_lonlat(1, 1, 1, 4096.0, 0.0, 0.0);
        assert!(c[0].abs() < 1e-9 && c[1].abs() < 1e-6);
    }

    #[test]
    fn lonlat_a_tile_y_zoom() {
        // z=1: -180° → x0, +179° → x1; ecuador/Greenwich cae en el borde.
        assert_eq!(lonlat_to_tile(1, -180.0, 80.0).0, 0);
        assert_eq!(lonlat_to_tile(1, 179.0, 80.0).0, 1);
        assert_eq!(lonlat_to_tile(1, -90.0, -80.0).1, 1); // hemisferio sur
        // Ida y vuelta aproximada con tile_to_lonlat.
        let (x, y) = lonlat_to_tile(4, -71.97, -13.5);
        let ll = tile_to_lonlat(4, x, y, 1.0, 0.5, 0.5);
        assert!((ll[0] - -71.97).abs() < 30.0 && (ll[1] - -13.5).abs() < 30.0);
        // Zoom para spans conocidos.
        assert_eq!(zoom_for_span(-180.0, 180.0, 512.0), 0);
        assert_eq!(zoom_for_span(-90.0, 90.0, 512.0), 1);
    }

    #[test]
    fn zigzag_decodifica() {
        assert_eq!(zigzag(0), 0);
        assert_eq!(zigzag(1), -1);
        assert_eq!(zigzag(2), 1);
        assert_eq!(zigzag(3), -2);
    }

    // --- Codificación MVT a mano para validar el decoder contra el spec ---

    fn varint(out: &mut Vec<u8>, mut v: u64) {
        loop {
            let mut byte = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if v == 0 {
                break;
            }
        }
    }
    fn tag(out: &mut Vec<u8>, field: u64, wire: u8) {
        varint(out, (field << 3) | wire as u64);
    }
    fn len_delim(out: &mut Vec<u8>, field: u64, payload: &[u8]) {
        tag(out, field, 2);
        varint(out, payload.len() as u64);
        out.extend_from_slice(payload);
    }
    fn zz(v: i32) -> u32 {
        ((v << 1) ^ (v >> 31)) as u32
    }

    #[test]
    fn decodifica_linestring() {
        // Geometría: MoveTo(1) a (10,10), LineTo(2) +(20,0) +(0,20).
        let mut geom: Vec<u8> = Vec::new();
        varint(&mut geom, ((1 << 3) | CMD_MOVE_TO) as u64);
        varint(&mut geom, zz(10) as u64);
        varint(&mut geom, zz(10) as u64);
        varint(&mut geom, ((2 << 3) | CMD_LINE_TO) as u64);
        varint(&mut geom, zz(20) as u64);
        varint(&mut geom, zz(0) as u64);
        varint(&mut geom, zz(0) as u64);
        varint(&mut geom, zz(20) as u64);

        // Feature { type=2 (LINESTRING); geometry=geom }.
        let mut feat: Vec<u8> = Vec::new();
        tag(&mut feat, 3, 0);
        varint(&mut feat, 2);
        len_delim(&mut feat, 4, &geom);

        // Layer { name="roads"; feature; extent=4096 }.
        let mut layer: Vec<u8> = Vec::new();
        len_delim(&mut layer, 1, b"roads");
        len_delim(&mut layer, 2, &feat);
        tag(&mut layer, 5, 0);
        varint(&mut layer, 4096);

        // Tile { layer }.
        let mut tile: Vec<u8> = Vec::new();
        len_delim(&mut tile, 3, &layer);

        let feats = decode_mvt_tile(&tile, 0, 0, 0);
        assert_eq!(feats.len(), 1);
        assert_eq!(feats[0].layer, "roads");
        let TileGeom::Line(pts) = &feats[0].geom else {
            panic!("esperaba Line, fue {:?}", feats[0].geom)
        };
        assert_eq!(pts.len(), 3);
        // El primer vértice (10,10) en extent 4096, tile 0/0/0.
        let expect = tile_to_lonlat(0, 0, 0, 4096.0, 10.0, 10.0);
        assert!((pts[0][0] - expect[0]).abs() < 1e-9);
        assert!((pts[0][1] - expect[1]).abs() < 1e-9);
    }

    #[test]
    fn decodifica_polygon_cerrado() {
        // MoveTo (0,0); LineTo +(10,0) +(0,10); ClosePath.
        let mut geom: Vec<u8> = Vec::new();
        varint(&mut geom, ((1 << 3) | CMD_MOVE_TO) as u64);
        varint(&mut geom, zz(0) as u64);
        varint(&mut geom, zz(0) as u64);
        varint(&mut geom, ((2 << 3) | CMD_LINE_TO) as u64);
        varint(&mut geom, zz(10) as u64);
        varint(&mut geom, zz(0) as u64);
        varint(&mut geom, zz(0) as u64);
        varint(&mut geom, zz(10) as u64);
        varint(&mut geom, ((1 << 3) | CMD_CLOSE_PATH) as u64);

        let mut feat: Vec<u8> = Vec::new();
        tag(&mut feat, 3, 0);
        varint(&mut feat, 3); // POLYGON
        len_delim(&mut feat, 4, &geom);
        let mut layer: Vec<u8> = Vec::new();
        len_delim(&mut layer, 1, b"buildings");
        len_delim(&mut layer, 2, &feat);
        let mut tile: Vec<u8> = Vec::new();
        len_delim(&mut tile, 3, &layer);

        let feats = decode_mvt_tile(&tile, 0, 0, 0);
        assert_eq!(feats.len(), 1);
        let TileGeom::Polygon(rings) = &feats[0].geom else {
            panic!("esperaba Polygon")
        };
        assert_eq!(rings.len(), 1);
        // Anillo cerrado: primer == último vértice.
        assert_eq!(rings[0].first(), rings[0].last());
    }

    #[test]
    fn decodifica_multipoint() {
        // MoveTo con count=2: dos puntos.
        let mut geom: Vec<u8> = Vec::new();
        varint(&mut geom, ((2 << 3) | CMD_MOVE_TO) as u64);
        varint(&mut geom, zz(5) as u64);
        varint(&mut geom, zz(5) as u64);
        varint(&mut geom, zz(10) as u64);
        varint(&mut geom, zz(0) as u64);
        let mut feat: Vec<u8> = Vec::new();
        tag(&mut feat, 3, 0);
        varint(&mut feat, 1); // POINT
        len_delim(&mut feat, 4, &geom);
        let mut layer: Vec<u8> = Vec::new();
        len_delim(&mut layer, 1, b"pois");
        len_delim(&mut layer, 2, &feat);
        let mut tile: Vec<u8> = Vec::new();
        len_delim(&mut tile, 3, &layer);

        let feats = decode_mvt_tile(&tile, 0, 0, 0);
        assert_eq!(feats.len(), 2);
        assert!(matches!(feats[0].geom, TileGeom::Point(_)));
    }

    #[test]
    fn basura_no_panica() {
        assert!(decode_mvt_tile(&[0xff, 0xff, 0x07, 0x00, 0x42], 0, 0, 0).is_empty()
            || !decode_mvt_tile(&[0xff, 0xff, 0x07, 0x00, 0x42], 0, 0, 0).is_empty());
    }
}
