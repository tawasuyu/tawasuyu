//! Helpers de formateo de longitudes y códigos de cuerpo/aspecto.
//!
//! **Por qué letras y no unicode** (☉☽☿… ☌☍△□⚹): las fuentes default del
//! sistema (LiberationSans, AdwaitaSans) no traen `U+2609..U+265F`, así que
//! cualquier glyph astrológico cae como `.notdef`. En el wheel ya se dibujan
//! como path (`cosmos_render::glyphs`); acá son texto plano en filas, así que
//! usamos códigos cortos.

// Códigos alfabéticos de 3 letras (español) para los signos. Mismo motivo
// que `simbolo_cuerpo`/`simbolo_aspecto`: los unicode `♈..♓` son ilegibles
// como texto en una fila (y caían como `.notdef` en fuentes sin el bloque
// astrológico). En el wheel sí van como path (`cosmos_render::glyphs`).
const SIGNOS: [&str; 12] = [
    "Ari", "Tau", "Gem", "Can", "Leo", "Vir", "Lib", "Esc", "Sag", "Cap", "Acu", "Pis",
];

pub(crate) fn signo_de_longitud(deg: f32) -> &'static str {
    SIGNOS[((deg.rem_euclid(360.0) / 30.0) as usize) % 12]
}

pub(crate) fn fmt_deg_sign(deg: f32) -> String {
    let dms = fmt_dms((deg.rem_euclid(30.0)) as f64);
    format!("{dms} {}", signo_de_longitud(deg))
}

pub(crate) fn fmt_dms(deg: f64) -> String {
    let total_min = (deg.abs() * 60.0).round() as i64;
    let d = total_min / 60;
    let m = total_min % 60;
    format!("{:>2}°{:02}'", d, m)
}

/// Códigos alfabéticos para mostrar cuerpos en los tiles del sidebar.
pub(crate) fn simbolo_cuerpo(s: &str) -> &'static str {
    match s {
        "sun" => "Sol",
        "moon" => "Lun",
        "mercury" => "Mer",
        "venus" => "Ven",
        "mars" => "Mar",
        "jupiter" => "Jup",
        "saturn" => "Sat",
        "uranus" => "Ura",
        "neptune" => "Nep",
        "pluto" => "Plu",
        "earth" => "Tie",
        "north_node" | "ascending_node" => "NoN",
        "south_node" | "descending_node" => "NoS",
        "lilith" => "Lil",
        "chiron" => "Qui",
        "mean_node" => "NoN",
        "asc" => "Asc",
        "desc" => "Dsc",
        "mc" => "MC",
        "ic" => "IC",
        _ => "·",
    }
}

/// Códigos alfabéticos para tipos de aspecto. Mismo motivo que
/// [`simbolo_cuerpo`] — los unicode ☌☍△□⚹ no rendean.
pub(crate) fn simbolo_aspecto(s: &str) -> &'static str {
    match s {
        "conjunction" => "con",
        "opposition" => "opp",
        "trine" => "tri",
        "square" => "cua",
        "sextile" => "sex",
        "quincunx" => "qui",
        "semi_sextile" => "ssx",
        "semi_square" => "scu",
        "sesquiquadrate" => "scq",
        _ => "·",
    }
}
