//! Hellenistic profections.
//!
//! In the profection technique, each year of life corresponds to one
//! house in the natal chart, advancing one house per year and cycling
//! every twelve years:
//!
//! | Age (years)        | Profected house |
//! |--------------------|-----------------|
//! | 0, 12, 24, 36, …   | House 1 (Asc)   |
//! | 1, 13, 25, 37, …   | House 2         |
//! | 2, 14, 26, …       | House 3         |
//! | ...                | ...             |
//! | 11, 23, 35, …      | House 12        |
//!
//! The sign on that house (Whole-Sign by convention — the most common
//! framing — though any house-system mapping is allowed) gives the
//! **profected sign**; its traditional ruler is the **Lord of the
//! Year**. Monthly and daily profections subdivide the same cycle.
//!
//! This module uses Whole-Sign assignment by default: profected house
//! `n` lands on the `n`-th sign counted from the natal Ascendant's
//! sign. Callers who prefer to align profections with their natal
//! chart's actual house system can pass `ProfectionHouses::Quadrant`.

use cosmos_sky::{Body, Instant};

use crate::chart::NatalChart;
use crate::zodiac::Sign;

/// Which house-frame to use when picking the profected *sign*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProfectionHouses {
    /// Profected sign = `n`-th sign from the natal Asc's sign. The
    /// classical Hellenistic convention.
    #[default]
    WholeSign,
    /// Profected sign = sign of the natal `n`-th house cusp.
    Quadrant,
}

/// One year's profection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnnualProfection {
    /// Whole years since birth.
    pub age_years: u32,
    /// Profected house, `1..=12`.
    pub profected_house: u8,
    /// Profected sign.
    pub profected_sign: Sign,
    /// Traditional ruler of the profected sign.
    pub lord_of_year: Body,
    /// Modern ruler of the profected sign (only differs for Scorpio
    /// → Pluto, Aquarius → Uranus, Pisces → Neptune).
    pub modern_lord_of_year: Body,
}

/// One month's profection within a profected year. Months advance one
/// house at the same cadence as years — so the year's first month lands
/// on the same house as the year itself, the second month on the next
/// house, and so on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonthlyProfection {
    pub annual: AnnualProfection,
    /// Months into the profected year, `0..=11`.
    pub month_in_year: u8,
    pub profected_house: u8,
    pub profected_sign: Sign,
    pub lord_of_month: Body,
}

/// Compute the annual profection for a given age in whole years.
pub fn annual_profection(
    chart: &NatalChart,
    age_years: u32,
    houses_frame: ProfectionHouses,
) -> AnnualProfection {
    let profected_house = ((age_years % 12) + 1) as u8;
    let profected_sign = profected_sign(chart, profected_house, houses_frame);
    AnnualProfection {
        age_years,
        profected_house,
        profected_sign,
        lord_of_year: traditional_ruler(profected_sign),
        modern_lord_of_year: modern_ruler(profected_sign),
    }
}

/// Compute the monthly profection for a given age and month-in-year.
/// `month_in_year = 0` is the first month (lands on the annual house);
/// `month_in_year = 11` is the last (one house before next year).
pub fn monthly_profection(
    chart: &NatalChart,
    age_years: u32,
    month_in_year: u8,
    houses_frame: ProfectionHouses,
) -> MonthlyProfection {
    let annual = annual_profection(chart, age_years, houses_frame);
    let house = ((annual.profected_house as u32 - 1 + month_in_year as u32) % 12 + 1) as u8;
    let sign = profected_sign(chart, house, houses_frame);
    MonthlyProfection {
        annual,
        month_in_year,
        profected_house: house,
        profected_sign: sign,
        lord_of_month: traditional_ruler(sign),
    }
}

/// Compute the profection in effect at `at`, taken as a "Solar Return
/// anniversary" cadence: each new profection year begins at the year's
/// solar return. This helper computes age in *whole* years from
/// `birth_instant` to `at` using a 365.2422-day average — caller can
/// supply a more accurate `(age_years, month_in_year)` via the
/// non-`_at` functions if needed.
pub fn profection_at(
    chart: &NatalChart,
    at: Instant,
    houses_frame: ProfectionHouses,
) -> MonthlyProfection {
    const TROPICAL_YEAR_DAYS: f64 = 365.242_190;
    const MONTH_DAYS: f64 = TROPICAL_YEAR_DAYS / 12.0;

    let days_elapsed = at.jd_utc() - chart.birth.instant.jd_utc();
    let elapsed_years = days_elapsed / TROPICAL_YEAR_DAYS;
    if days_elapsed < 0.0 {
        // Pre-natal date: clamp to age 0 month 0.
        return monthly_profection(chart, 0, 0, houses_frame);
    }
    let age_years = elapsed_years.floor() as u32;
    let day_in_year = days_elapsed - (age_years as f64) * TROPICAL_YEAR_DAYS;
    let month_in_year = ((day_in_year / MONTH_DAYS).floor() as i32).clamp(0, 11) as u8;
    monthly_profection(chart, age_years, month_in_year, houses_frame)
}

/// Traditional (Hellenistic) sign ruler.
pub fn traditional_ruler(sign: Sign) -> Body {
    match sign {
        Sign::Aries => Body::Mars,
        Sign::Taurus => Body::Venus,
        Sign::Gemini => Body::Mercury,
        Sign::Cancer => Body::Moon,
        Sign::Leo => Body::Sun,
        Sign::Virgo => Body::Mercury,
        Sign::Libra => Body::Venus,
        Sign::Scorpio => Body::Mars,
        Sign::Sagittarius => Body::Jupiter,
        Sign::Capricorn => Body::Saturn,
        Sign::Aquarius => Body::Saturn,
        Sign::Pisces => Body::Jupiter,
    }
}

/// Modern (post-Uranus discovery) sign ruler. Differs from
/// [`traditional_ruler`] only for Scorpio (Pluto), Aquarius (Uranus),
/// and Pisces (Neptune).
pub fn modern_ruler(sign: Sign) -> Body {
    match sign {
        Sign::Scorpio => Body::Pluto,
        Sign::Aquarius => Body::Uranus,
        Sign::Pisces => Body::Neptune,
        other => traditional_ruler(other),
    }
}

// ─── Internals ─────────────────────────────────────────────────────────

fn profected_sign(chart: &NatalChart, house_number: u8, frame: ProfectionHouses) -> Sign {
    let i = ((house_number as i32 - 1) % 12 + 12) % 12;
    match frame {
        ProfectionHouses::WholeSign => {
            let asc_sign = chart.ascendant().sign().index() as i32;
            Sign::from_index(((asc_sign + i) % 12) as usize)
        }
        ProfectionHouses::Quadrant => {
            let cusp_rad = chart.houses.cusps[i as usize];
            crate::zodiac::SignedLongitude::from_radians(
                (cusp_rad - chart.ayanamsha_rad).rem_euclid(std::f64::consts::TAU),
            )
            .sign()
        }
    }
}
