//! The twelve zodiac signs and helpers for decomposing an ecliptic
//! longitude into (sign, degree, minute, second).
//!
//! The astrology layer supports both **tropical** (longitude measured
//! from the vernal equinox of date — the default in Western astrology)
//! and **sidereal** (longitude minus an ayanamsha — the default in
//! Indian astrology). The signs themselves are identical 30°-wide
//! sectors of the ecliptic; what changes between the two zodiacs is
//! the zero point.

use cosmos_sky::Ayanamsha;

/// The twelve zodiac signs, in chart order starting from Aries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sign {
    Aries,
    Taurus,
    Gemini,
    Cancer,
    Leo,
    Virgo,
    Libra,
    Scorpio,
    Sagittarius,
    Capricorn,
    Aquarius,
    Pisces,
}

impl Sign {
    /// Sign index `0..=11` (Aries = 0).
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> Self {
        const ALL: [Sign; 12] = [
            Sign::Aries,
            Sign::Taurus,
            Sign::Gemini,
            Sign::Cancer,
            Sign::Leo,
            Sign::Virgo,
            Sign::Libra,
            Sign::Scorpio,
            Sign::Sagittarius,
            Sign::Capricorn,
            Sign::Aquarius,
            Sign::Pisces,
        ];
        ALL[i % 12]
    }

    /// Decompose a (already-normalised) ecliptic longitude in radians
    /// into the sign and the offset within that sign, in radians
    /// `[0, π/6)`.
    pub fn decompose(longitude_rad: f64) -> (Self, f64) {
        const TAU: f64 = std::f64::consts::TAU;
        const SIGN_WIDTH: f64 = TAU / 12.0;
        let lon = longitude_rad.rem_euclid(TAU);
        let index = (lon / SIGN_WIDTH).floor() as usize;
        let offset = lon - (index as f64) * SIGN_WIDTH;
        (Self::from_index(index), offset)
    }

    /// English short name (`"Ari"`, `"Tau"`, ...). Useful for compact
    /// chart printouts.
    pub fn short_name(self) -> &'static str {
        match self {
            Sign::Aries => "Ari",
            Sign::Taurus => "Tau",
            Sign::Gemini => "Gem",
            Sign::Cancer => "Can",
            Sign::Leo => "Leo",
            Sign::Virgo => "Vir",
            Sign::Libra => "Lib",
            Sign::Scorpio => "Sco",
            Sign::Sagittarius => "Sag",
            Sign::Capricorn => "Cap",
            Sign::Aquarius => "Aqu",
            Sign::Pisces => "Pis",
        }
    }
}

/// Selectable zodiac reference. `Tropical` measures longitudes from the
/// vernal equinox of date; `Sidereal` subtracts an ayanamsha so the
/// constellations stay fixed relative to the background stars.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zodiac {
    Tropical,
    Sidereal(Ayanamsha),
}

impl Default for Zodiac {
    fn default() -> Self {
        Zodiac::Tropical
    }
}

/// An ecliptic longitude paired with its zodiac decomposition. The same
/// underlying radian value drives all accessors; the helpers exist for
/// human-readable chart output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SignedLongitude {
    longitude_rad: f64,
    sign: Sign,
    offset_rad: f64,
}

impl SignedLongitude {
    /// Build from a (possibly un-normalised) ecliptic longitude in radians.
    pub fn from_radians(longitude_rad: f64) -> Self {
        let (sign, offset_rad) = Sign::decompose(longitude_rad);
        Self {
            longitude_rad: longitude_rad.rem_euclid(std::f64::consts::TAU),
            sign,
            offset_rad,
        }
    }

    pub fn longitude_rad(&self) -> f64 {
        self.longitude_rad
    }

    pub fn longitude_deg(&self) -> f64 {
        self.longitude_rad.to_degrees()
    }

    pub fn sign(&self) -> Sign {
        self.sign
    }

    /// Whole degree within the sign (`0..30`).
    pub fn degree_in_sign(&self) -> u32 {
        self.offset_rad.to_degrees().floor() as u32
    }

    /// Decimal degree within the sign (`0.0..30.0`).
    pub fn degree_in_sign_decimal(&self) -> f64 {
        self.offset_rad.to_degrees()
    }

    /// Whole minutes after the degree (`0..60`).
    pub fn minutes_in_sign(&self) -> u32 {
        let frac = (self.offset_rad.to_degrees().fract()) * 60.0;
        frac.floor() as u32
    }

    /// Seconds after the minute, with fractional part (`0.0..60.0`).
    pub fn seconds_in_sign(&self) -> f64 {
        let total_min = self.offset_rad.to_degrees().fract() * 60.0;
        (total_min.fract()) * 60.0
    }

    /// Human-readable like `"15°23'04\" Tau"`.
    pub fn to_chart_format(&self) -> String {
        format!(
            "{:02}°{:02}'{:05.2}\" {}",
            self.degree_in_sign(),
            self.minutes_in_sign(),
            self.seconds_in_sign(),
            self.sign.short_name(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decompose_aries_zero() {
        let s = SignedLongitude::from_radians(0.0);
        assert_eq!(s.sign(), Sign::Aries);
        assert_eq!(s.degree_in_sign(), 0);
    }

    #[test]
    fn decompose_15_taurus() {
        let lon = (30.0_f64 + 15.0).to_radians();
        let s = SignedLongitude::from_radians(lon);
        assert_eq!(s.sign(), Sign::Taurus);
        assert_eq!(s.degree_in_sign(), 15);
    }

    #[test]
    fn decompose_29_pisces() {
        let lon = (359.99_f64).to_radians();
        let s = SignedLongitude::from_radians(lon);
        assert_eq!(s.sign(), Sign::Pisces);
        assert_eq!(s.degree_in_sign(), 29);
    }
}
