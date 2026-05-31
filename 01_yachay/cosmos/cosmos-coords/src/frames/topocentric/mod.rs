use crate::{CoordResult, Distance};
use cosmos_core::constants::{HALF_PI, TWOPI};
use cosmos_core::{Angle, Location};
use cosmos_time::TT;

const EARTH_RADIUS_AU: f64 = 4.2635e-5; // 6378.137 km

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TopocentricPosition {
    azimuth: Angle,
    elevation: Angle,
    observer: Location,
    epoch: TT,
    distance: Option<Distance>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct HourAnglePosition {
    hour_angle: Angle,
    declination: Angle,
    observer: Location,
    epoch: TT,
    distance: Option<Distance>,
}


mod hour_angle;
mod position;
#[cfg(test)]
mod tests;

// Note: Topocentric coordinates cannot implement CoordinateFrame directly
// because they require both time AND observer location for transformation.
// They need specialized transformation methods.

impl std::fmt::Display for TopocentricPosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Topocentric(Az={:.2}° {}, El={:.2}°",
            self.azimuth.degrees(),
            self.cardinal_direction(),
            self.elevation.degrees()
        )?;

        if let Some(distance) = self.distance {
            write!(f, ", d={}", distance)?;
        }

        write!(f, ")")
    }
}

impl std::fmt::Display for HourAnglePosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "HourAngle(HA={:.4}h, Dec={:.4}°",
            self.hour_angle.hours(),
            self.declination.degrees()
        )?;

        if let Some(distance) = self.distance {
            write!(f, ", d={}", distance)?;
        }

        write!(f, ")")
    }
}
