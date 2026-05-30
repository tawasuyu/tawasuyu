//! `Wcs` — solución WCS construida: transforma píxel↔celeste y serializa keywords.

use cosmos_core::Angle;

use crate::coordinate::{CelestialCoord, IntermediateCoord, PixelCoord};
use crate::distortion::DistortionModel;
use crate::error::WcsResult;
use crate::linear::LinearTransform;
use crate::spherical::{Projection, SphericalRotation};

use super::WcsKeyword;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CoordType {
    #[default]
    Equatorial,
    Galactic,
    Ecliptic,
    Helioecliptic,
    Supergalactic,
    Generic,
}

impl CoordType {
    pub fn from_ctype_prefix(prefix: &str) -> Self {
        match prefix {
            "RA" | "DEC" => Self::Equatorial,
            "GLON" | "GLAT" => Self::Galactic,
            "ELON" | "ELAT" => Self::Ecliptic,
            "HLON" | "HLAT" => Self::Helioecliptic,
            "SLON" | "SLAT" => Self::Supergalactic,
            _ => Self::Generic,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Wcs {
    linear: LinearTransform,
    projection: Projection,
    rotation: SphericalRotation,
    coord_type: CoordType,
    proj_code: String,
    crval_deg: (f64, f64),
    distortion: Option<DistortionModel>,
}

impl Wcs {
    pub fn new(
        linear: LinearTransform,
        projection: Projection,
        rotation: SphericalRotation,
        coord_type: CoordType,
        proj_code: String,
        crval_deg: (f64, f64),
        distortion: Option<DistortionModel>,
    ) -> Self {
        Self {
            linear,
            projection,
            rotation,
            coord_type,
            proj_code,
            crval_deg,
            distortion,
        }
    }

    pub fn pixel_to_celestial(&self, pixel: PixelCoord) -> WcsResult<CelestialCoord> {
        let pixel = self.apply_sip_forward(pixel);
        let intermediate = self.linear.pixel_to_intermediate(pixel);
        let intermediate = self.apply_tpv_tnx_forward(intermediate);
        let native = self.projection.deproject(intermediate)?;
        self.rotation.native_to_celestial(native)
    }

    pub fn celestial_to_pixel(&self, celestial: CelestialCoord) -> WcsResult<PixelCoord> {
        let native = self.rotation.celestial_to_native(celestial)?;
        let intermediate = self.projection.project(native)?;
        let intermediate = self.apply_tpv_tnx_inverse(intermediate)?;
        let pixel = self.linear.intermediate_to_pixel(intermediate);
        self.apply_sip_inverse(pixel)
    }

    fn apply_sip_forward(&self, pixel: PixelCoord) -> PixelCoord {
        match &self.distortion {
            Some(DistortionModel::Sip(sip)) => {
                let (x, y) = sip.apply(pixel.x(), pixel.y());
                PixelCoord::new(x, y)
            }
            _ => pixel,
        }
    }

    fn apply_tpv_tnx_forward(&self, intermediate: IntermediateCoord) -> IntermediateCoord {
        match &self.distortion {
            Some(DistortionModel::Tpv(tpv)) => {
                let (x, y) = tpv
                    .as_ref()
                    .apply(intermediate.x_deg(), intermediate.y_deg());
                IntermediateCoord::new(x, y)
            }
            Some(DistortionModel::Tnx(tnx)) => {
                let (x, y) = tnx.apply(intermediate.x_deg(), intermediate.y_deg());
                IntermediateCoord::new(x, y)
            }
            _ => intermediate,
        }
    }

    fn apply_tpv_tnx_inverse(
        &self,
        intermediate: IntermediateCoord,
    ) -> WcsResult<IntermediateCoord> {
        match &self.distortion {
            Some(DistortionModel::Tpv(tpv)) => {
                let (x, y) = tpv
                    .as_ref()
                    .apply_inverse(intermediate.x_deg(), intermediate.y_deg())?;
                Ok(IntermediateCoord::new(x, y))
            }
            Some(DistortionModel::Tnx(tnx)) => {
                let (x, y) = tnx.apply_inverse(intermediate.x_deg(), intermediate.y_deg())?;
                Ok(IntermediateCoord::new(x, y))
            }
            _ => Ok(intermediate),
        }
    }

    fn apply_sip_inverse(&self, pixel: PixelCoord) -> WcsResult<PixelCoord> {
        match &self.distortion {
            Some(DistortionModel::Sip(sip)) => {
                let (x, y) = sip.apply_inverse(pixel.x(), pixel.y())?;
                Ok(PixelCoord::new(x, y))
            }
            _ => Ok(pixel),
        }
    }

    pub fn pix2world(&self, x: f64, y: f64) -> WcsResult<(f64, f64)> {
        let pixel = PixelCoord::new(x, y);
        let celestial = self.pixel_to_celestial(pixel)?;
        Ok((celestial.alpha().degrees(), celestial.delta().degrees()))
    }

    pub fn world2pix(&self, lon: f64, lat: f64) -> WcsResult<(f64, f64)> {
        let celestial = CelestialCoord::new(Angle::from_degrees(lon), Angle::from_degrees(lat));
        let pixel = self.celestial_to_pixel(celestial)?;
        Ok((pixel.x(), pixel.y()))
    }

    #[inline]
    pub fn projection_code(&self) -> &str {
        &self.proj_code
    }

    #[inline]
    pub fn coord_type(&self) -> CoordType {
        self.coord_type
    }

    #[inline]
    pub fn crpix(&self) -> [f64; 2] {
        self.linear.crpix()
    }

    #[inline]
    pub fn crval(&self) -> (f64, f64) {
        self.crval_deg
    }

    #[inline]
    pub fn pixel_scale(&self) -> f64 {
        self.linear.pixel_scale()
    }

    #[inline]
    pub fn projection(&self) -> &Projection {
        &self.projection
    }

    #[inline]
    pub fn rotation(&self) -> &SphericalRotation {
        &self.rotation
    }

    #[inline]
    pub fn linear(&self) -> &LinearTransform {
        &self.linear
    }

    pub fn to_keywords(&self) -> Vec<WcsKeyword> {
        let mut keywords = Vec::new();

        keywords.extend(self.ctype_keywords());
        keywords.extend(self.crpix_keywords());
        keywords.extend(self.crval_keywords());
        keywords.extend(self.cd_keywords());
        keywords.extend(self.pole_keywords());
        keywords.extend(self.pv_keywords());

        keywords
    }

    fn ctype_keywords(&self) -> Vec<WcsKeyword> {
        let (prefix1, prefix2) = ctype_prefixes(&self.coord_type);
        let code = &self.proj_code;

        vec![
            WcsKeyword::string("CTYPE1", format_ctype(prefix1, code)),
            WcsKeyword::string("CTYPE2", format_ctype(prefix2, code)),
        ]
    }

    fn crpix_keywords(&self) -> Vec<WcsKeyword> {
        let crpix = self.linear.crpix();
        vec![
            WcsKeyword::real("CRPIX1", crpix[0]),
            WcsKeyword::real("CRPIX2", crpix[1]),
        ]
    }

    fn crval_keywords(&self) -> Vec<WcsKeyword> {
        vec![
            WcsKeyword::real("CRVAL1", self.crval_deg.0),
            WcsKeyword::real("CRVAL2", self.crval_deg.1),
        ]
    }

    fn cd_keywords(&self) -> Vec<WcsKeyword> {
        let cd = self.linear.cd_matrix();
        vec![
            WcsKeyword::real("CD1_1", cd[0][0]),
            WcsKeyword::real("CD1_2", cd[0][1]),
            WcsKeyword::real("CD2_1", cd[1][0]),
            WcsKeyword::real("CD2_2", cd[1][1]),
        ]
    }

    fn pole_keywords(&self) -> Vec<WcsKeyword> {
        let mut keywords = Vec::new();
        let phi_p = self.rotation.phi_p_degrees();
        let delta_p = self.rotation.delta_p_degrees();

        let default_phi_p = default_lonpole(&self.coord_type, self.crval_deg.1, &self.projection);
        if (phi_p - default_phi_p).abs() > 1e-10 {
            keywords.push(WcsKeyword::real("LONPOLE", phi_p));
        }

        if (delta_p - 90.0).abs() > 1e-10 {
            keywords.push(WcsKeyword::real("LATPOLE", delta_p));
        }

        keywords
    }

    fn pv_keywords(&self) -> Vec<WcsKeyword> {
        projection_pv_keywords(&self.projection)
    }
}

fn ctype_prefixes(coord_type: &CoordType) -> (&'static str, &'static str) {
    match coord_type {
        CoordType::Equatorial => ("RA", "DEC"),
        CoordType::Galactic => ("GLON", "GLAT"),
        CoordType::Ecliptic => ("ELON", "ELAT"),
        CoordType::Helioecliptic => ("HLON", "HLAT"),
        CoordType::Supergalactic => ("SLON", "SLAT"),
        CoordType::Generic => ("XLON", "XLAT"),
    }
}

pub(crate) fn format_ctype(prefix: &str, proj_code: &str) -> String {
    let padding_len = 4 - prefix.len();
    let dashes = "-".repeat(padding_len + 1);
    format!("{}{}{}", prefix, dashes, proj_code)
}

fn default_lonpole(_coord_type: &CoordType, crval_lat: f64, projection: &Projection) -> f64 {
    let (_, theta_0) = projection.native_reference();
    if crval_lat >= theta_0 {
        0.0
    } else {
        180.0
    }
}

fn projection_pv_keywords(projection: &Projection) -> Vec<WcsKeyword> {
    match projection {
        Projection::Sin { xi, eta } if *xi != 0.0 || *eta != 0.0 => {
            vec![
                WcsKeyword::real("PV2_1", *xi),
                WcsKeyword::real("PV2_2", *eta),
            ]
        }
        Projection::Azp { mu, gamma } => {
            vec![
                WcsKeyword::real("PV2_1", *mu),
                WcsKeyword::real("PV2_2", *gamma),
            ]
        }
        Projection::Szp { mu, phi_c, theta_c } => {
            vec![
                WcsKeyword::real("PV2_1", *mu),
                WcsKeyword::real("PV2_2", *phi_c),
                WcsKeyword::real("PV2_3", *theta_c),
            ]
        }
        Projection::Zpn { coeffs } => coeffs
            .iter()
            .enumerate()
            .filter(|(_, &v)| v != 0.0)
            .map(|(i, &v)| WcsKeyword::real(format!("PV2_{}", i), v))
            .collect(),
        Projection::Air { theta_b } => {
            vec![WcsKeyword::real("PV2_1", *theta_b)]
        }
        Projection::Cea { lambda } if *lambda != 1.0 => {
            vec![WcsKeyword::real("PV2_1", *lambda)]
        }
        Projection::Cyp { mu, lambda } => {
            vec![
                WcsKeyword::real("PV2_1", *mu),
                WcsKeyword::real("PV2_2", *lambda),
            ]
        }
        Projection::Cop { theta_a }
        | Projection::Coe { theta_a }
        | Projection::Cod { theta_a }
        | Projection::Coo { theta_a } => {
            vec![WcsKeyword::real("PV2_1", *theta_a)]
        }
        Projection::Bon { theta_1 } => {
            vec![WcsKeyword::real("PV2_1", *theta_1)]
        }
        _ => Vec::new(),
    }
}
