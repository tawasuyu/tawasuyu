//! Arabic Parts (Hellenistic *Lots*).
//!
//! A Lot is a calculated point on the ecliptic of the form
//! `A + B − C` where each of `A`, `B`, `C` is the natal longitude of
//! the Ascendant, a body, or another previously-computed Lot. Most
//! classical Lots **reverse** by day/night sect — i.e. the roles of
//! `B` and `C` swap when the Sun sits below the horizon at the moment
//! of birth.
//!
//! The seven shipped here cover the bulk of practical Hellenistic
//! work; new ones can be expressed via [`custom_lot`].

use cosmos_sky::Body;

use crate::chart::NatalChart;
use crate::error::{AstrologyError, AstrologyResult};
use crate::zodiac::{Sign, SignedLongitude};

const TAU: f64 = std::f64::consts::TAU;

/// Day or night birth, determined by whether the natal Sun is above
/// the horizon. Houses 7..=12 are the diurnal hemisphere; 1..=6 the
/// nocturnal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sect {
    Day,
    Night,
}

impl Sect {
    /// Determine sect from a computed chart.
    pub fn of(chart: &NatalChart) -> AstrologyResult<Self> {
        let sun = chart.placement(Body::Sun).ok_or_else(|| {
            AstrologyError::BodyUnavailable("Sun not in chart — sect undefined".into())
        })?;
        // Houses 7..=12 lie above the horizon, 1..=6 below.
        Ok(if (7..=12).contains(&sun.house_number) {
            Sect::Day
        } else {
            Sect::Night
        })
    }
}

/// A point that can appear as `A`, `B`, or `C` in a Lot formula.
#[derive(Debug, Clone, Copy)]
pub enum LotPoint {
    Ascendant,
    Body(Body),
    Lot(LotName),
}

/// The canonical Hellenistic Lots wired here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LotName {
    Fortune,
    Spirit,
    Eros,
    Necessity,
    Courage,
    Victory,
    Nemesis,
}

impl LotName {
    pub fn label(self) -> &'static str {
        match self {
            LotName::Fortune => "Fortune",
            LotName::Spirit => "Spirit",
            LotName::Eros => "Eros",
            LotName::Necessity => "Necessity",
            LotName::Courage => "Courage",
            LotName::Victory => "Victory",
            LotName::Nemesis => "Nemesis",
        }
    }

    /// `(A_day, B_day, C_day)` — the diurnal formula `A + B − C`.
    fn diurnal_triplet(self) -> (LotPoint, LotPoint, LotPoint) {
        let asc = LotPoint::Ascendant;
        let body = LotPoint::Body;
        let lot = LotPoint::Lot;
        match self {
            LotName::Fortune => (asc, body(Body::Moon), body(Body::Sun)),
            LotName::Spirit => (asc, body(Body::Sun), body(Body::Moon)),
            LotName::Eros => (asc, body(Body::Venus), lot(LotName::Spirit)),
            LotName::Necessity => (asc, lot(LotName::Fortune), body(Body::Mercury)),
            LotName::Courage => (asc, body(Body::Mars), lot(LotName::Fortune)),
            LotName::Victory => (asc, body(Body::Jupiter), lot(LotName::Spirit)),
            LotName::Nemesis => (asc, lot(LotName::Fortune), body(Body::Saturn)),
        }
    }
}

/// One computed Lot.
#[derive(Debug, Clone, Copy)]
pub struct Lot {
    pub name: Option<LotName>,
    pub sect: Sect,
    pub longitude: SignedLongitude,
    pub house_number: u8,
}

impl Lot {
    pub fn sign(&self) -> Sign {
        self.longitude.sign()
    }
}

/// Compute one of the canonical lots.
pub fn compute_lot(chart: &NatalChart, name: LotName) -> AstrologyResult<Lot> {
    let sect = Sect::of(chart)?;
    let mut cache = std::collections::HashMap::new();
    let lon = resolve_lot(chart, sect, name, &mut cache)?;
    let house = chart.houses.house_containing(
        (lon + chart.ayanamsha_rad).rem_euclid(TAU),
    );
    Ok(Lot {
        name: Some(name),
        sect,
        longitude: SignedLongitude::from_radians(lon),
        house_number: house,
    })
}

/// Compute every canonical lot in dependency order. Convenience for
/// chart reports.
pub fn all_lots(chart: &NatalChart) -> AstrologyResult<Vec<Lot>> {
    let order = [
        LotName::Fortune,
        LotName::Spirit,
        LotName::Eros,
        LotName::Necessity,
        LotName::Courage,
        LotName::Victory,
        LotName::Nemesis,
    ];
    let mut out = Vec::with_capacity(order.len());
    for name in order {
        out.push(compute_lot(chart, name)?);
    }
    Ok(out)
}

/// Compute a user-defined Lot. Supply both diurnal and nocturnal
/// triplets; pass the same tuple twice for a non-sect-reversing Lot.
pub fn custom_lot(
    chart: &NatalChart,
    diurnal: (LotPoint, LotPoint, LotPoint),
    nocturnal: (LotPoint, LotPoint, LotPoint),
) -> AstrologyResult<Lot> {
    let sect = Sect::of(chart)?;
    let (a, b, c) = match sect {
        Sect::Day => diurnal,
        Sect::Night => nocturnal,
    };
    let mut cache = std::collections::HashMap::new();
    let lon = resolve_formula(chart, sect, a, b, c, &mut cache)?;
    let house = chart.houses.house_containing(
        (lon + chart.ayanamsha_rad).rem_euclid(TAU),
    );
    Ok(Lot {
        name: None,
        sect,
        longitude: SignedLongitude::from_radians(lon),
        house_number: house,
    })
}

// ─── Internals ─────────────────────────────────────────────────────────

fn resolve_lot(
    chart: &NatalChart,
    sect: Sect,
    name: LotName,
    cache: &mut std::collections::HashMap<LotName, f64>,
) -> AstrologyResult<f64> {
    if let Some(v) = cache.get(&name) {
        return Ok(*v);
    }
    let (a, b, c) = match sect {
        Sect::Day => name.diurnal_triplet(),
        Sect::Night => reverse_triplet(name.diurnal_triplet()),
    };
    let lon = resolve_formula(chart, sect, a, b, c, cache)?;
    cache.insert(name, lon);
    Ok(lon)
}

/// Swap `B` and `C` in a diurnal triplet to obtain the nocturnal one.
fn reverse_triplet(
    t: (LotPoint, LotPoint, LotPoint),
) -> (LotPoint, LotPoint, LotPoint) {
    (t.0, t.2, t.1)
}

fn resolve_formula(
    chart: &NatalChart,
    sect: Sect,
    a: LotPoint,
    b: LotPoint,
    c: LotPoint,
    cache: &mut std::collections::HashMap<LotName, f64>,
) -> AstrologyResult<f64> {
    let la = resolve_point(chart, sect, a, cache)?;
    let lb = resolve_point(chart, sect, b, cache)?;
    let lc = resolve_point(chart, sect, c, cache)?;
    let raw = la + lb - lc;
    Ok(raw.rem_euclid(TAU))
}

fn resolve_point(
    chart: &NatalChart,
    sect: Sect,
    point: LotPoint,
    cache: &mut std::collections::HashMap<LotName, f64>,
) -> AstrologyResult<f64> {
    match point {
        // Lots are expressed in the chart's *zodiac* (tropical or
        // sidereal). The Asc/MC stored in NatalChart are already in
        // zodiac frame, so use those directly.
        LotPoint::Ascendant => Ok(chart.ascendant().longitude_rad()),
        LotPoint::Body(b) => {
            let placement = chart.placement(b).ok_or_else(|| {
                AstrologyError::BodyUnavailable(format!(
                    "{} required by Lot is not in chart",
                    b.name()
                ))
            })?;
            Ok(placement.longitude.longitude_rad())
        }
        LotPoint::Lot(name) => resolve_lot(chart, sect, name, cache),
    }
}
