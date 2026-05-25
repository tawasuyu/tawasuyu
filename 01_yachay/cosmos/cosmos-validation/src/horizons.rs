//! JPL Horizons API client.
//!
//! Fetches state vectors (position + velocity) for a (body, center, jd_tdb)
//! query and serialises them as `Fixture` records. This module is only
//! compiled when the `fetch` feature is enabled, so the core validation
//! crate stays network-free.
//!
//! Horizons reference: <https://ssd-api.jpl.nasa.gov/doc/horizons.html>

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

use cosmos_core::constants::{AU_KM, SECONDS_PER_DAY_F64};
use cosmos_time::julian::JulianDate;
use cosmos_time::scales::ToTTFromTDB;
use cosmos_time::TDB;

use crate::fixture::{Corrections, Fixture, Frame, Source, Tolerance};

const HORIZONS_URL: &str = "https://ssd.jpl.nasa.gov/api/horizons.api";

#[derive(Debug, Deserialize)]
struct HorizonsResponse {
    result: String,
    #[serde(default)]
    signature: Option<HorizonsSignature>,
}

#[derive(Debug, Deserialize)]
struct HorizonsSignature {
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    source: Option<String>,
}

pub struct HorizonsFetcher {
    client: reqwest::blocking::Client,
}

impl HorizonsFetcher {
    pub fn new() -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .user_agent("eternal-validation/0.1")
            .timeout(std::time::Duration::from_secs(60))
            .build()?;
        Ok(Self { client })
    }

    /// Fetch a single state vector. Returns it as a Fixture with `Source::Horizons`.
    ///
    /// `body` and `center` follow NAIF integer IDs. `name` is a human label
    /// used for table rendering and JSON readability. `corrections` is
    /// translated to a Horizons `VEC_CORR` flag: empty → `NONE` (geometric),
    /// `{light_time}` → `LT`, `{light_time + stellar_aberration}` → `LT+S`.
    /// Fetch via Horizons EPHEM_TYPE='OBSERVER' as astrometric J2000 RA/Dec
    /// (QUANTITIES='1,20'), converted to a Cartesian position vector in
    /// ICRF km. Velocity is left at `[0, 0, 0]`.
    pub fn fetch_observer_astrometric(
        &self,
        name: &str,
        body: i32,
        observer: i32,
        jd_tdb: f64,
        tolerance: Tolerance,
    ) -> Result<Fixture> {
        self.fetch_observer_inner(name, body, observer, jd_tdb, tolerance, "1,20", Frame::Icrf)
    }

    /// Fetch via Horizons EPHEM_TYPE='OBSERVER' as apparent RA/Dec
    /// (QUANTITIES='2,20'). Output is in **true equator and equinox of
    /// date** — the classical equinox-based apparent frame — converted
    /// to a Cartesian vector in km. Velocity is `[0, 0, 0]`.
    pub fn fetch_observer_apparent(
        &self,
        name: &str,
        body: i32,
        observer: i32,
        jd_tdb: f64,
        tolerance: Tolerance,
    ) -> Result<Fixture> {
        self.fetch_observer_inner(
            name,
            body,
            observer,
            jd_tdb,
            tolerance,
            "2,20",
            Frame::TrueEquatorEquinoxOfDate,
        )
    }

    fn fetch_observer_inner(
        &self,
        name: &str,
        body: i32,
        observer: i32,
        jd_tdb: f64,
        tolerance: Tolerance,
        quantities: &str,
        output_frame: Frame,
    ) -> Result<Fixture> {
        let jd_tt = tdb_jd_to_tt_jd(jd_tdb)?;
        let command = format!("'{}'", body);
        let center_str = format!("'@{}'", observer);
        let tlist = format!("'{:.9}'", jd_tt);
        let quantities_str = format!("'{}'", quantities);

        let params = [
            ("format", "json"),
            ("COMMAND", &command),
            ("OBJ_DATA", "'NO'"),
            ("MAKE_EPHEM", "'YES'"),
            ("EPHEM_TYPE", "'OBSERVER'"),
            ("CENTER", &center_str),
            ("TLIST", &tlist),
            ("TLIST_TYPE", "'JD'"),
            ("TIME_TYPE", "'TT'"),
            ("QUANTITIES", &quantities_str),
            ("ANG_FORMAT", "'DEG'"),
            ("EXTRA_PREC", "'YES'"),
            ("CSV_FORMAT", "'YES'"),
            ("REF_SYSTEM", "'ICRF'"),
        ];

        let resp: HorizonsResponse = self
            .client
            .get(HORIZONS_URL)
            .query(&params)
            .send()
            .context("horizons request failed")?
            .error_for_status()?
            .json()
            .context("horizons response was not valid JSON")?;

        let (ra_deg, dec_deg, range_au) = parse_observer_astrometric(&resp.result)
            .with_context(|| format!("could not parse Horizons OBSERVER response for {}", name))?;

        let pos_km = ra_dec_range_to_cartesian_km(ra_deg, dec_deg, range_au);

        let ephemeris = resp
            .signature
            .and_then(|s| s.version.or(s.source))
            .unwrap_or_else(|| "horizons".to_string());

        Ok(Fixture {
            name: name.to_string(),
            body,
            center: observer,
            jd_tdb,
            frame: output_frame,
            pos_km,
            vel_km_s: [0.0, 0.0, 0.0],
            source: Source::Horizons {
                ephemeris,
                fetched_at: current_utc_iso(),
            },
            tolerance,
        })
    }

    pub fn fetch(
        &self,
        name: &str,
        body: i32,
        center: i32,
        jd_tdb: f64,
        tolerance: Tolerance,
        corrections: Corrections,
    ) -> Result<Fixture> {
        let command = format!("'{}'", body);
        let center_str = format!("'@{}'", center);
        let tlist = format!("'{:.6}'", jd_tdb);
        let vec_corr = vec_corr_for(corrections)?;
        let vec_corr_str = format!("'{}'", vec_corr);

        let params = [
            ("format", "json"),
            ("COMMAND", &command),
            ("OBJ_DATA", "'NO'"),
            ("MAKE_EPHEM", "'YES'"),
            ("EPHEM_TYPE", "'VECTORS'"),
            ("CENTER", &center_str),
            ("TLIST", &tlist),
            ("TLIST_TYPE", "'JD'"),
            ("TIME_TYPE", "'TDB'"),
            ("OUT_UNITS", "'KM-S'"),
            ("REF_PLANE", "'FRAME'"),
            ("REF_SYSTEM", "'ICRF'"),
            ("VEC_TABLE", "'2'"),
            ("VEC_LABELS", "'NO'"),
            ("VEC_CORR", &vec_corr_str),
            ("CSV_FORMAT", "'NO'"),
        ];

        let resp: HorizonsResponse = self
            .client
            .get(HORIZONS_URL)
            .query(&params)
            .send()
            .context("horizons request failed")?
            .error_for_status()?
            .json()
            .context("horizons response was not valid JSON")?;

        let (pos_km, vel_km_s) = parse_vector_block(&resp.result)
            .with_context(|| format!("could not parse Horizons response for {}", name))?;

        let ephemeris = resp
            .signature
            .and_then(|s| s.version.or(s.source))
            .unwrap_or_else(|| "horizons".to_string());

        let fetched_at = current_utc_iso();

        Ok(Fixture {
            name: name.to_string(),
            body,
            center,
            jd_tdb,
            frame: Frame::Icrf,
            pos_km,
            vel_km_s,
            source: Source::Horizons {
                ephemeris,
                fetched_at,
            },
            tolerance,
        })
    }
}

/// Convert a TDB Julian Date to TT Julian Date using the eternal-time
/// Fairhead-Bretagnon series. We pick the Greenwich variant; at OBSERVER
/// precision (< 1 mas) the observer-location term is negligible.
fn tdb_jd_to_tt_jd(jd_tdb: f64) -> Result<f64> {
    let tdb = TDB::from_julian_date(JulianDate::from_f64(jd_tdb));
    let tt = tdb
        .to_tt_greenwich()
        .map_err(|e| anyhow!("TDB→TT conversion failed: {:?}", e))?;
    let jd = tt.to_julian_date();
    Ok(jd.jd1() + jd.jd2())
}

/// Parse the `$$SOE` line of a Horizons OBSERVER + CSV response.
/// The format with QUANTITIES='1,20' and ANG_FORMAT='DEG' is:
/// `<date>, <flag1>, <flag2>, RA_J2000_deg, Dec_J2000_deg, range_AU, range_rate_kms,`
fn parse_observer_astrometric(text: &str) -> Result<(f64, f64, f64)> {
    let soe = text
        .find("$$SOE")
        .ok_or_else(|| anyhow!("no $$SOE marker"))?;
    let eoe = text
        .find("$$EOE")
        .ok_or_else(|| anyhow!("no $$EOE marker"))?;
    let block = &text[soe + 5..eoe];

    for line in block.lines() {
        let cells: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if cells.len() < 6 {
            continue;
        }
        // Try to parse the last few numeric cells: RA, Dec, range live at
        // positions [3], [4], [5] in the CSV row.
        let ra = cells[3].parse::<f64>();
        let dec = cells[4].parse::<f64>();
        let range = cells[5].parse::<f64>();
        if let (Ok(ra), Ok(dec), Ok(range)) = (ra, dec, range) {
            return Ok((ra, dec, range));
        }
    }
    Err(anyhow!(
        "could not find RA/Dec/range triple in OBSERVER block"
    ))
}

fn ra_dec_range_to_cartesian_km(ra_deg: f64, dec_deg: f64, range_au: f64) -> [f64; 3] {
    let ra_rad = ra_deg.to_radians();
    let dec_rad = dec_deg.to_radians();
    let cos_dec = libm::cos(dec_rad);
    let range_km = range_au * AU_KM;
    [
        range_km * cos_dec * libm::cos(ra_rad),
        range_km * cos_dec * libm::sin(ra_rad),
        range_km * libm::sin(dec_rad),
    ]
}

// SECONDS_PER_DAY_F64 retained for documentation; not used in this module
// after the TDB→TT helper landed.
#[allow(dead_code)]
const _UNUSED_SPD: f64 = SECONDS_PER_DAY_F64;

/// Translate `Corrections` to the matching Horizons `VEC_CORR` value.
/// Gravitational deflection is not available in vector output; callers
/// must use OBSERVER queries for that stage, which the validation crate
/// does not yet handle.
fn vec_corr_for(c: Corrections) -> Result<&'static str> {
    match (c.light_time, c.stellar_aberration, c.gravitational_deflection) {
        (false, false, false) => Ok("NONE"),
        (true, false, false) => Ok("LT"),
        (true, true, false) => Ok("LT+S"),
        (lt, ab, gd) => Err(anyhow!(
            "Horizons VEC_CORR has no value matching (light_time={}, stellar_aberration={}, gravitational_deflection={}); use VECTORS for the supported triples only",
            lt, ab, gd
        )),
    }
}

/// Parse the `$$SOE` … `$$EOE` block of a Horizons VECTORS response with
/// `VEC_LABELS='NO'` and `CSV_FORMAT='NO'`. The block looks like:
/// ```text
/// $$SOE
/// 2451545.000000000 = A.D. 2000-Jan-01 12:00:00.0000 TDB
///  -2.756674E+07  1.323613E+08  5.741865E+07
///  -2.978494E+01 -5.029753E+00 -2.180645E+00
/// $$EOE
/// ```
/// The JD header line must be skipped — its leading number is itself a
/// valid `f64` token, so naive whitespace-tokenisation would swallow it
/// as the first component of the position vector. We require lines that
/// hold *exactly three* numeric tokens.
fn parse_vector_block(text: &str) -> Result<([f64; 3], [f64; 3])> {
    let soe = text
        .find("$$SOE")
        .ok_or_else(|| anyhow!("no $$SOE marker in Horizons response"))?;
    let eoe = text
        .find("$$EOE")
        .ok_or_else(|| anyhow!("no $$EOE marker in Horizons response"))?;
    let block = &text[soe + 5..eoe];

    let mut triples: Vec<[f64; 3]> = Vec::with_capacity(2);
    for line in block.lines() {
        let nums: Vec<f64> = line
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter_map(|t| t.parse::<f64>().ok())
            .collect();
        if nums.len() == 3 {
            triples.push([nums[0], nums[1], nums[2]]);
            if triples.len() == 2 {
                break;
            }
        }
    }

    if triples.len() < 2 {
        return Err(anyhow!(
            "could not find position and velocity rows in $$SOE block; got {} triples",
            triples.len()
        ));
    }

    Ok((triples[0], triples[1]))
}

fn current_utc_iso() -> String {
    // Avoid pulling in chrono just for this; emit a UNIX-epoch ISO timestamp.
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}Z-unix", secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vectors_block() {
        let raw = "\
*******************************************************************************\n\
$$SOE\n\
2451545.000000000 = A.D. 2000-Jan-01 12:00:00.0000 TDB \n\
 -2.521092901737E+07  1.449664423548E+08  6.282516252412E+04\n\
 -2.983323754265E+01 -5.220626320595E+00  4.140890832720E-04\n\
$$EOE\n\
*******************************************************************************\n";
        let (pos, vel) = parse_vector_block(raw).unwrap();
        assert!((pos[0] - -2.521092901737e7).abs() < 1e-3, "pos[0] = {}", pos[0]);
        assert!((pos[1] - 1.449664423548e8).abs() < 1e-3, "pos[1] = {}", pos[1]);
        assert!((pos[2] - 6.282516252412e4).abs() < 1e-3, "pos[2] = {}", pos[2]);
        assert!((vel[0] - -2.983323754265e1).abs() < 1e-9, "vel[0] = {}", vel[0]);
        assert!((vel[1] - -5.220626320595e0).abs() < 1e-9, "vel[1] = {}", vel[1]);
        assert!((vel[2] - 4.140890832720e-4).abs() < 1e-9, "vel[2] = {}", vel[2]);
    }

    #[test]
    fn rejects_block_without_two_triples() {
        let raw = "\
$$SOE\n\
2451545.000000000 = A.D. 2000-Jan-01 12:00:00.0000 TDB\n\
$$EOE\n";
        assert!(parse_vector_block(raw).is_err());
    }
}
