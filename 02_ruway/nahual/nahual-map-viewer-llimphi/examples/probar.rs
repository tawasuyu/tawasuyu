//! Prueba headless del visor de mapas: carga un `.geojson` real con
//! [`load_map`] y reporta lo parseado (sin abrir ventana). Sirve para
//! ejercitar el parseo/aplanado sobre un archivo de verdad.
//!
//! ```bash
//! cargo run -p nahual-map-viewer-llimphi --example probar
//! cargo run -p nahual-map-viewer-llimphi --example probar -- ruta/a/otro.geojson
//! ```

use std::path::PathBuf;

use nahual_map_viewer_llimphi::{load_map, MapPreview, DEFAULT_MAP_BYTES_MAX};

fn main() {
    // Path del argumento, o el sample que viene con el crate.
    let path = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("samples/andes.geojson")
    });

    println!("Cargando: {}", path.display());
    match load_map(&path, DEFAULT_MAP_BYTES_MAX) {
        MapPreview::Map { data, truncated } => {
            println!("✓ GeoJSON parseado{}", if truncated { " (truncado)" } else { "" });
            println!("  puntos    : {}", data.points.len());
            println!("  líneas    : {}", data.lines.len());
            println!("  polígonos : {}", data.polygons.len());
            println!("  vértices  : {}", data.vertex_count());
            if let Some(b) = data.bbox() {
                println!(
                    "  bbox      : [{:.3}, {:.3}] → [{:.3}, {:.3}]",
                    b.min_lon, b.min_lat, b.max_lon, b.max_lat
                );
            }
            for (i, p) in data.points.iter().enumerate() {
                println!("  punto[{i}] = lon {:.3}, lat {:.3}", p[0], p[1]);
            }
        }
        MapPreview::NoGeometry => println!("✗ JSON sin geometrías GeoJSON"),
        MapPreview::TooBig(n) => println!("✗ archivo muy grande: {n} bytes"),
        MapPreview::Error(e) => println!("✗ error: {e}"),
        MapPreview::Empty => println!("✗ vacío"),
    }
}
