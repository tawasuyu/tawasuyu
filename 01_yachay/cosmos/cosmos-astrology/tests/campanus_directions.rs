//! Tests for the Campanus primary-direction method.
//!
//! Campanus mundane positions are computed from the body's local
//! horizontal Cartesian, projected onto the prime vertical. Properties
//! that must hold:
//!
//! 1. m_Campanus at MC = 1 (any body, any latitude).
//! 2. Campanus and Placidus agree on directions to **angles** (the
//!    four cardinal mundane positions are the same in all three
//!    classical frameworks by definition).
//! 3. Campanus differs from both Placidus and Regiomontanus on
//!    body-to-body directions.

use cosmos_astrology::{
    direct_to_aspect, AspectKind, BirthData, ChartConfig, DirectionKey, DirectionMethod,
    NatalChart, Significator,
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
fn campanus_agrees_with_placidus_for_directions_to_each_angle() {
    let s = session();
    let chart = NatalChart::compute(&fixture_birth(), &ChartConfig::default(), &s).unwrap();

    let angles = [
        Significator::Ascendant,
        Significator::Midheaven,
        Significator::Descendant,
        Significator::ImumCoeli,
    ];

    for body in [Body::Sun, Body::Moon, Body::Mars, Body::Saturn] {
        for sig in angles {
            let placidus = direct_to_aspect(
                &chart,
                body,
                sig,
                AspectKind::Conjunction,
                DirectionMethod::PlacidusMundane,
                DirectionKey::Ptolemy,
            )
            .unwrap()[0];
            let campanus = direct_to_aspect(
                &chart,
                body,
                sig,
                AspectKind::Conjunction,
                DirectionMethod::Campanus,
                DirectionKey::Ptolemy,
            )
            .unwrap()[0];
            // All three frameworks place angles at the same fixed
            // mundane positions, so the arc must match exactly.
            let diff = (placidus.arc_rad - campanus.arc_rad).abs();
            let diff = diff.min((std::f64::consts::TAU - diff).abs());
            assert!(
                diff < 1e-9,
                "{} → {:?} differs Plac={:.6}° Camp={:.6}° (Δ {:.6}°)",
                body.name(),
                sig,
                placidus.arc_deg(),
                campanus.arc_deg(),
                diff.to_degrees()
            );
        }
    }
}

#[test]
fn campanus_disagrees_with_placidus_for_body_to_body() {
    let s = session();
    let chart = NatalChart::compute(&fixture_birth(), &ChartConfig::default(), &s).unwrap();
    let placidus = direct_to_aspect(
        &chart,
        Body::Sun,
        Significator::Body(Body::Saturn),
        AspectKind::Conjunction,
        DirectionMethod::PlacidusMundane,
        DirectionKey::Ptolemy,
    )
    .unwrap()[0];
    let campanus = direct_to_aspect(
        &chart,
        Body::Sun,
        Significator::Body(Body::Saturn),
        AspectKind::Conjunction,
        DirectionMethod::Campanus,
        DirectionKey::Ptolemy,
    )
    .unwrap()[0];
    let diff = (placidus.arc_rad - campanus.arc_rad).abs();
    assert!(
        diff > 1e-4,
        "Placidus and Campanus should differ on body-to-body; got Plac={:.6}° Camp={:.6}°",
        placidus.arc_deg(),
        campanus.arc_deg()
    );
}

#[test]
fn campanus_disagrees_with_regiomontanus_for_body_to_body() {
    let s = session();
    let chart = NatalChart::compute(&fixture_birth(), &ChartConfig::default(), &s).unwrap();
    let regio = direct_to_aspect(
        &chart,
        Body::Mercury,
        Significator::Body(Body::Pluto),
        AspectKind::Conjunction,
        DirectionMethod::Regiomontanus,
        DirectionKey::Naibod,
    )
    .unwrap()[0];
    let campanus = direct_to_aspect(
        &chart,
        Body::Mercury,
        Significator::Body(Body::Pluto),
        AspectKind::Conjunction,
        DirectionMethod::Campanus,
        DirectionKey::Naibod,
    )
    .unwrap()[0];
    let diff = (regio.arc_rad - campanus.arc_rad).abs();
    assert!(
        diff > 1e-4,
        "Regio and Campanus should differ on body-to-body; got Regio={:.6}° Camp={:.6}°",
        regio.arc_deg(),
        campanus.arc_deg()
    );
}

#[test]
fn campanus_method_tag_preserved_in_direction_struct() {
    let s = session();
    let chart = NatalChart::compute(&fixture_birth(), &ChartConfig::default(), &s).unwrap();
    let d = direct_to_aspect(
        &chart,
        Body::Sun,
        Significator::Midheaven,
        AspectKind::Conjunction,
        DirectionMethod::Campanus,
        DirectionKey::Naibod,
    )
    .unwrap()[0];
    assert_eq!(d.method, DirectionMethod::Campanus);
}
