//! Helpers de formateo de longitudes y códigos de cuerpo/aspecto.
//!
//! **Por qué letras y no unicode** (☉☽☿… ☌☍△□⚹): las fuentes default del
//! sistema (LiberationSans, AdwaitaSans) no traen `U+2609..U+265F`, así que
//! cualquier glyph astrológico cae como `.notdef`. En el wheel ya se dibujan
//! como path (`cosmos_render::glyphs`); acá son texto plano en filas, así que
//! usamos códigos cortos.

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

