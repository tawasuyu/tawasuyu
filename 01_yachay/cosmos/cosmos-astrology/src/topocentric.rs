//! Corrección topocéntrica de posiciones planetarias.
//!
//! Las posiciones que entrega VSOP2013 (y por extensión `Placement`)
//! son **geocéntricas** — referidas al centro de la Tierra. El
//! observador real está en la superficie, desplazado del centro por
//! ~6378 km. La diferencia produce una **paralaje horizontal** que
//! desplaza la posición aparente del cuerpo, máxima para la Luna
//! (~1°), modesta para los planetas interiores (~30″ en Marte cerca
//! de oposición) y despreciable para los exteriores.
//!
//! En la práctica astrológica, el sistema topocéntrico es relevante
//! para:
//! - Lecturas precisas de la Luna (la diferencia es visible a simple
//!   vista en la rueda).
//! - Trabajos de rectificación con Direcciones Primarias del sistema
//!   GR / García Rosas, donde la paralaje cambia el resultado.
//! - Sinastrías comparativas (geocéntrico vs topocéntrico).
//!
//! Referencia: Meeus, *Astronomical Algorithms*, cap. 40 ("Correction
//! for Parallax"), ec. 40.6-40.7.
//!
//! ## Simplificaciones
//!
//! Tratamos la Tierra como esfera (sin flattening 1/298.257). Eso
//! introduce un error de ~10″ en latitudes medias — orden de
//! magnitud menor que la propia paralaje y aceptable para uso
//! astrológico. Si el caller necesita precisión sub-arc-second
//! debe usar el módulo Swiss Ephemeris directamente.

use std::f64::consts::TAU;

/// Paralaje solar standard (Meeus 40.1, en radianes): el ángulo que
/// subtiende el radio terrestre visto desde 1 AU. Equivale a 8.794″.
const SOLAR_PARALLAX_RAD: f64 = 4.263_452_25e-5;

/// Convierte una posición eclíptica geocéntrica a topocéntrica para
/// un observador dado. La conversión pasa por coordenadas
/// ecuatoriales (RA/Dec), aplica la paralaje en ese frame
/// (donde la geometría es separable en `Δα` y `Δδ` cleanly), y
/// vuelve a eclípticas.
///
/// Parámetros:
/// * `lon_rad`, `lat_rad`: longitud y latitud eclípticas geocéntricas.
/// * `dist_au`: distancia geocéntrica al cuerpo, en AU. Para cuerpos
///   con `dist_au > 50` (más allá de Plutón) el shift es < 10⁻⁶ rad
///   y se devuelve la entrada sin tocar.
/// * `obs_lat_rad`: latitud geográfica del observador.
/// * `lst_rad`: Local Apparent Sidereal Time del observador.
/// * `obliquity_rad`: obliquidad verdadera de la fecha.
///
/// Devuelve `(lon_topo_rad, lat_topo_rad)` con `lon_topo_rad ∈
/// [0, 2π)`.
pub fn topocentric_ecliptic(
    lon_rad: f64,
    lat_rad: f64,
    dist_au: f64,
    obs_lat_rad: f64,
    lst_rad: f64,
    obliquity_rad: f64,
) -> (f64, f64) {
    // Cuerpos muy lejanos: la paralaje es indistinguible numéricamente
    // de cero y devolver la geocéntrica evita ruido floating-point.
    if dist_au <= 0.0 || dist_au > 50.0 {
        return (lon_rad.rem_euclid(TAU), lat_rad);
    }

    // 1) Eclíptico → ecuatorial.
    let (ra, dec) = ecliptic_to_equatorial(lon_rad, lat_rad, obliquity_rad);

    // 2) Paralaje horizontal sin π = sin(8.794″) / dist_au. Para
    //    distancias > 0.0001 AU (≈15000 km) el seno es indistinguible
    //    del argumento; usamos la aproximación de ángulo pequeño.
    let sin_pi = SOLAR_PARALLAX_RAD / dist_au;

    // 3) Hour angle del cuerpo (H = LST - α).
    let h = lst_rad - ra;
    let (sin_h, cos_h) = libm::sincos(h);

    // 4) Componentes del observador (esfera, ρ=1, alt despreciable).
    let (sin_phi, cos_phi) = libm::sincos(obs_lat_rad);
    let rho_cos_phi = cos_phi;
    let rho_sin_phi = sin_phi;

    // 5) Δα y δ' según Meeus 40.6-40.7.
    let (sin_dec, cos_dec) = libm::sincos(dec);
    let denom = cos_dec - rho_cos_phi * sin_pi * cos_h;
    let delta_alpha = libm::atan2(-rho_cos_phi * sin_pi * sin_h, denom);
    let ra_topo = ra + delta_alpha;
    let dec_topo = libm::atan2(
        (sin_dec - rho_sin_phi * sin_pi) * libm::cos(delta_alpha),
        denom,
    );

    // 6) Ecuatorial topocéntrico → eclíptico topocéntrico.
    let (lon_topo, lat_topo) = equatorial_to_ecliptic(ra_topo, dec_topo, obliquity_rad);
    (lon_topo.rem_euclid(TAU), lat_topo)
}

/// Eclíptico → ecuatorial. (RA, Dec) en radianes; RA en [0, 2π).
fn ecliptic_to_equatorial(lon: f64, lat: f64, obliquity: f64) -> (f64, f64) {
    let (sin_lon, cos_lon) = libm::sincos(lon);
    let (sin_lat, cos_lat) = libm::sincos(lat);
    let (sin_eps, cos_eps) = libm::sincos(obliquity);
    let sin_dec = sin_lat * cos_eps + cos_lat * sin_eps * sin_lon;
    let dec = libm::asin(sin_dec);
    let ra = libm::atan2(sin_lon * cos_eps - libm::tan(lat) * sin_eps, cos_lon);
    let ra = ra.rem_euclid(TAU);
    (ra, dec)
}

/// Ecuatorial → eclíptico. (λ, β) en radianes; λ en [0, 2π).
fn equatorial_to_ecliptic(ra: f64, dec: f64, obliquity: f64) -> (f64, f64) {
    let (sin_ra, cos_ra) = libm::sincos(ra);
    let (sin_dec, cos_dec) = libm::sincos(dec);
    let (sin_eps, cos_eps) = libm::sincos(obliquity);
    let sin_beta = sin_dec * cos_eps - cos_dec * sin_eps * sin_ra;
    let beta = libm::asin(sin_beta);
    let lon = libm::atan2(
        sin_dec * sin_eps + cos_dec * cos_eps * sin_ra,
        cos_dec * cos_ra,
    );
    let lon = lon.rem_euclid(TAU);
    (lon, beta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn deg(r: f64) -> f64 {
        r.to_degrees()
    }

    #[test]
    fn distant_body_no_shift() {
        // Saturno ~9 AU: shift en arcsec ≈ 8.794" / 9 ≈ 1" — debería
        // estar bajo el error tolerable para un test relativo.
        let lon = 120.0_f64.to_radians();
        let lat = 0.5_f64.to_radians();
        let (lt, bt) = topocentric_ecliptic(
            lon,
            lat,
            9.0,                            // ~Saturno
            45.0_f64.to_radians(),
            60.0_f64.to_radians(),
            23.44_f64.to_radians(),
        );
        // < 5 arcsec de diferencia
        assert!(deg(lt - lon).abs() < 5.0 / 3600.0);
        assert!(deg(bt - lat).abs() < 5.0 / 3600.0);
    }

    #[test]
    fn very_distant_body_returns_unchanged() {
        // Pluto > 30 AU debería devolver exactamente la entrada
        // (short-circuit por threshold).
        let lon = 200.0_f64.to_radians();
        let lat = 0.7_f64.to_radians();
        let (lt, bt) = topocentric_ecliptic(
            lon,
            lat,
            32.0,
            40.0_f64.to_radians(),
            90.0_f64.to_radians(),
            23.44_f64.to_radians(),
        );
        // 32 AU sale del threshold de 50: aún se computa, pero el
        // shift es minúsculo. La diferencia tiene que ser < 1 arcsec.
        assert!(deg(lt - lon).abs() < 1.0 / 3600.0);
        assert!(deg(bt - lat).abs() < 1.0 / 3600.0);
    }

    #[test]
    fn moon_parallax_significant() {
        // Luna a ~60 radios terrestres = 0.00257 AU. La paralaje
        // horizontal es ~57'. El shift exacto depende del hour
        // angle y la latitud, pero debería estar en el orden de
        // arcmin, NUNCA cero, para una observación no-cenital.
        let lon = 120.0_f64.to_radians(); // Leo aprox.
        let lat = 0.0_f64.to_radians();
        let dist_au = 0.00257;
        let obs_lat = 45.0_f64.to_radians();
        let lst = 60.0_f64.to_radians(); // body NO en el meridiano
        let eps = 23.44_f64.to_radians();
        let (lt, _bt) = topocentric_ecliptic(lon, lat, dist_au, obs_lat, lst, eps);
        let shift_arcmin = deg(lt - lon).abs() * 60.0;
        // Esperamos shift entre 1' y 80' (rango amplio porque
        // depende mucho de la geometría exacta).
        assert!(
            (1.0..80.0).contains(&shift_arcmin),
            "shift Luna esperado en (1', 80'), fue {}'",
            shift_arcmin
        );
    }

    #[test]
    fn zenith_passage_no_shift() {
        // Si el cuerpo pasa por el cenit del observador (declinación
        // = latitud, hour angle = 0), la paralaje es exactamente
        // radial hacia abajo y no cambia la dirección angular.
        // Construimos: lon tal que ra=lst, lat=0 → δ = ε·sin(λ)·… ;
        // en lugar de invertir analíticamente, picamos un caso
        // simple: λ=0 (Aries 0°), β=0 → α=0, δ=0. Si lst=0 y obs_lat
        // = 0, el cuerpo está en el cenit. shift debe ser ~0.
        let (lt, bt) = topocentric_ecliptic(
            0.0,
            0.0,
            0.4, // distancia tipo Mercurio
            0.0_f64.to_radians(),
            0.0_f64.to_radians(),
            23.44_f64.to_radians(),
        );
        assert!(deg(lt).abs() < 0.001 || deg(lt - 360.0).abs() < 0.001);
        assert!(deg(bt).abs() < 0.001);
    }

    #[test]
    fn ecliptic_equatorial_round_trip() {
        let cases: [(f64, f64); 5] = [
            (0.0, 0.0),
            (90.0, 23.44),
            (120.0, -5.0),
            (270.0, 10.0),
            (359.9, -0.1),
        ];
        let eps = 23.44_f64.to_radians();
        for (lon_deg, lat_deg) in cases {
            let lon = lon_deg.to_radians();
            let lat = lat_deg.to_radians();
            let (ra, dec) = ecliptic_to_equatorial(lon, lat, eps);
            let (lon2, lat2) = equatorial_to_ecliptic(ra, dec, eps);
            // Roundtrip < 1 arcsec.
            let d_lon = ((lon - lon2 + PI).rem_euclid(2.0 * PI) - PI).abs();
            assert!(d_lon.to_degrees() * 3600.0 < 0.5, "lon {} → {}", lon_deg, lon2.to_degrees());
            assert!(
                ((lat - lat2).to_degrees() * 3600.0).abs() < 0.5,
                "lat {} → {}",
                lat_deg,
                lat2.to_degrees()
            );
        }
    }
}
