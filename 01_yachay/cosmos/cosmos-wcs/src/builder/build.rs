//! `WcsBuilder` — construye una `Wcs` desde valores sueltos o una cabecera FITS.

use std::collections::HashMap;

use cosmos_core::Angle;

use crate::distortion::DistortionModel;
use crate::error::{WcsError, WcsResult};
use crate::header::KeywordProvider;
use crate::linear::LinearTransform;
use crate::spherical::{Projection, SphericalRotation};

use super::{CoordType, Wcs};

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) enum MatrixSpec {
    #[default]
    None,
    Cd([[f64; 2]; 2]),
    PcCdelt {
        pc: [[f64; 2]; 2],
        cdelt: [f64; 2],
    },
}

#[derive(Debug, Clone, Default)]
pub struct WcsBuilder {
    pub(crate) crpix: Option<[f64; 2]>,
    pub(crate) crval: Option<[f64; 2]>,
    pub(crate) matrix: MatrixSpec,
    pub(crate) projection: Option<Projection>,
    pub(crate) lonpole: Option<f64>,
    pub(crate) latpole: Option<f64>,
    pub(crate) pv_params: HashMap<(u8, u8), f64>,
    pub(crate) coord_type: Option<CoordType>,
    pub(crate) proj_code: Option<String>,
    pub(crate) distortion: Option<DistortionModel>,
}

impl WcsBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn crpix(mut self, x: f64, y: f64) -> Self {
        self.crpix = Some([x, y]);
        self
    }

    pub fn crval(mut self, lon: f64, lat: f64) -> Self {
        self.crval = Some([lon, lat]);
        self
    }

    pub fn cd_matrix(mut self, cd: [[f64; 2]; 2]) -> Self {
        self.matrix = MatrixSpec::Cd(cd);
        self
    }

    pub fn pc_cdelt(mut self, pc: [[f64; 2]; 2], cdelt: [f64; 2]) -> Self {
        self.matrix = MatrixSpec::PcCdelt { pc, cdelt };
        self
    }

    pub fn projection(mut self, proj: Projection) -> Self {
        self.projection = Some(proj);
        self
    }

    pub fn lonpole(mut self, lonpole: f64) -> Self {
        self.lonpole = Some(lonpole);
        self
    }

    pub fn latpole(mut self, latpole: f64) -> Self {
        self.latpole = Some(latpole);
        self
    }

    pub fn pv(mut self, axis: u8, index: u8, value: f64) -> Self {
        self.pv_params.insert((axis, index), value);
        self
    }

    pub fn coord_type(mut self, coord_type: CoordType) -> Self {
        self.coord_type = Some(coord_type);
        self
    }

    pub fn proj_code(mut self, code: impl Into<String>) -> Self {
        self.proj_code = Some(code.into());
        self
    }

    pub fn distortion(mut self, distortion: DistortionModel) -> Self {
        self.distortion = Some(distortion);
        self
    }

    pub fn from_header(header: &impl KeywordProvider) -> WcsResult<Self> {
        let ctype1 = header.require_string("CTYPE1")?;
        let ctype2 = header.require_string("CTYPE2")?;

        let (prefix1, proj_code1) = parse_ctype(&ctype1)?;
        let (_prefix2, proj_code2) = parse_ctype(&ctype2)?;

        if proj_code1 != proj_code2 {
            return Err(WcsError::invalid_keyword(
                "CTYPE1/CTYPE2",
                format!(
                    "Mismatched projection codes: '{}' vs '{}'",
                    proj_code1, proj_code2
                ),
            ));
        }

        let coord_type = CoordType::from_ctype_prefix(prefix1);
        let proj_code = proj_code1.to_string();

        let crpix1 = header.require_float("CRPIX1")?;
        let crpix2 = header.require_float("CRPIX2")?;

        let crval1 = header.require_float("CRVAL1")?;
        let crval2 = header.require_float("CRVAL2")?;

        let matrix = parse_matrix(header)?;

        let lonpole = header.get_float("LONPOLE");
        let latpole = header.get_float("LATPOLE");

        let pv_params = parse_pv_params(header);

        let mut builder = Self::new()
            .crpix(crpix1, crpix2)
            .crval(crval1, crval2)
            .coord_type(coord_type)
            .proj_code(proj_code);

        builder.matrix = matrix;

        if let Some(lp) = lonpole {
            builder = builder.lonpole(lp);
        }
        if let Some(lp) = latpole {
            builder = builder.latpole(lp);
        }

        builder.pv_params = pv_params;

        Ok(builder)
    }

    pub fn validate(&self) -> WcsResult<()> {
        if self.crpix.is_none() {
            return Err(WcsError::missing_keyword("Missing CRPIX"));
        }
        if self.crval.is_none() {
            return Err(WcsError::missing_keyword("Missing CRVAL"));
        }
        if self.matrix == MatrixSpec::None {
            return Err(WcsError::missing_keyword(
                "Missing transformation matrix (CD or PC+CDELT)",
            ));
        }
        if self.projection.is_none() && self.proj_code.is_none() {
            return Err(WcsError::missing_keyword(
                "Missing projection (set projection or proj_code)",
            ));
        }
        Ok(())
    }

    pub fn build(self) -> WcsResult<Wcs> {
        self.validate()?;

        let crpix = self.crpix.unwrap();
        let crval = self.crval.unwrap();

        let linear = match &self.matrix {
            MatrixSpec::Cd(cd) => LinearTransform::from_cd(crpix, *cd)?,
            MatrixSpec::PcCdelt { pc, cdelt } => {
                LinearTransform::from_pc_cdelt(crpix, *pc, *cdelt)?
            }
            MatrixSpec::None => unreachable!("validate() ensures matrix is set"),
        };

        let projection = match self.projection {
            Some(proj) => proj,
            None => {
                let code = self.proj_code.as_ref().unwrap();
                create_projection_from_code(code, &self.pv_params)?
            }
        };

        let (_, theta_0) = projection.native_reference();

        let lonpole = self.lonpole.map(Angle::from_degrees);
        let latpole = self.latpole.map(Angle::from_degrees);

        let rotation = SphericalRotation::from_crval(
            Angle::from_degrees(crval[0]),
            Angle::from_degrees(crval[1]),
            Angle::from_degrees(theta_0),
            lonpole,
            latpole,
        )?;

        let coord_type = self.coord_type.unwrap_or_default();
        let proj_code = self
            .proj_code
            .unwrap_or_else(|| projection_to_code(&projection));

        Ok(Wcs::new(
            linear,
            projection,
            rotation,
            coord_type,
            proj_code,
            (crval[0], crval[1]),
            self.distortion,
        ))
    }
}

pub(crate) fn create_projection_from_code(
    code: &str,
    pv_params: &HashMap<(u8, u8), f64>,
) -> WcsResult<Projection> {
    match code {
        "TAN" => Ok(Projection::tan()),
        "SIN" => {
            let xi = pv_params.get(&(2, 1)).copied().unwrap_or(0.0);
            let eta = pv_params.get(&(2, 2)).copied().unwrap_or(0.0);
            if xi == 0.0 && eta == 0.0 {
                Ok(Projection::sin())
            } else {
                Ok(Projection::sin_with_params(xi, eta))
            }
        }
        "ARC" => Ok(Projection::arc()),
        "STG" => Ok(Projection::stg()),
        "ZEA" => Ok(Projection::zea()),
        "AZP" => {
            let mu = pv_params.get(&(2, 1)).copied().unwrap_or(0.0);
            let gamma = pv_params.get(&(2, 2)).copied().unwrap_or(0.0);
            Ok(Projection::azp(mu, gamma))
        }
        "SZP" => {
            let mu = pv_params.get(&(2, 1)).copied().unwrap_or(0.0);
            let phi_c = pv_params.get(&(2, 2)).copied().unwrap_or(0.0);
            let theta_c = pv_params.get(&(2, 3)).copied().unwrap_or(90.0);
            Ok(Projection::szp(mu, phi_c, theta_c))
        }
        "ZPN" => {
            let mut coeffs = Vec::new();
            for i in 0..=20 {
                if let Some(&val) = pv_params.get(&(2, i)) {
                    while coeffs.len() < i as usize {
                        coeffs.push(0.0);
                    }
                    coeffs.push(val);
                }
            }
            if coeffs.is_empty() {
                coeffs.push(0.0);
                coeffs.push(1.0);
            }
            Ok(Projection::zpn(coeffs))
        }
        "AIR" => {
            let theta_b = pv_params.get(&(2, 1)).copied().unwrap_or(90.0);
            Ok(Projection::air(theta_b))
        }
        "CAR" => Ok(Projection::car()),
        "MER" => Ok(Projection::mer()),
        "CEA" => {
            let lambda = pv_params.get(&(2, 1)).copied().unwrap_or(1.0);
            Ok(Projection::cea_with_lambda(lambda))
        }
        "CYP" => {
            let mu = pv_params.get(&(2, 1)).copied().unwrap_or(0.0);
            let lambda = pv_params.get(&(2, 2)).copied().unwrap_or(1.0);
            Ok(Projection::cyp(mu, lambda))
        }
        "SFL" => Ok(Projection::sfl()),
        "PAR" => Ok(Projection::par()),
        "MOL" => Ok(Projection::mol()),
        "AIT" => Ok(Projection::ait()),
        "COP" => {
            let theta_a = pv_params.get(&(2, 1)).ok_or_else(|| {
                WcsError::missing_keyword("COP projection requires PV2_1 (theta_a)")
            })?;
            Ok(Projection::cop(*theta_a))
        }
        "COE" => {
            let theta_a = pv_params.get(&(2, 1)).ok_or_else(|| {
                WcsError::missing_keyword("COE projection requires PV2_1 (theta_a)")
            })?;
            Ok(Projection::coe(*theta_a))
        }
        "COD" => {
            let theta_a = pv_params.get(&(2, 1)).ok_or_else(|| {
                WcsError::missing_keyword("COD projection requires PV2_1 (theta_a)")
            })?;
            Ok(Projection::cod(*theta_a))
        }
        "COO" => {
            let theta_a = pv_params.get(&(2, 1)).ok_or_else(|| {
                WcsError::missing_keyword("COO projection requires PV2_1 (theta_a)")
            })?;
            Ok(Projection::coo(*theta_a))
        }
        "BON" => {
            let theta_1 = pv_params.get(&(2, 1)).ok_or_else(|| {
                WcsError::missing_keyword("BON projection requires PV2_1 (theta_1)")
            })?;
            Ok(Projection::bon(*theta_1))
        }
        "PCO" => Ok(Projection::pco()),
        "TSC" => Ok(Projection::tsc()),
        "CSC" => Ok(Projection::csc()),
        "QSC" => Ok(Projection::qsc()),
        _ => Err(WcsError::unsupported_projection(code)),
    }
}

pub(crate) fn projection_to_code(proj: &Projection) -> String {
    match proj {
        Projection::Tan => "TAN",
        Projection::Sin { .. } => "SIN",
        Projection::Arc => "ARC",
        Projection::Stg => "STG",
        Projection::Zea => "ZEA",
        Projection::Azp { .. } => "AZP",
        Projection::Szp { .. } => "SZP",
        Projection::Zpn { .. } => "ZPN",
        Projection::Air { .. } => "AIR",
        Projection::Car => "CAR",
        Projection::Mer => "MER",
        Projection::Cea { .. } => "CEA",
        Projection::Cyp { .. } => "CYP",
        Projection::Sfl => "SFL",
        Projection::Par => "PAR",
        Projection::Mol => "MOL",
        Projection::Ait => "AIT",
        Projection::Cop { .. } => "COP",
        Projection::Coe { .. } => "COE",
        Projection::Cod { .. } => "COD",
        Projection::Coo { .. } => "COO",
        Projection::Bon { .. } => "BON",
        Projection::Pco => "PCO",
        Projection::Tsc => "TSC",
        Projection::Csc => "CSC",
        Projection::Qsc => "QSC",
    }
    .to_string()
}

pub(crate) fn parse_ctype(ctype: &str) -> WcsResult<(&str, &str)> {
    let trimmed = ctype.trim();

    if let Some(dash_pos) = trimmed.rfind('-') {
        if dash_pos == 0 {
            return Err(WcsError::invalid_keyword(
                "CTYPE",
                format!("Invalid CTYPE format: '{}'", ctype),
            ));
        }

        let prefix_part = &trimmed[..dash_pos];
        let proj_part = &trimmed[dash_pos + 1..];

        let prefix = prefix_part.trim_end_matches('-');

        if proj_part.is_empty() {
            return Err(WcsError::invalid_keyword(
                "CTYPE",
                format!("Missing projection code in CTYPE: '{}'", ctype),
            ));
        }

        Ok((prefix, proj_part))
    } else {
        Err(WcsError::invalid_keyword(
            "CTYPE",
            format!("Invalid CTYPE format (no dash separator): '{}'", ctype),
        ))
    }
}

fn parse_matrix(header: &impl KeywordProvider) -> WcsResult<MatrixSpec> {
    let cd11 = header.get_float("CD1_1");
    let cd12 = header.get_float("CD1_2");
    let cd21 = header.get_float("CD2_1");
    let cd22 = header.get_float("CD2_2");

    if cd11.is_some() || cd12.is_some() || cd21.is_some() || cd22.is_some() {
        let cd = [
            [cd11.unwrap_or(0.0), cd12.unwrap_or(0.0)],
            [cd21.unwrap_or(0.0), cd22.unwrap_or(0.0)],
        ];
        return Ok(MatrixSpec::Cd(cd));
    }

    let cdelt1 = header.get_float("CDELT1");
    let cdelt2 = header.get_float("CDELT2");

    if let (Some(c1), Some(c2)) = (cdelt1, cdelt2) {
        let pc11 = header.get_float("PC1_1").unwrap_or(1.0);
        let pc12 = header.get_float("PC1_2").unwrap_or(0.0);
        let pc21 = header.get_float("PC2_1").unwrap_or(0.0);
        let pc22 = header.get_float("PC2_2").unwrap_or(1.0);

        let pc = [[pc11, pc12], [pc21, pc22]];
        let cdelt = [c1, c2];

        return Ok(MatrixSpec::PcCdelt { pc, cdelt });
    }

    Err(WcsError::missing_keyword(
        "CD1_1 or CDELT1 (no transformation matrix found)",
    ))
}

fn parse_pv_params(header: &impl KeywordProvider) -> HashMap<(u8, u8), f64> {
    let mut pv_params = HashMap::new();

    for axis in 1..=2u8 {
        for index in 0..=20u8 {
            let key = format!("PV{}_{}", axis, index);
            if let Some(value) = header.get_float(&key) {
                pv_params.insert((axis, index), value);
            }
        }
    }

    pv_params
}
