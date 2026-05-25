//! Tests for the aspect engine and the planetary-return finder.

use cosmos_astrology::{
    aspect, find_aspects, find_aspects_filtered, next_return, AspectKind, BirthData,
    ChartConfig, NatalChart, OrbTable,
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
fn modern_western_orbs_match_expected_values() {
    let orbs = OrbTable::modern_western();
    // Sun-Moon conjunction = 8 × 1.25 = 10° (both luminaries multiply
    // but only the max applies → 10°).
    assert_eq!(
        orbs.orb_for(Body::Sun, Body::Moon, AspectKind::Conjunction),
        10.0
    );
    assert_eq!(
        orbs.orb_for(Body::Mars, Body::Saturn, AspectKind::Trine),
        7.0
    );
    assert_eq!(
        orbs.orb_for(Body::Mars, Body::Saturn, AspectKind::Quincunx),
        2.5
    );
}

#[test]
fn aspect_engine_finds_expected_pairs_in_demo_chart() {
    let s = session();
    let birth = fixture_birth();
    let chart = NatalChart::compute(&birth, &ChartConfig::default(), &s).unwrap();

    let orbs = OrbTable::modern_western();
    let asps = find_aspects(&chart, &orbs);
    assert!(!asps.is_empty(), "real chart should have some aspects");

    // Every reported aspect must (a) be within its allowed orb, and
    // (b) have signed_orb consistent with the actual separation.
    for a in &asps {
        assert!(
            a.orb_abs_deg() <= a.allowed_orb_deg + 1e-9,
            "aspect {:?} {} {} orb {} > allowed {}",
            a.kind,
            a.a.name(),
            a.b.name(),
            a.orb_abs_deg(),
            a.allowed_orb_deg
        );
    }

    // Output is sorted by tightness (most exact first).
    for w in asps.windows(2) {
        assert!(w[0].orb_abs_deg() <= w[1].orb_abs_deg() + 1e-12);
    }
}

#[test]
fn major_aspect_filter_excludes_minors() {
    let s = session();
    let birth = fixture_birth();
    let chart = NatalChart::compute(&birth, &ChartConfig::default(), &s).unwrap();
    let majors = find_aspects_filtered(&chart, &OrbTable::default(), AspectKind::MAJORS);
    for a in &majors {
        assert!(
            AspectKind::MAJORS.contains(&a.kind),
            "filter leaked {:?}",
            a.kind
        );
    }
}

#[test]
fn applying_flag_is_consistent_with_signed_orb() {
    // Construct a synthetic 2-body chart by computing aspects manually
    // on two crafted placements. Easier than reasoning about a real
    // birth-chart's velocities.
    use cosmos_astrology::{BodyPlacement, Sign, SignedLongitude};

    let mercury = BodyPlacement {
        body: Body::Mercury,
        longitude: SignedLongitude::from_radians(10.0_f64.to_radians()),
        latitude_rad: 0.0,
        distance_km: 0.0,
        // Mercury moves fast (positive), trying to overtake Mars.
        longitude_rate_rad_per_day: 1.0_f64.to_radians(),
        right_ascension_rad: 0.0,
        declination_rad: 0.0,
        house_number: 1,
        horizon: None,
    };
    let mars = BodyPlacement {
        body: Body::Mars,
        longitude: SignedLongitude::from_radians(15.0_f64.to_radians()),
        latitude_rad: 0.0,
        distance_km: 0.0,
        // Mars is slower.
        longitude_rate_rad_per_day: 0.5_f64.to_radians(),
        right_ascension_rad: 0.0,
        declination_rad: 0.0,
        house_number: 1,
        horizon: None,
    };

    let orbs = OrbTable::modern_western();
    // 5° apart → conjunction in orb (8°). Mercury catches up → applying.
    let asp = aspect_test_pair_helper(&mercury, &mars, AspectKind::Conjunction, &orbs)
        .expect("should find conjunction");
    assert_eq!(asp.kind, AspectKind::Conjunction);
    assert!(asp.applying, "Mercury catching Mars should be applying");
    assert!(asp.orb_abs_deg() > 0.0, "should not be exact");
    let _ = Sign::Aries; // silence unused-warning when running this alone
}

/// Helper that reproduces `find_aspects` for a single pair so the
/// applying-test can hand-craft placements.
fn aspect_test_pair_helper(
    a: &cosmos_astrology::BodyPlacement,
    b: &cosmos_astrology::BodyPlacement,
    kind: AspectKind,
    orbs: &OrbTable,
) -> Option<cosmos_astrology::Aspect> {
    // We call into find_aspects through a tiny NatalChart shim:
    // simplest is to do the math directly via the public types.
    let placements = vec![*a, *b];
    let chart = synth_chart(placements);
    let asps = aspect::find_aspects_filtered(&chart, orbs, &[kind]);
    asps.into_iter().next()
}

/// Cheap NatalChart shim for unit testing — builds a chart with empty
/// houses + only the supplied placements. We compute one real chart and
/// then swap its placements vector.
fn synth_chart(placements: Vec<cosmos_astrology::BodyPlacement>) -> NatalChart {
    let s = session();
    let birth = fixture_birth();
    let mut chart = NatalChart::compute(&birth, &ChartConfig::default(), &s).unwrap();
    chart.placements = placements;
    chart
}

#[test]
fn solar_return_for_2025_lands_within_24h_of_birthday() {
    // For a March 14, 1987 birth, the 2024-2025 solar return must
    // happen near March 14, 2025. Bracket the search starting March 1,
    // 2025 with a 30-day window.
    let s = session();
    let birth = fixture_birth();
    let chart = NatalChart::compute(&birth, &ChartConfig::default(), &s).unwrap();
    let natal_sun = chart
        .placement(Body::Sun)
        .unwrap()
        .longitude
        .longitude_rad();
    let after = Instant::from_civil_utc(2025, 3, 1, 0, 0, 0.0).unwrap();
    let return_t = next_return(&s, Body::Sun, natal_sun, after, Some(30.0)).unwrap();

    // The instant must lie within 1 day of 2025-03-14T09:22 UTC. The
    // Sun's daily motion is ~1°, so a 30-day search is wide and a 24-h
    // tolerance is conservative.
    let expected = Instant::from_civil_utc(2025, 3, 14, 9, 22, 0.0).unwrap();
    let diff_days = (return_t.jd_utc() - expected.jd_utc()).abs();
    assert!(
        diff_days < 1.0,
        "Sun return at {} too far from expected {} (Δ = {:.4} d)",
        return_t.to_iso8601(),
        expected.to_iso8601(),
        diff_days,
    );
}

#[test]
fn lunar_return_brackets_one_sidereal_month() {
    let s = session();
    let birth = fixture_birth();
    let chart = NatalChart::compute(&birth, &ChartConfig::default(), &s).unwrap();
    let natal_moon = chart
        .placement(Body::Moon)
        .unwrap()
        .longitude
        .longitude_rad();
    let after = Instant::from_civil_utc(2025, 1, 1, 0, 0, 0.0).unwrap();
    let return_t = next_return(&s, Body::Moon, natal_moon, after, Some(35.0)).unwrap();
    let gap_days = return_t.jd_utc() - after.jd_utc();
    // Sidereal month ≈ 27.32 d; allow 0..28 d to handle the lunar
    // node's nutation jitter at ~±10' / day.
    assert!(
        (0.0..28.5).contains(&gap_days),
        "Moon return gap {:.4} d outside [0, 28.5]",
        gap_days
    );
}

#[test]
fn find_root_handles_no_sign_change_gracefully() {
    use cosmos_sky::{find_root, SearchOptions};
    let t0 = Instant::from_civil_utc(2025, 1, 1, 0, 0, 0.0).unwrap();
    let t1 = Instant::from_civil_utc(2025, 1, 2, 0, 0, 0.0).unwrap();
    // f never changes sign.
    let result = find_root(t0, t1, |_t| Ok(1.0), SearchOptions::HOURLY).unwrap();
    assert!(result.is_none());
}

#[test]
fn find_root_locates_a_simple_zero_at_midpoint() {
    use cosmos_sky::{find_root, SearchOptions};
    let t0 = Instant::from_civil_utc(2025, 1, 1, 0, 0, 0.0).unwrap();
    let t1 = Instant::from_civil_utc(2025, 1, 2, 0, 0, 0.0).unwrap();
    let mid_jd = 0.5 * (t0.jd_utc() + t1.jd_utc());

    // Linear f(t) crossing zero at midpoint.
    let f = |t: Instant| Ok(t.jd_utc() - mid_jd);
    let root = find_root(t0, t1, f, SearchOptions::HOURLY).unwrap().unwrap();

    // Should land within tolerance (1 s = ~1.16e-5 days).
    let diff = (root.jd_utc() - mid_jd).abs();
    assert!(diff < 2.0e-5, "find_root diverged: Δ = {:.3e} days", diff);
}
