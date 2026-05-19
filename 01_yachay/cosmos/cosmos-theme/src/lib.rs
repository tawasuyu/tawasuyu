//! `cosmobiologia-theme` — paleta simbólica + presets místicos.
//!
//! Una capa fina sobre [`yahweh_theme::Theme`]: el theme base aporta los
//! slots de panel/foreground/accent; nosotros agregamos paletas
//! semánticas para los elementos (fuego/tierra/aire/agua), los modos
//! (cardinal/fijo/mutable), los planetas y los aspectos.
//!
//! El canvas pide colores por símbolo (`palette.element(Element::Fire)`),
//! nunca hex directos. Así una sola tabla controla tanto el dark como el
//! light, y cambiar la paleta no requiere tocar el render.

#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

use gpui::{Hsla, hsla};

// =====================================================================
// Símbolos
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Element {
    Fire,
    Earth,
    Air,
    Water,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Modality {
    Cardinal,
    Fixed,
    Mutable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Planet {
    Sun,
    Moon,
    Mercury,
    Venus,
    Mars,
    Jupiter,
    Saturn,
    Uranus,
    Neptune,
    Pluto,
    Chiron,
    NorthNode,
    SouthNode,
    Lilith,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AspectKind {
    Conjunction,
    Sextile,
    Square,
    Trine,
    Opposition,
    Quincunx,
    Semisextile,
    Semisquare,
    Sesquisquare,
    Quintile,
    Biquintile,
}

// =====================================================================
// Paleta
// =====================================================================

/// Paleta completa de símbolos astrológicos resuelta a colores HSLA. Las
/// dos variantes (`dark` / `light`) comparten estructura — el canvas
/// elige según `yahweh_theme::Theme::is_dark`.
#[derive(Debug, Clone)]
pub struct AstroPalette {
    pub is_dark: bool,

    pub fire: Hsla,
    pub earth: Hsla,
    pub air: Hsla,
    pub water: Hsla,

    pub cardinal: Hsla,
    pub fixed: Hsla,
    pub mutable: Hsla,

    pub sun: Hsla,
    pub moon: Hsla,
    pub mercury: Hsla,
    pub venus: Hsla,
    pub mars: Hsla,
    pub jupiter: Hsla,
    pub saturn: Hsla,
    pub uranus: Hsla,
    pub neptune: Hsla,
    pub pluto: Hsla,
    pub chiron: Hsla,
    pub north_node: Hsla,
    pub south_node: Hsla,
    pub lilith: Hsla,

    pub conjunction: Hsla,
    pub sextile: Hsla,
    pub square: Hsla,
    pub trine: Hsla,
    pub opposition: Hsla,
    pub minor_aspect: Hsla,

    /// Color del dial zodiacal (anillo exterior).
    pub dial_ring: Hsla,
    /// Cusps de casas.
    pub house_cusp: Hsla,
    /// Resaltado del ascendente / MC.
    pub angle_highlight: Hsla,
}

impl AstroPalette {
    /// Variante oscura — calibrada para sentirse cálida y mística sin
    /// caer en saturación de carnaval. Las cusps quedan apenas más
    /// claras que el fondo, los planetas tienen luminancia media-alta
    /// para destacar sin glow falso.
    pub fn dark() -> Self {
        Self {
            is_dark: true,

            // Elementos — saturación alta + luminancia media. Familiares
            // al símbolo pero suaves para coexistir.
            fire: hsla(11.0 / 360.0, 0.78, 0.58, 1.0),
            earth: hsla(95.0 / 360.0, 0.40, 0.48, 1.0),
            air: hsla(48.0 / 360.0, 0.72, 0.66, 1.0),
            water: hsla(210.0 / 360.0, 0.68, 0.58, 1.0),

            cardinal: hsla(340.0 / 360.0, 0.55, 0.62, 1.0),
            fixed: hsla(258.0 / 360.0, 0.48, 0.58, 1.0),
            mutable: hsla(170.0 / 360.0, 0.42, 0.55, 1.0),

            sun: hsla(45.0 / 360.0, 0.92, 0.62, 1.0),
            moon: hsla(220.0 / 360.0, 0.25, 0.85, 1.0),
            mercury: hsla(140.0 / 360.0, 0.40, 0.62, 1.0),
            venus: hsla(330.0 / 360.0, 0.55, 0.70, 1.0),
            mars: hsla(8.0 / 360.0, 0.78, 0.55, 1.0),
            jupiter: hsla(38.0 / 360.0, 0.72, 0.62, 1.0),
            saturn: hsla(28.0 / 360.0, 0.20, 0.50, 1.0),
            uranus: hsla(195.0 / 360.0, 0.65, 0.62, 1.0),
            neptune: hsla(225.0 / 360.0, 0.55, 0.66, 1.0),
            pluto: hsla(280.0 / 360.0, 0.40, 0.45, 1.0),
            chiron: hsla(75.0 / 360.0, 0.30, 0.55, 1.0),
            north_node: hsla(35.0 / 360.0, 0.35, 0.70, 1.0),
            south_node: hsla(35.0 / 360.0, 0.20, 0.45, 1.0),
            lilith: hsla(310.0 / 360.0, 0.45, 0.40, 1.0),

            conjunction: hsla(50.0 / 360.0, 0.65, 0.70, 0.85),
            sextile: hsla(195.0 / 360.0, 0.60, 0.62, 0.75),
            square: hsla(8.0 / 360.0, 0.75, 0.58, 0.85),
            trine: hsla(140.0 / 360.0, 0.55, 0.55, 0.80),
            opposition: hsla(280.0 / 360.0, 0.55, 0.62, 0.85),
            minor_aspect: hsla(220.0 / 360.0, 0.20, 0.55, 0.55),

            dial_ring: hsla(40.0 / 360.0, 0.18, 0.78, 0.85),
            house_cusp: hsla(40.0 / 360.0, 0.12, 0.55, 0.60),
            angle_highlight: hsla(50.0 / 360.0, 0.95, 0.65, 1.0),
        }
    }

    /// Variante clara — desaturada y con luminancias bajas para que los
    /// símbolos no compitan con el fondo blanco. Pensada para imprimir.
    pub fn light() -> Self {
        Self {
            is_dark: false,

            fire: hsla(11.0 / 360.0, 0.65, 0.42, 1.0),
            earth: hsla(95.0 / 360.0, 0.45, 0.30, 1.0),
            air: hsla(48.0 / 360.0, 0.55, 0.42, 1.0),
            water: hsla(210.0 / 360.0, 0.60, 0.38, 1.0),

            cardinal: hsla(340.0 / 360.0, 0.55, 0.42, 1.0),
            fixed: hsla(258.0 / 360.0, 0.45, 0.40, 1.0),
            mutable: hsla(170.0 / 360.0, 0.42, 0.35, 1.0),

            sun: hsla(38.0 / 360.0, 0.85, 0.45, 1.0),
            moon: hsla(220.0 / 360.0, 0.22, 0.45, 1.0),
            mercury: hsla(140.0 / 360.0, 0.45, 0.36, 1.0),
            venus: hsla(330.0 / 360.0, 0.55, 0.45, 1.0),
            mars: hsla(8.0 / 360.0, 0.75, 0.40, 1.0),
            jupiter: hsla(38.0 / 360.0, 0.72, 0.42, 1.0),
            saturn: hsla(28.0 / 360.0, 0.25, 0.30, 1.0),
            uranus: hsla(195.0 / 360.0, 0.65, 0.40, 1.0),
            neptune: hsla(225.0 / 360.0, 0.55, 0.42, 1.0),
            pluto: hsla(280.0 / 360.0, 0.45, 0.30, 1.0),
            chiron: hsla(75.0 / 360.0, 0.32, 0.35, 1.0),
            north_node: hsla(35.0 / 360.0, 0.45, 0.45, 1.0),
            south_node: hsla(35.0 / 360.0, 0.20, 0.30, 1.0),
            lilith: hsla(310.0 / 360.0, 0.50, 0.30, 1.0),

            // Aspectos en light: alpha alta y luminancia media-baja para
            // que las líneas tengan presencia contra fondo claro. En dark
            // las alphas pueden ser más bajas porque el contraste contra
            // el fondo oscuro ya las hace destacar.
            conjunction: hsla(45.0 / 360.0, 0.70, 0.38, 0.95),
            sextile: hsla(195.0 / 360.0, 0.65, 0.36, 0.90),
            square: hsla(8.0 / 360.0, 0.80, 0.38, 0.95),
            trine: hsla(140.0 / 360.0, 0.60, 0.32, 0.92),
            opposition: hsla(280.0 / 360.0, 0.60, 0.40, 0.95),
            minor_aspect: hsla(220.0 / 360.0, 0.30, 0.38, 0.75),

            // dial_ring: luminancia baja (oscuro sobre blanco) para que
            // el anillo de signos tenga peso. house_cusp: subimos alpha
            // y bajamos luminancia para que las cúspides no se laven en
            // un beige translúcido.
            dial_ring: hsla(40.0 / 360.0, 0.20, 0.28, 0.95),
            house_cusp: hsla(40.0 / 360.0, 0.15, 0.32, 0.80),
            angle_highlight: hsla(38.0 / 360.0, 0.90, 0.38, 1.0),
        }
    }

    /// Variante "papel coloreado" — para preview de impresión. Hue de
    /// cada slot mantenido; luminancia 0.26-0.34 y saturación alta
    /// para que sobreviva el ink-bleed sin perder identidad. Sin glow.
    pub fn print_color() -> Self {
        Self {
            is_dark: false,

            fire: hsla(11.0 / 360.0, 0.78, 0.34, 1.0),
            earth: hsla(95.0 / 360.0, 0.55, 0.26, 1.0),
            air: hsla(48.0 / 360.0, 0.78, 0.34, 1.0),
            water: hsla(210.0 / 360.0, 0.72, 0.32, 1.0),

            cardinal: hsla(340.0 / 360.0, 0.65, 0.34, 1.0),
            fixed: hsla(258.0 / 360.0, 0.55, 0.32, 1.0),
            mutable: hsla(170.0 / 360.0, 0.55, 0.28, 1.0),

            sun: hsla(35.0 / 360.0, 0.95, 0.34, 1.0),
            moon: hsla(220.0 / 360.0, 0.35, 0.34, 1.0),
            mercury: hsla(140.0 / 360.0, 0.55, 0.28, 1.0),
            venus: hsla(330.0 / 360.0, 0.65, 0.36, 1.0),
            mars: hsla(8.0 / 360.0, 0.85, 0.34, 1.0),
            jupiter: hsla(38.0 / 360.0, 0.85, 0.34, 1.0),
            saturn: hsla(28.0 / 360.0, 0.30, 0.26, 1.0),
            uranus: hsla(195.0 / 360.0, 0.75, 0.34, 1.0),
            neptune: hsla(225.0 / 360.0, 0.65, 0.34, 1.0),
            pluto: hsla(280.0 / 360.0, 0.55, 0.28, 1.0),
            chiron: hsla(75.0 / 360.0, 0.42, 0.30, 1.0),
            north_node: hsla(35.0 / 360.0, 0.55, 0.36, 1.0),
            south_node: hsla(35.0 / 360.0, 0.30, 0.28, 1.0),
            lilith: hsla(310.0 / 360.0, 0.60, 0.26, 1.0),

            conjunction: hsla(45.0 / 360.0, 0.75, 0.32, 1.0),
            sextile: hsla(195.0 / 360.0, 0.70, 0.32, 1.0),
            square: hsla(8.0 / 360.0, 0.85, 0.34, 1.0),
            trine: hsla(140.0 / 360.0, 0.65, 0.28, 1.0),
            opposition: hsla(280.0 / 360.0, 0.65, 0.36, 1.0),
            minor_aspect: hsla(220.0 / 360.0, 0.40, 0.40, 0.85),

            dial_ring: hsla(40.0 / 360.0, 0.30, 0.22, 1.0),
            house_cusp: hsla(40.0 / 360.0, 0.20, 0.28, 0.90),
            angle_highlight: hsla(15.0 / 360.0, 0.85, 0.36, 1.0),
        }
    }

    /// Variante "papel B&N" — preview de impresión monocromática.
    /// TODO los slots de planeta y aspecto se reducen a niveles de
    /// gris. El canvas se encarga de diferenciar aspectos por dash
    /// pattern y planetas por glyph (el unicode astronómico es
    /// distintivo aunque pierda color).
    pub fn print_bw() -> Self {
        // Tres niveles funcionales: muy oscuro (texto, glyphs
        // principales), medio (cusps, líneas), claro (fondos, minors).
        let ink_strong = hsla(0.0, 0.0, 0.10, 1.0);
        let ink_mid = hsla(0.0, 0.0, 0.30, 1.0);
        let ink_soft = hsla(0.0, 0.0, 0.50, 0.90);
        let ink_faint = hsla(0.0, 0.0, 0.55, 0.75);

        Self {
            is_dark: false,

            fire: ink_strong,
            earth: ink_strong,
            air: ink_strong,
            water: ink_strong,

            cardinal: ink_mid,
            fixed: ink_mid,
            mutable: ink_mid,

            // Planetas: todos en ink_strong para que los glyphs se
            // lean fuerte. El usuario distingue por el unicode
            // astronómico, no por hue.
            sun: ink_strong,
            moon: ink_strong,
            mercury: ink_strong,
            venus: ink_strong,
            mars: ink_strong,
            jupiter: ink_strong,
            saturn: ink_strong,
            uranus: ink_strong,
            neptune: ink_strong,
            pluto: ink_strong,
            chiron: ink_mid,
            north_node: ink_mid,
            south_node: ink_mid,
            lilith: ink_mid,

            // Aspectos: el color es uniforme; la diferenciación es por
            // dash pattern en el painter (square=dashed, trine=solid,
            // sextile=dotted, etc.). Acá solo damos el "intensity"
            // base que el painter modula.
            conjunction: ink_strong,
            sextile: ink_mid,
            square: ink_strong,
            trine: ink_mid,
            opposition: ink_strong,
            minor_aspect: ink_faint,

            dial_ring: ink_mid,
            house_cusp: ink_soft,
            angle_highlight: ink_strong,
        }
    }

    pub fn for_theme(theme: &yahweh_theme::Theme) -> Self {
        // Dispatcher por nombre para los themes "papel"; el resto cae
        // al binary dark/light según `is_dark`. Mantenemos el match
        // case-insensitive por defensa contra cambios de naming.
        match theme.name {
            "Print Color" => Self::print_color(),
            "Print B&W" => Self::print_bw(),
            _ if theme.is_dark => Self::dark(),
            _ => Self::light(),
        }
    }

    /// Devuelve `true` si la paleta es monocromática — los painters
    /// la usan para activar dash patterns en lugar de diferenciar
    /// aspectos por color.
    pub fn is_monochrome(&self) -> bool {
        // Heurística simple: si conjunction y square (que en color
        // siempre tienen hues distintos) tienen el mismo hue,
        // estamos en BW.
        (self.conjunction.h - self.square.h).abs() < 1e-3
    }

    pub fn element(&self, e: Element) -> Hsla {
        match e {
            Element::Fire => self.fire,
            Element::Earth => self.earth,
            Element::Air => self.air,
            Element::Water => self.water,
        }
    }

    pub fn modality(&self, m: Modality) -> Hsla {
        match m {
            Modality::Cardinal => self.cardinal,
            Modality::Fixed => self.fixed,
            Modality::Mutable => self.mutable,
        }
    }

    pub fn planet(&self, p: Planet) -> Hsla {
        match p {
            Planet::Sun => self.sun,
            Planet::Moon => self.moon,
            Planet::Mercury => self.mercury,
            Planet::Venus => self.venus,
            Planet::Mars => self.mars,
            Planet::Jupiter => self.jupiter,
            Planet::Saturn => self.saturn,
            Planet::Uranus => self.uranus,
            Planet::Neptune => self.neptune,
            Planet::Pluto => self.pluto,
            Planet::Chiron => self.chiron,
            Planet::NorthNode => self.north_node,
            Planet::SouthNode => self.south_node,
            Planet::Lilith => self.lilith,
        }
    }

    pub fn aspect(&self, a: AspectKind) -> Hsla {
        match a {
            AspectKind::Conjunction => self.conjunction,
            AspectKind::Sextile => self.sextile,
            AspectKind::Square => self.square,
            AspectKind::Trine => self.trine,
            AspectKind::Opposition => self.opposition,
            _ => self.minor_aspect,
        }
    }
}

/// Resuelve un símbolo zodiacal (string) a su elemento.
/// Ej. `"aries" → Fire`, `"taurus" → Earth`, …
pub fn element_for_sign(sign: &str) -> Option<Element> {
    Some(match sign.to_ascii_lowercase().as_str() {
        "aries" | "leo" | "sagittarius" => Element::Fire,
        "taurus" | "virgo" | "capricorn" => Element::Earth,
        "gemini" | "libra" | "aquarius" => Element::Air,
        "cancer" | "scorpio" | "pisces" => Element::Water,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn element_lookup() {
        assert_eq!(element_for_sign("aries"), Some(Element::Fire));
        assert_eq!(element_for_sign("CAPRICORN"), Some(Element::Earth));
        assert_eq!(element_for_sign("zod"), None);
    }

    #[test]
    fn palette_indexes() {
        let p = AstroPalette::dark();
        assert_eq!(p.planet(Planet::Sun), p.sun);
        assert_eq!(p.element(Element::Water), p.water);
    }
}
