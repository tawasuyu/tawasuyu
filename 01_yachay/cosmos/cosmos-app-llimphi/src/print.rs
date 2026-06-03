//! Hoja imprimible: genera un HTML B/N (cabecera de la carta + tabla de
//! aspectos) y lo abre en el navegador del sistema para usar su diálogo
//! de impresión nativo (Ctrl+P → impresora o «Guardar como PDF»).
//!
//! **Por qué HTML y no render a PNG/PDF propio.** La app pinta por GPU
//! (wgpu); un render headless a imagen sería otra tubería entera. El
//! navegador ya sabe paginar, escalar y mandar a la impresora en los tres
//! sistemas — y el contenido de la hoja es texto + glyphs unicode, que se
//! imprime nítido a cualquier DPI. El HTML lleva `@media print` para que
//! salga en blanco y negro sin los adornos de pantalla.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::Command;

use cosmos_model::Chart;
use cosmos_render::{LayerKind, RenderModel};

use crate::format::{fmt_dms, simbolo_cuerpo};
use crate::glyphs::sign_id;

/// Arma la hoja, la escribe a un archivo temporal y la abre en el
/// navegador del sistema. Devuelve la ruta escrita (para la nota de
/// estado) o un mensaje de error.
pub(crate) fn imprimir_carta(chart: &Chart, render: &RenderModel) -> Result<PathBuf, String> {
    let html = build_html(chart, render);
    let path = std::env::temp_dir().join("cosmos-hoja.html");
    std::fs::write(&path, html.as_bytes()).map_err(|e| format!("no se pudo escribir {path:?}: {e}"))?;
    abrir_en_navegador(&path)?;
    Ok(path)
}

/// Una fila de la tabla de aspectos, agregando geo (natal) + topo
/// (topocéntrico) por par de cuerpos como hace la tabla en pantalla.
struct AspRow {
    kind: String,
    from: String,
    to: String,
    geo: Option<f64>,
    topo: Option<f64>,
    applying: Option<bool>,
}

fn sorted_pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.into(), b.into())
    } else {
        (b.into(), a.into())
    }
}

/// Agrega `aspect_summary` (geo natal + topo) en filas ordenadas por orbe
/// más cerrado primero — misma lógica que `view::tile_aspectos`.
fn aspect_rows(render: &RenderModel) -> Vec<AspRow> {
    let mut map: HashMap<(String, String, String), AspRow> = HashMap::new();
    for a in &render.aspect_summary {
        let topo = a.module_id == "topocentric";
        if !topo && a.module_id != "natal" {
            continue;
        }
        let (from, to) = sorted_pair(&a.from_body, &a.to_body);
        let key = (from.clone(), to.clone(), a.kind.clone());
        let row = map.entry(key).or_insert_with(|| AspRow {
            kind: a.kind.clone(),
            from,
            to,
            geo: None,
            topo: None,
            applying: None,
        });
        if topo {
            row.topo = Some(a.orb_deg);
        } else {
            row.geo = Some(a.orb_deg);
            row.applying = a.applying;
        }
    }
    let mut rows: Vec<AspRow> = map.into_values().collect();
    rows.sort_by(|a, b| {
        let oa = a.geo.or(a.topo).unwrap_or(99.0);
        let ob = b.geo.or(b.topo).unwrap_or(99.0);
        oa.partial_cmp(&ob).unwrap_or(std::cmp::Ordering::Equal)
    });
    rows
}

/// Mapa cuerpo→longitud eclíptica desde la capa natal de cuerpos, para
/// resolver el signo de cada extremo del aspecto.
fn body_lons(render: &RenderModel) -> HashMap<String, f32> {
    let mut m = HashMap::new();
    for l in &render.layers {
        if l.module_id == "natal" && matches!(l.kind, LayerKind::Bodies) {
            for g in &l.glyphs {
                m.insert(g.symbol.clone(), g.deg);
            }
        }
    }
    m.entry("asc".into()).or_insert(render.ascendant_deg);
    m.entry("mc".into()).or_insert(render.midheaven_deg);
    m
}

/// Cuerpo como `☉ Sol`. Si no hay símbolo unicode conocido, cae al código.
fn cuerpo_txt(name: &str, lons: &HashMap<String, f32>) -> String {
    let sym = planet_unicode(name);
    let code = simbolo_cuerpo(name);
    match lons.get(name) {
        Some(&deg) => format!("{sym} {code} {} {}", fmt_dms(deg.rem_euclid(30.0) as f64), sign_unicode(sign_id(deg))),
        None => format!("{sym} {code}"),
    }
}

fn build_html(chart: &Chart, render: &RenderModel) -> String {
    let bd = &chart.birth_data;
    let lons = body_lons(render);
    let rows = aspect_rows(render);

    let lugar = bd
        .birthplace_label
        .clone()
        .unwrap_or_else(|| "(sin lugar)".into());
    let fecha = format!(
        "{:04}-{:02}-{:02} {:02}:{:02} UTC{:+}",
        bd.year,
        bd.month,
        bd.day,
        bd.hour,
        bd.minute,
        bd.tz_offset_minutes as f32 / 60.0
    );
    let coords = format!(
        "{:.4}°{} · {:.4}°{}",
        bd.latitude_deg.abs(),
        if bd.latitude_deg >= 0.0 { "N" } else { "S" },
        bd.longitude_deg.abs(),
        if bd.longitude_deg >= 0.0 { "E" } else { "W" },
    );

    let mut out = String::with_capacity(8192);
    out.push_str(
        r#"<!DOCTYPE html>
<html lang="es"><head><meta charset="utf-8">
<title>Cosmos — Hoja de carta</title>
<style>
  @page { margin: 18mm; }
  * { box-sizing: border-box; }
  body { font-family: "DejaVu Sans", "Noto Sans", system-ui, sans-serif;
         color: #000; background: #fff; max-width: 760px; margin: 24px auto;
         padding: 0 16px; line-height: 1.4; }
  h1 { font-size: 22px; margin: 0 0 2px; }
  h2 { font-size: 14px; text-transform: uppercase; letter-spacing: .08em;
       border-bottom: 1.5px solid #000; padding-bottom: 4px; margin: 22px 0 10px; }
  .meta { font-size: 13px; color: #222; margin: 1px 0; }
  table { width: 100%; border-collapse: collapse; font-size: 13px; }
  th, td { text-align: left; padding: 5px 8px; border-bottom: 1px solid #bbb; }
  th { border-bottom: 1.5px solid #000; font-size: 11px; text-transform: uppercase;
       letter-spacing: .04em; }
  td.num, th.num { text-align: right; font-variant-numeric: tabular-nums; }
  .angles td { border: none; padding: 2px 14px 2px 0; }
  .print-btn { display: inline-block; margin: 16px 0; padding: 8px 16px;
       border: 1.5px solid #000; background: #fff; color: #000; font-size: 14px;
       cursor: pointer; border-radius: 4px; }
  footer { margin-top: 28px; font-size: 11px; color: #555;
       border-top: 1px solid #000; padding-top: 6px; }
  @media print { .print-btn { display: none; } body { margin: 0; } }
</style></head><body>
<button class="print-btn" onclick="window.print()">Imprimir / Guardar PDF</button>
"#,
    );

    // Cabecera de la carta.
    let _ = writeln!(out, "<h1>{}</h1>", esc(&chart.label));
    let _ = writeln!(out, r#"<div class="meta">{}</div>"#, esc(&lugar));
    let _ = writeln!(out, r#"<div class="meta">{}</div>"#, esc(&fecha));
    let _ = writeln!(out, r#"<div class="meta">{}</div>"#, esc(&coords));

    // Ángulos.
    out.push_str("<h2>Ángulos</h2>\n<table class=\"angles\"><tr>");
    for (name, deg) in [
        ("Asc", render.ascendant_deg),
        ("MC", render.midheaven_deg),
        ("Dc", render.descendant_deg),
        ("IC", render.imum_coeli_deg),
    ] {
        let _ = write!(
            out,
            "<td><b>{name}</b> {} {}</td>",
            fmt_dms(deg.rem_euclid(30.0) as f64),
            sign_unicode(sign_id(deg))
        );
    }
    out.push_str("</tr></table>\n");

    // Tabla de aspectos.
    out.push_str("<h2>Aspectos</h2>\n");
    if rows.is_empty() {
        out.push_str("<p class=\"meta\">Sin aspectos calculados.</p>\n");
    } else {
        out.push_str(
            "<table><thead><tr><th>Aspecto</th><th>Cuerpo</th><th>Cuerpo</th>\
             <th class=\"num\">Geo</th><th class=\"num\">Topo</th>\
             <th class=\"num\">Δ</th><th>Fase</th></tr></thead><tbody>\n",
        );
        for r in rows {
            let geo = r.geo.map(fmt_dms).unwrap_or_else(|| "—".into());
            let topo = r.topo.map(fmt_dms).unwrap_or_else(|| "—".into());
            let diff = match (r.geo, r.topo) {
                (Some(g), Some(t)) => format!("{:+.0}′", (t - g) * 60.0),
                _ => "—".into(),
            };
            let fase = match r.applying {
                Some(true) => "aplicando",
                Some(false) => "separando",
                None => "",
            };
            let (sym, nombre) = aspecto_es(&r.kind);
            // Aspecto desconocido: mostrar el id crudo en vez de perderlo.
            let nombre = if nombre == "—" { r.kind.as_str() } else { nombre };
            let _ = writeln!(
                out,
                "<tr><td>{sym} {nombre}</td><td>{}</td><td>{}</td>\
                 <td class=\"num\">{geo}</td><td class=\"num\">{topo}</td>\
                 <td class=\"num\">{diff}</td><td>{fase}</td></tr>",
                esc(&cuerpo_txt(&r.from, &lons)),
                esc(&cuerpo_txt(&r.to, &lons)),
            );
        }
        out.push_str("</tbody></table>\n");
    }

    out.push_str("<footer>cosmos · astronomía + astrología — hoja imprimible</footer>\n</body></html>");
    out
}

/// Abre `path` con el navegador/visor por defecto del SO. Cross-platform:
/// Linux usa `xdg-open`, macOS `open`, Windows `cmd /C start`.
fn abrir_en_navegador(path: &PathBuf) -> Result<(), String> {
    let p = path.to_string_lossy().to_string();
    let res = if cfg!(target_os = "macos") {
        Command::new("open").arg(&p).spawn()
    } else if cfg!(target_os = "windows") {
        Command::new("cmd").args(["/C", "start", "", &p]).spawn()
    } else {
        Command::new("xdg-open").arg(&p).spawn()
    };
    res.map(|_| ())
        .map_err(|e| format!("no se pudo abrir el navegador: {e} (la hoja quedó en {p})"))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn sign_unicode(name: &str) -> &'static str {
    match name {
        "aries" => "♈",
        "taurus" => "♉",
        "gemini" => "♊",
        "cancer" => "♋",
        "leo" => "♌",
        "virgo" => "♍",
        "libra" => "♎",
        "scorpio" => "♏",
        "sagittarius" => "♐",
        "capricorn" => "♑",
        "aquarius" => "♒",
        "pisces" => "♓",
        _ => "",
    }
}

fn planet_unicode(name: &str) -> &'static str {
    match name {
        "sun" => "☉",
        "moon" => "☽",
        "mercury" => "☿",
        "venus" => "♀",
        "mars" => "♂",
        "jupiter" => "♃",
        "saturn" => "♄",
        "uranus" => "♅",
        "neptune" => "♆",
        "pluto" => "♇",
        "north_node" | "mean_node" | "ascending_node" => "☊",
        "south_node" | "descending_node" => "☋",
        "chiron" => "⚷",
        "lilith" => "⚸",
        "ceres" => "⚳",
        "pallas" => "⚴",
        "juno" => "⚵",
        "vesta" => "⚶",
        _ => "•",
    }
}

/// (símbolo, nombre en español) de un tipo de aspecto.
fn aspecto_es(kind: &str) -> (&'static str, &'static str) {
    match kind {
        "conjunction" => ("☌", "Conjunción"),
        "opposition" => ("☍", "Oposición"),
        "trine" => ("△", "Trígono"),
        "square" => ("□", "Cuadratura"),
        "sextile" => ("⚹", "Sextil"),
        "quincunx" | "inconjunct" => ("⚻", "Quincuncio"),
        "semisextile" => ("⚺", "Semisextil"),
        "semisquare" => ("∠", "Semicuadratura"),
        "sesquisquare" | "sesquiquadrate" => ("⚼", "Sesquicuadratura"),
        "quintile" => ("Q", "Quintil"),
        "biquintile" => ("bQ", "Biquintil"),
        _ => ("•", "—"),
    }
}
