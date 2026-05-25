//! Tests for lunar phases and the eclipse-on-natal helpers.
//!
//! Lunar phases use the VSOP backend (Sun + Moon longitudes are well-
//! defined analytically). Eclipses require SPK, so those tests only
//! exercise the error path; the underlying eclipse code itself is
//! already validated by `eternal-validation`.

use cosmos_astrology::{
    classify_lunation_phase, eclipses_on_natal, next_canonical_phase, next_lunar_phase,
    phase_angle_at_deg, BirthData, ChartConfig, LunarPhase, LunationPhase, NatalChart,
};
use cosmos_sky::{EphemerisSession, Instant, Observer, SessionConfig};

fn session() -> EphemerisSession {
    EphemerisSession::open(SessionConfig::vsop2013()).unwrap()
}

fn fixture_birth() -> BirthData {
    BirthData::new(
        Instant::from_civil_local(1987, 3, 14, 5, 22, 0.0, -240).unwrap(),
        Observer::from_degrees(10.4806, -66.9036, 900.0),
    )
}

// ─── Lunar phases ─────────────────────────────────────────────────────

#[test]
fn phase_angle_at_known_new_moon_is_near_zero() {
    // New Moon on 2025-02-28 around 00:45 UTC. The phase angle in
    // VSOP-only is at the ~arc-minute level, so allow ±0.5° to cover
    // analytical-vs-SPK lunar differences.
    let s = session();
    let t = Instant::from_civil_utc(2025, 2, 28, 0, 45, 0.0).unwrap();
    let p = phase_angle_at_deg(&s, t).unwrap();
    let dist = p.min(360.0 - p); // distance to 0/360 boundary
    assert!(
        dist < 1.0,
        "phase angle {}° not within 1° of new moon",
        p
    );
}

#[test]
fn next_new_moon_lands_near_2025_02_28() {
    // From 2025-02-15 the next new moon must be near 2025-02-28.
    let s = session();
    let after = Instant::from_civil_utc(2025, 2, 15, 0, 0, 0.0).unwrap();
    let t = next_lunar_phase(&s, LunarPhase::NewMoon, after, 20.0)
        .unwrap()
        .expect("new moon must occur within 20 d of 2025-02-15");
    let expected = Instant::from_civil_utc(2025, 2, 28, 0, 45, 0.0).unwrap();
    let diff_hours = (t.jd_utc() - expected.jd_utc()) * 24.0;
    assert!(
        diff_hours.abs() < 6.0,
        "new moon {} differs from expected 2025-02-28 00:45 UTC by {:.2} h",
        t.to_iso8601(),
        diff_hours
    );
}

#[test]
fn next_full_moon_after_new_moon_is_about_15_days_later() {
    let s = session();
    let after = Instant::from_civil_utc(2025, 2, 28, 0, 45, 0.0).unwrap();
    let full = next_lunar_phase(&s, LunarPhase::FullMoon, after, 20.0)
        .unwrap()
        .expect("full moon within 20 d of new moon");
    let gap_days = full.jd_utc() - after.jd_utc();
    assert!(
        (13.5..16.0).contains(&gap_days),
        "new→full gap {:.4} d outside [13.5, 16.0]",
        gap_days
    );
}

#[test]
fn next_canonical_phase_returns_the_nearest_phase() {
    // From 2025-03-01 the next phase should be the First Quarter (around 2025-03-06).
    let s = session();
    let after = Instant::from_civil_utc(2025, 3, 1, 0, 0, 0.0).unwrap();
    let (t, phase) = next_canonical_phase(&s, after, 10.0)
        .unwrap()
        .expect("a canonical phase must occur within 10 d");
    assert_eq!(phase, LunarPhase::FirstQuarter);
    let gap = t.jd_utc() - after.jd_utc();
    assert!(
        (0.0..10.0).contains(&gap),
        "First Quarter gap {:.4} d outside [0, 10]",
        gap
    );
}

#[test]
fn classify_lunation_phase_covers_eight_bands() {
    // Boundaries at 0°, 22.5°, 67.5°, 112.5°, 157.5°, 202.5°, 247.5°,
    // 292.5°, 337.5°.
    let cases = [
        (10.0_f64, LunationPhase::NewMoon),
        (45.0, LunationPhase::WaxingCrescent),
        (90.0, LunationPhase::FirstQuarter),
        (135.0, LunationPhase::WaxingGibbous),
        (180.0, LunationPhase::FullMoon),
        (225.0, LunationPhase::WaningGibbous),
        (270.0, LunationPhase::LastQuarter),
        (315.0, LunationPhase::WaningCrescent),
    ];
    for (deg, expected) in cases {
        let p = classify_lunation_phase(deg.to_radians());
        assert_eq!(p, expected, "phase angle {}°", deg);
    }
}

// ─── Eclipses (error path only — full path requires SPK) ─────────────

#[test]
fn eclipses_on_natal_returns_clear_error_without_spk() {
    let s = session();
    let chart = NatalChart::compute(&fixture_birth(), &ChartConfig::default(), &s).unwrap();
    let after = Instant::from_civil_utc(2026, 1, 1, 0, 0, 0.0).unwrap();

    let result = eclipses_on_natal(&chart, &s, after, 12, 3.0, None);
    assert!(
        result.is_err(),
        "eclipses_on_natal must error without an SPK kernel"
    );
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.to_lowercase().contains("spk"),
        "error message should mention SPK kernel: got {:?}",
        msg
    );
}
