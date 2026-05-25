//! User-controlled options for a natal-chart computation.

use cosmos_sky::Body;

use crate::house_system::HouseSystem;
use crate::zodiac::Zodiac;

/// Which bodies to include in a chart.
#[derive(Debug, Clone)]
pub struct BodySet {
    pub bodies: Vec<Body>,
    /// Append the South Node automatically as `mean_node + 180°`?
    /// (`Body::MeanNode` and `Body::TrueNode` give the *ascending* node.)
    pub include_south_node: bool,
}

impl BodySet {
    /// The ten luminaries + planets used in most modern Western charts,
    /// plus the mean lunar node (ascending). This is the baseline most
    /// astrologers expect when no extra configuration is supplied.
    pub fn classical_modern() -> Self {
        Self {
            bodies: vec![
                Body::Sun,
                Body::Moon,
                Body::Mercury,
                Body::Venus,
                Body::Mars,
                Body::Jupiter,
                Body::Saturn,
                Body::Uranus,
                Body::Neptune,
                Body::Pluto,
                Body::MeanNode,
            ],
            include_south_node: true,
        }
    }

    /// Classical-modern set plus mean Lilith.
    pub fn with_lilith(mut self) -> Self {
        self.bodies.push(Body::MeanLilith);
        self
    }

    /// Add the four main-belt asteroids (Ceres, Pallas, Juno, Vesta).
    /// Requires an asteroid SPK kernel attached to the
    /// [`cosmos_sky::EphemerisSession`].
    pub fn with_main_belt_asteroids(mut self) -> Self {
        self.bodies.push(Body::Ceres);
        self.bodies.push(Body::Pallas);
        self.bodies.push(Body::Juno);
        self.bodies.push(Body::Vesta);
        self
    }
}

impl Default for BodySet {
    fn default() -> Self {
        Self::classical_modern()
    }
}

/// Combined chart configuration. The defaults produce a Placidus
/// tropical chart with the classical-modern body set.
#[derive(Debug, Clone)]
pub struct ChartConfig {
    pub house_system: HouseSystem,
    pub zodiac: Zodiac,
    pub bodies: BodySet,
    /// If `true`, request topocentric horizon coordinates for every body
    /// in addition to the geocentric ecliptic position. Slightly more
    /// expensive but useful for charts that care about local visibility
    /// (rising / setting, mundane positions).
    pub include_horizon: bool,
}

impl Default for ChartConfig {
    fn default() -> Self {
        Self {
            house_system: HouseSystem::default(),
            zodiac: Zodiac::default(),
            bodies: BodySet::default(),
            include_horizon: false,
        }
    }
}
