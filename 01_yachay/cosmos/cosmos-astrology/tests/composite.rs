//! Tests for the midpoint composite chart.

use cosmos_astrology::{
    angular_midpoint_rad, composite, BirthData, ChartConfig, NatalChart,
};
use cosmos_sky::{Body, EphemerisSession, Instant, Observer, SessionConfig};

fn session() -> EphemerisSession {
    EphemerisSession::open(SessionConfig::vsop2013()).unwrap()
}

fn fixture_a() -> BirthData {
    BirthData::new(
        Instant::from_civil_local(1987, 3, 14, 5, 22, 0.0, -240).unwrap(),
        Observer::from_degrees(10.4806, -66.9036, 900.0),
    )
    .with_name("Subject A")
}

fn fixture_b() -> BirthData {
    BirthData::new(
        Instant::from_civil_local(1990, 7, 22, 14, 17, 0.0, 60).unwrap(),
        Observer::from_degrees(40.4168, -3.7038, 650.0),
    )
    .with_name("Subject B")
}

#[test]
fn composite_of_identical_charts_reproduces_the_chart() {
    let s = session();
    let chart = NatalChart::compute(&fixture_a(), &ChartConfig::default(), &s).unwrap();
    let comp = composite(&chart, &chart).unwrap();

    // Every angular midpoint equals the original.
    let diff_asc = (comp.ascendant.longitude_rad() - chart.ascendant().longitude_rad()).abs();
    assert!(diff_asc < 1e-12);
    let diff_mc = (comp.midheaven.longitude_rad() - chart.midheaven().longitude_rad()).abs();
    assert!(diff_mc < 1e-12);

    // Each placement matches its natal counterpart.
    assert_eq!(comp.placements.len(), chart.placements.len());
    for (c, n) in comp.placements.iter().zip(chart.placements.iter()) {
        assert_eq!(c.body, n.body);
        let diff =
            (c.longitude.longitude_rad() - n.longitude.longitude_rad()).abs();
        assert!(diff < 1e-12, "{} off by {}", c.body.name(), diff);
    }
}

#[test]
fn composite_is_symmetric_under_a_b_swap() {
    let s = session();
    let chart_a = NatalChart::compute(&fixture_a(), &ChartConfig::default(), &s).unwrap();
    let chart_b = NatalChart::compute(&fixture_b(), &ChartConfig::default(), &s).unwrap();

    let ab = composite(&chart_a, &chart_b).unwrap();
    let ba = composite(&chart_b, &chart_a).unwrap();

    assert_eq!(ab.placements.len(), ba.placements.len());
    for (x, y) in ab.placements.iter().zip(ba.placements.iter()) {
        assert_eq!(x.body, y.body);
        let diff = (x.longitude.longitude_rad() - y.longitude.longitude_rad()).abs();
        assert!(
            diff < 1e-12,
            "Composite midpoint differs between A→B and B→A for {}: {}",
            x.body.name(),
            diff
        );
    }
    let diff_asc =
        (ab.ascendant.longitude_rad() - ba.ascendant.longitude_rad()).abs();
    let diff_mc =
        (ab.midheaven.longitude_rad() - ba.midheaven.longitude_rad()).abs();
    assert!(diff_asc < 1e-12);
    assert!(diff_mc < 1e-12);
}

#[test]
fn angular_midpoint_picks_shorter_arc() {
    use std::f64::consts::TAU;
    // 350° and 10° — shorter arc midpoint is 0° (not 180°).
    let mid_a = angular_midpoint_rad(
        350.0_f64.to_radians(),
        10.0_f64.to_radians(),
    );
    let mid_b = angular_midpoint_rad(
        10.0_f64.to_radians(),
        350.0_f64.to_radians(),
    );
    let target = 0.0_f64;
    let diff_a = ((mid_a - target).rem_euclid(TAU)).min((target - mid_a).rem_euclid(TAU));
    let diff_b = ((mid_b - target).rem_euclid(TAU)).min((target - mid_b).rem_euclid(TAU));
    assert!(
        diff_a < 1e-12 && diff_b < 1e-12,
        "midpoints {} and {} should both be ~0°",
        mid_a.to_degrees(),
        mid_b.to_degrees()
    );

    // Right-angle case: 0° and 90° → midpoint 45°.
    let mid = angular_midpoint_rad(0.0, 90.0_f64.to_radians());
    let diff = (mid - 45.0_f64.to_radians()).abs();
    assert!(diff < 1e-12);
}

#[test]
fn composite_placements_carry_whole_sign_houses() {
    let s = session();
    let chart_a = NatalChart::compute(&fixture_a(), &ChartConfig::default(), &s).unwrap();
    let chart_b = NatalChart::compute(&fixture_b(), &ChartConfig::default(), &s).unwrap();
    let comp = composite(&chart_a, &chart_b).unwrap();

    let asc_sign = comp.ascendant.sign();
    for p in &comp.placements {
        // Whole-sign: house = (sign index − asc sign index) mod 12 + 1
        let expected = ((p.sign.index() as i32 - asc_sign.index() as i32).rem_euclid(12) + 1) as u8;
        assert_eq!(
            p.house_number, expected,
            "{} sign {:?} should be H{} (Asc sign {:?})",
            p.body.name(),
            p.sign,
            expected,
            asc_sign
        );
    }
}

#[test]
fn composite_sun_lies_between_inputs() {
    // For inputs that are not antipodal, the composite Sun longitude
    // should fall on the shorter arc between the two natal Suns.
    let s = session();
    let chart_a = NatalChart::compute(&fixture_a(), &ChartConfig::default(), &s).unwrap();
    let chart_b = NatalChart::compute(&fixture_b(), &ChartConfig::default(), &s).unwrap();
    let comp = composite(&chart_a, &chart_b).unwrap();

    let sun_a = chart_a.placement(Body::Sun).unwrap().longitude.longitude_rad();
    let sun_b = chart_b.placement(Body::Sun).unwrap().longitude.longitude_rad();
    let sun_c = comp.placement(Body::Sun).unwrap().longitude.longitude_rad();

    // The composite must equal angular_midpoint(sun_a, sun_b).
    let expected = angular_midpoint_rad(sun_a, sun_b);
    assert!((sun_c - expected).abs() < 1e-12);
}

#[test]
fn composite_descendant_opposes_ascendant() {
    let s = session();
    let chart_a = NatalChart::compute(&fixture_a(), &ChartConfig::default(), &s).unwrap();
    let chart_b = NatalChart::compute(&fixture_b(), &ChartConfig::default(), &s).unwrap();
    let comp = composite(&chart_a, &chart_b).unwrap();

    let asc = comp.ascendant.longitude_rad();
    let desc = comp.descendant.longitude_rad();
    let diff = ((desc - asc).rem_euclid(std::f64::consts::TAU)
        - std::f64::consts::PI)
        .abs();
    assert!(diff < 1e-12, "Desc not opposite Asc, off by {}", diff);

    let mc = comp.midheaven.longitude_rad();
    let ic = comp.imum_coeli.longitude_rad();
    let diff = ((ic - mc).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI).abs();
    assert!(diff < 1e-12);
}
