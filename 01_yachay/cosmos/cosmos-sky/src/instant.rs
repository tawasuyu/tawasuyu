//! A civil moment in time, with on-demand conversions to the dynamical
//! time scales used by ephemerides.
//!
//! `Instant` is the entry point for everything astronomical: an
//! astrologer types in birth data, builds an `Instant`, and from there
//! every downstream computation (planet positions, houses, sidereal time,
//! progressions, returns) consumes the same instant.
//!
//! The internal representation is **UTC** with sub-nanosecond split-JD
//! precision (inherited from `cosmos_time::UTC`). Conversion to TT / TDB
//! / UT1 happens on demand and is cheap (`<1 µs` for TT, `~ms` for TDB
//! because of the Fairhead-Bretagnon series).

use std::str::FromStr;

use cosmos_time::julian::JulianDate;
use cosmos_time::scales::conversions::{
    ToTAI, ToTDB, ToTT, ToTTFromTDB, ToUT1WithDeltaT,
};
use cosmos_time::scales::utc::utc_from_calendar;
use cosmos_time::{TDB, TT, UT1, UTC};

use crate::delta_t::delta_t_seconds;
use crate::error::{SkyError, SkyResult};

/// A civil moment in time. Cheap to clone and copy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Instant {
    utc: UTC,
}

impl Instant {
    /// Build from UTC calendar components.
    ///
    /// `second` is a real number (e.g. `12.345` for 12.345 seconds), so
    /// sub-second precision is preserved.
    pub fn from_civil_utc(
        year: i32,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: f64,
    ) -> SkyResult<Self> {
        validate_calendar(year, month, day, hour, minute, second)?;
        Ok(Self {
            utc: utc_from_calendar(year, month, day, hour, minute, second),
        })
    }

    /// Build from a *local* civil time and a UTC offset in minutes.
    ///
    /// `tz_offset_minutes` is positive east of Greenwich (e.g. `-240` for
    /// EST/Caracas, `+330` for India Standard Time). The local clock
    /// reading is converted to UTC by **subtracting** the offset.
    pub fn from_civil_local(
        year: i32,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: f64,
        tz_offset_minutes: i32,
    ) -> SkyResult<Self> {
        validate_calendar(year, month, day, hour, minute, second)?;
        let local = utc_from_calendar(year, month, day, hour, minute, second);
        let offset_seconds = tz_offset_minutes as f64 * 60.0;
        Ok(Self {
            utc: local.add_seconds(-offset_seconds),
        })
    }

    /// Parse an ISO 8601 UTC timestamp (e.g. `"1987-03-14T09:22:00"`).
    /// Time zone designators other than `Z` are not yet honored — use
    /// [`Instant::from_civil_local`] for those.
    pub fn from_utc_iso8601(s: &str) -> SkyResult<Self> {
        let trimmed = s.trim().trim_end_matches('Z');
        let utc = UTC::from_str(trimmed)
            .map_err(|e| SkyError::InvalidIso8601(format!("{}: {}", s, e)))?;
        Ok(Self { utc })
    }

    /// Build from a Unix timestamp (seconds + sub-second nanoseconds
    /// since 1970-01-01T00:00:00Z).
    pub fn from_unix(seconds: i64, nanos: u32) -> Self {
        Self {
            utc: UTC::new(seconds, nanos),
        }
    }

    /// Build from an explicit `UTC` time-scale value.
    pub fn from_utc(utc: UTC) -> Self {
        Self { utc }
    }

    /// Build from an explicit TDB Julian Date. Mainly an escape hatch for
    /// callers reproducing existing fixtures.
    pub fn from_jd_tdb(jd_tdb: f64) -> SkyResult<Self> {
        let tdb = TDB::from_julian_date(JulianDate::new(jd_tdb, 0.0));
        let tt = tdb
            .to_tt_greenwich()
            .map_err(SkyError::Time)?;
        // TT → TAI → UTC.
        let tai = tt.to_tai().map_err(SkyError::Time)?;
        use cosmos_time::scales::conversions::ToUTC;
        let utc = tai.to_utc().map_err(SkyError::Time)?;
        Ok(Self { utc })
    }

    /// Wall-clock now (UTC).
    pub fn now() -> Self {
        Self { utc: UTC::now() }
    }

    /// Underlying UTC value.
    pub fn utc(&self) -> UTC {
        self.utc
    }

    /// Convert to TT (Terrestrial Time). Cheap: integer leap-second
    /// lookup + a constant `32.184 s` offset.
    pub fn tt(&self) -> SkyResult<TT> {
        let tai = self.utc.to_tai().map_err(SkyError::Time)?;
        tai.to_tt().map_err(SkyError::Time)
    }

    /// Convert to TDB (Barycentric Dynamical Time), using Greenwich as
    /// reference location. Adequate for sub-millisecond astrology and
    /// sub-mas geocentric astrometry.
    pub fn tdb(&self) -> SkyResult<TDB> {
        self.tt()?.to_tdb_greenwich().map_err(SkyError::Time)
    }

    /// Convert to UT1 using the bundled ΔT table.
    pub fn ut1(&self) -> SkyResult<UT1> {
        let tt = self.tt()?;
        tt.to_ut1_with_delta_t(self.delta_t_seconds())
            .map_err(SkyError::Time)
    }

    /// TDB Julian Date as an `f64`. Convenient for ephemeris APIs.
    pub fn jd_tdb(&self) -> SkyResult<f64> {
        Ok(self.tdb()?.to_julian_date().to_f64())
    }

    /// UTC Julian Date as an `f64`.
    pub fn jd_utc(&self) -> f64 {
        self.utc.to_julian_date().to_f64()
    }

    /// ΔT (= TT − UT1) in seconds at this instant, from the bundled
    /// IERS table.
    pub fn delta_t_seconds(&self) -> f64 {
        // The ΔT table is evaluated at the JD level — the difference
        // between UTC and TT for this purpose is sub-ms and irrelevant.
        delta_t_seconds(self.jd_utc())
    }

    /// ISO 8601 UTC string with millisecond precision.
    pub fn to_iso8601(&self) -> String {
        self.utc.to_iso8601()
    }
}

impl std::fmt::Display for Instant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.utc.to_iso8601())
    }
}

fn validate_calendar(
    year: i32,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: f64,
) -> SkyResult<()> {
    let in_range = (1..=12).contains(&month)
        && (1..=31).contains(&day)
        && hour < 24
        && minute < 60
        && (0.0..61.0).contains(&second);
    if !in_range {
        return Err(SkyError::InvalidCivilTime(format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:09.6}",
            year, month, day, hour, minute, second
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_civil_utc_round_trips_to_iso() {
        let t = Instant::from_civil_utc(2026, 5, 15, 12, 0, 0.0).unwrap();
        let iso = t.to_iso8601();
        assert!(iso.starts_with("2026-05-15T12:00:00"), "got {}", iso);
    }

    #[test]
    fn local_offset_shifts_correctly() {
        // 09:00 in Caracas (UTC-4) is 13:00 UTC.
        let local = Instant::from_civil_local(2026, 5, 15, 9, 0, 0.0, -240).unwrap();
        let utc = Instant::from_civil_utc(2026, 5, 15, 13, 0, 0.0).unwrap();
        assert!((local.jd_utc() - utc.jd_utc()).abs() < 1e-9);
    }

    #[test]
    fn delta_t_in_modern_era_is_reasonable() {
        let t = Instant::from_civil_utc(2024, 1, 1, 0, 0, 0.0).unwrap();
        let dt = t.delta_t_seconds();
        assert!(
            (60.0..80.0).contains(&dt),
            "ΔT at 2024 should be ~69 s, got {}",
            dt
        );
    }

    #[test]
    fn tdb_roundtrips_through_jd() {
        let t = Instant::from_civil_utc(2000, 1, 1, 12, 0, 0.0).unwrap();
        let jd_tdb = t.jd_tdb().unwrap();
        // J2000 TDB ≈ 2451545.0007428 (TT-UTC = 64.184 s at J2000, plus
        // ~1.6 ms for TDB-TT).
        assert!(
            (2_451_545.000_5..2_451_545.001_0).contains(&jd_tdb),
            "got JD_TDB = {}",
            jd_tdb
        );
    }

    #[test]
    fn invalid_month_is_rejected() {
        assert!(Instant::from_civil_utc(2026, 13, 1, 0, 0, 0.0).is_err());
    }
}
