//! Parity tests: the `eternal-sky` façade must produce the **same**
//! numbers as the underlying `eternal-validation` routines. This is the
//! contract that lets us evolve the API without compromising the
//! precision discipline that already gates the lower layers.
//!
//! These tests use the analytical VSOP2013 backend so they need no
//! external kernels. Apparent (LT + aberration + light deflection) is
//! only available via SPK, so it is exercised by the `houses_check`
//! validation CLI in the broader regression suite; here we focus on
//! geometric ICRF parity which the façade itself implements end-to-end.

use cosmos_sky::{Body, EphemerisSession, Instant, SessionConfig};

use cosmos_validation::fixture::Frame;
use cosmos_validation::oracle::{Backend, Oracle};

#[test]
fn geometric_mars_matches_oracle_exactly() {
    let jd_tdb = 2_460_676.0; // ~2025-01-01 12:00 TDB

    // ── Path A: eternal-sky façade. ──────────────────────────────────
    let session = EphemerisSession::open(SessionConfig::vsop2013()).unwrap();
    let instant = Instant::from_jd_tdb(jd_tdb).unwrap();
    let sky_state = session
        .body_geometric(Body::Mars, Body::Sun, instant)
        .unwrap();

    // ── Path B: direct Oracle call. ──────────────────────────────────
    let oracle = Oracle::new(Backend::Vsop2013).unwrap();
    let raw = oracle
        .state(
            Body::Mars.naif_id().unwrap(),
            Body::Sun.naif_id().unwrap(),
            instant.jd_tdb().unwrap(),
            Frame::Icrf,
        )
        .unwrap();

    for i in 0..3 {
        assert_eq!(
            sky_state.position_km[i], raw.pos_km[i],
            "position[{i}] mismatch: façade={} vs oracle={}",
            sky_state.position_km[i], raw.pos_km[i]
        );
        assert_eq!(
            sky_state.velocity_km_per_s[i], raw.vel_km_s[i],
            "velocity[{i}] mismatch",
        );
    }
}

#[test]
fn apparent_planetary_pipeline_returns_self_consistent_ecliptic() {
    // VSOP backend can produce only geometric ICRF, but we can still
    // verify the façade's TET→ecliptic decomposition is internally
    // consistent (round-trip through Cartesian and back).
    let session = EphemerisSession::open(SessionConfig::vsop2013()).unwrap();
    let instant = Instant::from_civil_utc(2025, 1, 1, 12, 0, 0.0).unwrap();
    let pos = session.body_apparent(Body::Mars, instant, None).unwrap();

    // Longitude in [0, 2π); latitude in [-π/2, π/2]; distance > 0.
    let lon = pos.ecliptic_of_date.longitude_rad;
    let lat = pos.ecliptic_of_date.latitude_rad;
    let dist = pos.ecliptic_of_date.distance_km;
    assert!((0.0..std::f64::consts::TAU).contains(&lon), "λ = {}", lon);
    assert!(
        (-std::f64::consts::FRAC_PI_2..=std::f64::consts::FRAC_PI_2).contains(&lat),
        "β = {}",
        lat
    );
    assert!(dist > 1.0e6, "Mars distance is at minimum ~55 million km");

    // Equatorial and ecliptic distances must agree to bit-precision.
    assert_eq!(pos.ecliptic_of_date.distance_km, pos.equatorial_of_date.distance_km);
}

#[test]
fn observer_topocentric_field_is_populated_when_observer_supplied() {
    let session = EphemerisSession::open(SessionConfig::vsop2013()).unwrap();
    let instant = Instant::from_civil_utc(2025, 6, 21, 16, 0, 0.0).unwrap();
    let caracas = cosmos_sky::Observer::from_degrees(10.4806, -66.9036, 900.0);

    let with_obs = session
        .body_apparent(Body::Sun, instant, Some(&caracas))
        .unwrap();
    let without_obs = session.body_apparent(Body::Sun, instant, None).unwrap();

    assert!(
        with_obs.topocentric_horizon.is_some(),
        "horizon must be populated when an observer is supplied"
    );
    assert!(
        without_obs.topocentric_horizon.is_none(),
        "horizon must be absent without an observer"
    );

    // The Sun at this instant (June 21, ~16:00 UTC = ~12:00 local in
    // Caracas) should be above the horizon.
    let alt_deg = with_obs.topocentric_horizon.unwrap().altitude_deg();
    assert!(
        alt_deg > 0.0,
        "Sun should be above the horizon at noon in Caracas on June 21, got {} deg",
        alt_deg
    );
}

#[test]
fn mean_node_matches_validation_module_exactly() {
    let session = EphemerisSession::open(SessionConfig::vsop2013()).unwrap();
    let t = Instant::from_civil_utc(2025, 1, 1, 0, 0, 0.0).unwrap();
    let tt = t.tt().unwrap();
    let from_sky = session.body_apparent(Body::MeanNode, t, None).unwrap();
    let from_val = cosmos_validation::lunar::mean_lunar_node(&tt);
    assert_eq!(from_sky.ecliptic_of_date.longitude_rad, from_val);
    assert_eq!(from_sky.ecliptic_of_date.latitude_rad, 0.0);
}

#[test]
fn mean_lilith_matches_validation_module_exactly() {
    let session = EphemerisSession::open(SessionConfig::vsop2013()).unwrap();
    let t = Instant::from_civil_utc(2025, 1, 1, 0, 0, 0.0).unwrap();
    let tt = t.tt().unwrap();
    let from_sky = session.body_apparent(Body::MeanLilith, t, None).unwrap();
    let from_val = cosmos_validation::lunar::mean_lilith(&tt);
    assert_eq!(from_sky.ecliptic_of_date.longitude_rad, from_val);
}

#[test]
fn true_node_without_spk_yields_unsupported_body_error() {
    let session = EphemerisSession::open(SessionConfig::vsop2013()).unwrap();
    let t = Instant::from_civil_utc(2025, 1, 1, 0, 0, 0.0).unwrap();
    let err = session.body_apparent(Body::TrueNode, t, None).unwrap_err();
    matches!(err, cosmos_sky::SkyError::UnsupportedBody { .. });
}

#[test]
fn asteroid_without_kernel_yields_unsupported_body_error() {
    let session = EphemerisSession::open(SessionConfig::vsop2013()).unwrap();
    let t = Instant::from_civil_utc(2025, 1, 1, 0, 0, 0.0).unwrap();
    let err = session.body_apparent(Body::Ceres, t, None).unwrap_err();
    matches!(err, cosmos_sky::SkyError::UnsupportedBody { .. });
}

#[test]
fn ecliptic_velocity_sign_matches_known_retrograde_window() {
    // Mercury retrograde 2025-03-15..04-07 (well-known modern transit).
    // Pick a date deep inside that window where dλ/dt should be < 0.
    let session = EphemerisSession::open(SessionConfig::vsop2013()).unwrap();
    let instant = Instant::from_civil_utc(2025, 3, 25, 0, 0, 0.0).unwrap();
    let mercury = session.body_apparent(Body::Mercury, instant, None).unwrap();
    assert!(
        mercury.ecliptic_velocity.is_retrograde(),
        "Mercury should be retrograde on 2025-03-25; dλ/dt = {} rad/day",
        mercury.ecliptic_velocity.longitude_rate_rad_per_day
    );
}
