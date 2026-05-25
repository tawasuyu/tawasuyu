//! Celestial bodies that the façade knows how to compute.
//!
//! The enum is `#[non_exhaustive]` because the catalogue will grow
//! (fixed stars, more asteroids, hypothetical points). Backends declare
//! which bodies they cover; bodies not covered raise
//! `SkyError::UnsupportedBody`.

/// Categorisation of a body — useful for picking the right compute path
/// without pattern-matching on the full enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyKind {
    /// Sun, Moon, classical and modern major planets. Reachable from a
    /// DE-series SPK kernel or the analytical (VSOP/ELP) backend.
    Major,
    /// Lunar special points (mean/true node, mean/true Lilith). Computed
    /// from polynomial series or osculating element extraction.
    LunarPoint,
    /// Main-belt or trans-neptunian small body that lives in an asteroid
    /// SPK kernel separate from the planet kernel.
    SmallBody,
}

/// Celestial body identifier. Pattern-match on this in user code instead
/// of carrying raw NAIF integers around.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Body {
    // ─── Luminaries ────────────────────────────────────────────────────
    Sun,
    Moon,

    // ─── Classical planets ─────────────────────────────────────────────
    Mercury,
    Venus,
    Mars,
    Jupiter,
    Saturn,

    // ─── Modern planets ────────────────────────────────────────────────
    Uranus,
    Neptune,
    Pluto,

    // ─── Lunar special points ──────────────────────────────────────────
    /// Mean ascending lunar node (true ecliptic of date, with nutation).
    MeanNode,
    /// True (osculating) ascending lunar node. Needs an SPK kernel.
    TrueNode,
    /// Mean lunar apogee — the astrological "Mean Black Moon Lilith".
    MeanLilith,
    /// True (osculating) lunar apogee. Needs an SPK kernel.
    TrueLilith,

    // ─── Main-belt asteroids (DE441 sb441-n16 kernel) ─────────────────
    Ceres,
    Pallas,
    Juno,
    Vesta,

    // ─── Centaurs and TNOs (require SPK Type 21 — not yet supported) ──
    Chiron,
    Pholus,
    Eris,
    Sedna,
}

impl Body {
    /// Human-readable English name. Used for chart metadata and logs.
    pub fn name(self) -> &'static str {
        match self {
            Body::Sun => "Sun",
            Body::Moon => "Moon",
            Body::Mercury => "Mercury",
            Body::Venus => "Venus",
            Body::Mars => "Mars",
            Body::Jupiter => "Jupiter",
            Body::Saturn => "Saturn",
            Body::Uranus => "Uranus",
            Body::Neptune => "Neptune",
            Body::Pluto => "Pluto",
            Body::MeanNode => "Mean Node",
            Body::TrueNode => "True Node",
            Body::MeanLilith => "Mean Lilith",
            Body::TrueLilith => "True Lilith",
            Body::Ceres => "Ceres",
            Body::Pallas => "Pallas",
            Body::Juno => "Juno",
            Body::Vesta => "Vesta",
            Body::Chiron => "Chiron",
            Body::Pholus => "Pholus",
            Body::Eris => "Eris",
            Body::Sedna => "Sedna",
        }
    }

    pub fn kind(self) -> BodyKind {
        match self {
            Body::Sun
            | Body::Moon
            | Body::Mercury
            | Body::Venus
            | Body::Mars
            | Body::Jupiter
            | Body::Saturn
            | Body::Uranus
            | Body::Neptune
            | Body::Pluto => BodyKind::Major,
            Body::MeanNode | Body::TrueNode | Body::MeanLilith | Body::TrueLilith => {
                BodyKind::LunarPoint
            }
            Body::Ceres
            | Body::Pallas
            | Body::Juno
            | Body::Vesta
            | Body::Chiron
            | Body::Pholus
            | Body::Eris
            | Body::Sedna => BodyKind::SmallBody,
        }
    }

    /// NAIF integer ID used by SPK kernels and the underlying `Oracle`.
    /// Returns `None` for analytical-only points (mean node, mean Lilith)
    /// that have no SPK representation.
    pub fn naif_id(self) -> Option<i32> {
        match self {
            // Planetary barycenters (DE-series). For inner planets the
            // barycenter is the planet itself to femto-precision.
            Body::Mercury => Some(1),
            Body::Venus => Some(2),
            Body::Mars => Some(4),
            Body::Jupiter => Some(5),
            Body::Saturn => Some(6),
            Body::Uranus => Some(7),
            Body::Neptune => Some(8),
            Body::Pluto => Some(9),
            Body::Sun => Some(10),
            Body::Moon => Some(301),
            // Small bodies: NAIF ID = 2_000_000 + designation number.
            Body::Ceres => Some(2_000_001),
            Body::Pallas => Some(2_000_002),
            Body::Juno => Some(2_000_003),
            Body::Vesta => Some(2_000_004),
            Body::Chiron => Some(2_002_060),
            Body::Pholus => Some(2_005_145),
            Body::Eris => Some(2_136_199),
            Body::Sedna => Some(2_090_377),
            // Analytical points — no SPK representation.
            Body::MeanNode | Body::TrueNode | Body::MeanLilith | Body::TrueLilith => None,
        }
    }
}
