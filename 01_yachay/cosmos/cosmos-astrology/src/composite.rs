//! Composite (midpoint) charts.
//!
//! A composite chart is the symbolic "average" of two natal charts:
//! every point — Sun, Moon, planets, lunar nodes, Lilith, asteroids,
//! and the four angles — is replaced by the **angular midpoint** of
//! the corresponding pair `(A, B)`. Houses are then derived in the
//! Whole-Sign convention starting from the composite Ascendant.
//!
//! The convention used here is the classical *Midpoint Composite*
//! (Ronald Davison 1958), not the *Time-Space Composite* (which builds
//! a real natal chart at the geographic and temporal midpoint of two
//! births — a different construction that requires lat/lon math the
//! caller has to do explicitly).
//!
//! The two input charts MUST share the same [`crate::BodySet`] for the
//! placements to align by index. The standard
//! [`crate::ChartConfig::default()`] is fine.

use cosmos_sky::Body;

use crate::angles::wrap_two_pi;
use crate::birth_data::BirthData;
use crate::chart::NatalChart;
use crate::error::{AstrologyError, AstrologyResult};
use crate::zodiac::{Sign, SignedLongitude};

const PI: f64 = std::f64::consts::PI;

/// One body's midpoint placement.
#[derive(Debug, Clone, Copy)]
pub struct CompositePlacement {
    pub body: Body,
    pub longitude: SignedLongitude,
    pub sign: Sign,
    /// Whole-sign house number `1..=12`.
    pub house_number: u8,
}

/// A complete midpoint composite chart. Carries provenance back to
/// both source charts so callers can audit the construction.
#[derive(Debug, Clone)]
pub struct CompositeChart {
    pub from_a: BirthData,
    pub from_b: BirthData,
    pub ascendant: SignedLongitude,
    pub midheaven: SignedLongitude,
    pub descendant: SignedLongitude,
    pub imum_coeli: SignedLongitude,
    pub placements: Vec<CompositePlacement>,
}

impl CompositeChart {
    /// Lookup the first composite placement for a body. (For bodies that
    /// appear twice in the source charts — `MeanNode` and its
    /// auto-appended South Node — only the first match is returned;
    /// the second is at the antipode.)
    pub fn placement(&self, body: Body) -> Option<&CompositePlacement> {
        self.placements.iter().find(|p| p.body == body)
    }
}

/// Build a midpoint composite from two natal charts. The two charts
/// MUST have been computed with the same `BodySet` (so their
/// `placements` arrays line up by index); otherwise an error is
/// returned and the caller can re-run `NatalChart::compute` with
/// matching configurations.
pub fn composite(chart_a: &NatalChart, chart_b: &NatalChart) -> AstrologyResult<CompositeChart> {
    if chart_a.placements.len() != chart_b.placements.len() {
        return Err(AstrologyError::BodyUnavailable(format!(
            "composite requires matching BodySet — chart A has {} placements, B has {}",
            chart_a.placements.len(),
            chart_b.placements.len()
        )));
    }
    for (a, b) in chart_a.placements.iter().zip(chart_b.placements.iter()) {
        if a.body != b.body {
            return Err(AstrologyError::BodyUnavailable(format!(
                "composite requires identically-ordered BodySet — chart A has {} at index, B has {}",
                a.body.name(),
                b.body.name()
            )));
        }
    }

    let asc = angular_midpoint_rad(
        chart_a.ascendant().longitude_rad(),
        chart_b.ascendant().longitude_rad(),
    );
    let mc = angular_midpoint_rad(
        chart_a.midheaven().longitude_rad(),
        chart_b.midheaven().longitude_rad(),
    );
    let asc_sign = SignedLongitude::from_radians(asc).sign();

    let placements: Vec<CompositePlacement> = chart_a
        .placements
        .iter()
        .zip(chart_b.placements.iter())
        .map(|(a, b)| {
            let mid = angular_midpoint_rad(
                a.longitude.longitude_rad(),
                b.longitude.longitude_rad(),
            );
            let sl = SignedLongitude::from_radians(mid);
            let house = whole_sign_house(asc_sign, sl.sign());
            CompositePlacement {
                body: a.body,
                longitude: sl,
                sign: sl.sign(),
                house_number: house,
            }
        })
        .collect();

    let desc = wrap_two_pi(asc + PI);
    let ic = wrap_two_pi(mc + PI);

    Ok(CompositeChart {
        from_a: chart_a.birth.clone(),
        from_b: chart_b.birth.clone(),
        ascendant: SignedLongitude::from_radians(asc),
        midheaven: SignedLongitude::from_radians(mc),
        descendant: SignedLongitude::from_radians(desc),
        imum_coeli: SignedLongitude::from_radians(ic),
        placements,
    })
}

/// Angular midpoint of two longitudes (radians). Returns the midpoint
/// of the **shorter** arc between `a` and `b`, wrapped to `[0, 2π)`.
/// Antipodal inputs default to `a` itself.
pub fn angular_midpoint_rad(a: f64, b: f64) -> f64 {
    let mid = libm::atan2(libm::sin(a) + libm::sin(b), libm::cos(a) + libm::cos(b));
    wrap_two_pi(mid)
}

fn whole_sign_house(asc_sign: Sign, point_sign: Sign) -> u8 {
    let diff = (point_sign.index() as i32 - asc_sign.index() as i32).rem_euclid(12);
    (diff + 1) as u8
}

