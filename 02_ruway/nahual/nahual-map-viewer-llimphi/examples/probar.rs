//! Prueba headless del visor de mapas: carga un `.geojson` real con
//! [`load_map`] y reporta lo parseado (sin abrir ventana). Sirve para
//! ejercitar el parseo/aplanado sobre un archivo de verdad.
//!
//! ```bash
//! cargo run -p nahual-map-viewer-llimphi --example probar
//! cargo run -p nahual-map-viewer-llimphi --example probar -- ruta/a/otro.geojson
//! ```

use std::path::PathBuf;

use nahual_map_viewer_llimphi::{
    load_map, world_base_stats, Basemap, MapPreview, MapView, DEFAULT_MAP_BYTES_MAX,
};

fn main() {
    let (polys, verts, labels) = world_base_stats();
    println!("Mapa-base embebido: {polys} polígonos · {verts} vértices · {labels} países\n");
    // Path del argumento, o el sample que viene con el crate.
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("samples/andes.geojson"));

    // Si es PMTiles, además de la vista general, prueba el streaming por
    // viewport a varios zooms (debe mostrar más detalle al acercar).
    if std::fs::read(&path)
        .map(|b| b.starts_with(b"PMTiles"))
        .unwrap_or(false)
    {
        streaming_demo(&path);
    }

    println!("Cargando: {}", path.display());
    match load_map(&path, DEFAULT_MAP_BYTES_MAX) {
        MapPreview::Map { data, truncated } => {
            println!(
                "✓ mapa parseado{}",
                if truncated { " (truncado)" } else { "" }
            );
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
            println!("  etiquetas : {}", data.labels.len());
            for (i, p) in data.points.iter().enumerate() {
                println!("  punto[{i}] = lon {:.3}, lat {:.3}", p[0], p[1]);
            }
            for l in &data.labels {
                println!(
                    "  rótulo    : {:?} @ lon {:.3}, lat {:.3}",
                    l.text, l.at[0], l.at[1]
                );
            }
        }
        MapPreview::NoGeometry => println!("✗ JSON sin geometrías GeoJSON"),
        MapPreview::TooBig(n) => println!("✗ archivo muy grande: {n} bytes"),
        MapPreview::Error(e) => println!("✗ error: {e}"),
        MapPreview::Empty => println!("✗ vacío"),
    }
}

/// Abre un `.pmtiles` como basemap vivo y stremea el viewport a zoom 1, 8 y 64
/// (centrado en el medio del archivo). A más zoom, más tiles de detalle → más
/// features: la prueba de que el streaming funciona contra datos reales.
fn streaming_demo(path: &PathBuf) {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            println!("✗ no se pudo leer: {e}");
            return;
        }
    };
    let mut bm = match Basemap::open(bytes) {
        Ok(b) => b,
        Err(e) => {
            println!("✗ pmtiles: {e}");
            return;
        }
    };
    println!("Streaming por viewport (1200×800 px):");
    for zoom in [1.0_f64, 8.0, 64.0] {
        let mut view = MapView::default();
        view.zoom = zoom;
        view.record_rect((0.0, 0.0, 1200.0, 800.0));
        let md = bm.viewport(&view);
        println!(
            "  zoom {zoom:>4.0}× → {} puntos · {} líneas · {} polígonos · {} vértices · caché {} tiles",
            md.points.len(),
            md.lines.len(),
            md.polygons.len(),
            md.vertex_count(),
            bm.cache_len(),
        );
    }
    println!();
}
