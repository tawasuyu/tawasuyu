//! Tests for planetary stations and primary directions to non-
//! conjunction aspects.

use cosmos_astrology::{
    all_directions, all_directions_with_aspects, all_stations, direct, direct_to_aspect,
    next_station, AspectKind, BirthData, ChartConfig, DirectionKey, DirectionMethod,
    NatalChart, Significator, StationKind,
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

// ─── Stations ────────────────────────────────────────────────────────

#[test]
fn mercury_2025_march_retrograde_station_lands_near_2025_03_15() {
    // Mercury retrograde 2025: stations Rx on 2025-03-15 around 06 UTC.
    let s = session();
    let after = Instant::from_civil_utc(2025, 3, 1, 0, 0, 0.0).unwrap();
    let station = next_station(&s, Body::Mercury, after, 30.0)
        .unwrap()
        .expect("Mercury must station in March 2025");

    assert_eq!(station.kind, StationKind::Retrograde);
    let expected = Instant::from_civil_utc(2025, 3, 15, 6, 0, 0.0).unwrap();
    let diff_days = (station.instant.jd_utc() - expected.jd_utc()).abs();
    assert!(
        diff_days < 1.0,
        "Mercury Rx station {} differs from expected ~2025-03-15 06 UTC by {:.4} d",
        station.instant.to_iso8601(),
        diff_days,
    );
}

#[test]
fn mercury_2025_march_retrograde_pair_inside_window() {
    // The retrograde pair (Rx then Direct) should both fall inside a
    // 6-week window starting 2025-03-01.
    let s = session();
    let after = Instant::from_civil_utc(2025, 3, 1, 0, 0, 0.0).unwrap();
    let stations = all_stations(&s, Body::Mercury, after, 45.0).unwrap();
    assert_eq!(
        stations.len(),
        2,
        "expected 1 Rx + 1 Direct station, got {}",
        stations.len()
    );
    assert_eq!(stations[0].kind, StationKind::Retrograde);
    assert_eq!(stations[1].kind, StationKind::Direct);
    // The Direct station follows the Rx by ~22 days.
    let gap = stations[1].instant.jd_utc() - stations[0].instant.jd_utc();
    assert!(
        (15.0..30.0).contains(&gap),
        "Mercury Rx → Direct gap {} d outside [15, 30]",
        gap
    );
}

#[test]
fn moon_does_not_station() {
    // The Moon's longitude rate is always positive (~13°/day). A 30-day
    // search should find no station.
    let s = session();
    let after = Instant::from_civil_utc(2025, 1, 1, 0, 0, 0.0).unwrap();
    let s_opt = next_station(&s, Body::Moon, after, 30.0).unwrap();
    assert!(s_opt.is_none(), "Moon should never station");
}

// ─── Primary directions to aspects ───────────────────────────────────

#[test]
fn direct_conjunction_matches_legacy_direct_function() {
    // direct_to_aspect(..., Conjunction) must return exactly one
    // Direction equal to direct(...) for the same args.
    let s = session();
    let birth = fixture_birth();
    let chart = NatalChart::compute(&birth, &ChartConfig::default(), &s).unwrap();

    let legacy = direct(
        &chart,
        Body::Sun,
        Significator::Midheaven,
        DirectionMethod::PlacidusMundane,
        DirectionKey::Ptolemy,
    )
    .unwrap();
    let extended = direct_to_aspect(
        &chart,
        Body::Sun,
        Significator::Midheaven,
        AspectKind::Conjunction,
        DirectionMethod::PlacidusMundane,
        DirectionKey::Ptolemy,
    )
    .unwrap();
    assert_eq!(extended.len(), 1);
    assert!(
        (extended[0].arc_rad - legacy.arc_rad).abs() < 1e-12,
        "Conjunction arc differs between direct() and direct_to_aspect()"
    );
    assert_eq!(extended[0].aspect, AspectKind::Conjunction);
}

#[test]
fn trine_yields_two_branches_with_distinct_arcs() {
    let s = session();
    let birth = fixture_birth();
    let chart = NatalChart::compute(&birth, &ChartConfig::default(), &s).unwrap();
    let trines = direct_to_aspect(
        &chart,
        Body::Sun,
        Significator::Body(Body::Moon),
        AspectKind::Trine,
        DirectionMethod::PlacidusMundane,
        DirectionKey::Ptolemy,
    )
    .unwrap();
    assert_eq!(trines.len(), 2, "Trine should yield ±120° branches");
    assert_eq!(trines[0].aspect, AspectKind::Trine);
    assert_eq!(trines[1].aspect, AspectKind::Trine);
    let arc0 = trines[0].arc_deg();
    let arc1 = trines[1].arc_deg();
    assert!(
        (arc0 - arc1).abs() > 1.0,
        "two trine branches should produce distinct arcs (got {:.4}° and {:.4}°)",
        arc0,
        arc1
    );
}

#[test]
fn opposition_yields_single_branch() {
    let s = session();
    let birth = fixture_birth();
    let chart = NatalChart::compute(&birth, &ChartConfig::default(), &s).unwrap();
    let opp = direct_to_aspect(
        &chart,
        Body::Sun,
        Significator::Body(Body::Mars),
        AspectKind::Opposition,
        DirectionMethod::PlacidusMundane,
        DirectionKey::Ptolemy,
    )
    .unwrap();
    assert_eq!(opp.len(), 1, "Opposition is symmetric, one branch");
    assert_eq!(opp[0].aspect, AspectKind::Opposition);
}

#[test]
fn all_directions_remains_conjunction_only_for_back_compat() {
    let s = session();
    let birth = fixture_birth();
    let chart = NatalChart::compute(&birth, &ChartConfig::default(), &s).unwrap();
    let all = all_directions(
        &chart,
        DirectionMethod::PlacidusMundane,
        DirectionKey::Naibod,
        90.0,
    );
    for d in &all {
        assert_eq!(d.aspect, AspectKind::Conjunction);
    }
}

#[test]
fn all_directions_with_aspects_includes_trines_and_squares() {
    let s = session();
    let birth = fixture_birth();
    let chart = NatalChart::compute(&birth, &ChartConfig::default(), &s).unwrap();
    let all = all_directions_with_aspects(
        &chart,
        DirectionMethod::PlacidusMundane,
        DirectionKey::Naibod,
        AspectKind::MAJORS,
        90.0,
    );
    let kinds: std::collections::HashSet<_> = all.iter().map(|d| d.aspect).collect();
    for k in AspectKind::MAJORS {
        // For a chart spanning many bodies + 4 angles, all major
        // aspects should perfect at some age in [0, 90].
        assert!(
            kinds.contains(k),
            "no direction found for {:?}",
            k
        );
    }
    for d in &all {
        assert!(d.age_years <= 90.0 + 1e-9);
    }
    for w in all.windows(2) {
        assert!(w[0].age_years <= w[1].age_years + 1e-12);
    }
}
