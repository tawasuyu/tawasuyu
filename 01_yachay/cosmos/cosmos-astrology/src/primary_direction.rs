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

use cosmos_sky::Body;

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

/// Which directional method to use. The three classical mundane
/// frameworks differ only in how the "house position" `m ∈ [0, 4)` is
/// projected from a body's (RA, Dec).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DirectionMethod {
    /// Placidus mundane: position = proportional position within the
    /// body's own diurnal/nocturnal semi-arc. The dominant choice in
    /// modern practice.
    #[default]
    PlacidusMundane,
    /// Regiomontanus mundane: position depends only on hour angle —
    /// the framework is anchored to the celestial equator and the
    /// poles of the world, so every body of any declination shares
    /// the same `m(H)` function. As a consequence the arc of
    /// direction between any two points reduces to a pure RA delta.
    Regiomontanus,
    /// Campanus mundane: position is the angle along the prime
    /// vertical at which the great circle through (N, body, S)
    /// crosses. The framework is anchored to the observer's horizon —
    /// East horizon = m=0, zenith = m=1 (MC slot), West horizon =
    /// m=2, nadir = m=3.
    Campanus,
}

impl DirectionMethod {
    /// Compute the natal mundane position `m ∈ [0, 4)` of a target
    /// point with the given equatorial coordinates, at the observer's
    /// latitude, with the chart's RAMC.
    fn mundane_position_for(
        self,
        ramc_rad: f64,
        ra_rad: f64,
        dec_rad: f64,
        lat_rad: f64,
    ) -> f64 {
        match self {
            DirectionMethod::PlacidusMundane => {
                natal_mundane_position(ramc_rad, ra_rad, dec_rad, lat_rad)
            }
            DirectionMethod::Regiomontanus => {
                let h = signed_hour_angle_rad(ramc_rad, ra_rad);
                // m = 1 + H · (2/π), wrapped into [0, 4).
                let m = 1.0 + h * 2.0 / std::f64::consts::PI;
                wrap_mundane(m)
            }
            DirectionMethod::Campanus => campanus_mundane_position(
                ramc_rad, ra_rad, dec_rad, lat_rad,
            ),
        }
    }

    /// Given a target mundane position `m_target` and the promissor's
    /// declination, return the hour angle the promissor needs to have
    /// so that its mundane position (under this method) equals
    /// `m_target`.
    fn hour_angle_for_target(
        self,
        m_target: f64,
        promissor_dec_rad: f64,
        lat_rad: f64,
    ) -> f64 {
        match self {
            DirectionMethod::PlacidusMundane => {
                let dsa = diurnal_semi_arc_rad(promissor_dec_rad, lat_rad);
                let nsa = nocturnal_semi_arc_rad(promissor_dec_rad, lat_rad);
                hour_angle_for_mundane(m_target, dsa, nsa)
            }
            DirectionMethod::Regiomontanus => {
                // Inverse of `m = 1 + H · (2/π)`.
                (m_target - 1.0) * std::f64::consts::PI / 2.0
            }
            DirectionMethod::Campanus => {
                campanus_hour_angle_for_target(m_target, promissor_dec_rad, lat_rad)
            }
        }
    }
}

/// Campanus mundane position of a body at `(RA, Dec)` for an observer
/// at latitude `φ` with chart RAMC. Result in `[0, 4)`.
///
/// The body's local horizontal Cartesian:
/// * `y_local = -cos(δ) · sin(H)`  (east component)
/// * `z_local =  cos(δ) · cos(H) · cos(φ) + sin(δ) · sin(φ)`  (up)
///
/// Then `θ = atan2(z, y)` is the Campanus angle along the prime
/// vertical, and `m_Camp = θ · (2/π)` wrapped to `[0, 4)`.
fn campanus_mundane_position(
    ramc_rad: f64,
    ra_rad: f64,
    dec_rad: f64,
    lat_rad: f64,
) -> f64 {
    let h = signed_hour_angle_rad(ramc_rad, ra_rad);
    let cos_dec = libm::cos(dec_rad);
    let sin_dec = libm::sin(dec_rad);
    let cos_lat = libm::cos(lat_rad);
    let sin_lat = libm::sin(lat_rad);
    let cos_h = libm::cos(h);
    let sin_h = libm::sin(h);

    let y = -cos_dec * sin_h;
    let z = cos_dec * cos_h * cos_lat + sin_dec * sin_lat;
    let theta = libm::atan2(z, y);
    let theta = if theta < 0.0 {
        theta + std::f64::consts::TAU
    } else {
        theta
    };
    wrap_mundane(theta * 2.0 / std::f64::consts::PI)
}

/// Inverse of [`campanus_mundane_position`]: given the target Campanus
/// position `m_target` and the promissor's declination + observer
/// latitude, return the hour angle the promissor needs to occupy.
///
/// Solves `A · cos(H) + B · sin(H) = C` where:
/// * `A = cos(φ) · cos(θ)`, `B = sin(θ)`, `C = -tan(δ_p) · sin(φ) · cos(θ)`,
/// * `θ = m_target · (π/2)`.
///
/// Two algebraic solutions exist; we pick the one whose `(y, z)` sign
/// places the body in the correct prime-vertical quadrant (i.e. the
/// one for which `z·sin(θ) + y·cos(θ)` is positive — the analogue of
/// `r` in `atan2(z, y) = θ`).
fn campanus_hour_angle_for_target(
    m_target: f64,
    dec_rad: f64,
    lat_rad: f64,
) -> f64 {
    let theta = m_target * std::f64::consts::PI / 2.0;
    let cos_dec = libm::cos(dec_rad);
    let sin_dec = libm::sin(dec_rad);
    let cos_lat = libm::cos(lat_rad);
    let sin_lat = libm::sin(lat_rad);
    let cos_theta = libm::cos(theta);
    let sin_theta = libm::sin(theta);

    // Degenerate cases.
    if libm::fabs(cos_dec) < 1.0e-15 {
        // Body essentially on a celestial pole — never moves through
        // a Campanus cycle. Return 0.
        return 0.0;
    }

    let a = cos_lat * cos_theta;
    let b = sin_theta;
    let c = -libm::tan(dec_rad) * sin_lat * cos_theta;

    let r = libm::sqrt(a * a + b * b);
    if r < 1.0e-15 {
        return 0.0;
    }
    let argument = c / r;
    if argument.abs() > 1.0 {
        // No real solution at this latitude/declination combination —
        // body cannot reach the requested Campanus position.
        return f64::NAN;
    }

    let psi = libm::atan2(b, a);
    let delta = libm::acos(argument);
    let h_plus = psi + delta;
    let h_minus = psi - delta;

    let check = |h: f64| -> f64 {
        let y = -cos_dec * libm::sin(h);
        let z = cos_dec * libm::cos(h) * cos_lat + sin_dec * sin_lat;
        z * sin_theta + y * cos_theta
    };
    if check(h_plus) >= check(h_minus) {
        h_plus
    } else {
        h_minus
    }
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
    /// Natal zodiacal longitude of this significator in radians.
    /// Returns `None` only when the significator refers to a body not
    /// present in the chart's [`crate::BodySet`].
    pub fn longitude_rad(self, natal: &NatalChart) -> Option<f64> {
        match self {
            Significator::Body(b) => Some(natal.placement(b)?.longitude.longitude_rad()),
            Significator::Ascendant => Some(natal.ascendant().longitude_rad()),
            Significator::Midheaven => Some(natal.midheaven().longitude_rad()),
            Significator::Descendant => Some(natal.descendant().longitude_rad()),
            Significator::ImumCoeli => Some(natal.imum_coeli().longitude_rad()),
        }
    }

    /// Natal zodiacal longitude in degrees `[0, 360)`.
    pub fn longitude_deg(self, natal: &NatalChart) -> Option<f64> {
        self.longitude_rad(natal).map(f64::to_degrees)
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

/// Direct vs Converse — sentido de la rotación que mueve el promissor.
/// Direct = forward in time (la esfera sigue rotando después del
/// nacimiento). Converse = backward in time (rotación simétrica
/// inversa, como si fuera "el tiempo desplegándose al revés"). En la
/// escuela GR las conversas se usan en paralelo con las directas para
/// rectificar: un mismo evento debería aparecer en ambos rings con
/// arcos consistentes si la hora natal es correcta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PrimaryDirection {
    #[default]
    Direct,
    Converse,
}

/// Proyecta un cuerpo natal según las direcciones primarias a la edad
/// dada. Convierte la edad a un arco en RA usando `key`
/// (Ptolemy/Naibod), aplica la rotación al RA natal del cuerpo
/// (manteniendo su declinación constante — convención clásica), y
/// devuelve la nueva longitud eclíptica proyectada sobre la
/// eclíptica de la fecha.
///
/// Es el cómputo "moderno" de direcciones primarias usado para
/// visualización en time-scrubbing: en lugar de buscar el evento de
/// llegada a un significator, devuelve directamente "dónde está el
/// cuerpo natal después de N años de rotación diurna".
pub fn directed_longitude(
    natal_ra_rad: f64,
    natal_dec_rad: f64,
    age_years: f64,
    direction: PrimaryDirection,
    key: DirectionKey,
    obliquity_rad: f64,
) -> f64 {
    let arc_rad = (age_years * key.degrees_per_year()).to_radians();
    let sign = match direction {
        PrimaryDirection::Direct => 1.0,
        PrimaryDirection::Converse => -1.0,
    };
    let new_ra = (natal_ra_rad + sign * arc_rad).rem_euclid(std::f64::consts::TAU);
    // RA + Dec → longitud eclíptica de fecha. Declinación fija
    // (la rotación diurna no la cambia para un punto natal).
    let (sin_ra, cos_ra) = libm::sincos(new_ra);
    let (sin_dec, cos_dec) = libm::sincos(natal_dec_rad);
    let (sin_eps, cos_eps) = libm::sincos(obliquity_rad);
    let lon = libm::atan2(
        sin_dec * sin_eps + cos_dec * cos_eps * sin_ra,
        cos_dec * cos_ra,
    );
    lon.rem_euclid(std::f64::consts::TAU)
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
    let placement = natal.placement(promissor).ok_or_else(|| {
        crate::error::AstrologyError::BodyUnavailable(format!(
            "{} not in chart",
            promissor.name()
        ))
    })?;
    direct_to_aspect_with_placement(natal, placement, significator, aspect, method, key)
}

/// Same as [`direct_to_aspect`] but accepts a pre-resolved
/// [`BodyPlacement`] reference. Used by [`all_directions_with_aspects`]
/// to skip the body lookup in the inner loop.
fn direct_to_aspect_with_placement(
    natal: &NatalChart,
    placement: &crate::placement::BodyPlacement,
    significator: Significator,
    aspect: AspectKind,
    method: DirectionMethod,
    key: DirectionKey,
) -> AstrologyResult<Vec<Direction>> {
    let promissor = placement.body;
    let phi = natal.birth.observer.lat_rad;
    let ramc = natal.local_apparent_sidereal_time_rad;
    let obliquity = natal.obliquity_rad;
    // Placidus needs real semi-arcs at the promissor's declination;
    // Regiomontanus and Campanus do not (they don't depend on the
    // semi-arc construction). Skip the circumpolar guard for those.
    if matches!(method, DirectionMethod::PlacidusMundane) {
        let dsa_p = diurnal_semi_arc_rad(placement.declination_rad, phi);
        let nsa_p = nocturnal_semi_arc_rad(placement.declination_rad, phi);
        if dsa_p.is_nan() || nsa_p.is_nan() {
            return Err(crate::error::AstrologyError::HouseSystemUnavailable(
                "promissor is circumpolar at the observer's latitude — \
                 Placidus primary directions undefined",
            ));
        }
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
    //
    // Convention: a CONJUNCTION to a natal body uses the body's
    // **actual** (RA, Dec) — preserving its true ecliptic latitude
    // (this is the classical "in mundo" direction to the body). All
    // *other* aspects, and conjunctions to angles, use the zodiacal
    // projection at β=0 (the aspect point is a longitude-only
    // construct).
    let (offsets_deg, n_offsets) = aspect_branch_offsets_deg(aspect);
    let mut out = Vec::with_capacity(n_offsets);
    for &offset_deg in &offsets_deg[..n_offsets] {
        let (target_ra, target_dec) = if offset_deg == 0.0 {
            // Conjunction case.
            match significator {
                Significator::Body(b) => {
                    let p = natal.placement(b).expect("checked above");
                    (p.right_ascension_rad, p.declination_rad)
                }
                _ => ecliptic_to_equatorial(sig_lon_rad, 0.0, obliquity),
            }
        } else {
            let aspect_point_lon = (sig_lon_rad + offset_deg.to_radians())
                .rem_euclid(std::f64::consts::TAU);
            ecliptic_to_equatorial(aspect_point_lon, 0.0, obliquity)
        };
        let m_target = method.mundane_position_for(ramc, target_ra, target_dec, phi);

        let h_target = method.hour_angle_for_target(m_target, placement.declination_rad, phi);
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
/// → `[180]`; symmetric aspects → both `+exact` and `−exact`. Returns
/// a small stack buffer to keep the per-direction loop allocation-free.
fn aspect_branch_offsets_deg(aspect: AspectKind) -> ([f64; 2], usize) {
    let exact = aspect.exact_angle_deg();
    match aspect {
        AspectKind::Conjunction => ([0.0, 0.0], 1),
        AspectKind::Opposition => ([180.0, 0.0], 1),
        _ => ([exact, -exact], 2),
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
    // Pre-size to a reasonable upper bound to avoid Vec growth in the
    // inner accumulation loop. Each promissor produces up to
    // (4 angles + (N − 1) bodies) × |aspect_kinds| × 2 branches directions.
    let n = natal.placements.len();
    let mut out: Vec<Direction> =
        Vec::with_capacity(n * (4 + n) * aspect_kinds.len() * 2);

    // Outer loop walks each placement exactly once and resolves it
    // here — this hoists the linear-scan body lookup that
    // `direct_to_aspect` would otherwise repeat for every (sig, aspect)
    // triple. Inner calls use `direct_to_aspect_with_placement`.
    for promissor_p in &natal.placements {
        for sig in [
            Significator::Ascendant,
            Significator::Midheaven,
            Significator::Descendant,
            Significator::ImumCoeli,
        ] {
            for &aspect in aspect_kinds {
                if let Ok(dirs) = direct_to_aspect_with_placement(
                    natal, promissor_p, sig, aspect, method, key,
                ) {
                    for d in dirs {
                        if (0.0..=max_age_years).contains(&d.age_years) {
                            out.push(d);
                        }
                    }
                }
            }
        }
        for sig_p in &natal.placements {
            if sig_p.body == promissor_p.body {
                continue;
            }
            for &aspect in aspect_kinds {
                if let Ok(dirs) = direct_to_aspect_with_placement(
                    natal,
                    promissor_p,
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
