//! Synastry: aspect grid between two natal charts.
//!
//! Synastry compares two charts by considering every pair `(body in A,
//! body in B)` and reporting the aspects whose angular separation falls
//! within an [`OrbTable`]'s allowance. It is symmetric with respect to
//! the chart order — if A→B reports a 5° conjunction Sun↔Moon, B→A
//! reports the same. The convention used here is that `person_a_body`
//! always sits in chart A and `person_b_body` always sits in chart B.

use cosmos_sky::Body;

use crate::angles::signed_delta_deg;
use crate::aspect::{AspectKind, OrbTable};
use crate::chart::NatalChart;

/// One aspect between a body in chart A and a body in chart B.
#[derive(Debug, Clone, Copy)]
pub struct SynastryAspect {
    pub person_a_body: Body,
    pub person_b_body: Body,
    pub kind: AspectKind,
    pub orb_signed_deg: f64,
    pub allowed_orb_deg: f64,
}

impl SynastryAspect {
    pub fn orb_abs_deg(&self) -> f64 {
        self.orb_signed_deg.abs()
    }
}

/// Cross-chart aspect grid. Returns aspects sorted by orb (tightest
/// first). `kinds` selects which aspect families to test.
pub fn find_synastry_aspects(
    chart_a: &NatalChart,
    chart_b: &NatalChart,
    orbs: &OrbTable,
    kinds: &[AspectKind],
) -> Vec<SynastryAspect> {
    let mut out: Vec<SynastryAspect> = Vec::new();

    for a in &chart_a.placements {
        for b in &chart_b.placements {
            // Identical-body cross-chart aspects ARE meaningful here
            // — Sun(A) conjunct Sun(B) is the canonical "same-sign
            // birthday" indicator. So we do NOT skip same-body pairs.
            for &kind in kinds {
                let allowed = orbs.orb_for(a.body, b.body, kind);
                if allowed <= 0.0 {
                    continue;
                }
                let raw =
                    signed_delta_deg(a.longitude.longitude_deg(), b.longitude.longitude_deg());
                let separation = raw.abs();
                let exact = kind.exact_angle_deg();
                let diff = separation - exact;
                if diff.abs() > allowed {
                    continue;
                }
                out.push(SynastryAspect {
                    person_a_body: a.body,
                    person_b_body: b.body,
                    kind,
                    orb_signed_deg: diff,
                    allowed_orb_deg: allowed,
                });
            }
        }
    }
    out.sort_by(|x, y| {
        x.orb_abs_deg()
            .partial_cmp(&y.orb_abs_deg())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

