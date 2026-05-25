//! Secondary, tertiary, and minor progressions.
//!
//! Progressions advance a natal chart in time using a symbolic
//! "day-for-a-period" rate. Each method picks a different *period*:
//!
//! | Method | 1 day of ephemeris ↔ | Approx. shift per year of life |
//! |---|---|---|
//! | Secondary | 1 tropical year (≈ 365.2422 d) | 1 day |
//! | Tertiary | 1 mean synodic month (≈ 29.5306 d) | ≈ 12.4 days |
//! | Minor | 1 mean sidereal month (≈ 27.3217 d) | ≈ 13.4 days |
//!
//! Every progression reduces to the same three steps:
//!
//! 1. Compute a *progressed instant* = `birth + (life_years / period_years) days`.
//! 2. Recompute a full `NatalChart` at the progressed instant — using
//!    the **natal observer**, not the location of the subject at age N.
//! 3. Wrap the natal + progressed pair so callers can compare.
//!
//! Houses are recomputed at the progressed instant by default (the
//! Swiss / Astrodienst convention). Pass [`ProgressedHouses::Natal`] to
//! freeze the natal cusps and only progress the bodies.

use cosmos_sky::{EphemerisSession, Instant};

use crate::birth_data::BirthData;
use crate::chart::NatalChart;
use crate::error::AstrologyResult;

/// Which symbolic period a day of ephemeris represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressionMethod {
    /// Secondary (day-for-a-year). The classical Naibod / Ptolemy method.
    Secondary,
    /// Tertiary (day-for-a-mean-synodic-month). Brahy variant.
    Tertiary,
    /// Minor (day-for-a-mean-sidereal-month).
    Minor,
}

impl ProgressionMethod {
    /// Period associated with this method, in mean solar days. One day
    /// of ephemeris corresponds to one of these.
    pub fn period_days(self) -> f64 {
        match self {
            // Tropical year (vernal-equinox to vernal-equinox).
            ProgressionMethod::Secondary => 365.242_190,
            // Mean synodic month (new-moon to new-moon).
            ProgressionMethod::Tertiary => 29.530_588_85,
            // Mean sidereal month (Moon's return to the same star).
            ProgressionMethod::Minor => 27.321_661,
        }
    }
}

/// How to handle the houses of a progressed chart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProgressedHouses {
    /// Recompute houses at the progressed instant using the natal
    /// observer's coordinates. (Swiss / Astrodienst default.)
    #[default]
    Progressed,
    /// Reuse the natal house cusps unchanged. The progressed planets
    /// are placed against the natal house framework.
    Natal,
}

/// Compute the *progressed instant* corresponding to a target age in
/// life, for the given method. The result is a real instant on the
/// ephemeris timeline.
pub fn progressed_instant(
    birth: Instant,
    target_age_years: f64,
    method: ProgressionMethod,
) -> Instant {
    let life_days = target_age_years * ProgressionMethod::Secondary.period_days();
    let shift_days = life_days / method.period_days();
    Instant::from_utc(birth.utc().add_days(shift_days))
}

/// A progressed chart bundled with the natal chart it derives from.
#[derive(Debug, Clone)]
pub struct ProgressedChart {
    pub natal: NatalChart,
    pub progressed: NatalChart,
    pub method: ProgressionMethod,
    pub houses_treatment: ProgressedHouses,
    pub target_age_years: f64,
    pub progressed_instant: Instant,
}

impl ProgressedChart {
    pub fn progressed(&self) -> &NatalChart {
        &self.progressed
    }
    pub fn natal(&self) -> &NatalChart {
        &self.natal
    }
}

/// Build a progressed chart at the requested age, using `method` and
/// `houses_treatment`.
pub fn progress(
    natal: &NatalChart,
    session: &EphemerisSession,
    target_age_years: f64,
    method: ProgressionMethod,
    houses_treatment: ProgressedHouses,
) -> AstrologyResult<ProgressedChart> {
    let prog_instant = progressed_instant(natal.birth.instant, target_age_years, method);

    let prog_birth = BirthData {
        instant: prog_instant,
        observer: natal.birth.observer,
        name: natal.birth.name.clone(),
        time_certainty: natal.birth.time_certainty,
        note: natal.birth.note.clone(),
    };

    let mut progressed = NatalChart::compute(&prog_birth, &natal.config, session)?;

    if houses_treatment == ProgressedHouses::Natal {
        // Replace the freshly-computed houses with the natal ones, then
        // re-assign every body to its natal-frame house number. Other
        // chart geometry (asc/mc/etc.) reflects the natal angles.
        progressed.houses = natal.houses;
        progressed.local_apparent_sidereal_time_rad =
            natal.local_apparent_sidereal_time_rad;
        progressed.obliquity_rad = natal.obliquity_rad;
        // Asc / MC / Desc / IC come from the *natal* angles in radians,
        // but the SignedLongitude needs to be rebuilt to honour the
        // progressed chart's zodiac (which is identical to the natal's
        // ChartConfig, so the shift logic matches).
        progressed.replace_angles_with(natal);
        for p in progressed.placements.iter_mut() {
            p.house_number = natal.houses.house_containing(
                p.longitude.longitude_rad() + progressed.ayanamsha_rad,
            );
        }
    }

    Ok(ProgressedChart {
        natal: natal.clone(),
        progressed,
        method,
        houses_treatment,
        target_age_years,
        progressed_instant: prog_instant,
    })
}

/// Convenience: secondary progression with the default house treatment.
pub fn secondary_progression(
    natal: &NatalChart,
    session: &EphemerisSession,
    target_age_years: f64,
) -> AstrologyResult<ProgressedChart> {
    progress(
        natal,
        session,
        target_age_years,
        ProgressionMethod::Secondary,
        ProgressedHouses::default(),
    )
}

/// Convenience: tertiary progression with the default house treatment.
pub fn tertiary_progression(
    natal: &NatalChart,
    session: &EphemerisSession,
    target_age_years: f64,
) -> AstrologyResult<ProgressedChart> {
    progress(
        natal,
        session,
        target_age_years,
        ProgressionMethod::Tertiary,
        ProgressedHouses::default(),
    )
}

/// Convenience: minor progression (1 day = 1 sidereal month).
pub fn minor_progression(
    natal: &NatalChart,
    session: &EphemerisSession,
    target_age_years: f64,
) -> AstrologyResult<ProgressedChart> {
    progress(
        natal,
        session,
        target_age_years,
        ProgressionMethod::Minor,
        ProgressedHouses::default(),
    )
}
