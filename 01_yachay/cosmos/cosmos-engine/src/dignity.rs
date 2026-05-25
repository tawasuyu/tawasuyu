//! Dignidades esenciales clásicas — tabla data-only.
//!
//! Cada planeta tradicional tiene cuatro estatus posibles según el
//! signo en el que cae:
//!
//! - **Domicilio** (rulership) — el signo del que es regente.
//! - **Exaltación** — un signo "huésped" que le da fuerza extra.
//! - **Exilio** (detriment) — opuesto al domicilio, debilita.
//! - **Caída** (fall) — opuesto a la exaltación, debilita.
//!
//! Esta tabla usa las regencias **clásicas** (Aries=Marte, Escorpio=
//! Marte, Acuario=Saturno, Piscis=Júpiter) — los planetas modernos
//! (Urano/Neptuno/Plutón) no tienen regencia clásica por convención.
//! En una fase futura podemos exponer un toggle "regencias modernas"
//! que mapee Escorpio→Plutón, Acuario→Urano, Piscis→Neptuno.

use cosmos_sky::Body;

/// Status de dignidad esencial de un cuerpo en un signo dado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dignity {
    /// Domicilio. Marker `"+"`.
    Rulership,
    /// Exaltación. Marker `"·"`.
    Exaltation,
    /// Exilio. Marker `"−"`.
    Detriment,
    /// Caída. Marker `"*"`.
    Fall,
}

impl Dignity {
    pub fn marker(self) -> &'static str {
        match self {
            Dignity::Rulership => "+",
            Dignity::Exaltation => "·",
            Dignity::Detriment => "−",
            Dignity::Fall => "*",
        }
    }
}

/// Devuelve el status de dignidad de `body` en `sign_index` (0..12,
/// Aries=0) o `None` si no aplica (sin dignidad / cuerpo moderno sin
/// regencia clásica).
pub fn essential_dignity(body: Body, sign_index: u8) -> Option<Dignity> {
    let sign = sign_index % 12;
    let opposite = (sign + 6) % 12;

    // Rulership clásico — el "regente" del signo.
    if rules_classical(body, sign) {
        return Some(Dignity::Rulership);
    }
    // Detriment = el cuerpo gobierna el signo opuesto.
    if rules_classical(body, opposite) {
        return Some(Dignity::Detriment);
    }
    // Exaltación tabular.
    if exalts_at(body) == Some(sign) {
        return Some(Dignity::Exaltation);
    }
    // Caída = opuesto a la exaltación.
    if exalts_at(body) == Some(opposite) {
        return Some(Dignity::Fall);
    }
    None
}

/// Devuelve true si `body` gobierna `sign` (0=Aries..11=Pisces) en el
/// esquema clásico de 7 planetas.
fn rules_classical(body: Body, sign: u8) -> bool {
    match (body, sign) {
        // Sol: Leo (4)
        (Body::Sun, 4) => true,
        // Luna: Cancer (3)
        (Body::Moon, 3) => true,
        // Mercurio: Gemini (2), Virgo (5)
        (Body::Mercury, 2) | (Body::Mercury, 5) => true,
        // Venus: Taurus (1), Libra (6)
        (Body::Venus, 1) | (Body::Venus, 6) => true,
        // Marte: Aries (0), Scorpio (7)
        (Body::Mars, 0) | (Body::Mars, 7) => true,
        // Júpiter: Sagittarius (8), Pisces (11)
        (Body::Jupiter, 8) | (Body::Jupiter, 11) => true,
        // Saturno: Capricorn (9), Aquarius (10)
        (Body::Saturn, 9) | (Body::Saturn, 10) => true,
        _ => false,
    }
}

/// Devuelve el signo (0..12) donde el cuerpo exalta, o `None` si no
/// tiene exaltación clásica documentada.
fn exalts_at(body: Body) -> Option<u8> {
    Some(match body {
        Body::Sun => 0,       // Aries
        Body::Moon => 1,      // Taurus
        Body::Mercury => 5,   // Virgo (algunas tradiciones la ponen acá)
        Body::Venus => 11,    // Pisces
        Body::Mars => 9,      // Capricorn
        Body::Jupiter => 3,   // Cancer
        Body::Saturn => 6,    // Libra
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rulership_examples() {
        assert_eq!(essential_dignity(Body::Sun, 4), Some(Dignity::Rulership)); // Sol en Leo
        assert_eq!(essential_dignity(Body::Moon, 3), Some(Dignity::Rulership)); // Luna en Cancer
        assert_eq!(essential_dignity(Body::Mars, 7), Some(Dignity::Rulership)); // Marte en Scorpio
    }

    #[test]
    fn detriment_examples() {
        assert_eq!(essential_dignity(Body::Sun, 10), Some(Dignity::Detriment)); // Sol en Acuario
        assert_eq!(essential_dignity(Body::Moon, 9), Some(Dignity::Detriment)); // Luna en Capricornio
    }

    #[test]
    fn exaltation_examples() {
        assert_eq!(essential_dignity(Body::Sun, 0), Some(Dignity::Exaltation)); // Sol en Aries
        assert_eq!(essential_dignity(Body::Saturn, 6), Some(Dignity::Exaltation)); // Saturno en Libra
    }

    #[test]
    fn fall_examples() {
        assert_eq!(essential_dignity(Body::Sun, 6), Some(Dignity::Fall)); // Sol en Libra
        assert_eq!(essential_dignity(Body::Saturn, 0), Some(Dignity::Fall)); // Saturno en Aries
    }

    #[test]
    fn modern_planets_no_classical_dignity() {
        assert_eq!(essential_dignity(Body::Uranus, 10), None);
        assert_eq!(essential_dignity(Body::Neptune, 11), None);
        assert_eq!(essential_dignity(Body::Pluto, 7), None);
    }
}
