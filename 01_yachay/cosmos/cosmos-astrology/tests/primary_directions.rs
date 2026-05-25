//! Tests for the mundane helpers and the Placidus primary-direction
//! engine.

use cosmos_astrology::{
    all_directions, direct, directions_to_angles, mundane, BirthData, ChartConfig,
    DirectionKey, DirectionMethod, NatalChart, Significator,
};
use cosmos_sky::{Body, EphemerisSession, Instant, Observer, SessionConfig};

fn session() -> EphemerisSession {
    EphemerisSession::open(SessionConfig::vsop2013()).unwrap()
}

fn fixture_birth() -> BirthData {
    let instant = Instant::from_civil_local(1987, 3, 14, 5, 22, 0.0, -240).unwrap();
    let observer = Observer::from_degrees(10.4806, -66.9036, 900.0);
    BirthData::new(instant, observer).with_name("Fixture A")
}

#[test]
fn body_to_mc_arc_equals_negative_natal_hour_angle() {
    // For any promissor, directing it to the MC should require an arc
    // equal in magnitude to the natal hour angle (mod 2π), because the
    // MC has m=1 → target H = 0.
    let s = session();
    let birth = fixture_birth();
    let chart = NatalChart::compute(&birth, &ChartConfig::default(), &s).unwrap();

    for body in [Body::Sun, Body::Moon, Body::Mars, Body::Jupiter] {
        let placement = chart.placement(body).unwrap();
        let ramc = chart.local_apparent_sidereal_time_rad;
        let h_natal =
            mundane::signed_hour_angle_rad(ramc, placement.right_ascension_rad);

        let dir = direct(
            &chart,
            body,
            Significator::Midheaven,
            DirectionMethod::PlacidusMundane,
            DirectionKey::Ptolemy,
        )
        .unwrap();

        // arc + h_natal ≡ 0 (mod 2π), since target H = 0.
        let recovered = (dir.arc_rad + h_natal).rem_euclid(std::f64::consts::TAU);
        let diff = recovered.min(std::f64::consts::TAU - recovered);
        assert!(
            diff < 1e-9,
            "{} → MC: arc={:.6}° + H_natal={:.6}° ≠ 0 (mod 360°)",
            body.name(),
            dir.arc_deg(),
            h_natal.to_degrees(),
        );
    }
}

#[test]
fn body_to_ic_is_body_to_mc_plus_180() {
    let s = session();
    let birth = fixture_birth();
    let chart = NatalChart::compute(&birth, &ChartConfig::default(), &s).unwrap();

    for body in [Body::Sun, Body::Moon, Body::Saturn] {
        let p = chart.placement(body).unwrap();
        let phi = chart.birth.observer.lat_rad;
        let dsa = mundane::diurnal_semi_arc_rad(p.declination_rad, phi);
        let nsa = mundane::nocturnal_semi_arc_rad(p.declination_rad, phi);
        if dsa.is_nan() || nsa.is_nan() {
            continue;
        }
        let to_mc = direct(
            &chart,
            body,
            Significator::Midheaven,
            DirectionMethod::PlacidusMundane,
            DirectionKey::Ptolemy,
        )
        .unwrap();
        let to_ic = direct(
            &chart,
            body,
            Significator::ImumCoeli,
            DirectionMethod::PlacidusMundane,
            DirectionKey::Ptolemy,
        )
        .unwrap();

        // IC mundane = 3 (m=3, H = ±π). MC mundane = 1 (H=0).
        // Target H_IC = -π + 0 · NSA_p = -π. Target H_MC = 0.
        // Δarc = (target_H_IC - h_natal) − (target_H_MC - h_natal) = -π.
        // After wrapping into [0, 2π), the relation is to_ic.arc - to_mc.arc ≡ π (mod 2π).
        let delta = (to_ic.arc_rad - to_mc.arc_rad).rem_euclid(std::f64::consts::TAU);
        let diff = (delta - std::f64::consts::PI).abs();
        assert!(
            diff < 1e-9,
            "{} → IC vs MC delta is {:.4}° not 180°",
            body.name(),
            delta.to_degrees(),
        );
    }
}

#[test]
fn naibod_key_yields_slightly_more_years_than_ptolemy() {
    let s = session();
    let birth = fixture_birth();
    let chart = NatalChart::compute(&birth, &ChartConfig::default(), &s).unwrap();
    let ptolemy = direct(
        &chart,
        Body::Sun,
        Significator::Midheaven,
        DirectionMethod::PlacidusMundane,
        DirectionKey::Ptolemy,
    )
    .unwrap();
    let naibod = direct(
        &chart,
        Body::Sun,
        Significator::Midheaven,
        DirectionMethod::PlacidusMundane,
        DirectionKey::Naibod,
    )
    .unwrap();
    // Same arc, different key. Naibod degrees/year < 1 → years > Ptolemy's.
    assert!((ptolemy.arc_rad - naibod.arc_rad).abs() < 1e-12);
    assert!(
        naibod.age_years > ptolemy.age_years,
        "Naibod years ({}) should exceed Ptolemy years ({})",
        naibod.age_years,
        ptolemy.age_years,
    );
    // Naibod years ≈ ptolemy * 1.0146.
    let ratio = naibod.age_years / ptolemy.age_years;
    assert!(
        (ratio - 1.014_56).abs() < 1e-3,
        "Naibod/Ptolemy ratio {} far from 1.0146",
        ratio,
    );
}

#[test]
fn directions_to_angles_returns_consistent_four_angle_set() {
    let s = session();
    let birth = fixture_birth();
    let chart = NatalChart::compute(&birth, &ChartConfig::default(), &s).unwrap();
    let arcs = directions_to_angles(
        &chart,
        Body::Sun,
        DirectionMethod::PlacidusMundane,
        DirectionKey::Ptolemy,
    )
    .unwrap();
    // All four directions live in [0, 360°).
    for d in &arcs {
        assert!((0.0..std::f64::consts::TAU).contains(&d.arc_rad));
    }
}

#[test]
fn all_directions_filters_by_max_age_and_sorts() {
    let s = session();
    let birth = fixture_birth();
    let chart = NatalChart::compute(&birth, &ChartConfig::default(), &s).unwrap();
    let arcs = all_directions(
        &chart,
        DirectionMethod::PlacidusMundane,
        DirectionKey::Naibod,
        90.0,
    );
    assert!(!arcs.is_empty(), "modern chart should have many directions in 90 yr");
    for d in &arcs {
        assert!(
            d.age_years <= 90.0 + 1e-9,
            "direction at {} yrs exceeds max",
            d.age_years
        );
    }
    for w in arcs.windows(2) {
        assert!(w[0].age_years <= w[1].age_years + 1e-12);
    }
}

#[test]
fn sun_to_self_direction_to_other_body_is_well_defined() {
    let s = session();
    let birth = fixture_birth();
    let chart = NatalChart::compute(&birth, &ChartConfig::default(), &s).unwrap();
    let d = direct(
        &chart,
        Body::Sun,
        Significator::Body(Body::Moon),
        DirectionMethod::PlacidusMundane,
        DirectionKey::Ptolemy,
    )
    .unwrap();
    // Sanity: arc in [0, 2π); age = arc/key.
    assert!((0.0..std::f64::consts::TAU).contains(&d.arc_rad));
    assert!((d.age_years - d.arc_deg()).abs() < 1e-12); // Ptolemy: 1°=1yr
}
