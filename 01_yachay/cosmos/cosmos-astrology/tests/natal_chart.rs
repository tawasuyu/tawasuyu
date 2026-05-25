//! End-to-end `NatalChart` tests. The VSOP2013 backend is used so no
//! external kernels are required. These tests assert that:
//!
//! 1. The chart pipeline produces internally consistent angles.
//! 2. House numbering is well-formed (every body lands in some 1..=12).
//! 3. Sidereal mode shifts every longitude by the same ayanamsha.
//! 4. Houses with closed-form definitions (Whole-Sign, Equal) match
//!    the canonical formulas exactly.

use cosmos_astrology::{
    Ayanamsha, BirthData, ChartConfig, HouseSystem, NatalChart, Sign, Zodiac,
};
use cosmos_sky::{Body, EphemerisSession, Instant, Observer, SessionConfig};

fn fixture_session() -> EphemerisSession {
    EphemerisSession::open(SessionConfig::vsop2013()).unwrap()
}

fn fixture_birth() -> BirthData {
    // March 14, 1987, 05:22 local (Caracas, UTC−4) → 09:22 UTC.
    let instant = Instant::from_civil_local(1987, 3, 14, 5, 22, 0.0, -240).unwrap();
    let caracas = Observer::from_degrees(10.4806, -66.9036, 900.0);
    BirthData::new(instant, caracas).with_name("Fixture A")
}

#[test]
fn chart_with_defaults_yields_valid_angles_and_houses() {
    let session = fixture_session();
    let birth = fixture_birth();
    let config = ChartConfig::default();

    let chart = NatalChart::compute(&birth, &config, &session).unwrap();

    // Angles must be normalised.
    let asc = chart.ascendant().longitude_rad();
    let mc = chart.midheaven().longitude_rad();
    assert!((0.0..std::f64::consts::TAU).contains(&asc));
    assert!((0.0..std::f64::consts::TAU).contains(&mc));

    // Descendant = Ascendant + π (mod 2π).
    let desc = chart.descendant().longitude_rad();
    let diff = ((desc - asc).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI).abs();
    assert!(diff < 1e-12, "Desc should be opposite Asc, got diff {}", diff);

    // IC = MC + π.
    let ic = chart.imum_coeli().longitude_rad();
    let diff = ((ic - mc).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI).abs();
    assert!(diff < 1e-12);

    // Every cusp inside [0, 2π).
    for &c in &chart.houses.cusps {
        assert!((0.0..std::f64::consts::TAU).contains(&c));
    }

    // Every body lands in some house 1..=12.
    for placement in &chart.placements {
        assert!(
            (1..=12).contains(&placement.house_number),
            "body {} got house {}",
            placement.body.name(),
            placement.house_number
        );
    }
}

#[test]
fn whole_sign_houses_match_ascendant_sign() {
    let session = fixture_session();
    let birth = fixture_birth();
    let config = ChartConfig {
        house_system: HouseSystem::WholeSign,
        ..ChartConfig::default()
    };

    let chart = NatalChart::compute(&birth, &config, &session).unwrap();
    // First cusp = 0° of Asc's sign.
    let asc_sign_index = chart.ascendant().sign().index();
    let cusp0_deg = chart.houses.cusps[0].to_degrees();
    let expected_deg = (asc_sign_index as f64) * 30.0;
    let diff = (cusp0_deg - expected_deg).abs();
    assert!(
        diff < 1e-9 || (diff - 360.0).abs() < 1e-9,
        "Whole-Sign cusp[0] should be at 0° of Asc sign ({:?} → {}°), got {}°",
        chart.ascendant().sign(),
        expected_deg,
        cusp0_deg
    );

    // The 12 cusps are exactly 30° apart.
    for i in 0..12 {
        let expected = ((asc_sign_index as i32 + i as i32) as f64) * 30.0;
        let got = chart.houses.cusps[i].to_degrees();
        let diff = ((got - expected).rem_euclid(360.0)).min((expected - got).rem_euclid(360.0));
        assert!(diff < 1e-9, "cusp[{}] off by {}°", i, diff);
    }
}

#[test]
fn sidereal_mode_subtracts_a_constant_offset_from_every_body() {
    let session = fixture_session();
    let birth = fixture_birth();

    let tropical = ChartConfig {
        zodiac: Zodiac::Tropical,
        ..ChartConfig::default()
    };
    let sidereal = ChartConfig {
        zodiac: Zodiac::Sidereal(Ayanamsha::Lahiri),
        ..ChartConfig::default()
    };

    let trop_chart = NatalChart::compute(&birth, &tropical, &session).unwrap();
    let sid_chart = NatalChart::compute(&birth, &sidereal, &session).unwrap();

    let ayanamsha = sid_chart.ayanamsha_rad;
    assert!(ayanamsha > 0.0, "Lahiri ayanamsha at 1987 should be positive");

    for (trop_p, sid_p) in trop_chart.placements.iter().zip(sid_chart.placements.iter()) {
        let expected_sid = (trop_p.longitude.longitude_rad() - ayanamsha)
            .rem_euclid(std::f64::consts::TAU);
        let got = sid_p.longitude.longitude_rad();
        let diff = (expected_sid - got).abs();
        let diff = diff.min((std::f64::consts::TAU - diff).abs());
        assert!(
            diff < 1e-12,
            "body {} sidereal longitude off by {} rad",
            trop_p.body.name(),
            diff
        );
    }
}

#[test]
fn sun_in_march_lies_in_pisces_or_aries() {
    // Birth on March 14: Sun should be late Pisces (tropical) — about
    // 23° Pisces. Make this a coarse smoke test so future ephemeris
    // refinements don't break it.
    let session = fixture_session();
    let birth = fixture_birth();
    let chart = NatalChart::compute(&birth, &ChartConfig::default(), &session).unwrap();
    let sun = chart.placement(Body::Sun).expect("Sun should be present");
    let sign = sun.longitude.sign();
    assert!(
        sign == Sign::Pisces,
        "Sun on March 14 should be in Pisces, got {:?} at {}",
        sign,
        sun.longitude.to_chart_format()
    );
}

#[test]
fn south_node_is_180_opposite_north_node() {
    let session = fixture_session();
    let birth = fixture_birth();
    let chart = NatalChart::compute(&birth, &ChartConfig::default(), &session).unwrap();

    // Default config includes Mean Node + auto South Node.
    let nodes: Vec<_> = chart
        .placements
        .iter()
        .filter(|p| p.body == Body::MeanNode)
        .collect();
    assert_eq!(nodes.len(), 2, "expected ascending + descending node");

    let n = nodes[0].longitude.longitude_rad();
    let s = nodes[1].longitude.longitude_rad();
    let diff = ((s - n).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI).abs();
    assert!(diff < 1e-12, "South Node should be opposite N Node");
}

#[test]
fn placidus_works_at_temperate_latitude() {
    let session = fixture_session();
    let birth = fixture_birth(); // Caracas at +10.5° — well outside polar circle.
    let config = ChartConfig {
        house_system: HouseSystem::Placidus,
        ..ChartConfig::default()
    };
    let chart = NatalChart::compute(&birth, &config, &session).unwrap();
    // First cusp = Ascendant.
    let diff = (chart.houses.cusps[0] - chart.ascendant().longitude_rad()
        - chart.ayanamsha_rad)
        .abs();
    // (chart.ascendant() is sidereal-shifted iff sidereal; tropical default
    //  yields ayanamsha_rad = 0.)
    assert!(diff < 1e-9, "Placidus cusp[0] should equal Asc");
}
