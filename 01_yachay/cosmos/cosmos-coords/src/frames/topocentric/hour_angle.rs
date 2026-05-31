use super::*;

impl HourAnglePosition {
    pub fn new(
        hour_angle: Angle,
        declination: Angle,
        observer: Location,
        epoch: TT,
    ) -> CoordResult<Self> {
        let hour_angle = hour_angle.wrapped(); // [-180°, +180°]
        let declination = declination.validate_declination(true)?; // beyond_pole for GEM pier-flips

        Ok(Self {
            hour_angle,
            declination,
            observer,
            epoch,
            distance: None,
        })
    }

    pub fn with_distance(
        hour_angle: Angle,
        declination: Angle,
        observer: Location,
        epoch: TT,
        distance: Distance,
    ) -> CoordResult<Self> {
        let mut pos = Self::new(hour_angle, declination, observer, epoch)?;
        pos.distance = Some(distance);
        Ok(pos)
    }

    pub fn hour_angle(&self) -> Angle {
        self.hour_angle
    }

    pub fn declination(&self) -> Angle {
        self.declination
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

    pub fn to_topocentric(&self) -> CoordResult<TopocentricPosition> {
        // Step 1: Pre-compute trigonometric functions
        let (sin_ha, cos_ha) = self.hour_angle.sin_cos(); // sh, ch
        let (sin_dec, cos_dec) = self.declination.sin_cos(); // sd, cd
        let (sin_lat, cos_lat) = self.observer.latitude_angle().sin_cos(); // sp, cp

        // Step 2: Compute 3D unit vector in Az/El system
        // This implements the rotation matrix transformation
        let x = -cos_ha * cos_dec * sin_lat + sin_dec * cos_lat; // X component
        let y = -sin_ha * cos_dec; // Y component
        let z = cos_ha * cos_dec * cos_lat + sin_dec * sin_lat; // Z component

        // Step 3: Convert to spherical coordinates
        let r = libm::sqrt(x * x + y * y); // Horizontal distance

        let raw_azimuth = if r != 0.0 { libm::atan2(y, x) } else { 0.0 }; // Raw azimuth angle

        let azimuth = if raw_azimuth < 0.0 {
            // Normalize to [0, 2π]
            raw_azimuth + TWOPI
        } else {
            raw_azimuth
        };

        let elevation = libm::atan2(z, r); // Elevation angle

        let mut topo = TopocentricPosition::new(
            Angle::from_radians(azimuth),
            Angle::from_radians(elevation),
            self.observer,
            self.epoch,
        )?;

        if let Some(distance) = self.distance {
            topo.set_distance(distance);
        }

        Ok(topo)
    }

    pub fn is_circumpolar(&self) -> bool {
        let lat = self.observer.latitude_angle();
        self.declination.radians() > (Angle::HALF_PI - lat).radians()
    }

    pub fn never_rises(&self) -> bool {
        let lat = self.observer.latitude_angle();
        self.declination.radians() < -(Angle::HALF_PI - lat).radians()
    }

    pub fn to_cirs(&self, delta_t: f64) -> CoordResult<crate::frames::CIRSPosition> {
        use cosmos_time::scales::conversions::ToUT1WithDeltaT;
        use cosmos_time::sidereal::GAST;

        let ut1 = self.epoch.to_ut1_with_delta_t(delta_t)?;
        let gast = GAST::from_ut1_and_tt(&ut1, &self.epoch)?;
        let last = gast.to_last(&self.observer);

        let ra_rad = last.radians() - self.hour_angle.radians();
        let ra = cosmos_core::angle::wrap_0_2pi(ra_rad);

        let mut cirs = crate::frames::CIRSPosition::new(
            Angle::from_radians(ra),
            self.declination,
            self.epoch,
        )?;

        if let Some(distance) = self.distance {
            cirs.set_distance(distance);
        }

        Ok(cirs)
    }
}
