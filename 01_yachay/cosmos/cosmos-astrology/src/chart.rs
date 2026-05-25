//! The `NatalChart`: assembly of birth data → angles → houses → placements.

use cosmos_core::Location;
use cosmos_sky::{ApparentPosition, Body, EphemerisSession, HorizonCoord, Instant, Observer};
use cosmos_time::sidereal::GAST;
use cosmos_time::scales::conversions::ToUT1WithDeltaT;
use cosmos_validation::sidereal::{ayanamsha as ayanamsha_value, true_obliquity_iau2006a};

use crate::birth_data::BirthData;
use crate::chart_config::ChartConfig;
use crate::error::{AstrologyError, AstrologyResult};
use crate::house_system::Houses;
use crate::placement::BodyPlacement;
use crate::zodiac::{SignedLongitude, Zodiac};

/// A computed angle (Ascendant, MC, Descendant, IC). Wraps a
/// [`SignedLongitude`] so callers can speak in sign-decimal form.
#[derive(Debug, Clone, Copy)]
pub struct Angle {
    inner: SignedLongitude,
}

impl Angle {
    fn new(longitude_rad: f64) -> Self {
        Self {
            inner: SignedLongitude::from_radians(longitude_rad),
        }
    }

    /// Construct an angle from a raw zodiacal longitude in radians.
    /// Internal helper exposed for directed charts (Solar Arc).
    pub fn from_radians(longitude_rad: f64) -> Self {
        Self::new(longitude_rad)
    }
    pub fn longitude_rad(&self) -> f64 {
        self.inner.longitude_rad()
    }
    pub fn longitude_deg(&self) -> f64 {
        self.inner.longitude_deg()
    }
    pub fn sign(&self) -> crate::zodiac::Sign {
        self.inner.sign()
    }
    pub fn degree_in_sign(&self) -> u32 {
        self.inner.degree_in_sign()
    }
    pub fn degree_in_sign_decimal(&self) -> f64 {
        self.inner.degree_in_sign_decimal()
    }
    pub fn to_chart_format(&self) -> String {
        self.inner.to_chart_format()
    }
}

/// A computed natal chart. All longitudes are stored in radians; signed
/// decompositions are derived via [`SignedLongitude`].
#[derive(Debug, Clone)]
pub struct NatalChart {
    pub birth: BirthData,
    pub config: ChartConfig,

    // ─── Core geometry ────────────────────────────────────────────────
    /// True obliquity of date (mean + nutation in obliquity), radians.
    pub obliquity_rad: f64,
    /// Local Apparent Sidereal Time at the observer's longitude, radians.
    pub local_apparent_sidereal_time_rad: f64,
    /// Ayanamsha applied for sidereal mode, radians. `0.0` for tropical.
    pub ayanamsha_rad: f64,

    // ─── Angles ───────────────────────────────────────────────────────
    ascendant: Angle,
    midheaven: Angle,
    descendant: Angle,
    imum_coeli: Angle,

    // ─── Houses ───────────────────────────────────────────────────────
    pub houses: Houses,

    // ─── Placements (parallel to `config.bodies.bodies`) ───────────────
    pub placements: Vec<BodyPlacement>,
}

impl NatalChart {
    pub fn ascendant(&self) -> Angle {
        self.ascendant
    }
    pub fn midheaven(&self) -> Angle {
        self.midheaven
    }
    pub fn descendant(&self) -> Angle {
        self.descendant
    }
    pub fn imum_coeli(&self) -> Angle {
        self.imum_coeli
    }

    /// Lookup a placement by body. `None` if the requested body was not
    /// in the configured [`crate::BodySet`].
    pub fn placement(&self, body: Body) -> Option<&BodyPlacement> {
        self.placements.iter().find(|p| p.body == body)
    }

    /// Overwrite this chart's four angles with another chart's, leaving
    /// every other field untouched. Used by [`crate::progress`] when the
    /// caller asks for `ProgressedHouses::Natal`: angles and cusps freeze
    /// to the natal values while placements advance with the progressed
    /// chart.
    pub fn replace_angles_with(&mut self, other: &NatalChart) {
        self.ascendant = other.ascendant;
        self.midheaven = other.midheaven;
        self.descendant = other.descendant;
        self.imum_coeli = other.imum_coeli;
    }

    /// Overwrite the four angles explicitly. Used by Solar Arc to apply
    /// the uniform-rotation shift after copying from the natal chart.
    pub fn set_directed_angles(
        &mut self,
        ascendant: Angle,
        midheaven: Angle,
        descendant: Angle,
        imum_coeli: Angle,
    ) {
        self.ascendant = ascendant;
        self.midheaven = midheaven;
        self.descendant = descendant;
        self.imum_coeli = imum_coeli;
    }

    /// Compute a natal chart end-to-end.
    pub fn compute(
        birth: &BirthData,
        config: &ChartConfig,
        session: &EphemerisSession,
    ) -> AstrologyResult<Self> {
        let last_rad = compute_last_rad(&birth.instant, &birth.observer)?;
        let tt = birth.instant.tt()?;
        let obliquity_rad = true_obliquity_iau2006a(&tt).map_err(|e| {
            AstrologyError::Sky(cosmos_sky::SkyError::Ephemeris(
                cosmos_validation::oracle::OracleError::Inner(format!("obliquity: {}", e)),
            ))
        })?;

        // Houses + angles are always tropical (ecliptic of date).
        let houses = Houses::compute(
            config.house_system,
            last_rad,
            birth.observer.lat_rad,
            obliquity_rad,
        )?;

        let ayanamsha_rad = match config.zodiac {
            Zodiac::Tropical => 0.0,
            Zodiac::Sidereal(mode) => ayanamsha_value(mode, &tt),
        };

        let zodiac_offset = |tropical_rad: f64| -> f64 {
            const TAU: f64 = std::f64::consts::TAU;
            let v = tropical_rad - ayanamsha_rad;
            let v = v.rem_euclid(TAU);
            if v < 0.0 {
                v + TAU
            } else {
                v
            }
        };

        let asc_for_zodiac = zodiac_offset(houses.ascendant_rad);
        let mc_for_zodiac = zodiac_offset(houses.midheaven_rad);
        let ascendant = Angle::new(asc_for_zodiac);
        let midheaven = Angle::new(mc_for_zodiac);
        let descendant = Angle::new(zodiac_offset(houses.ascendant_rad + std::f64::consts::PI));
        let imum_coeli = Angle::new(zodiac_offset(houses.midheaven_rad + std::f64::consts::PI));

        let observer = if config.include_horizon {
            Some(&birth.observer)
        } else {
            None
        };

        let mut placements = Vec::with_capacity(config.bodies.bodies.len() + 1);
        for &body in &config.bodies.bodies {
            let apparent = compute_body(body, birth.instant, observer, session)?;
            let tropical_lon = apparent.ecliptic_of_date.longitude_rad;
            let zodiac_lon = zodiac_offset(tropical_lon);
            let house = houses.house_containing(tropical_lon);
            placements.push(BodyPlacement::from_apparent(
                body, &apparent, zodiac_lon, house,
            ));
        }

        // Auto-add South Node opposite the (ascending) Mean / True node.
        if config.bodies.include_south_node {
            if let Some(node) = placements.iter().find(|p| {
                matches!(p.body, Body::MeanNode | Body::TrueNode)
            }) {
                let south_lon_zodiac =
                    (node.longitude.longitude_rad() + std::f64::consts::PI)
                        .rem_euclid(std::f64::consts::TAU);
                let south_lon_tropical = (south_lon_zodiac + ayanamsha_rad)
                    .rem_euclid(std::f64::consts::TAU);
                let south_house = houses.house_containing(south_lon_tropical);
                let south_horizon = node.horizon.map(|h| HorizonCoord {
                    // South node is the antipode direction; we don't
                    // recompute horizon for it. Mark altitude as the
                    // anti-altitude (180° around in azimuth).
                    altitude_rad: -h.altitude_rad,
                    azimuth_rad: (h.azimuth_rad + std::f64::consts::PI)
                        .rem_euclid(std::f64::consts::TAU),
                });
                // South node has the antipode RA / Dec of the north node.
                let south_ra = (node.right_ascension_rad + std::f64::consts::PI)
                    .rem_euclid(std::f64::consts::TAU);
                let south_dec = -node.declination_rad;
                placements.push(BodyPlacement {
                    body: south_node_body_for(node.body),
                    longitude: SignedLongitude::from_radians(south_lon_zodiac),
                    latitude_rad: 0.0,
                    distance_km: 0.0,
                    longitude_rate_rad_per_day: node.longitude_rate_rad_per_day,
                    right_ascension_rad: south_ra,
                    declination_rad: south_dec,
                    house_number: south_house,
                    horizon: south_horizon,
                });
            }
        }

        Ok(Self {
            birth: birth.clone(),
            config: config.clone(),
            obliquity_rad,
            local_apparent_sidereal_time_rad: last_rad,
            ayanamsha_rad,
            ascendant,
            midheaven,
            descendant,
            imum_coeli,
            houses,
            placements,
        })
    }
}

/// The South Node is the opposition of the ascending node, so we
/// preserve which family the node came from (Mean / True) and just
/// label the placement accordingly. We use the same `Body::MeanNode` /
/// `Body::TrueNode` identifier with a synthetic `is_retrograde` left
/// to match the ascending node, since the two nodes share motion by
/// construction.
fn south_node_body_for(ascending_body: Body) -> Body {
    // No `Body::SouthNode` variant in the sky façade yet; for now we
    // reuse the ascending-node identifier. Consumers wanting to
    // distinguish should check the placement's longitude (south is
    // exactly +180° opposite). When the sky façade grows dedicated
    // South Node variants, this mapping becomes trivial.
    ascending_body
}

fn compute_body(
    body: Body,
    instant: Instant,
    observer: Option<&Observer>,
    session: &EphemerisSession,
) -> AstrologyResult<ApparentPosition> {
    session.body_apparent(body, instant, observer).map_err(|e| {
        AstrologyError::BodyUnavailable(format!("{}: {}", body.name(), e))
    })
}

/// Local Apparent Sidereal Time at the observer's longitude, radians.
fn compute_last_rad(instant: &Instant, observer: &Observer) -> AstrologyResult<f64> {
    let tt = instant.tt()?;
    let ut1 = tt
        .to_ut1_with_delta_t(instant.delta_t_seconds())
        .map_err(|e| AstrologyError::Sky(cosmos_sky::SkyError::Time(e)))?;
    let location = Location::from_degrees(
        observer.lat_rad.to_degrees(),
        observer.lon_rad.to_degrees(),
        observer.elev_m,
    )
    .map_err(|e| {
        AstrologyError::Sky(cosmos_sky::SkyError::Ephemeris(
            cosmos_validation::oracle::OracleError::Inner(format!("Location: {:?}", e)),
        ))
    })?;
    let gast = GAST::from_ut1_and_tt(&ut1, &tt).map_err(|e| {
        AstrologyError::Sky(cosmos_sky::SkyError::Ephemeris(
            cosmos_validation::oracle::OracleError::Inner(format!("GAST: {:?}", e)),
        ))
    })?;
    Ok(gast.to_last(&location).angle().radians())
}
