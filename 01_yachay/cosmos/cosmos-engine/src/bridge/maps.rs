//! Traducciones `Stored*`/agnóstico → tipos eternales + símbolos.

use super::*;

// =====================================================================
// Traducciones Stored* → eternal
// =====================================================================

pub(crate) fn map_house_system(h: HouseSystem) -> EHouseSystem {
    match h {
        HouseSystem::Placidus => EHouseSystem::Placidus,
        HouseSystem::Koch => EHouseSystem::Koch,
        HouseSystem::Regiomontanus => EHouseSystem::Regiomontanus,
        HouseSystem::Campanus => EHouseSystem::Campanus,
        HouseSystem::Porphyry => EHouseSystem::Porphyry,
        HouseSystem::Equal => EHouseSystem::Equal,
        HouseSystem::WholeSign => EHouseSystem::WholeSign,
    }
}

pub(crate) fn map_zodiac(z: Zodiac, ayanamsha_hint: Option<&str>) -> EZodiac {
    match z {
        Zodiac::Tropical => EZodiac::Tropical,
        Zodiac::Sidereal => {
            let mode = match ayanamsha_hint.unwrap_or("lahiri").to_ascii_lowercase().as_str() {
                "fagan_bradley" | "fagan-bradley" | "faganbradley" => Ayanamsha::FaganBradley,
                "raman" => Ayanamsha::Raman,
                "krishnamurti" => Ayanamsha::Krishnamurti,
                "de_luce" | "deluce" => Ayanamsha::DeLuce,
                "djwhal_khul" | "djwhalkhul" => Ayanamsha::DjwhalKhul,
                "ushashashi" => Ayanamsha::Ushashashi,
                "yukteshwar" => Ayanamsha::Yukteshwar,
                _ => Ayanamsha::Lahiri,
            };
            EZodiac::Sidereal(mode)
        }
        // Dracónico aún no soportado en eternal — caemos a tropical por
        // ahora; cuando eternal lo agregue, lo cableamos acá.
        Zodiac::Draconic => EZodiac::Tropical,
    }
}

pub(crate) fn map_body_set(cfg: &StoredChartConfig) -> BodySet {
    let mut bodies: Vec<Body> = Vec::new();
    for name in &cfg.bodies {
        if let Some(b) = map_body(name) {
            bodies.push(b);
        }
    }
    if bodies.is_empty() {
        // Default razonable si el config vino vacío.
        return BodySet::classical_modern();
    }
    let mut set = BodySet {
        bodies,
        include_south_node: cfg.include_south_node,
    };
    if cfg.include_lilith {
        set = set.with_lilith();
    }
    if cfg.include_main_belt_asteroids {
        set = set.with_main_belt_asteroids();
    }
    set
}

pub(crate) fn map_body(name: &str) -> Option<Body> {
    Some(match name.to_ascii_lowercase().as_str() {
        "sun" => Body::Sun,
        "moon" => Body::Moon,
        "mercury" => Body::Mercury,
        "venus" => Body::Venus,
        "mars" => Body::Mars,
        "jupiter" => Body::Jupiter,
        "saturn" => Body::Saturn,
        "uranus" => Body::Uranus,
        "neptune" => Body::Neptune,
        "pluto" => Body::Pluto,
        "mean_node" | "meannode" => Body::MeanNode,
        "true_node" | "truenode" => Body::TrueNode,
        "mean_lilith" | "lilith" => Body::MeanLilith,
        "true_lilith" => Body::TrueLilith,
        "ceres" => Body::Ceres,
        "pallas" => Body::Pallas,
        "juno" => Body::Juno,
        "vesta" => Body::Vesta,
        _ => return None,
    })
}

pub(crate) fn body_symbol(b: Body) -> &'static str {
    match b {
        Body::Sun => "sun",
        Body::Moon => "moon",
        Body::Mercury => "mercury",
        Body::Venus => "venus",
        Body::Mars => "mars",
        Body::Jupiter => "jupiter",
        Body::Saturn => "saturn",
        Body::Uranus => "uranus",
        Body::Neptune => "neptune",
        Body::Pluto => "pluto",
        Body::MeanNode => "north_node",
        Body::TrueNode => "north_node",
        Body::MeanLilith => "lilith",
        Body::TrueLilith => "lilith",
        Body::Ceres => "ceres",
        Body::Pallas => "pallas",
        Body::Juno => "juno",
        Body::Vesta => "vesta",
        Body::Chiron => "chiron",
        Body::Pholus => "chiron",
        Body::Eris => "chiron",
        Body::Sedna => "chiron",
        // `Body` es `#[non_exhaustive]` — cualquier cuerpo nuevo
        // upstream cae al símbolo de fallback hasta que lo cableemos.
        _ => "custom",
    }
}

pub(crate) fn aspect_kind_id(k: EAspectKind) -> &'static str {
    match k {
        EAspectKind::Conjunction => "conjunction",
        EAspectKind::Opposition => "opposition",
        EAspectKind::Trine => "trine",
        EAspectKind::Square => "square",
        EAspectKind::Sextile => "sextile",
        EAspectKind::Quincunx => "quincunx",
        EAspectKind::SemiSextile => "semi_sextile",
        EAspectKind::SemiSquare => "semi_square",
        EAspectKind::Sesquiquadrate => "sesquiquadrate",
        EAspectKind::Quintile => "quintile",
        EAspectKind::BiQuintile => "biquintile",
        EAspectKind::Septile => "septile",
    }
}
