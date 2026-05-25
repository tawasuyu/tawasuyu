//! Print a complete natal chart: angles, houses, placements, aspects.
//!
//! Run with `cargo run --example natal_chart -p eternal-astrology` — uses
//! the analytical VSOP2013 backend, so no kernels need to be downloaded.
//! For sub-mas precision you would swap in a JPL SPK kernel via
//! `SessionConfig::with_spk(...)`.

use cosmos_astrology::{
    find_aspects, AspectKind, BirthData, ChartConfig, HouseSystem, NatalChart, OrbTable, Zodiac,
};
use cosmos_sky::{EphemerisSession, Instant, Observer, SessionConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ── Birth data ────────────────────────────────────────────────────
    // Demo subject: 14 March 1987, 05:22 local time in Caracas (UTC−4).
    // Change these constants to compute another chart.
    let instant = Instant::from_civil_local(1987, 3, 14, 5, 22, 0.0, -240)?;
    let observer = Observer::from_degrees(10.4806, -66.9036, 900.0);
    let birth = BirthData::new(instant, observer).with_name("Demo Subject");

    // ── Session + configuration ───────────────────────────────────────
    let session = EphemerisSession::open(SessionConfig::vsop2013())?;
    let config = ChartConfig {
        house_system: HouseSystem::Placidus,
        zodiac: Zodiac::Tropical,
        ..ChartConfig::default()
    };

    let chart = NatalChart::compute(&birth, &config, &session)?;

    // ── Header ────────────────────────────────────────────────────────
    println!("Natal Chart — {}", birth.name.as_deref().unwrap_or("(unnamed)"));
    println!("  UTC instant : {}", chart.birth.instant);
    println!(
        "  Location    : lat {:+.4}°  lon {:+.4}°  elev {} m",
        birth.observer.lat_rad.to_degrees(),
        birth.observer.lon_rad.to_degrees(),
        birth.observer.elev_m as i32,
    );
    println!("  House system: {:?}", config.house_system);
    println!("  Zodiac      : {:?}", config.zodiac);
    println!();

    // ── Angles ────────────────────────────────────────────────────────
    println!("Angles");
    println!("  Asc : {}", chart.ascendant().to_chart_format());
    println!("  MC  : {}", chart.midheaven().to_chart_format());
    println!("  Desc: {}", chart.descendant().to_chart_format());
    println!("  IC  : {}", chart.imum_coeli().to_chart_format());
    println!();

    // ── Houses ────────────────────────────────────────────────────────
    println!("House Cusps");
    for (i, cusp) in chart.houses.cusps.iter().enumerate() {
        let sl = cosmos_astrology::SignedLongitude::from_radians(*cusp);
        println!("  H{:>2}: {}", i + 1, sl.to_chart_format());
    }
    println!();

    // ── Placements ────────────────────────────────────────────────────
    println!("Placements");
    println!("  {:<12}  {:<14}  {:>5}  {:>4}",
        "Body", "Position", "House", "Mode");
    for p in &chart.placements {
        println!("  {:<12}  {:<14}  H{:>3}  {:>4}",
            p.body.name(),
            p.longitude.to_chart_format(),
            p.house_number,
            if p.is_retrograde() { "R" } else { "" },
        );
    }
    println!();

    // ── Aspects ───────────────────────────────────────────────────────
    println!("Aspects (modern Western orbs)");
    let aspects = find_aspects(&chart, &OrbTable::modern_western());
    let majors: Vec<_> = aspects
        .iter()
        .filter(|a| AspectKind::MAJORS.contains(&a.kind))
        .collect();
    println!("  {:<10}  {:<14}  {:<10}  {:>6}  {}", "A", "Aspect", "B", "Orb", "Phase");
    for a in &majors {
        println!("  {:<10}  {:<14}  {:<10}  {:>5.2}°  {}",
            a.a.name(),
            format!("{:?}", a.kind),
            a.b.name(),
            a.orb_abs_deg(),
            if a.applying { "applying" } else { "separating" },
        );
    }
    if majors.is_empty() {
        println!("  (no major aspects within configured orbs)");
    }

    Ok(())
}
