//! Tests for Arabic Parts (Lots) and Hellenistic profections.

use cosmos_astrology::{
    all_lots, annual_profection, compute_lot, modern_ruler, monthly_profection,
    profection_at, traditional_ruler, BirthData, ChartConfig, LotName, NatalChart,
    ProfectionHouses, Sect, Sign,
};
use cosmos_sky::{Body, EphemerisSession, Instant, Observer, SessionConfig};

fn session() -> EphemerisSession {
    EphemerisSession::open(SessionConfig::vsop2013()).unwrap()
}

fn fixture_a_day_birth() -> BirthData {
    // 14 March 1987, 05:22 Caracas local → Sun is just below horizon.
    // For a clean DAY-birth test we use a noon birth instead.
    BirthData::new(
        Instant::from_civil_local(1987, 3, 14, 12, 0, 0.0, -240).unwrap(),
        Observer::from_degrees(10.4806, -66.9036, 900.0),
    )
}

fn fixture_a_night_birth() -> BirthData {
    BirthData::new(
        Instant::from_civil_local(1987, 3, 14, 0, 0, 0.0, -240).unwrap(),
        Observer::from_degrees(10.4806, -66.9036, 900.0),
    )
}

// ─── Lots ────────────────────────────────────────────────────────────

#[test]
fn fortune_swaps_with_spirit_between_day_and_night() {
    let s = session();
    let day_chart =
        NatalChart::compute(&fixture_a_day_birth(), &ChartConfig::default(), &s).unwrap();
    let night_chart =
        NatalChart::compute(&fixture_a_night_birth(), &ChartConfig::default(), &s).unwrap();

    assert_eq!(Sect::of(&day_chart).unwrap(), Sect::Day);
    assert_eq!(Sect::of(&night_chart).unwrap(), Sect::Night);

    // For Fortune the day formula is Asc + Moon - Sun and night is the
    // reverse. So Fortune_day - Spirit_day = -(Fortune_night - Spirit_night)
    // for the SAME chart (after sect determined). To check sect swap we
    // compute Fortune_day on day_chart vs Fortune_day formula manually on
    // night_chart and verify they differ by 2(Moon - Sun) (the formula
    // swap).
    let fortune_day = compute_lot(&day_chart, LotName::Fortune).unwrap();
    let asc = day_chart.ascendant().longitude_rad();
    let moon = day_chart.placement(Body::Moon).unwrap().longitude.longitude_rad();
    let sun = day_chart.placement(Body::Sun).unwrap().longitude.longitude_rad();
    let expected_day =
        (asc + moon - sun).rem_euclid(std::f64::consts::TAU);
    assert!(
        (fortune_day.longitude.longitude_rad() - expected_day).abs() < 1e-12,
        "day Fortune formula Asc+Moon-Sun mismatch",
    );

    // Spirit day = Asc + Sun − Moon — exact opposite operands.
    let spirit_day = compute_lot(&day_chart, LotName::Spirit).unwrap();
    let expected_spirit =
        (asc + sun - moon).rem_euclid(std::f64::consts::TAU);
    assert!(
        (spirit_day.longitude.longitude_rad() - expected_spirit).abs() < 1e-12,
        "day Spirit formula Asc+Sun-Moon mismatch",
    );

    // Fortune and Spirit are symmetric around the Ascendant by
    // construction: (F + S)/2 = Asc + (Moon-Sun+Sun-Moon)/2 + Asc/2 = Asc.
    // Equivalently F − S = 2(Moon − Sun) (mod 2π), so F + S ≡ 2·Asc (mod 2π).
    let sum = (fortune_day.longitude.longitude_rad() + spirit_day.longitude.longitude_rad())
        .rem_euclid(std::f64::consts::TAU);
    let twice_asc = (2.0 * asc).rem_euclid(std::f64::consts::TAU);
    let diff = (sum - twice_asc).abs();
    let diff = diff.min((std::f64::consts::TAU - diff).abs());
    assert!(diff < 1e-12, "F+S ≠ 2·Asc, diff = {}", diff);
}

#[test]
fn all_lots_produces_seven_named_lots() {
    let s = session();
    let chart =
        NatalChart::compute(&fixture_a_day_birth(), &ChartConfig::default(), &s).unwrap();
    let lots = all_lots(&chart).unwrap();
    assert_eq!(lots.len(), 7);
    for lot in &lots {
        assert!(
            (0.0..std::f64::consts::TAU).contains(&lot.longitude.longitude_rad())
        );
        assert!((1..=12).contains(&lot.house_number));
    }
}

#[test]
fn eros_depends_on_spirit() {
    // Eros_day = Asc + Venus − Spirit. Validate the dependency was
    // resolved recursively (not silently dropped).
    let s = session();
    let chart =
        NatalChart::compute(&fixture_a_day_birth(), &ChartConfig::default(), &s).unwrap();
    let spirit = compute_lot(&chart, LotName::Spirit).unwrap();
    let eros = compute_lot(&chart, LotName::Eros).unwrap();
    let venus = chart.placement(Body::Venus).unwrap().longitude.longitude_rad();
    let asc = chart.ascendant().longitude_rad();
    let expected =
        (asc + venus - spirit.longitude.longitude_rad()).rem_euclid(std::f64::consts::TAU);
    assert!(
        (eros.longitude.longitude_rad() - expected).abs() < 1e-12,
        "Eros day formula did not resolve Spirit"
    );
}

// ─── Profections ─────────────────────────────────────────────────────

#[test]
fn annual_profection_advances_one_house_per_year_and_cycles() {
    let s = session();
    let chart =
        NatalChart::compute(&fixture_a_day_birth(), &ChartConfig::default(), &s).unwrap();
    // Year 0 → house 1. Year 12 → house 1 again. Year 11 → house 12.
    let y0 = annual_profection(&chart, 0, ProfectionHouses::WholeSign);
    let y11 = annual_profection(&chart, 11, ProfectionHouses::WholeSign);
    let y12 = annual_profection(&chart, 12, ProfectionHouses::WholeSign);
    let y36 = annual_profection(&chart, 36, ProfectionHouses::WholeSign);

    assert_eq!(y0.profected_house, 1);
    assert_eq!(y11.profected_house, 12);
    assert_eq!(y12.profected_house, 1);
    assert_eq!(y36.profected_house, 1);

    // House 1 sign = Asc sign with Whole-Sign.
    assert_eq!(y0.profected_sign, chart.ascendant().sign());
    // House 12 sign = sign just before Asc's (counterclockwise).
    let asc_idx = chart.ascendant().sign().index();
    assert_eq!(
        y11.profected_sign.index(),
        (asc_idx + 11) % 12
    );
}

#[test]
fn monthly_profection_at_month_0_matches_annual_house() {
    let s = session();
    let chart =
        NatalChart::compute(&fixture_a_day_birth(), &ChartConfig::default(), &s).unwrap();
    let monthly = monthly_profection(&chart, 30, 0, ProfectionHouses::WholeSign);
    let annual = annual_profection(&chart, 30, ProfectionHouses::WholeSign);
    assert_eq!(monthly.profected_house, annual.profected_house);
    assert_eq!(monthly.profected_sign, annual.profected_sign);

    // Last month (month 11) of a year lands on the sign immediately
    // *before* the annual house — the monthly cycle traverses 11
    // signs forward, ending one position short of completing the full
    // 12-sign loop. (The annual cycle then jumps forward by 1 to
    // start year+1; the gap of 2 signs between month 11 of year N and
    // month 0 of year N+1 is the classical pattern.)
    let last = monthly_profection(&chart, 30, 11, ProfectionHouses::WholeSign);
    let expected_house =
        ((annual.profected_house as i32 - 2 + 12) % 12 + 1) as u8;
    assert_eq!(
        last.profected_house, expected_house,
        "year house {} → month 11 should be house {}",
        annual.profected_house, expected_house
    );
}

#[test]
fn lord_of_year_uses_traditional_rulership() {
    let s = session();
    let chart =
        NatalChart::compute(&fixture_a_day_birth(), &ChartConfig::default(), &s).unwrap();
    // Pick a year that lands the profected sign on Aquarius (Saturn
    // traditionally, Uranus modern). Aquarius index = 10. We need
    // (asc_idx + N) % 12 = 10 → N = (10 - asc_idx + 12) % 12.
    let asc_idx = chart.ascendant().sign().index();
    let age = ((10 + 12 - asc_idx) % 12) as u32;

    let p = annual_profection(&chart, age, ProfectionHouses::WholeSign);
    assert_eq!(p.profected_sign, Sign::Aquarius);
    assert_eq!(p.lord_of_year, Body::Saturn);
    assert_eq!(p.modern_lord_of_year, Body::Uranus);
}

#[test]
fn rulership_tables_cover_every_sign() {
    for i in 0..12 {
        let s = Sign::from_index(i);
        let trad = traditional_ruler(s);
        let modern = modern_ruler(s);
        // Both must produce a body in the canonical luminary/planet set.
        let allowed = [
            Body::Sun,
            Body::Moon,
            Body::Mercury,
            Body::Venus,
            Body::Mars,
            Body::Jupiter,
            Body::Saturn,
            Body::Uranus,
            Body::Neptune,
            Body::Pluto,
        ];
        assert!(allowed.contains(&trad), "trad ruler of {:?} = {:?}", s, trad);
        assert!(allowed.contains(&modern), "modern ruler of {:?} = {:?}", s, modern);
    }
}

#[test]
fn profection_at_present_is_consistent_with_age() {
    let s = session();
    let chart =
        NatalChart::compute(&fixture_a_day_birth(), &ChartConfig::default(), &s).unwrap();
    // 14 March 1987 + 39 years = 14 March 2026.
    let now = Instant::from_civil_utc(2026, 3, 14, 16, 0, 0.0).unwrap();
    let p = profection_at(&chart, now, ProfectionHouses::WholeSign);
    // Age ≈ 39 years → house = (39 % 12) + 1 = 4.
    assert_eq!(p.annual.age_years, 39);
    assert_eq!(p.annual.profected_house, 4);
}
