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
    /// The fixed mundane position of an angle, for the cases where the
    /// significator is one of the four cardinal points.
    fn angle_mundane(self) -> Option<f64> {
        match self {
            Significator::Ascendant => Some(0.0),
            Significator::Midheaven => Some(1.0),
            Significator::Descendant => Some(2.0),
            Significator::ImumCoeli => Some(3.0),
            Significator::Body(_) => None,
        }
    }

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

/// Compute a single direction from `promissor` to `significator`.
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
    let _ = method; // Only PlacidusMundane today, but the parameter is honoured.

    let placement = natal.placement(promissor).ok_or_else(|| {
        crate::error::AstrologyError::BodyUnavailable(format!(
            "{} not in chart",
            promissor.name()
        ))
    })?;

    let phi = natal.birth.observer.lat_rad;
    let ramc = natal.local_apparent_sidereal_time_rad;
    let dsa_p = diurnal_semi_arc_rad(placement.declination_rad, phi);
    let nsa_p = nocturnal_semi_arc_rad(placement.declination_rad, phi);
    if dsa_p.is_nan() || nsa_p.is_nan() {
        return Err(crate::error::AstrologyError::HouseSystemUnavailable(
            "promissor is circumpolar at the observer's latitude — \
             primary directions undefined",
        ));
    }
    let h_p_natal = signed_hour_angle_rad(ramc, placement.right_ascension_rad);

    // Target mundane position.
    let m_target = match significator {
        Significator::Body(target_body) => {
            if let Some(target_p) = natal.placement(target_body) {
                natal_mundane_position(
                    ramc,
                    target_p.right_ascension_rad,
                    target_p.declination_rad,
                    phi,
                )
            } else {
                return Err(crate::error::AstrologyError::BodyUnavailable(format!(
                    "significator {} not in chart",
                    target_body.name()
                )));
            }
        }
        _ => significator
            .angle_mundane()
            .expect("non-Body significator must have a fixed mundane"),
    };

    // Hour angle the promissor needs to have so its NEW mundane (computed
    // with its OWN semi-arcs) equals `m_target`.
    let h_target = hour_angle_for_mundane(m_target, dsa_p, nsa_p);

    // Arc of direction in RA: how far must the sphere rotate so the
    // promissor's hour angle changes from `h_p_natal` to `h_target`?
    // The promissor's RA is fixed (the natal ecliptic position is
    // frozen), so `h_new = h_natal + Δ_RA`.
    let raw_arc = h_target - h_p_natal;
    let arc_rad = normalise_forward(raw_arc);

    let age_years = arc_rad.to_degrees() / key.degrees_per_year();
    Ok(Direction {
        promissor,
        significator,
        method,
        key,
        arc_rad,
        age_years,
    })
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
pub fn all_directions(
    natal: &NatalChart,
    method: DirectionMethod,
    key: DirectionKey,
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
            if let Ok(d) = direct(natal, promissor, sig, method, key) {
                if d.age_years <= max_age_years && d.age_years >= 0.0 {
                    out.push(d);
                }
            }
        }
        // To every other body.
        for sig_p in &natal.placements {
            if sig_p.body == promissor {
                continue;
            }
            if let Ok(d) = direct(
                natal,
                promissor,
                Significator::Body(sig_p.body),
                method,
                key,
            ) {
                if d.age_years <= max_age_years && d.age_years >= 0.0 {
                    out.push(d);
                }
            }
        }
    }

    out.sort_by(|a, b| a.age_years.partial_cmp(&b.age_years).unwrap_or(std::cmp::Ordering::Equal));
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
