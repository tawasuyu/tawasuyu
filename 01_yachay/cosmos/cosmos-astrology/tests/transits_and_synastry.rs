//! Tests for the transit engine and the synastry aspect grid.

use cosmos_astrology::{
    aspect::AspectKind, default_natal_targets, find_current_transits,
    find_next_exact_transit, find_synastry_aspects, BirthData, ChartConfig, NatalChart,
    OrbTable, Significator,
};
use cosmos_sky::{Body, EphemerisSession, Instant, Observer, SessionConfig};

fn session() -> EphemerisSession {
    EphemerisSession::open(SessionConfig::vsop2013()).unwrap()
}

fn fixture_a() -> BirthData {
    let instant = Instant::from_civil_local(1987, 3, 14, 5, 22, 0.0, -240).unwrap();
    let observer = Observer::from_degrees(10.4806, -66.9036, 900.0);
    BirthData::new(instant, observer).with_name("Subject A")
}

fn fixture_b() -> BirthData {
    let instant = Instant::from_civil_local(1990, 7, 22, 14, 17, 0.0, 60).unwrap();
    let observer = Observer::from_degrees(40.4168, -3.7038, 650.0); // Madrid
    BirthData::new(instant, observer).with_name("Subject B")
}

// ─── Transits ─────────────────────────────────────────────────────────

#[test]
fn self_transit_at_natal_moment_produces_exact_self_aspects() {
    // At the natal moment, every planet transits its own natal position
    // with orb 0° (conjunction). This is the trivial sanity case for
    // the transit engine: feed the chart's own instant in.
    let s = session();
    let birth = fixture_a();
    let chart = NatalChart::compute(&birth, &ChartConfig::default(), &s).unwrap();

    let targets = default_natal_targets(&chart);
    let transits = find_current_transits(
        &chart,
        &s,
        chart.birth.instant,
        &[Body::Sun, Body::Moon, Body::Mars],
        &targets,
        &OrbTable::modern_western(),
        &[AspectKind::Conjunction],
    )
    .unwrap();

    // Each of the three transiting bodies should have a near-zero-orb
    // conjunction with its own natal point.
    for body in [Body::Sun, Body::Moon, Body::Mars] {
        let self_aspect = transits.iter().find(|t| {
            t.transiting == body && matches!(t.natal_target, Significator::Body(b) if b == body)
        });
        let asp = self_aspect.unwrap_or_else(|| {
            panic!("expected {} to transit its own natal position", body.name())
        });
        assert!(
            asp.orb_abs_deg() < 1e-6,
            "self-transit orb for {} = {} ° (expected ~0)",
            body.name(),
            asp.orb_abs_deg(),
        );
    }
}

#[test]
fn transits_are_sorted_and_within_orb() {
    let s = session();
    let birth = fixture_a();
    let chart = NatalChart::compute(&birth, &ChartConfig::default(), &s).unwrap();

    let targets = default_natal_targets(&chart);
    let now = Instant::from_civil_utc(2026, 5, 15, 0, 0, 0.0).unwrap();
    let transits = find_current_transits(
        &chart,
        &s,
        now,
        &[Body::Mars, Body::Saturn, Body::Jupiter],
        &targets,
        &OrbTable::modern_western(),
        AspectKind::MAJORS,
    )
    .unwrap();

    for t in &transits {
        assert!(
            t.orb_abs_deg() <= t.allowed_orb_deg + 1e-9,
            "transit out of orb"
        );
    }
    for w in transits.windows(2) {
        assert!(w[0].orb_abs_deg() <= w[1].orb_abs_deg() + 1e-12);
    }
}

#[test]
fn next_exact_sun_conjunction_returns_within_a_year() {
    // Transiting Sun conjunct natal Sun must perfect within ~365 d of
    // any starting instant (it's the literal definition of the solar
    // year — same as a solar return).
    let s = session();
    let birth = fixture_a();
    let chart = NatalChart::compute(&birth, &ChartConfig::default(), &s).unwrap();
    let natal_sun = chart
        .placement(Body::Sun)
        .unwrap()
        .longitude
        .longitude_rad();
    let after = Instant::from_civil_utc(2025, 6, 1, 0, 0, 0.0).unwrap();
    let exact = find_next_exact_transit(
        &s,
        Body::Sun,
        natal_sun,
        AspectKind::Conjunction,
        after,
        400.0,
    )
    .unwrap()
    .expect("Sun should reach natal longitude within 400 days");

    let days = exact.jd_utc() - after.jd_utc();
    assert!(
        (0.0..380.0).contains(&days),
        "expected gap in [0, 380] d, got {:.4}",
        days
    );
}

#[test]
fn next_exact_moon_trine_resolves_within_a_month() {
    let s = session();
    let birth = fixture_a();
    let chart = NatalChart::compute(&birth, &ChartConfig::default(), &s).unwrap();
    let natal_sun = chart
        .placement(Body::Sun)
        .unwrap()
        .longitude
        .longitude_rad();
    let after = Instant::from_civil_utc(2025, 6, 1, 0, 0, 0.0).unwrap();
    let exact = find_next_exact_transit(
        &s,
        Body::Moon,
        natal_sun,
        AspectKind::Trine,
        after,
        35.0,
    )
    .unwrap();
    assert!(
        exact.is_some(),
        "Moon must form a trine to natal Sun within 35 days of any instant"
    );
}

// ─── Synastry ─────────────────────────────────────────────────────────

#[test]
fn synastry_finds_aspects_between_two_real_charts() {
    let s = session();
    let chart_a = NatalChart::compute(&fixture_a(), &ChartConfig::default(), &s).unwrap();
    let chart_b = NatalChart::compute(&fixture_b(), &ChartConfig::default(), &s).unwrap();

    let asps = find_synastry_aspects(
        &chart_a,
        &chart_b,
        &OrbTable::modern_western(),
        AspectKind::ALL,
    );
    assert!(!asps.is_empty(), "two real charts should share aspects");
    for a in &asps {
        assert!(a.orb_abs_deg() <= a.allowed_orb_deg + 1e-9);
    }
    for w in asps.windows(2) {
        assert!(w[0].orb_abs_deg() <= w[1].orb_abs_deg() + 1e-12);
    }
}

#[test]
fn synastry_is_symmetric_under_chart_swap() {
    // find_synastry_aspects(A, B) and find_synastry_aspects(B, A) must
    // produce the same set of aspects up to (person_a ↔ person_b) swap.
    let s = session();
    let chart_a = NatalChart::compute(&fixture_a(), &ChartConfig::default(), &s).unwrap();
    let chart_b = NatalChart::compute(&fixture_b(), &ChartConfig::default(), &s).unwrap();

    let ab = find_synastry_aspects(
        &chart_a,
        &chart_b,
        &OrbTable::modern_western(),
        AspectKind::MAJORS,
    );
    let ba = find_synastry_aspects(
        &chart_b,
        &chart_a,
        &OrbTable::modern_western(),
        AspectKind::MAJORS,
    );

    assert_eq!(ab.len(), ba.len());
    for (x, y) in ab.iter().zip(ba.iter()) {
        assert_eq!(x.kind, y.kind);
        assert_eq!(x.person_a_body, y.person_b_body);
        assert_eq!(x.person_b_body, y.person_a_body);
        // Signed orbs: when computed as |sep|−exact they are equal,
        // because |sep| is symmetric in (a, b).
        assert!((x.orb_abs_deg() - y.orb_abs_deg()).abs() < 1e-9);
    }
}

#[test]
fn synastry_self_self_yields_exact_self_conjunctions() {
    // Synastry of a chart against itself contains exact self-
    // conjunctions for every body — useful sanity check.
    let s = session();
    let chart_a = NatalChart::compute(&fixture_a(), &ChartConfig::default(), &s).unwrap();
    let asps = find_synastry_aspects(
        &chart_a,
        &chart_a,
        &OrbTable::modern_western(),
        &[AspectKind::Conjunction],
    );

    for body in [Body::Sun, Body::Moon, Body::Mars] {
        let self_aspect = asps.iter().find(|a| {
            a.person_a_body == body
                && a.person_b_body == body
                && a.kind == AspectKind::Conjunction
        });
        let asp = self_aspect
            .unwrap_or_else(|| panic!("missing self-conjunction for {}", body.name()));
        assert!(asp.orb_abs_deg() < 1e-9);
    }
}
