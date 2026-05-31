use super::*;

pub fn wcs_to_xisf_properties(wcs_keywords: &[cosmos_wcs::WcsKeyword]) -> Vec<XisfProperty> {
    let mut properties = Vec::new();
    let mut crval1: Option<f64> = None;
    let mut crval2: Option<f64> = None;
    let mut crpix1: Option<f64> = None;
    let mut crpix2: Option<f64> = None;
    let mut cd1_1: Option<f64> = None;
    let mut cd1_2: Option<f64> = None;
    let mut cd2_1: Option<f64> = None;
    let mut cd2_2: Option<f64> = None;
    let mut proj_code: Option<String> = None;
    let mut lonpole: Option<f64> = None;
    let mut latpole: Option<f64> = None;

    for kw in wcs_keywords {
        match kw.name.as_str() {
            "CTYPE1" => {
                if let cosmos_wcs::WcsKeywordValue::String(s) = &kw.value {
                    proj_code = extract_projection_code(s);
                }
            }
            "CRVAL1" => {
                if let cosmos_wcs::WcsKeywordValue::Real(v) = &kw.value {
                    crval1 = Some(*v);
                }
            }
            "CRVAL2" => {
                if let cosmos_wcs::WcsKeywordValue::Real(v) = &kw.value {
                    crval2 = Some(*v);
                }
            }
            "CRPIX1" => {
                if let cosmos_wcs::WcsKeywordValue::Real(v) = &kw.value {
                    crpix1 = Some(*v);
                }
            }
            "CRPIX2" => {
                if let cosmos_wcs::WcsKeywordValue::Real(v) = &kw.value {
                    crpix2 = Some(*v);
                }
            }
            "CD1_1" => {
                if let cosmos_wcs::WcsKeywordValue::Real(v) = &kw.value {
                    cd1_1 = Some(*v);
                }
            }
            "CD1_2" => {
                if let cosmos_wcs::WcsKeywordValue::Real(v) = &kw.value {
                    cd1_2 = Some(*v);
                }
            }
            "CD2_1" => {
                if let cosmos_wcs::WcsKeywordValue::Real(v) = &kw.value {
                    cd2_1 = Some(*v);
                }
            }
            "CD2_2" => {
                if let cosmos_wcs::WcsKeywordValue::Real(v) = &kw.value {
                    cd2_2 = Some(*v);
                }
            }
            "LONPOLE" => {
                if let cosmos_wcs::WcsKeywordValue::Real(v) = &kw.value {
                    lonpole = Some(*v);
                }
            }
            "LATPOLE" => {
                if let cosmos_wcs::WcsKeywordValue::Real(v) = &kw.value {
                    latpole = Some(*v);
                }
            }
            _ => {}
        }
    }

    if let Some(code) = proj_code {
        properties.push(XisfProperty::string(
            "PCL:AstrometricSolution:ProjectionSystem",
            code,
        ));
    }

    if let (Some(ra), Some(dec)) = (crval1, crval2) {
        properties.push(XisfProperty::f64_vector(
            "PCL:AstrometricSolution:ReferenceCelestialCoordinates",
            vec![ra, dec],
        ));
    }

    if let (Some(x), Some(y)) = (crpix1, crpix2) {
        properties.push(XisfProperty::f64_vector(
            "PCL:AstrometricSolution:ReferenceImageCoordinates",
            vec![x, y],
        ));
    }

    if let (Some(c11), Some(c12), Some(c21), Some(c22)) = (cd1_1, cd1_2, cd2_1, cd2_2) {
        properties.push(XisfProperty::f64_matrix(
            "PCL:AstrometricSolution:LinearTransformationMatrix",
            2,
            2,
            vec![c11, c12, c21, c22],
        ));
    }

    // ReferenceNativeCoordinates: (φ₀, θ₀) in degrees
    // For zenithal projections (TAN, SIN, etc.), this is (0, 90)
    properties.push(XisfProperty::f64_vector(
        "PCL:AstrometricSolution:ReferenceNativeCoordinates",
        vec![0.0, 90.0],
    ));

    // CelestialPoleNativeCoordinates: (φₚ, θₚ) derived from LONPOLE/LATPOLE
    // φₚ = LONPOLE (default 180° if CRVAL2 >= θ₀, else 0°)
    // θₚ = LATPOLE (default = CRVAL2 for zenithal)
    let phi_p = lonpole.unwrap_or_else(|| {
        if crval2.unwrap_or(0.0) >= 90.0 {
            0.0
        } else {
            180.0
        }
    });
    let theta_p = latpole.unwrap_or_else(|| crval2.unwrap_or(0.0));
    properties.push(XisfProperty::f64_vector(
        "PCL:AstrometricSolution:CelestialPoleNativeCoordinates",
        vec![phi_p, theta_p],
    ));

    properties
}

pub(crate) fn extract_projection_code(ctype: &str) -> Option<String> {
    let trimmed = ctype.trim();
    if trimmed.len() < 8 {
        return None;
    }
    let dashes_pos = trimmed.find('-')?;
    let after_dashes = &trimmed[dashes_pos..];
    let code_start = after_dashes.trim_start_matches('-');
    if code_start.is_empty() {
        return None;
    }
    Some(wcs_code_to_pixinsight_name(code_start))
}

pub(crate) fn wcs_code_to_pixinsight_name(code: &str) -> String {
    match code {
        "TAN" => "Gnomonic",
        "SIN" => "Orthographic",
        "STG" => "Stereographic",
        "ARC" => "ZenithalEqualArea",
        "ZEA" => "ZenithalEqualArea",
        "AZP" => "ZenithalPerspective",
        "SZP" => "SlantZenithalPerspective",
        "ZPN" => "ZenithalPolynomial",
        "AIR" => "Airy",
        "CYP" => "CylindricalPerspective",
        "CEA" => "CylindricalEqualArea",
        "CAR" => "PlateCarree",
        "MER" => "Mercator",
        "SFL" => "SansonFlamsteed",
        "PAR" => "Parabolic",
        "MOL" => "Mollweide",
        "AIT" => "HammerAitoff",
        "COP" => "ConicPerspective",
        "COE" => "ConicEqualArea",
        "COD" => "ConicEquidistant",
        "COO" => "ConicOrthomorphic",
        other => return other.to_string(),
    }
    .to_string()
}

