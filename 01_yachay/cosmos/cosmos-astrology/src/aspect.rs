//! Aspect engine: detect angular relationships between bodies in a chart.
//!
//! An *aspect* is an angular distance close (within an "orb") to a
//! traditional ratio of the circle. The classical majors are
//! conjunction (0°), opposition (180°), trine (120°), square (90°), and
//! sextile (60°); the harmonic minors (quincunx, semi-square, quintile,
//! septile, …) are wired in too for completeness.
//!
//! Each aspect carries:
//!   * the two bodies involved (commutative — `a` ≤ `b` by NAIF ID),
//!   * the [`AspectKind`] family,
//!   * the *signed* delta from exact: `+` means the smaller-longitude
//!     body is below the exact angle,
//!   * the orb used (the threshold the pair was tested against),
//!   * whether the aspect is **applying** (closing toward exact) or
//!     **separating** (already past).

use std::collections::HashMap;

use cosmos_sky::Body;

use crate::angles::signed_delta_deg;
use crate::chart::NatalChart;
use crate::placement::BodyPlacement;

/// Family of aspects. The exact angle of each is fixed; their orbs are
/// configured via [`OrbTable`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AspectKind {
    Conjunction,
    Opposition,
    Trine,
    Square,
    Sextile,
    Quincunx,
    SemiSextile,
    SemiSquare,
    Sesquiquadrate,
    Quintile,
    BiQuintile,
    Septile,
}

impl AspectKind {
    pub const MAJORS: &'static [AspectKind] = &[
        AspectKind::Conjunction,
        AspectKind::Opposition,
        AspectKind::Trine,
        AspectKind::Square,
        AspectKind::Sextile,
    ];

    pub const MINORS: &'static [AspectKind] = &[
        AspectKind::Quincunx,
        AspectKind::SemiSextile,
        AspectKind::SemiSquare,
        AspectKind::Sesquiquadrate,
        AspectKind::Quintile,
        AspectKind::BiQuintile,
        AspectKind::Septile,
    ];

    pub const ALL: &'static [AspectKind] = &[
        AspectKind::Conjunction,
        AspectKind::Opposition,
        AspectKind::Trine,
        AspectKind::Square,
        AspectKind::Sextile,
        AspectKind::Quincunx,
        AspectKind::SemiSextile,
        AspectKind::SemiSquare,
        AspectKind::Sesquiquadrate,
        AspectKind::Quintile,
        AspectKind::BiQuintile,
        AspectKind::Septile,
    ];

    /// Exact angle in degrees.
    pub fn exact_angle_deg(self) -> f64 {
        match self {
            AspectKind::Conjunction => 0.0,
            AspectKind::Opposition => 180.0,
            AspectKind::Trine => 120.0,
            AspectKind::Square => 90.0,
            AspectKind::Sextile => 60.0,
            AspectKind::Quincunx => 150.0,
            AspectKind::SemiSextile => 30.0,
            AspectKind::SemiSquare => 45.0,
            AspectKind::Sesquiquadrate => 135.0,
            AspectKind::Quintile => 72.0,
            AspectKind::BiQuintile => 144.0,
            AspectKind::Septile => 360.0 / 7.0,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            AspectKind::Conjunction => "conjunction",
            AspectKind::Opposition => "opposition",
            AspectKind::Trine => "trine",
            AspectKind::Square => "square",
            AspectKind::Sextile => "sextile",
            AspectKind::Quincunx => "quincunx",
            AspectKind::SemiSextile => "semi-sextile",
            AspectKind::SemiSquare => "semi-square",
            AspectKind::Sesquiquadrate => "sesquiquadrate",
            AspectKind::Quintile => "quintile",
            AspectKind::BiQuintile => "bi-quintile",
            AspectKind::Septile => "septile",
        }
    }
}

/// Per-aspect base orbs (in degrees) and optional per-body luminary
/// multipliers. Designed to be cheap to copy and serialise.
#[derive(Debug, Clone)]
pub struct OrbTable {
    base_orb_deg: HashMap<AspectKind, f64>,
    body_multiplier: HashMap<Body, f64>,
    /// Multiplier used when *neither* body is in [`Self::body_multiplier`].
    pub default_body_multiplier: f64,
}

impl OrbTable {
    /// A reasonably tight modern Western set:
    /// 8° for conjunctions/oppositions, 7° for trines/squares, 5° for
    /// sextiles, 2° for minors; Sun and Moon get a 1.25× multiplier.
    pub fn modern_western() -> Self {
        let mut base = HashMap::new();
        base.insert(AspectKind::Conjunction, 8.0);
        base.insert(AspectKind::Opposition, 8.0);
        base.insert(AspectKind::Trine, 7.0);
        base.insert(AspectKind::Square, 7.0);
        base.insert(AspectKind::Sextile, 5.0);
        base.insert(AspectKind::Quincunx, 2.5);
        base.insert(AspectKind::SemiSextile, 2.0);
        base.insert(AspectKind::SemiSquare, 2.0);
        base.insert(AspectKind::Sesquiquadrate, 2.0);
        base.insert(AspectKind::Quintile, 1.5);
        base.insert(AspectKind::BiQuintile, 1.5);
        base.insert(AspectKind::Septile, 1.5);

        let mut mult = HashMap::new();
        mult.insert(Body::Sun, 1.25);
        mult.insert(Body::Moon, 1.25);

        Self {
            base_orb_deg: base,
            body_multiplier: mult,
            default_body_multiplier: 1.0,
        }
    }

    /// Tight orbs: ~half of [`Self::modern_western`]. Good for
    /// progressions / directions where wider orbs become meaningless.
    pub fn tight() -> Self {
        let mut t = Self::modern_western();
        for v in t.base_orb_deg.values_mut() {
            *v *= 0.5;
        }
        t
    }

    /// Set the base orb for a specific aspect family.
    pub fn set_orb(&mut self, kind: AspectKind, orb_deg: f64) -> &mut Self {
        self.base_orb_deg.insert(kind, orb_deg);
        self
    }

    /// Set a per-body orb multiplier (useful for luminaries, chart-ruler,
    /// or stellium reductions).
    pub fn set_body_multiplier(&mut self, body: Body, mult: f64) -> &mut Self {
        self.body_multiplier.insert(body, mult);
        self
    }

    /// Effective allowed orb for an aspect between `a` and `b`.
    /// Uses the *maximum* of the two body multipliers — convention is
    /// that a Sun-aspect-Mercury gets the Sun's wider orb, not the
    /// Mercury orb.
    pub fn orb_for(&self, a: Body, b: Body, kind: AspectKind) -> f64 {
        let base = self.base_orb_deg.get(&kind).copied().unwrap_or(0.0);
        let ma = self
            .body_multiplier
            .get(&a)
            .copied()
            .unwrap_or(self.default_body_multiplier);
        let mb = self
            .body_multiplier
            .get(&b)
            .copied()
            .unwrap_or(self.default_body_multiplier);
        base * ma.max(mb)
    }

    /// Build a flat lookup snapshot for use in tight pair-iteration
    /// loops. The snapshot replaces three HashMap hashings per
    /// `orb_for` call with two array indexes — meaningful when the
    /// outer loop is N² in chart placements × `AspectKind::ALL`.
    pub(crate) fn snapshot(&self) -> OrbSnapshot {
        let mut base = [0.0_f64; AspectKind::ALL.len()];
        for (i, &kind) in AspectKind::ALL.iter().enumerate() {
            base[i] = self.base_orb_deg.get(&kind).copied().unwrap_or(0.0);
        }
        OrbSnapshot {
            base_orb_deg: base,
            body_multiplier: self
                .body_multiplier
                .iter()
                .map(|(&b, &m)| (b, m))
                .collect(),
            default_body_multiplier: self.default_body_multiplier,
        }
    }
}

/// Flat, fixed-size view of an [`OrbTable`]'s contents, suitable for
/// inner pair-iteration loops.
pub(crate) struct OrbSnapshot {
    base_orb_deg: [f64; AspectKind::ALL.len()],
    body_multiplier: Vec<(Body, f64)>,
    default_body_multiplier: f64,
}

impl OrbSnapshot {
    #[inline]
    pub(crate) fn orb_for(&self, a: Body, b: Body, kind: AspectKind) -> f64 {
        let idx = AspectKind::ALL.iter().position(|k| *k == kind).unwrap_or(0);
        let base = self.base_orb_deg[idx];
        let ma = self.lookup_mult(a);
        let mb = self.lookup_mult(b);
        base * ma.max(mb)
    }

    #[inline]
    fn lookup_mult(&self, body: Body) -> f64 {
        for &(b, m) in &self.body_multiplier {
            if b == body {
                return m;
            }
        }
        self.default_body_multiplier
    }
}

impl Default for OrbTable {
    fn default() -> Self {
        Self::modern_western()
    }
}

/// A single aspect detected in a chart.
#[derive(Debug, Clone, Copy)]
pub struct Aspect {
    pub a: Body,
    pub b: Body,
    pub kind: AspectKind,
    /// Signed distance from exact, degrees. Positive means the pair is
    /// past the exact angle (`|Δλ| > exact_angle`); negative means it
    /// is short of exact. Useful for "how exact is this aspect?" reports.
    pub orb_signed_deg: f64,
    /// Allowed orb at the time of detection (degrees). The aspect is
    /// reported iff `orb_signed_deg.abs() <= allowed_orb_deg`.
    pub allowed_orb_deg: f64,
    /// `true` if the angular distance between `a` and `b` is closing
    /// toward the exact angle; `false` if it is widening.
    pub applying: bool,
}

impl Aspect {
    pub fn orb_abs_deg(&self) -> f64 {
        self.orb_signed_deg.abs()
    }
    /// How "tight" the aspect is, normalised to the allowed orb.
    /// 0.0 = exact; 1.0 = at the edge of the orb.
    pub fn tightness(&self) -> f64 {
        if self.allowed_orb_deg == 0.0 {
            0.0
        } else {
            self.orb_abs_deg() / self.allowed_orb_deg
        }
    }
}

/// Scan every pair of body placements in `chart` and return all
/// aspects whose orb sits within the table's allowance. The returned
/// list is sorted by tightness (most exact first).
pub fn find_aspects(chart: &NatalChart, orbs: &OrbTable) -> Vec<Aspect> {
    find_aspects_filtered(chart, orbs, AspectKind::ALL)
}

/// Same as [`find_aspects`] but restricted to a subset of [`AspectKind`].
pub fn find_aspects_filtered(
    chart: &NatalChart,
    orbs: &OrbTable,
    kinds: &[AspectKind],
) -> Vec<Aspect> {
    let placements = &chart.placements;
    let snapshot = orbs.snapshot();
    // Upper bound on aspects: every pair × every kind (worst case).
    let mut out = Vec::with_capacity(
        placements.len() * (placements.len() - 1) / 2 * kinds.len(),
    );

    for i in 0..placements.len() {
        for j in (i + 1)..placements.len() {
            for &kind in kinds {
                if let Some(asp) = test_pair(&placements[i], &placements[j], kind, &snapshot) {
                    out.push(asp);
                }
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

fn test_pair(
    a: &BodyPlacement,
    b: &BodyPlacement,
    kind: AspectKind,
    orbs: &OrbSnapshot,
) -> Option<Aspect> {
    // Same-body pairs (e.g. mean node duplicated as ascending + descending
    // in `BodySet::include_south_node`) would otherwise trigger spurious
    // conjunctions/oppositions to themselves. Skip them.
    if a.body == b.body {
        return None;
    }

    let allowed = orbs.orb_for(a.body, b.body, kind);
    if allowed <= 0.0 {
        return None;
    }
    // Signed angular separation `lon_b − lon_a`, normalised to
    // `[-180°, 180°]`. The unsigned separation is what we compare
    // against the exact angle. (We pass `(b, a)` to the helper which
    // computes `arg0 − arg1`.)
    let raw_delta_deg = signed_delta_deg(
        b.longitude.longitude_deg(),
        a.longitude.longitude_deg(),
    );
    let separation = raw_delta_deg.abs();
    let exact = kind.exact_angle_deg();
    let diff = separation - exact;
    if diff.abs() > allowed {
        return None;
    }

    // Applying / separating: signed separation `raw_delta_deg` evolves
    // at `(b_rate − a_rate)`. The unsigned separation evolves at
    // `sign(raw_delta) × (b_rate − a_rate)`. The aspect is closing
    // (applying) when (separation − exact) and d(separation)/dt have
    // opposite signs.
    let rate_b_minus_a_deg_per_day =
        (b.longitude_rate_rad_per_day - a.longitude_rate_rad_per_day).to_degrees();
    let dseparation_dt = if raw_delta_deg >= 0.0 {
        rate_b_minus_a_deg_per_day
    } else {
        -rate_b_minus_a_deg_per_day
    };
    let applying = if diff > 0.0 {
        // sep > exact → closing means d sep / dt < 0.
        dseparation_dt < 0.0
    } else if diff < 0.0 {
        // sep < exact → closing means d sep / dt > 0.
        dseparation_dt > 0.0
    } else {
        // Exactly on the angle.
        false
    };

    // Normalise the body order so the aspect is canonical regardless of
    // input pair order: alphabetise by name (cheap, stable).
    let (canon_a, canon_b) = if a.body.name() <= b.body.name() {
        (a.body, b.body)
    } else {
        (b.body, a.body)
    };

    Some(Aspect {
        a: canon_a,
        b: canon_b,
        kind,
        orb_signed_deg: diff,
        allowed_orb_deg: allowed,
        applying,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aspect_exact_angles_round_to_traditional_values() {
        assert!((AspectKind::Trine.exact_angle_deg() - 120.0).abs() < 1e-12);
        assert!((AspectKind::Septile.exact_angle_deg() - 360.0 / 7.0).abs() < 1e-12);
    }

    #[test]
    fn orb_table_modern_western_luminary_multiplier() {
        let orbs = OrbTable::modern_western();
        // Sun-Mercury conjunction: 8 × 1.25 = 10°.
        let sun_mercury = orbs.orb_for(Body::Sun, Body::Mercury, AspectKind::Conjunction);
        assert!((sun_mercury - 10.0).abs() < 1e-12);
        // Mercury-Venus conjunction: 8 × 1.0 = 8°.
        let m_v = orbs.orb_for(Body::Mercury, Body::Venus, AspectKind::Conjunction);
        assert!((m_v - 8.0).abs() < 1e-12);
    }
}
