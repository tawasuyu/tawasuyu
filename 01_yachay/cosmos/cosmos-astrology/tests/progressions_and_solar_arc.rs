//! Tests for secondary / tertiary / minor progressions and solar arc.
//!
//! Strategy: every progression reduces to "compute a chart at a shifted
//! instant", so we verify the math by comparing against direct chart
//! computations at the expected shifted instant. The solar-arc direction
//! is checked structurally: every body shifts by the same arc, and
//! house numbers are preserved.

use cosmos_astrology::{
    progress, progressed_instant, secondary_progression, solar_arc_naibod, solar_arc_true,
    BirthData, ChartConfig, NatalChart, ProgressedHouses, ProgressionMethod,
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
fn progressed_instant_secondary_at_age_1_is_one_day_later() {
    let birth = Instant::from_civil_utc(1987, 3, 14, 9, 22, 0.0).unwrap();
    let prog = progressed_instant(birth, 1.0, ProgressionMethod::Secondary);
    // Tropical year = 365.2422 days, so 1 yr of life → 1 d ephemeris.
    let diff_days = prog.jd_utc() - birth.jd_utc();
    assert!((diff_days - 1.0).abs() < 1e-9);
}

#[test]
fn progressed_instant_minor_at_age_1_is_one_sidereal_month_scaled() {
    let birth = Instant::from_civil_utc(1987, 3, 14, 9, 22, 0.0).unwrap();
    let prog = progressed_instant(birth, 1.0, ProgressionMethod::Minor);
    let diff_days = prog.jd_utc() - birth.jd_utc();
    // 1 year of life × (1 sidereal month / 27.3217 d × 365.2422 d/yr) ≈ 13.37 d.
    let expected = 365.242_190 / 27.321_661;
    assert!(
        (diff_days - expected).abs() < 1e-6,
        "minor progression at age 1 yields {} d, expected {}",
        diff_days,
        expected
    );
}

#[test]
fn secondary_progression_at_age_30_advances_sun_about_30_degrees() {
    // Real Sun moves ~0.9856°/day. After 30 days the secondary-
    // progressed Sun should be ~29.5° farther along the ecliptic.
    let s = session();
    let birth = fixture_birth();
    let natal = NatalChart::compute(&birth, &ChartConfig::default(), &s).unwrap();
    let prog = secondary_progression(&natal, &s, 30.0).unwrap();

    let natal_sun = natal
        .placement(Body::Sun)
        .unwrap()
        .longitude
        .longitude_deg();
    let prog_sun = prog
        .progressed()
        .placement(Body::Sun)
        .unwrap()
        .longitude
        .longitude_deg();
    let arc = signed_delta_deg(prog_sun, natal_sun);
    assert!(
        (28.0..31.0).contains(&arc),
        "Sun advance over 30 yrs of secondary ≈ 30°, got {:.3}°",
        arc
    );
}

#[test]
fn secondary_progression_with_natal_houses_preserves_cusps() {
    let s = session();
    let birth = fixture_birth();
    let natal = NatalChart::compute(&birth, &ChartConfig::default(), &s).unwrap();
    let prog = progress(
        &natal,
        &s,
        30.0,
        ProgressionMethod::Secondary,
        ProgressedHouses::Natal,
    )
    .unwrap();
    for i in 0..12 {
        let diff = (prog.progressed().houses.cusps[i] - natal.houses.cusps[i]).abs();
        assert!(diff < 1e-12, "cusp[{}] drift {} rad under Natal treatment", i, diff);
    }
}

#[test]
fn solar_arc_true_shifts_every_placement_by_the_same_amount() {
    let s = session();
    let birth = fixture_birth();
    let natal = NatalChart::compute(&birth, &ChartConfig::default(), &s).unwrap();
    let arc_chart = solar_arc_true(&natal, &s, 30.0).unwrap();

    // Same arc applied to every body — verify by comparing the
    // wrapped delta of one body against the arc.
    let directed_sun = arc_chart
        .directed
        .placement(Body::Sun)
        .unwrap()
        .longitude
        .longitude_rad();
    let natal_sun = natal
        .placement(Body::Sun)
        .unwrap()
        .longitude
        .longitude_rad();
    let sun_arc = signed_delta_rad(directed_sun, natal_sun);
    assert!(
        (sun_arc - arc_chart.arc_rad).abs() < 1e-12,
        "Sun delta {} rad ≠ stored arc {} rad",
        sun_arc,
        arc_chart.arc_rad
    );

    // Same arc for Mars.
    let directed_mars = arc_chart
        .directed
        .placement(Body::Mars)
        .unwrap()
        .longitude
        .longitude_rad();
    let natal_mars = natal
        .placement(Body::Mars)
        .unwrap()
        .longitude
        .longitude_rad();
    let mars_arc = signed_delta_rad(directed_mars, natal_mars);
    assert!(
        (mars_arc - arc_chart.arc_rad).abs() < 1e-12,
        "Mars delta {} rad ≠ arc {} rad",
        mars_arc,
        arc_chart.arc_rad
    );
}

#[test]
fn solar_arc_preserves_natal_house_numbers() {
    let s = session();
    let birth = fixture_birth();
    let natal = NatalChart::compute(&birth, &ChartConfig::default(), &s).unwrap();
    let arc_chart = solar_arc_true(&natal, &s, 30.0).unwrap();

    // Walk parallel indices — `placement(body)` returns the first
    // match, which is wrong for the two MeanNode entries (ascending +
    // auto-added descending). The two `placements` arrays were built
    // from the same BodySet in the same order.
    assert_eq!(natal.placements.len(), arc_chart.directed.placements.len());
    for (natal_p, directed_p) in natal
        .placements
        .iter()
        .zip(arc_chart.directed.placements.iter())
    {
        assert_eq!(natal_p.body, directed_p.body);
        assert_eq!(
            natal_p.house_number, directed_p.house_number,
            "body {} (index entry) switched house under solar arc",
            natal_p.body.name()
        );
    }
}

#[test]
fn solar_arc_naibod_yields_30_degree_arc_at_30_years() {
    let s = session();
    let birth = fixture_birth();
    let natal = NatalChart::compute(&birth, &ChartConfig::default(), &s).unwrap();
    let arc = solar_arc_naibod(&natal, 30.0);
    // Naibod constant = 0°59'08.33"/yr → 30 yr ≈ 29.572°.
    let arc_deg = arc.arc_deg();
    assert!(
        (29.5..29.7).contains(&arc_deg),
        "Naibod arc at 30 yrs should be ~29.57°, got {:.4}°",
        arc_deg
    );
}

fn signed_delta_rad(a: f64, b: f64) -> f64 {
    const PI: f64 = std::f64::consts::PI;
    const TAU: f64 = std::f64::consts::TAU;
    let mut d = a - b;
    while d > PI {
        d -= TAU;
    }
    while d < -PI {
        d += TAU;
    }
    d
}

fn signed_delta_deg(a: f64, b: f64) -> f64 {
    let mut d = a - b;
    while d > 180.0 {
        d -= 360.0;
    }
    while d < -180.0 {
        d += 360.0;
    }
    d
}
