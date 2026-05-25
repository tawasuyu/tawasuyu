//! Parser ANSI mínimo para la salida en streaming de los subprocesos.
//!
//! Cubre lo que un pipe non-TTY recibe en el mundo real:
//!
//! - **SGR** (Select Graphic Rendition): `\x1b[<n>(;<m>)*m`. Colores
//!   16+8+8 (foreground/background 0..7, brillantes 60..67), atributos
//!   (bold/dim/italic/underline/reverse), reset (`\x1b[m` o `\x1b[0m`).
//! - **CR** (`\r`): "vuelve al inicio de la línea actual". Las
//!   herramientas que emiten progreso (cargo, claude, docker pull…)
//!   reescriben la misma línea con `\r<nuevo contenido>`. El parser
//!   colapsa lo anterior y emite sólo el último estado de la línea.
//!
//! NO cubre (por ahora):
//!
//! - Movimientos de cursor (`\x1b[H`, `\x1b[A/B/C/D`, etc.) — son
//!   propios de aplicaciones fullscreen tipo vim/htop, que necesitan
//!   PTY (los pipes no las recibirán). Se ignoran al ver `\x1b[` con
//!   un terminator que no es `m`.
//! - Borrado de pantalla / línea (`\x1b[J`, `\x1b[K`).
//! - OSC (títulos, hyperlinks).
//!
//! Esas funcionalidades caen en la Fase B (PTY + vt100 emulator).

use serde::{Deserialize, Serialize};

/// Atributos de estilo de un span. Todos son opcionales para que el
/// frontend sepa "no toques este aspecto" vs "fíjalo a este valor".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AnsiStyle {
    pub fg: Option<AnsiColor>,
    pub bg: Option<AnsiColor>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub reverse: bool,
}

impl AnsiStyle {
    /// `true` si todo está vacío — el frontend puede usarlo como
    /// "saltar a renderizado plano".
    pub fn is_plain(&self) -> bool {
        self.fg.is_none()
            && self.bg.is_none()
            && !self.bold
            && !self.dim
            && !self.italic
            && !self.underline
            && !self.reverse
    }
}

/// Los 16 colores ANSI estándar (8 normales + 8 brillantes). Los
/// frontends los mapean a sus propios valores HSL/Hex.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnsiColor {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
}

impl AnsiColor {
    fn from_fg_code(c: u32) -> Option<Self> {
        match c {
            30 => Some(Self::Black),
            31 => Some(Self::Red),
            32 => Some(Self::Green),
            33 => Some(Self::Yellow),
            34 => Some(Self::Blue),
            35 => Some(Self::Magenta),
            36 => Some(Self::Cyan),
            37 => Some(Self::White),
            90 => Some(Self::BrightBlack),
            91 => Some(Self::BrightRed),
            92 => Some(Self::BrightGreen),
            93 => Some(Self::BrightYellow),
            94 => Some(Self::BrightBlue),
            95 => Some(Self::BrightMagenta),
            96 => Some(Self::BrightCyan),
            97 => Some(Self::BrightWhite),
            _ => None,
        }
    }
    fn from_bg_code(c: u32) -> Option<Self> {
        Self::from_fg_code(c.saturating_sub(10))
    }
}

/// Un trozo de texto con un estilo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnsiSpan {
    pub text: String,
    pub style: AnsiStyle,
}

/// Parsea una línea de texto que puede contener secuencias ANSI y la
/// descompone en spans con su estilo.
///
/// Tratamiento de `\r`:
///
/// 1. Acumulamos spans en un buffer "lineal" (como si fuera una línea
///    de terminal).
/// 2. Al ver `\r`, "rebobinamos" el cursor al inicio de la línea — el
///    siguiente texto **sobreescribe** los chars previos columna a
///    columna; lo que sobre del estado anterior (si es más largo) se
///    conserva al final.
///
/// Devuelve los spans finales (después de aplicar todos los `\r`).
pub fn parse_ansi_line(input: &str) -> Vec<AnsiSpan> {
    // Línea conceptual: un Vec<(char, AnsiStyle)>. Tras todos los `\r`,
    // colapsamos en spans contiguos por estilo.
    let mut chars: Vec<(char, AnsiStyle)> = Vec::new();
    let mut col: usize = 0;
    let mut style = AnsiStyle::default();
    let mut it = input.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\x1b' && it.peek() == Some(&'[') {
            it.next(); // consume '['
            // Leer dígitos y `;` hasta el terminador (letra ASCII).
            let mut params = String::new();
            let mut terminator = None;
            for nc in it.by_ref() {
                if nc.is_ascii_alphabetic() {
                    terminator = Some(nc);
                    break;
                }
                params.push(nc);
            }
            if terminator == Some('m') {
                apply_sgr(&mut style, &params);
            }
            // Otros terminadores se ignoran — son cursor movement /
            // erase / etc., que en streaming pipe sin PTY no aplican.
            continue;
        }
        if c == '\r' {
            col = 0;
            continue;
        }
        if c == '\n' {
            // Una línea no debería traer `\n` (el shell entrega un
            // string por línea), pero por robustez lo tratamos como
            // separador: cortamos aquí.
            break;
        }
        if col < chars.len() {
            chars[col] = (c, style);
        } else {
            chars.push((c, style));
        }
        col += 1;
    }
    // Colapsar `chars` en spans por estilo contiguo.
    let mut out: Vec<AnsiSpan> = Vec::new();
    let mut cur: Option<(String, AnsiStyle)> = None;
    for (c, s) in chars {
        match &mut cur {
            Some((text, st)) if *st == s => text.push(c),
            _ => {
                if let Some((text, st)) = cur.take() {
                    out.push(AnsiSpan { text, style: st });
                }
                let mut t = String::new();
                t.push(c);
                cur = Some((t, s));
            }
        }
    }
    if let Some((text, style)) = cur {
        out.push(AnsiSpan { text, style });
    }
    out
}

/// Devuelve el texto plano (sin estilos) de una línea con secuencias
/// ANSI. Útil para historial / búsqueda fuzzy / persistir sin colores.
pub fn strip_ansi(input: &str) -> String {
    parse_ansi_line(input)
        .into_iter()
        .map(|s| s.text)
        .collect::<Vec<_>>()
        .join("")
}

fn apply_sgr(style: &mut AnsiStyle, params: &str) {
    let nums: Vec<u32> = if params.is_empty() {
        vec![0]
    } else {
        params
            .split(';')
            .map(|p| p.parse::<u32>().unwrap_or(0))
            .collect()
    };
    let mut i = 0;
    while i < nums.len() {
        let n = nums[i];
        match n {
            0 => *style = AnsiStyle::default(),
            1 => style.bold = true,
            2 => style.dim = true,
            3 => style.italic = true,
            4 => style.underline = true,
            7 => style.reverse = true,
            22 => {
                style.bold = false;
                style.dim = false;
            }
            23 => style.italic = false,
            24 => style.underline = false,
            27 => style.reverse = false,
            30..=37 | 90..=97 => style.fg = AnsiColor::from_fg_code(n),
            39 => style.fg = None,
            40..=47 | 100..=107 => style.bg = AnsiColor::from_bg_code(n),
            49 => style.bg = None,
            38 => {
                // 256-color o 24-bit. `38;5;<idx>` o `38;2;r;g;b`. Lo
                // saltamos por ahora (no es lo más común); reconocemos
                // el tamaño del subparámetro para no descarrilar.
                match nums.get(i + 1) {
                    Some(5) => i += 2,
                    Some(2) => i += 4,
                    _ => {}
                }
            }
            48 => match nums.get(i + 1) {
                Some(5) => i += 2,
                Some(2) => i += 4,
                _ => {}
            },
            _ => {}
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(s: &str) -> Vec<AnsiSpan> {
        vec![AnsiSpan { text: s.to_string(), style: AnsiStyle::default() }]
    }

    #[test]
    fn empty_input_yields_no_spans() {
        assert!(parse_ansi_line("").is_empty());
    }

    #[test]
    fn text_without_escapes_is_one_plain_span() {
        assert_eq!(parse_ansi_line("hola mundo"), plain("hola mundo"));
    }

    #[test]
    fn red_text_picks_up_fg() {
        let spans = parse_ansi_line("\x1b[31mROJO\x1b[0m fin");
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].text, "ROJO");
        assert_eq!(spans[0].style.fg, Some(AnsiColor::Red));
        assert_eq!(spans[1].text, " fin");
        assert!(spans[1].style.is_plain());
    }

    #[test]
    fn bold_underline_combo() {
        let spans = parse_ansi_line("\x1b[1;4mTITULO\x1b[0m");
        assert_eq!(spans.len(), 1);
        assert!(spans[0].style.bold);
        assert!(spans[0].style.underline);
    }

    #[test]
    fn bg_color_is_offset_from_fg() {
        let spans = parse_ansi_line("\x1b[44m sobre azul \x1b[0m");
        assert_eq!(spans[0].style.bg, Some(AnsiColor::Blue));
    }

    #[test]
    fn bright_colors_high_range() {
        let spans = parse_ansi_line("\x1b[91mbrillante\x1b[0m");
        assert_eq!(spans[0].style.fg, Some(AnsiColor::BrightRed));
    }

    #[test]
    fn reset_at_end_clears_style() {
        let spans = parse_ansi_line("\x1b[33mwarn:\x1b[0m algo");
        assert_eq!(spans[0].style.fg, Some(AnsiColor::Yellow));
        assert!(spans[1].style.is_plain());
    }

    #[test]
    fn cr_overwrites_previous_chars() {
        // Progreso clásico de cargo/claude/docker.
        let spans = parse_ansi_line("12% [###      ]\r50% [#####    ]");
        let text: String = spans.iter().map(|s| s.text.as_str()).collect();
        assert!(text.starts_with("50% [#####"));
        // El final de la primera línea (`]`) queda detrás del segundo,
        // que es más corto en este test artificial — comprueba que la
        // segunda escritura sobreescribe columna a columna.
        assert_eq!(strip_ansi("12% [###      ]\r50% [#####    ]"), "50% [#####    ]");
    }

    #[test]
    fn cr_keeps_trailing_chars_when_overwrite_is_shorter() {
        // Si el segundo "estado" es más corto que el primero, los
        // chars del final del primero siguen visibles — exactamente lo
        // que hace un terminal real.
        let stripped = strip_ansi("...........\rABC");
        assert_eq!(stripped, "ABC........");
    }

    #[test]
    fn strip_ansi_drops_all_sgr() {
        assert_eq!(strip_ansi("\x1b[31mrojo\x1b[0m"), "rojo");
        assert_eq!(strip_ansi("texto plano"), "texto plano");
    }

    #[test]
    fn unknown_escape_terminator_is_skipped_gracefully() {
        // `\x1b[2K` (clear line) no es SGR; lo descartamos sin caer.
        let s = parse_ansi_line("\x1b[2Kdespués");
        assert_eq!(strip_ansi("\x1b[2Kdespués"), "después");
        assert!(s.iter().all(|sp| sp.style.is_plain()));
    }

    #[test]
    fn truecolor_sequences_dont_corrupt_subsequent_parsing() {
        // `\x1b[38;2;255;128;0m` — 24-bit color. Lo saltamos pero no
        // debemos rompernos en lo que viene después.
        let s = parse_ansi_line("\x1b[38;2;255;128;0mhola\x1b[0m mundo");
        let text: String = s.iter().map(|sp| sp.text.as_str()).collect();
        assert_eq!(text, "hola mundo");
    }
}
