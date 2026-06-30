//! `tullpu-icon` — generador de íconos headless.
//!
//! Toma un [`IconSpec`] (JSON), lo compila a vectores nativos de tullpu vía
//! `tullpu-icon-core`, y lo exporta:
//!
//! - **SVG** (vectorial): vía `foreign-svg::exportar_svg`, en la grilla de
//!   diseño del spec (viewBox `0 0 lienzo lienzo`).
//! - **PNG/WebP/JPEG** (raster): escala la geometría a `--size` px, rasteriza
//!   cada capa con `tullpu-ops::rasterizar_vector` (tiny-skia, anti-aliased) y
//!   las compone con el compositor maduro de `tullpu-render`.
//!
//! El color `Corriente` (currentColor) de un spec se resuelve con `--color`
//! (default negro opaco). La resolución de colores de **marca** por app llega
//! en la fase de wiring con el catálogo de marcas.
//!
//! ```text
//! tullpu-icon generar  spec.json --out logo.svg
//! tullpu-icon generar  spec.json --out logo.png --size 64 --color '#e55b7a'
//! tullpu-icon lote     ./specs   --out ./dist --formato svg
//! ```

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use tullpu_core::{Capa, Lienzo, ParamsVector};
use tullpu_icon_core::{ColorFijo, IconSpec};
use tullpu_render::{exportar, AlmacenEnMemoria, FormatoExport};

#[derive(Parser)]
#[command(name = "tullpu-icon", about = "Generador de íconos headless sobre tullpu")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Genera un ícono a partir de un IconSpec JSON. El formato de salida se
    /// infiere de la extensión de `--out` (.svg/.png/.webp/.jpg).
    Generar {
        /// Ruta del IconSpec en JSON.
        spec: PathBuf,
        /// Ruta de salida; la extensión determina el formato.
        #[arg(short, long)]
        out: PathBuf,
        /// Lado en px del raster (ignorado para SVG).
        #[arg(long, default_value_t = 24)]
        size: u32,
        /// Color para `Corriente`/marca sin resolver: `#RRGGBB` o `#RRGGBBAA`.
        #[arg(long)]
        color: Option<String>,
    },
    /// Genera en lote: cada `*.json` de `dir` produce un `<nombre>.<formato>`
    /// en el directorio `--out`.
    Lote {
        /// Directorio con los IconSpec `*.json`.
        dir: PathBuf,
        /// Directorio de salida (se crea si no existe).
        #[arg(short, long)]
        out: PathBuf,
        /// Formato de salida: svg, png, webp o jpg.
        #[arg(long, default_value = "svg")]
        formato: String,
        /// Lado en px del raster (ignorado para SVG).
        #[arg(long, default_value_t = 24)]
        size: u32,
        /// Color para `Corriente`/marca sin resolver: `#RRGGBB` o `#RRGGBBAA`.
        #[arg(long)]
        color: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Generar { spec, out, size, color } => {
            let corriente = parse_color(color.as_deref())?;
            let spec = leer_spec(&spec)?;
            let fmt = formato_de_ext(&out)?;
            generar(&spec, &out, size, corriente, fmt)?;
            println!("ok  {} → {}", spec.nombre, out.display());
        }
        Cmd::Lote { dir, out, formato, size, color } => {
            let corriente = parse_color(color.as_deref())?;
            let fmt = formato_de_nombre(&formato)?;
            let ext = ext_de_formato(fmt, &formato);
            std::fs::create_dir_all(&out)
                .with_context(|| format!("creando {}", out.display()))?;
            let mut n = 0usize;
            for entrada in std::fs::read_dir(&dir).with_context(|| format!("leyendo {}", dir.display()))? {
                let ruta = entrada?.path();
                if ruta.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let spec = leer_spec(&ruta)?;
                let salida = out.join(format!("{}.{ext}", spec.nombre));
                generar(&spec, &salida, size, corriente, fmt)?;
                println!("ok  {} → {}", spec.nombre, salida.display());
                n += 1;
            }
            println!("{n} íconos generados en {}", out.display());
        }
    }
    Ok(())
}

/// Salida que el CLI sabe escribir. `Svg` es vectorial; el resto rasteriza.
#[derive(Clone, Copy)]
enum Salida {
    Svg,
    Raster(FormatoExport),
}

fn leer_spec(ruta: &Path) -> Result<IconSpec> {
    let txt = std::fs::read_to_string(ruta).with_context(|| format!("leyendo {}", ruta.display()))?;
    serde_json::from_str(&txt).with_context(|| format!("parseando IconSpec {}", ruta.display()))
}

fn generar(spec: &IconSpec, out: &Path, size: u32, corriente: [u8; 4], fmt: Salida) -> Result<()> {
    let resolver = ColorFijo::nuevo(corriente);
    let capas = spec.compilar(&resolver);
    match fmt {
        Salida::Svg => {
            let lado = spec.lienzo.round().max(1.0) as u32;
            let svg = foreign_svg::exportar_svg(&capas, lado, lado);
            std::fs::write(out, svg).with_context(|| format!("escribiendo {}", out.display()))?;
        }
        Salida::Raster(formato) => {
            let lienzo = componer_raster(spec, &capas, size)?;
            let alm = lienzo.1;
            exportar(&lienzo.0, &alm, out, formato)
                .with_context(|| format!("exportando {}", out.display()))?;
        }
    }
    Ok(())
}

/// Escala la geometría de la grilla a `size` px, rasteriza cada capa y arma un
/// `Lienzo` plano (una capa raster Normal por vector) listo para el compositor.
fn componer_raster(spec: &IconSpec, capas: &[ParamsVector], size: u32) -> Result<(Lienzo, AlmacenEnMemoria)> {
    if size == 0 {
        bail!("--size debe ser > 0");
    }
    let s = spec.escala_para(size as f32);
    let mut lienzo = Lienzo::nuevo(size, size);
    let mut alm = AlmacenEnMemoria::nuevo();
    for (i, pv) in capas.iter().enumerate() {
        let mut esc = pv.clone();
        esc.transformar([s, 0.0, 0.0, s, 0.0, 0.0]);
        let buf = tullpu_ops::rasterizar_vector(&esc, size, size);
        let hash = alm.insertar(buf);
        lienzo.capas.push(Capa::raster(format!("{}#{i}", spec.nombre), hash));
    }
    Ok((lienzo, alm))
}

// ---- Parsing de formato y color --------------------------------------------

fn formato_de_ext(out: &Path) -> Result<Salida> {
    let ext = out
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .with_context(|| format!("--out sin extensión: {}", out.display()))?;
    formato_de_nombre(&ext)
}

fn formato_de_nombre(s: &str) -> Result<Salida> {
    match s.to_ascii_lowercase().as_str() {
        "svg" => Ok(Salida::Svg),
        "png" => Ok(Salida::Raster(FormatoExport::Png)),
        "webp" => Ok(Salida::Raster(FormatoExport::Webp)),
        "jpg" | "jpeg" => Ok(Salida::Raster(FormatoExport::Jpeg { calidad: 92 })),
        otro => bail!("formato no soportado: {otro} (use svg/png/webp/jpg)"),
    }
}

fn ext_de_formato(fmt: Salida, nombre: &str) -> String {
    match fmt {
        Salida::Svg => "svg".into(),
        Salida::Raster(FormatoExport::Png) => "png".into(),
        Salida::Raster(FormatoExport::Webp) => "webp".into(),
        Salida::Raster(FormatoExport::Jpeg { .. }) => {
            // Respeta "jpeg" si así lo escribió el usuario.
            if nombre.eq_ignore_ascii_case("jpeg") { "jpeg".into() } else { "jpg".into() }
        }
    }
}

/// `#RRGGBB`, `#RRGGBBAA` o sin `#`. Default negro opaco si `None`.
fn parse_color(s: Option<&str>) -> Result<[u8; 4]> {
    let Some(s) = s else { return Ok([0, 0, 0, 255]) };
    let h = s.strip_prefix('#').unwrap_or(s);
    let byte = |i: usize| -> Result<u8> {
        u8::from_str_radix(&h[i..i + 2], 16).with_context(|| format!("color inválido: {s}"))
    };
    match h.len() {
        6 => Ok([byte(0)?, byte(2)?, byte(4)?, 255]),
        8 => Ok([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
        _ => bail!("color debe ser #RRGGBB o #RRGGBBAA: {s}"),
    }
}
