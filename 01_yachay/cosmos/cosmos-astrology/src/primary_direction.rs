//! Primary Directions — the "diurnal" forecasting motor.
//!
//! Primary directions are the oldest astrological forecasting method:
//! after birth the celestial sphere continues to rotate, and points
//! that started in particular positions eventually rotate to meet
//! other natal points. The *arc* covered by the rotation, expressed in
//! equatorial degrees, is translated to *years of life* by a "key":
//!
//! * **Ptolemy**: 1° of RA = 1 year (the original classical key).
//! * **Naibod**: 0°59'08.33"/year (≈ 1.0146 years/°) — the Sun's mean
//!   daily motion, more astronomically grounded.
//! * **Brahe / Placidus / others**: variants on the Sun's true motion
//!   year by year; not implemented in this first cut.
//!
//! Two natal points are involved:
//!
//! * **Promissor (P)** — the "moving" point. As the sphere rotates,
//!   the natal P's mundane position changes.
//! * **Significator (S)** — the "fixed" target. Its natal mundane
//!   position is the goalpost the promissor must reach.
//!
//! The **arc of direction** is the equatorial angle the sphere must
//! rotate so that `m_P(new) = m_S(natal)`, where `m` is the Placidus
//! mundane coordinate (see [`crate::mundane`]). Years follow by the
//! key conversion.
//!
//! This module ships the **Placidus mundane** method, which is the
//! standard. Regiomontanus and Campanus variants reuse the same
//! framework with different mundane formulas and are scoped for a
//! follow-up.

use eternal_sky::Body;

use crate::aspect::AspectKind;
use crate::chart::NatalChart;
use crate::error::AstrologyResult;
use crate::mundane::{
    diurnal_semi_arc_rad, hour_angle_for_mundane, natal_mundane_position,
    nocturnal_semi_arc_rad, signed_hour_angle_rad, wrap_mundane,
};

/// Time-arc conversion. Pick one when configuring a direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectionKey {
    /// 1° of right ascension = 1 year of life.
    Ptolemy,
    /// 0°59'08.33"/year — the Sun's mean daily motion. The most
    /// commonly used modern key.
    Naibod,
}

impl DirectionKey {
    /// Degrees of right ascension that correspond to one year of life
    /// under this key.
    pub fn degrees_per_year(self) -> f64 {
        match self {
            DirectionKey::Ptolemy => 1.0,
            DirectionKey::Naibod => 0.985_647_3,
        }
    }
}

/// Which directional method to use. Today only Placidus-mundane is
/// implemented; the variant is kept here as a forward-compatible enum
/// so callers don't need to rewire when more methods land.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DirectionMethod {
    /// Placidus mundane: the promissor must reach the significator's
    /// natal Placidus quadrant position.
    #[default]
    PlacidusMundane,
}

/// A "significator" target — either a natal body or an angle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Significator {
    Body(Body),
    Ascendant,
    Midheaven,
    Descendant,
    ImumCoeli,
}

impl Significator {
    pub fn label(self) -> String {
        match self {
            Significator::Body(b) => b.name().to_string(),
            Significator::Ascendant => "Ascendant".into(),
            Significator::Midheaven => "Midheaven".into(),
            Significator::Descendant => "Descendant".into(),
            Significator::ImumCoeli => "Imum Coeli".into(),
        }
    }
}

/// A computed primary direction.
#[derive(Debug, Clone, Copy)]
pub struct Direction {
    pub promissor: Body,
    pub significator: Significator,
    /// Which aspect family this direction targets. `Conjunction` means
    /// the promissor reaches the significator's natal mundane position
    /// directly; other aspects target the corresponding aspect points
    /// (the ecliptic longitudes `significator ± aspect.exact_angle`).
    pub aspect: AspectKind,
    pub method: DirectionMethod,
    pub key: DirectionKey,
    /// Arc of direction, radians. Always normalised to `[0, 2π)` —
    /// the *next* forward rotation that brings the promissor to the
    /// significator's mundane position. Negative-arc (converse)
    /// directions are not produced by this module today.
    pub arc_rad: f64,
    /// Years of life at which the direction perfects. Arc translated
    /// by the chosen key.
    pub age_years: f64,
}

impl Direction {
    pub fn arc_deg(&self) -> f64 {
        self.arc_rad.to_degrees()
    }
}

/// Compute a single conjunctional direction from `promissor` to
/// `significator`.
///
/// Equivalent to [`direct_to_aspect`] with `AspectKind::Conjunction`,
/// unwrapping the single-element result for ergonomics.
///
/// `Err` is returned if the promissor has no real semi-arc at the
/// observer's latitude (circumpolar / never-rising case).
pub fn direct(
    natal: &NatalChart,
    promissor: Body,
    significator: Significator,
    method: DirectionMethod,
    key: DirectionKey,
) -> AstrologyResult<Direction> {
    let mut out = direct_to_aspect(
        natal,
        promissor,
        significator,
        AspectKind::Conjunction,
        method,
        key,
    )?;
    Ok(out.remove(0))
}

/// Compute every direction of `promissor` to `significator` for the
/// given aspect family. Returns one direction for Conjunction and
/// Opposition (the aspect is symmetric and lands on a unique ecliptic
/// point); two for every other family — one for the "dexter" branch
/// (`significator − exact_angle`) and one for the "sinister" branch
/// (`significator + exact_angle`).
///
/// The promissor still reaches the *mundane* position of each aspect
/// point in the Placidus framework — i.e. the rotation arc is in
/// equatorial degrees, not zodiacal.
pub fn direct_to_aspect(
    natal: &NatalChart,
    promissor: Body,
    significator: Significator,
    aspect: AspectKind,
    method: DirectionMethod,
    key: DirectionKey,
) -> AstrologyResult<Vec<Direction>> {
    let _ = method; // Only PlacidusMundane today, but the parameter is honoured.

    let placement = natal.placement(promissor).ok_or_else(|| {
        crate::error::AstrologyError::BodyUnavailable(format!(
            "{} not in chart",
            promissor.name()
        ))
    })?;

    let phi = natal.birth.observer.lat_rad;
    let ramc = natal.local_apparent_sidereal_time_rad;
    let obliquity = natal.obliquity_rad;
    let dsa_p = diurnal_semi_arc_rad(placement.declination_rad, phi);
    let nsa_p = nocturnal_semi_arc_rad(placement.declination_rad, phi);
    if dsa_p.is_nan() || nsa_p.is_nan() {
        return Err(crate::error::AstrologyError::HouseSystemUnavailable(
            "promissor is circumpolar at the observer's latitude — \
             primary directions undefined",
        ));
    }
    let h_p_natal = signed_hour_angle_rad(ramc, placement.right_ascension_rad);

    // The natal ecliptic longitude of the significator, and a marker
    // for whether it is an angle (which has no defined ecliptic
    // latitude — we treat all aspect points as ecliptic-latitude zero).
    let sig_lon_rad = match significator {
        Significator::Body(target_body) => {
            let target_p = natal.placement(target_body).ok_or_else(|| {
                crate::error::AstrologyError::BodyUnavailable(format!(
                    "significator {} not in chart",
                    target_body.name()
                ))
            })?;
            // Convert back to *tropical* ecliptic longitude (placement
            // stores it in chart's zodiac — add ayanamsha to undo
            // sidereal offset if applicable).
            (target_p.longitude.longitude_rad() + natal.ayanamsha_rad)
                .rem_euclid(std::f64::consts::TAU)
        }
        Significator::Ascendant => (natal.ascendant().longitude_rad() + natal.ayanamsha_rad)
            .rem_euclid(std::f64::consts::TAU),
        Significator::Midheaven => (natal.midheaven().longitude_rad() + natal.ayanamsha_rad)
            .rem_euclid(std::f64::consts::TAU),
        Significator::Descendant => (natal.descendant().longitude_rad() + natal.ayanamsha_rad)
            .rem_euclid(std::f64::consts::TAU),
        Significator::ImumCoeli => (natal.imum_coeli().longitude_rad() + natal.ayanamsha_rad)
            .rem_euclid(std::f64::consts::TAU),
    };

    // Build each aspect branch's mundane target.
    let offsets_deg = aspect_branch_offsets_deg(aspect);
    let mut out = Vec::with_capacity(offsets_deg.len());
    for offset_deg in offsets_deg {
        let aspect_point_lon = (sig_lon_rad + offset_deg.to_radians())
            .rem_euclid(std::f64::consts::TAU);
        let (target_ra, target_dec) =
            ecliptic_to_equatorial(aspect_point_lon, 0.0, obliquity);
        let m_target = natal_mundane_position(ramc, target_ra, target_dec, phi);

        let h_target = hour_angle_for_mundane(m_target, dsa_p, nsa_p);
        let arc_rad = normalise_forward(h_target - h_p_natal);
        let age_years = arc_rad.to_degrees() / key.degrees_per_year();
        out.push(Direction {
            promissor,
            significator,
            aspect,
            method,
            key,
            arc_rad,
            age_years,
        });
    }
    Ok(out)
}

/// Offsets (in degrees) at which the aspect family lands relative to
/// the significator's natal longitude. Conjunction → `[0]`; opposition
/// → `[180]`; symmetric aspects → both `+exact` and `−exact`.
fn aspect_branch_offsets_deg(aspect: AspectKind) -> Vec<f64> {
    let exact = aspect.exact_angle_deg();
    match aspect {
        AspectKind::Conjunction => vec![0.0],
        AspectKind::Opposition => vec![180.0],
        _ => vec![exact, -exact],
    }
}

/// Convert an ecliptic longitude / latitude (radians) to equatorial
/// (RA, Dec) at the given true obliquity. Standard textbook formula —
/// kept inline so the primary-direction module is self-contained.
fn ecliptic_to_equatorial(lon: f64, lat: f64, obliquity: f64) -> (f64, f64) {
    let (sin_lon, cos_lon) = libm::sincos(lon);
    let (sin_lat, cos_lat) = libm::sincos(lat);
    let (sin_eps, cos_eps) = libm::sincos(obliquity);
    let sin_dec = sin_lat * cos_eps + cos_lat * sin_eps * sin_lon;
    let dec = libm::asin(sin_dec);
    let ra = libm::atan2(
        sin_lon * cos_eps - libm::tan(lat) * sin_eps,
        cos_lon,
    );
    let ra = if ra < 0.0 {
        ra + std::f64::consts::TAU
    } else {
        ra
    };
    (ra, dec)
}

/// Compute directions of `promissor` to each of the four angles.
pub fn directions_to_angles(
    natal: &NatalChart,
    promissor: Body,
    method: DirectionMethod,
    key: DirectionKey,
) -> AstrologyResult<[Direction; 4]> {
    Ok([
        direct(natal, promissor, Significator::Ascendant, method, key)?,
        direct(natal, promissor, Significator::Midheaven, method, key)?,
        direct(natal, promissor, Significator::Descendant, method, key)?,
        direct(natal, promissor, Significator::ImumCoeli, method, key)?,
    ])
}

/// Compute every conjunctional direction (each natal body as promissor
/// to each angle and to each other body) whose perfection lies within
/// `max_age_years`. Sorted by `age_years` ascending.
///
/// Use [`all_directions_with_aspects`] to also include non-conjunction
/// aspects (trine, square, sextile, …).
pub fn all_directions(
    natal: &NatalChart,
    method: DirectionMethod,
    key: DirectionKey,
    max_age_years: f64,
) -> Vec<Direction> {
    all_directions_with_aspects(
        natal,
        method,
        key,
        &[AspectKind::Conjunction],
        max_age_years,
    )
}

/// Compute every direction across the requested aspect families
/// (promissor × significator × aspect) that perfects within
/// `max_age_years`. Sorted by `age_years` ascending.
///
/// `aspect_kinds` selects which aspect families to include. Pass
/// `AspectKind::MAJORS` for the classical five, or `AspectKind::ALL`
/// for every wired aspect.
pub fn all_directions_with_aspects(
    natal: &NatalChart,
    method: DirectionMethod,
    key: DirectionKey,
    aspect_kinds: &[AspectKind],
    max_age_years: f64,
) -> Vec<Direction> {
    let mut out: Vec<Direction> = Vec::new();

    for promissor_p in &natal.placements {
        let promissor = promissor_p.body;
        // To the four angles.
        for sig in [
            Significator::Ascendant,
            Significator::Midheaven,
            Significator::Descendant,
            Significator::ImumCoeli,
        ] {
            for &aspect in aspect_kinds {
                if let Ok(dirs) = direct_to_aspect(natal, promissor, sig, aspect, method, key) {
                    for d in dirs {
                        if (0.0..=max_age_years).contains(&d.age_years) {
                            out.push(d);
                        }
                    }
                }
            }
        }
        // To every other body.
        for sig_p in &natal.placements {
            if sig_p.body == promissor {
                continue;
            }
            for &aspect in aspect_kinds {
                if let Ok(dirs) = direct_to_aspect(
                    natal,
                    promissor,
                    Significator::Body(sig_p.body),
                    aspect,
                    method,
                    key,
                ) {
                    for d in dirs {
                        if (0.0..=max_age_years).contains(&d.age_years) {
                            out.push(d);
                        }
                    }
                }
            }
        }
    }

    out.sort_by(|a, b| {
        a.age_years
            .partial_cmp(&b.age_years)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

/// Normalise an arc into the *forward* half-line `[0, 2π)`. Backward
/// arcs (converse) wrap to their forward equivalents; users who care
/// about the distinction should compare against the natural arc.
fn normalise_forward(arc: f64) -> f64 {
    let _ = wrap_mundane; // silence unused-import in absence of converse mode
    let v = arc.rem_euclid(std::f64::consts::TAU);
    if v < 0.0 {
        v + std::f64::consts::TAU
    } else {
        v
    }
}
