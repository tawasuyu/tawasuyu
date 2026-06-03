//! Paleta astrológica agnóstica (`Rgba`, sin tipos de UI nativa).
//! Replica los slots de `cosmos_app-theme::AstroPalette` con `dark()`
//! y `light()` para que canvas Llimphi y cliente WASM compartan los
//! mismos colores sin arrastrar deps de UI.

use crate::draw::Rgba;

/// Color en HSL `[0..1]^4`, helper local para construir la palette
/// con la misma convención que las paletas anteriores.
fn hsla(h_deg: f32, s: f32, l: f32, a: f32) -> Rgba {
    // Conversión HSL → RGB (algoritmo estándar). H en grados.
    let h = h_deg / 360.0;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h6 = (h * 6.0).rem_euclid(6.0);
    let x = c * (1.0 - (h6 % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match h6 as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    Rgba {
        r: (r1 + m).clamp(0.0, 1.0),
        g: (g1 + m).clamp(0.0, 1.0),
        b: (b1 + m).clamp(0.0, 1.0),
        a,
    }
}

/// Paleta astrológica completa. Mismos slots que el theme nativo —
/// permite que el cliente WASM y el canvas Llimphi generen las mismas
/// decisiones de color en su superficie.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub is_dark: bool,
    // Elementos
    pub fire: Rgba,
    pub earth: Rgba,
    pub air: Rgba,
    pub water: Rgba,
    // Planetas
    pub sun: Rgba,
    pub moon: Rgba,
    pub mercury: Rgba,
    pub venus: Rgba,
    pub mars: Rgba,
    pub jupiter: Rgba,
    pub saturn: Rgba,
    pub uranus: Rgba,
    pub neptune: Rgba,
    pub pluto: Rgba,
    pub chiron: Rgba,
    pub north_node: Rgba,
    pub south_node: Rgba,
    pub lilith: Rgba,
    // Aspectos
    pub conjunction: Rgba,
    pub sextile: Rgba,
    pub square: Rgba,
    pub trine: Rgba,
    pub opposition: Rgba,
    pub minor_aspect: Rgba,
    // Estructura
    pub dial_ring: Rgba,
    pub house_cusp: Rgba,
    pub angle_highlight: Rgba,
    // Estructura del lienzo (background panel / texto)
    pub bg_panel: Rgba,
    pub fg_text: Rgba,
    pub fg_muted: Rgba,
}

impl Palette {
    /// Paleta dark — equivalente a `AstroPalette::dark()` del theme
    /// nativo.
    pub fn dark() -> Self {
        Self {
            is_dark: true,
            fire: hsla(11.0, 0.78, 0.58, 1.0),
            earth: hsla(95.0, 0.40, 0.48, 1.0),
            air: hsla(48.0, 0.72, 0.66, 1.0),
            water: hsla(210.0, 0.68, 0.58, 1.0),
            sun: hsla(45.0, 0.92, 0.62, 1.0),
            moon: hsla(220.0, 0.25, 0.85, 1.0),
            mercury: hsla(140.0, 0.40, 0.62, 1.0),
            venus: hsla(330.0, 0.55, 0.70, 1.0),
            mars: hsla(8.0, 0.78, 0.55, 1.0),
            jupiter: hsla(38.0, 0.72, 0.62, 1.0),
            saturn: hsla(28.0, 0.20, 0.50, 1.0),
            uranus: hsla(195.0, 0.65, 0.62, 1.0),
            neptune: hsla(225.0, 0.55, 0.66, 1.0),
            pluto: hsla(280.0, 0.40, 0.45, 1.0),
            chiron: hsla(75.0, 0.30, 0.55, 1.0),
            north_node: hsla(35.0, 0.35, 0.70, 1.0),
            south_node: hsla(35.0, 0.20, 0.45, 1.0),
            lilith: hsla(310.0, 0.45, 0.40, 1.0),
            conjunction: hsla(50.0, 0.65, 0.70, 0.85),
            sextile: hsla(195.0, 0.60, 0.62, 0.75),
            square: hsla(8.0, 0.75, 0.58, 0.85),
            trine: hsla(140.0, 0.55, 0.55, 0.80),
            opposition: hsla(280.0, 0.55, 0.62, 0.85),
            minor_aspect: hsla(220.0, 0.20, 0.55, 0.55),
            dial_ring: hsla(40.0, 0.18, 0.78, 0.85),
            house_cusp: hsla(40.0, 0.12, 0.55, 0.60),
            angle_highlight: hsla(50.0, 0.95, 0.65, 1.0),
            bg_panel: hsla(245.0, 0.28, 0.10, 1.0),
            fg_text: hsla(210.0, 0.35, 0.88, 1.0),
            fg_muted: hsla(215.0, 0.22, 0.58, 1.0),
        }
    }

    /// Paleta light — análoga a `AstroPalette::light()`.
    pub fn light() -> Self {
        Self {
            is_dark: false,
            fire: hsla(11.0, 0.65, 0.42, 1.0),
            earth: hsla(95.0, 0.45, 0.30, 1.0),
            air: hsla(48.0, 0.55, 0.42, 1.0),
            water: hsla(210.0, 0.60, 0.38, 1.0),
            sun: hsla(38.0, 0.85, 0.45, 1.0),
            moon: hsla(220.0, 0.22, 0.45, 1.0),
            mercury: hsla(140.0, 0.45, 0.36, 1.0),
            venus: hsla(330.0, 0.55, 0.45, 1.0),
            mars: hsla(8.0, 0.75, 0.40, 1.0),
            jupiter: hsla(38.0, 0.72, 0.42, 1.0),
            saturn: hsla(28.0, 0.25, 0.30, 1.0),
            uranus: hsla(195.0, 0.65, 0.40, 1.0),
            neptune: hsla(225.0, 0.55, 0.42, 1.0),
            pluto: hsla(280.0, 0.45, 0.30, 1.0),
            chiron: hsla(75.0, 0.32, 0.35, 1.0),
            north_node: hsla(35.0, 0.45, 0.45, 1.0),
            south_node: hsla(35.0, 0.20, 0.30, 1.0),
            lilith: hsla(310.0, 0.50, 0.30, 1.0),
            conjunction: hsla(45.0, 0.70, 0.38, 0.95),
            sextile: hsla(195.0, 0.65, 0.36, 0.90),
            square: hsla(8.0, 0.80, 0.38, 0.95),
            trine: hsla(140.0, 0.60, 0.32, 0.92),
            opposition: hsla(280.0, 0.60, 0.40, 0.95),
            minor_aspect: hsla(220.0, 0.30, 0.38, 0.75),
            dial_ring: hsla(40.0, 0.20, 0.28, 0.95),
            house_cusp: hsla(40.0, 0.15, 0.32, 0.80),
            angle_highlight: hsla(38.0, 0.90, 0.38, 1.0),
            bg_panel: hsla(40.0, 0.25, 0.97, 1.0),
            fg_text: hsla(30.0, 0.15, 0.18, 1.0),
            fg_muted: hsla(30.0, 0.12, 0.40, 1.0),
        }
    }

    /// Color del planeta por su id simbólico (`"sun"`, `"moon"`, …).
    pub fn planet(&self, sym: &str) -> Rgba {
        match sym {
            "sun" => self.sun,
            "moon" => self.moon,
            "mercury" => self.mercury,
            "venus" => self.venus,
            "mars" => self.mars,
            "jupiter" => self.jupiter,
            "saturn" => self.saturn,
            "uranus" => self.uranus,
            "neptune" => self.neptune,
            "pluto" => self.pluto,
            "chiron" => self.chiron,
            "north_node" => self.north_node,
            "south_node" => self.south_node,
            "lilith" => self.lilith,
            _ => self.fg_muted,
        }
    }

    /// Color del aspecto por su kind.
    pub fn aspect(&self, kind: &str) -> Rgba {
        match kind {
            "conjunction" => self.conjunction,
            "sextile" => self.sextile,
            "square" => self.square,
            "trine" => self.trine,
            "opposition" => self.opposition,
            _ => self.minor_aspect,
        }
    }

    /// Color tradicional del signo zodiacal — su **color del lore**, no el
    /// del elemento. Cada signo lleva su correspondencia clásica (Aries
    /// rojo, Tauro verde, Géminis amarillo, Cáncer plata, Leo oro, Virgo
    /// añil-pizarra, Libra rosa, Escorpio carmesí, Sagitario púrpura,
    /// Capricornio tierra, Acuario azul eléctrico, Piscis verde-mar). La
    /// claridad se adapta al tema para que se lea sobre fondo claro u
    /// oscuro; el matiz es el mismo.
    pub fn sign(&self, sym: &str) -> Rgba {
        // (hue°, saturación, L oscuro, L claro)
        let (h, s, ld, ll) = match sym {
            "aries" => (2.0, 0.78, 0.60, 0.46),
            "taurus" => (128.0, 0.45, 0.52, 0.36),
            "gemini" => (50.0, 0.80, 0.62, 0.46),
            "cancer" => (205.0, 0.20, 0.78, 0.52),
            "leo" => (38.0, 0.88, 0.60, 0.46),
            "virgo" => (235.0, 0.24, 0.60, 0.42),
            "libra" => (330.0, 0.55, 0.72, 0.55),
            "scorpio" => (348.0, 0.62, 0.50, 0.38),
            "sagittarius" => (275.0, 0.55, 0.64, 0.48),
            "capricorn" => (24.0, 0.42, 0.44, 0.32),
            "aquarius" => (195.0, 0.74, 0.62, 0.46),
            "pisces" => (165.0, 0.50, 0.58, 0.42),
            _ => return self.fg_muted,
        };
        hsla(h, s, if self.is_dark { ld } else { ll }, 1.0)
    }

    /// Ids zodiacales en orden natural (Aries=0 … Piscis=11).
    pub const ZODIAC: [&'static str; 12] = [
        "aries", "taurus", "gemini", "cancer", "leo", "virgo", "libra",
        "scorpio", "sagittarius", "capricorn", "aquarius", "pisces",
    ];

    /// Color del lore de una **casa** por su número (0 = Casa I … 11 =
    /// Casa XII). La casa toma el color de su signo natural: la Casa I
    /// es la casa de Aries, la II la de Tauro, etc. — la correspondencia
    /// rueda-zodíaco del lore. Da a cada casa una identidad estable,
    /// distinta de qué signo cae sobre su cúspide en una carta dada.
    pub fn house(&self, idx: usize) -> Rgba {
        self.sign(Self::ZODIAC[idx % 12])
    }

    /// Color del anillo de casas (sistema ascensional Polich-Page).
    /// Hue-shift de 140° respecto a `house_cusp` para diferenciar
    /// del dial zodiacal — replica el shift que hace el canvas
    /// nativo via `house_ring_color`.
    pub fn house_ring(&self) -> Rgba {
        // Aproximación rápida en HSL: rotamos el hue por 140°
        // manteniendo aspecto similar.
        let base = self.house_cusp;
        // Conversión RGB → HSL → shift → RGB. Para no agregar
        // dependencias, lo aproximamos con un mix simple hacia el
        // verde/teal de la palette base.
        let target = if self.is_dark {
            hsla(170.0, 0.30, 0.55, base.a)
        } else {
            hsla(170.0, 0.40, 0.32, base.a)
        };
        target
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::dark()
    }
}
