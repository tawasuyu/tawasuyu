use super::*;

impl TopocentricPosition {
    pub fn new(
        azimuth: Angle,
        elevation: Angle,
        observer: Location,
        epoch: TT,
    ) -> CoordResult<Self> {
        let azimuth = azimuth.validate_longitude(true)?;
        let elevation = elevation.validate_latitude()?;

        Ok(Self {
            azimuth,
            elevation,
            observer,
            epoch,
            distance: None,
        })
    }

    pub fn with_distance(
        azimuth: Angle,
        elevation: Angle,
        observer: Location,
        epoch: TT,
        distance: Distance,
    ) -> CoordResult<Self> {
        let mut pos = Self::new(azimuth, elevation, observer, epoch)?;
        pos.distance = Some(distance);
        Ok(pos)
    }

    pub fn from_degrees(
        az_deg: f64,
        el_deg: f64,
        observer: Location,
        epoch: TT,
    ) -> CoordResult<Self> {
        Self::new(
            Angle::from_degrees(az_deg),
            Angle::from_degrees(el_deg),
            observer,
            epoch,
        )
    }

    pub fn azimuth(&self) -> Angle {
        self.azimuth
    }

    pub fn elevation(&self) -> Angle {
        self.elevation
    }

    pub fn observer(&self) -> &Location {
        &self.observer
    }

    pub fn epoch(&self) -> TT {
        self.epoch
    }

    pub fn distance(&self) -> Option<Distance> {
        self.distance
    }

    pub fn set_distance(&mut self, distance: Distance) {
        self.distance = Some(distance);
    }

    pub fn zenith_angle(&self) -> Angle {
        Angle::HALF_PI - self.elevation
    }

    pub fn air_mass(&self) -> f64 {
        self.air_mass_rozenberg()
    }

    pub fn air_mass_rozenberg(&self) -> f64 {
        let zenith = self.zenith_angle();
        if zenith.degrees() >= 90.0 {
            return 40.0;
        }
        let cos_z = libm::cos(zenith.radians());
        let term = cos_z + 0.025 * libm::exp(-11.0 * cos_z);
        1.0 / term
    }

    /// Computes airmass using Pickering's (2002) empirical formula.
    ///
    /// # Valid Range
    /// - Returns `f64::INFINITY` for elevations ≤ -2° (below horizon)
    /// - Accurate for elevations > -2° (including astronomical twilight)
    /// - Values become increasingly unreliable as elevation approaches -2°
    ///
    /// # Numerical Stability
    /// Near the horizon (0° to 5°), results can be very large but remain finite.
    /// Use this method only if observations extend below the horizon; otherwise
    /// prefer `air_mass_hardie()` or `air_mass_kasten_young()`.
    ///
    /// Reference: Pickering, K. A. (2002). "The Southern Limits of the Ancient Star
    /// Catalog". DIO 12, 3-27.
    pub fn air_mass_pickering(&self) -> f64 {
        let h = self.elevation.degrees();
        if h <= -2.0 {
            return f64::INFINITY;
        }
        let h_term = 244.0 / (165.0 + 47.0 * h.abs().powf(1.1));
        let sin_arg = h + h_term;
        1.0 / libm::sin(sin_arg * cosmos_core::constants::DEG_TO_RAD)
    }

    pub fn air_mass_kasten_young(&self) -> f64 {
        let zenith_deg = self.zenith_angle().degrees();
        if zenith_deg >= 90.0 {
            return 38.0;
        }
        let cos_z = libm::cos(self.zenith_angle().radians());
        let term = (96.07995 - zenith_deg).powf(-1.6364);
        1.0 / (cos_z + 0.50572 * term)
    }

    pub fn atmospheric_refraction(
        &self,
        pressure_hpa: f64,
        temp_celsius: f64,
        rel_humidity: f64,
        wavelength_um: f64,
    ) -> Angle {
        let (refa, refb) =
            self.refraction_coefficients(pressure_hpa, temp_celsius, rel_humidity, wavelength_um);

        // Apply refraction formula: dZ = A tan(Z) + B tan³(Z)
        let z_obs = self.zenith_angle().radians();
        let tan_z = libm::tan(z_obs);
        let refraction_rad = refa * tan_z + refb * tan_z.powi(3);

        Angle::from_radians(refraction_rad)
    }

    fn refraction_coefficients(
        &self,
        mut pressure_hpa: f64,
        mut temp_celsius: f64,
        mut rel_humidity: f64,
        wavelength_um: f64,
    ) -> (f64, f64) {
        pressure_hpa = pressure_hpa.clamp(0.0, 10000.0);
        temp_celsius = temp_celsius.clamp(-150.0, 200.0);
        rel_humidity = rel_humidity.clamp(0.0, 1.0);

        // Zero pressure means no refraction
        if pressure_hpa <= 0.0 {
            return (0.0, 0.0);
        }

        // Determine if optical/IR or radio
        let is_optical = (0.0..100.0).contains(&wavelength_um);

        // Temperature in Kelvin
        let temp_kelvin = temp_celsius + 273.15;

        let pw = if pressure_hpa > 0.0 {
            // Saturation pressure (Gill 1982, with pressure correction)
            let ps = 10.0_f64
                .powf((0.7859 + 0.03477 * temp_celsius) / (1.0 + 0.00412 * temp_celsius))
                * (1.0 + pressure_hpa * (4.5e-6 + 6e-10 * temp_celsius * temp_celsius));
            // Actual water vapor pressure (Crane 1976)
            rel_humidity * ps / (1.0 - (1.0 - rel_humidity) * ps / pressure_hpa)
        } else {
            0.0
        };

        if is_optical {
            // Optical/IR refraction (Hohenkerk & Sinclair 1985, IAG 1999)
            let wl_sq = wavelength_um * wavelength_um;
            let gamma = ((77.53484e-6 + (4.39108e-7 + 3.666e-9 / wl_sq) / wl_sq) * pressure_hpa
                - 11.2684e-6 * pw)
                / temp_kelvin;

            // Beta from Stone (1996) with empirical adjustments
            let beta = 4.4474e-6 * temp_kelvin;

            // Green (1987) Equation 4.31
            let refa = gamma * (1.0 - beta);
            let refb = -gamma * (beta - gamma / 2.0);

            (refa, refb)
        } else {
            // Radio refraction (Rueger 2002)
            let gamma = (77.6890e-6 * pressure_hpa - (6.3938e-6 - 0.375463 / temp_kelvin) * pw)
                / temp_kelvin;

            // Beta with humidity correction for radio
            let mut beta = 4.4474e-6 * temp_kelvin;
            beta -= 0.0074 * pw * beta;

            // Green (1987) Equation 4.31
            let refa = gamma * (1.0 - beta);
            let refb = -gamma * (beta - gamma / 2.0);

            (refa, refb)
        }
    }

    pub fn with_refraction(
        &self,
        pressure_hpa: f64,
        temp_celsius: f64,
        rel_humidity: f64,
        wavelength_um: f64,
    ) -> Self {
        let refraction =
            self.atmospheric_refraction(pressure_hpa, temp_celsius, rel_humidity, wavelength_um);

        // Refraction increases apparent elevation (decreases zenith distance)
        let apparent_elevation = self.elevation + refraction;

        Self {
            azimuth: self.azimuth,
            elevation: apparent_elevation,
            observer: self.observer,
            epoch: self.epoch,
            distance: self.distance,
        }
    }

    pub fn without_refraction(
        &self,
        pressure_hpa: f64,
        temp_celsius: f64,
        rel_humidity: f64,
        wavelength_um: f64,
    ) -> Self {
        let refraction =
            self.atmospheric_refraction(pressure_hpa, temp_celsius, rel_humidity, wavelength_um);

        // Remove refraction to get true elevation
        let true_elevation = self.elevation - refraction;

        Self {
            azimuth: self.azimuth,
            elevation: true_elevation,
            observer: self.observer,
            epoch: self.epoch,
            distance: self.distance,
        }
    }

    pub fn diurnal_parallax(&self) -> Option<Angle> {
        self.distance.map(|d| {
            // Earth's equatorial radius in AU
            let distance_au = d.au();
            let zenith_angle = self.zenith_angle();

            // Parallax formula: p = arcsin((a/r) × sin(z))
            let ratio = EARTH_RADIUS_AU / distance_au;
            let parallax_rad = if ratio < 1.0 {
                libm::asin(ratio * zenith_angle.sin())
            } else {
                // Object inside Earth - return maximum parallax
                HALF_PI
            };

            Angle::from_radians(parallax_rad)
        })
    }

    pub fn horizontal_parallax(&self) -> Option<Angle> {
        self.distance.map(|d| {
            let distance_au = d.au();
            let ratio = EARTH_RADIUS_AU / distance_au;

            if ratio < 1.0 {
                Angle::from_radians(libm::asin(ratio))
            } else {
                Angle::from_radians(HALF_PI)
            }
        })
    }

    pub fn with_diurnal_parallax(&self) -> Self {
        if let Some(parallax) = self.diurnal_parallax() {
            // Parallax correction decreases elevation (object appears lower)
            let corrected_elevation = self.elevation - parallax;

            Self {
                azimuth: self.azimuth,
                elevation: corrected_elevation,
                observer: self.observer,
                epoch: self.epoch,
                distance: self.distance,
            }
        } else {
            self.clone()
        }
    }

    pub fn without_diurnal_parallax(&self) -> Self {
        if let Some(parallax) = self.diurnal_parallax() {
            // Remove parallax correction (increases elevation)
            let geocentric_elevation = self.elevation + parallax;

            Self {
                azimuth: self.azimuth,
                elevation: geocentric_elevation,
                observer: self.observer,
                epoch: self.epoch,
                distance: self.distance,
            }
        } else {
            self.clone()
        }
    }

    pub fn is_above_horizon(&self) -> bool {
        self.elevation.degrees() > 0.0
    }

    pub fn is_near_zenith(&self) -> bool {
        self.elevation.degrees() > 89.0
    }

    pub fn is_near_horizon(&self) -> bool {
        self.elevation.degrees() < 10.0 && self.is_above_horizon()
    }

    pub fn cardinal_direction(&self) -> &'static str {
        let az_deg = self.azimuth.degrees();
        if !(22.5..337.5).contains(&az_deg) {
            "N"
        } else if az_deg < 67.5 {
            "NE"
        } else if az_deg < 112.5 {
            "E"
        } else if az_deg < 157.5 {
            "SE"
        } else if az_deg < 202.5 {
            "S"
        } else if az_deg < 247.5 {
            "SW"
        } else if az_deg < 292.5 {
            "W"
        } else {
            "NW"
        }
    }

    pub fn parallactic_angle(&self, hour_angle: Angle, declination: Angle) -> Angle {
        let lat = self.observer.latitude_angle();
        let (sin_ha, cos_ha) = hour_angle.sin_cos();
        let (sin_lat, cos_lat) = lat.sin_cos();
        let (sin_dec, cos_dec) = declination.sin_cos();

        let numerator = sin_ha;
        let denominator = cos_dec * sin_lat - sin_dec * cos_lat * cos_ha;

        Angle::from_radians(libm::atan2(numerator, denominator))
    }

    pub fn to_hour_angle(&self) -> CoordResult<HourAnglePosition> {
        let (sin_az, cos_az) = self.azimuth.sin_cos();
        let (sin_el, cos_el) = self.elevation.sin_cos();
        let (sin_lat, cos_lat) = self.observer.latitude_angle().sin_cos();

        let sin_dec = sin_el * sin_lat + cos_el * cos_lat * cos_az;
        let dec = libm::asin(sin_dec);

        let cos_dec = libm::cos(dec);

        let cos_ha = if cos_dec.abs() < 1e-10 {
            0.0
        } else {
            (sin_el - sin_dec * sin_lat) / (cos_dec * cos_lat)
        };

        let sin_ha = if cos_dec.abs() < 1e-10 {
            0.0
        } else {
            -sin_az * cos_el / cos_dec
        };

        let ha = libm::atan2(sin_ha, cos_ha);

        let mut ha_pos = HourAnglePosition::new(
            Angle::from_radians(ha),
            Angle::from_radians(dec),
            self.observer,
            self.epoch,
        )?;

        if let Some(distance) = self.distance {
            ha_pos.set_distance(distance);
        }

        Ok(ha_pos)
    }

    pub fn to_cirs(&self, delta_t: f64) -> CoordResult<crate::frames::CIRSPosition> {
        let ha = self.to_hour_angle()?;
        ha.to_cirs(delta_t)
    }
}
