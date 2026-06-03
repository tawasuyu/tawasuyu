//! Value-parsers de propiedades: `parse_color` (público) + funciones de color,
//! parsers de enums (display/flex/align/...), longitudes, gradientes, transforms,
//! grid, sombras de texto, y `evaluate_media_query` (público) + supports. Sub-
//! módulo de `parser` (regla #1). `use super::*`.
use super::*;

/// Parsea un color CSS (`#rgb`/`#rrggbb`/`#rrggbbaa`, `rgb()`/`rgba()`,
/// `hsl()`/`hsla()`, named colors) a [`Color`]. Público para que el chrome
/// pinte `fillStyle`/`strokeStyle` de canvas (Fase 7.196). `None` si no
/// parsea.
pub fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    // hex #RRGGBB / #RGB / #RRGGBBAA / #RGBA
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some(Color::rgb(r, g, b));
        }
        if hex.len() == 3 {
            let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
            return Some(Color::rgb(r, g, b));
        }
        if hex.len() == 8 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            return Some(Color { r, g, b, a });
        }
        if hex.len() == 4 {
            let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
            let a = u8::from_str_radix(&hex[3..4], 16).ok()? * 17;
            return Some(Color { r, g, b, a });
        }
    }
    // rgb()/rgba() — coma legacy o whitespace moderno, con alpha por
    // 4to arg o sufijo `/ alpha`.
    if let Some(args) = strip_fn(s, "rgba").or_else(|| strip_fn(s, "rgb")) {
        return parse_rgb_func(args);
    }
    if let Some(args) = strip_fn(s, "hsla").or_else(|| strip_fn(s, "hsl")) {
        return parse_hsl_func(args);
    }
    if let Some(args) = strip_fn(s, "hwb") {
        return parse_hwb_func(args);
    }
    // CSS Color 4 — espacios perceptuales y polar. `oklch`/`oklab` antes
    // que `lch`/`lab` no es necesario (`strip_fn` matchea prefijo exacto)
    // pero se ordena por familia.
    if let Some(args) = strip_fn(s, "oklch") {
        return parse_oklch_func(args);
    }
    if let Some(args) = strip_fn(s, "oklab") {
        return parse_oklab_func(args);
    }
    if let Some(args) = strip_fn(s, "lch") {
        return parse_lch_func(args);
    }
    if let Some(args) = strip_fn(s, "lab") {
        return parse_lab_func(args);
    }
    // `color-mix(...)` antes que `color(...)` (no colisionan en `strip_fn`
    // —`color(` no matchea `color-mix(`— pero se ordena por claridad).
    if let Some(args) = strip_fn(s, "color-mix") {
        return parse_color_mix(args);
    }
    if let Some(args) = strip_fn(s, "color") {
        return parse_color_func(args);
    }
    // Nombres comunes.
    NAMED_COLORS.iter().find(|(n, _)| n.eq_ignore_ascii_case(s)).map(|(_, c)| *c)
}

/// Si `s` es de la forma `name(…)`, devuelve los argumentos crudos
/// (sin paréntesis). Tolera espacios entre el nombre y `(`. Match del
/// nombre case-insensitive.
pub(crate) fn strip_fn<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let s = s.trim();
    if !s.get(..name.len())?.eq_ignore_ascii_case(name) {
        return None;
    }
    let rest = s[name.len()..].trim_start();
    let inner = rest.strip_prefix('(')?.strip_suffix(')')?;
    Some(inner.trim())
}

/// Parsea los argumentos de `rgb(…)` o `rgba(…)`. Acepta sintaxis
/// legacy (separador coma, alpha como 4to arg) y moderna (whitespace
/// + `/ alpha`). Cada canal RGB tolera entero 0-255 o porcentaje. El
/// alpha tolera fracción 0-1 o porcentaje.
pub(crate) fn parse_rgb_func(args: &str) -> Option<Color> {
    let (rgb, alpha) = split_color_args(args)?;
    if rgb.len() != 3 {
        return None;
    }
    let r = parse_color_chan(rgb[0])?;
    let g = parse_color_chan(rgb[1])?;
    let b = parse_color_chan(rgb[2])?;
    let a = match alpha {
        Some(a_str) => parse_alpha(a_str)?,
        None => 255,
    };
    Some(Color { r, g, b, a })
}

/// Parsea `hsl(…)` / `hsla(…)`. H = grados (0-360, se wrappea), S/L =
/// porcentaje (0-100). Alpha igual que rgba.
pub(crate) fn parse_hsl_func(args: &str) -> Option<Color> {
    let (parts, alpha) = split_color_args(args)?;
    if parts.len() != 3 {
        return None;
    }
    let h = parse_hue(parts[0])?;
    let s = parse_pct(parts[1])?;
    let l = parse_pct(parts[2])?;
    let (r, g, b) = hsl_to_rgb(h, s, l);
    let a = match alpha {
        Some(a_str) => parse_alpha(a_str)?,
        None => 255,
    };
    Some(Color { r, g, b, a })
}

/// Tokeniza los args de un color function. Devuelve `(canales, alpha?)`.
/// Resuelve coma vs whitespace y la sintaxis moderna `r g b / a`.
pub(crate) fn split_color_args(args: &str) -> Option<(Vec<&str>, Option<&str>)> {
    let args = args.trim();
    // Sintaxis moderna: `R G B / A`. La barra separa el alpha.
    if let Some(slash) = args.find('/') {
        let main = args[..slash].trim();
        let alpha = args[slash + 1..].trim();
        let parts: Vec<&str> = main.split_whitespace().collect();
        if parts.is_empty() {
            return None;
        }
        return Some((parts, Some(alpha)));
    }
    // Legacy: comas separan TODO (incluido el alpha).
    if args.contains(',') {
        let parts: Vec<&str> = args.split(',').map(|s| s.trim()).collect();
        if parts.len() == 4 {
            let (rgb, a) = parts.split_at(3);
            return Some((rgb.to_vec(), Some(a[0])));
        }
        return Some((parts, None));
    }
    // Moderna sin alpha: solo whitespace.
    let parts: Vec<&str> = args.split_whitespace().collect();
    Some((parts, None))
}

/// Canal RGB: entero 0-255 o porcentaje 0%-100%.
pub(crate) fn parse_color_chan(s: &str) -> Option<u8> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix('%') {
        let pct: f32 = num.trim().parse().ok()?;
        return Some((pct.clamp(0.0, 100.0) * 2.55).round() as u8);
    }
    s.parse::<i32>().ok().map(|n| n.clamp(0, 255) as u8)
}

/// Alpha: fracción 0.0-1.0 o porcentaje 0%-100%. `none` (CSS Color 4) ⇒ 0.
pub(crate) fn parse_alpha(s: &str) -> Option<u8> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("none") {
        return Some(0);
    }
    if let Some(num) = s.strip_suffix('%') {
        let pct: f32 = num.trim().parse().ok()?;
        return Some((pct.clamp(0.0, 100.0) * 2.55).round() as u8);
    }
    let f: f32 = s.parse().ok()?;
    Some((f.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// Hue (CSS `<angle>`): número crudo (grados implícitos) o con unidad
/// `deg`/`grad`/`rad`/`turn`. `none` (CSS Color 4) ⇒ 0. El resultado son
/// grados; el caller lo wrappea con `rem_euclid(360)` según convenga.
pub(crate) fn parse_hue(s: &str) -> Option<f32> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("none") {
        return Some(0.0);
    }
    // `grad` antes que `rad` (sufijo solapado) y `turn`/`rad`/`deg`.
    if let Some(n) = s.strip_suffix("grad") {
        let g: f32 = n.trim().parse().ok()?;
        return Some(g * 0.9); // 400grad = 360deg
    }
    if let Some(n) = s.strip_suffix("turn") {
        let t: f32 = n.trim().parse().ok()?;
        return Some(t * 360.0);
    }
    if let Some(n) = s.strip_suffix("rad") {
        let r: f32 = n.trim().parse().ok()?;
        return Some(r.to_degrees());
    }
    if let Some(n) = s.strip_suffix("deg") {
        return n.trim().parse().ok();
    }
    s.parse().ok()
}

/// Porcentaje 0%-100% → fracción 0.0-1.0.
pub(crate) fn parse_pct(s: &str) -> Option<f32> {
    let s = s.trim().strip_suffix('%')?;
    let pct: f32 = s.trim().parse().ok()?;
    Some((pct / 100.0).clamp(0.0, 1.0))
}

/// HSL→RGB estándar (CSS Color Module L3). h en grados, s/l en 0..1.
pub(crate) fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h_prime = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (h_prime.rem_euclid(2.0) - 1.0).abs());
    let (r1, g1, b1) = match h_prime as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to_u8 = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (to_u8(r1), to_u8(g1), to_u8(b1))
}

/// Número crudo o porcentaje, donde `100%` equivale a `pct_full`. `none`
/// (CSS Color 4) ⇒ 0. Sin clamp (el caller acota el espacio de color).
/// Usado por los color functions modernos (`oklch`/`lab`/`color()`…),
/// cada uno con su escala (`pct_full` = 1.0 para L de oklch, 100 para L de
/// lab, 0.4 para C de oklch, etc.).
pub(crate) fn parse_num_or_pct(s: &str, pct_full: f32) -> Option<f32> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("none") {
        return Some(0.0);
    }
    if let Some(p) = s.strip_suffix('%') {
        let v: f32 = p.trim().parse().ok()?;
        return Some(v / 100.0 * pct_full);
    }
    s.parse().ok()
}

/// RGB de hue puro (saturación 100%, lightness 50%) como floats 0..1.
/// Base para `hwb()` (CSS Color 4 §7).
fn hue_to_rgb_pure(h: f32) -> (f32, f32, f32) {
    let hp = h.rem_euclid(360.0) / 60.0;
    let x = 1.0 - (hp.rem_euclid(2.0) - 1.0).abs();
    match hp as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    }
}

/// Componente lineal → sRGB con gamma (transferencia sRGB estándar).
fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// sRGB con gamma → componente lineal (inversa de `linear_to_srgb`).
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Empaqueta tres componentes lineales sRGB (0..1, fuera de gamut se
/// recorta) + alpha en un `Color` sRGB con gamma.
fn linear_srgb_to_color(r: f32, g: f32, b: f32, a: u8) -> Color {
    let to_u8 = |v: f32| (linear_to_srgb(v) * 255.0).round().clamp(0.0, 255.0) as u8;
    Color { r: to_u8(r), g: to_u8(g), b: to_u8(b), a }
}

/// `hwb(H W B [/ A])` (CSS Color 4). H = `<angle>`, W/B = porcentaje de
/// blancura/negrura. Si W+B ≥ 100% el resultado es el gris W/(W+B).
pub(crate) fn parse_hwb_func(args: &str) -> Option<Color> {
    let (parts, alpha) = split_color_args(args)?;
    if parts.len() != 3 {
        return None;
    }
    let h = parse_hue(parts[0])?;
    let w = parse_pct_or_none(parts[1])?;
    let bl = parse_pct_or_none(parts[2])?;
    let a = match alpha {
        Some(s) => parse_alpha(s)?,
        None => 255,
    };
    let to_u8 = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    if w + bl >= 1.0 {
        let gray = if w + bl > 0.0 { w / (w + bl) } else { 0.0 };
        let g = to_u8(gray);
        return Some(Color { r: g, g, b: g, a });
    }
    let (hr, hg, hb) = hue_to_rgb_pure(h);
    let mix = |c: f32| c * (1.0 - w - bl) + w;
    Some(Color { r: to_u8(mix(hr)), g: to_u8(mix(hg)), b: to_u8(mix(hb)), a })
}

/// Porcentaje 0%-100% → 0..1, o `none` ⇒ 0. (Variante de `parse_pct` que
/// tolera `none`, para los color functions de CSS Color 4.)
fn parse_pct_or_none(s: &str) -> Option<f32> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("none") {
        return Some(0.0);
    }
    parse_pct(s)
}

/// OKLab → sRGB lineal (Björn Ottosson). L 0..1, a/b ~-0.4..0.4.
fn oklab_to_linear_srgb(l: f32, a: f32, b: f32) -> (f32, f32, f32) {
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let l3 = l_ * l_ * l_;
    let m3 = m_ * m_ * m_;
    let s3 = s_ * s_ * s_;
    (
        4.076_741_7 * l3 - 3.307_711_6 * m3 + 0.230_969_94 * s3,
        -1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_38 * s3,
        -0.004_196_086_3 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3,
    )
}

/// `oklab(L a b [/ A])` (CSS Color 4). L 0..1 (o %), a/b número (o % de 0.4).
pub(crate) fn parse_oklab_func(args: &str) -> Option<Color> {
    let (parts, alpha) = split_color_args(args)?;
    if parts.len() != 3 {
        return None;
    }
    let l = parse_num_or_pct(parts[0], 1.0)?;
    let a = parse_num_or_pct(parts[1], 0.4)?;
    let b = parse_num_or_pct(parts[2], 0.4)?;
    let al = match alpha {
        Some(s) => parse_alpha(s)?,
        None => 255,
    };
    let (r, g, bb) = oklab_to_linear_srgb(l, a, b);
    Some(linear_srgb_to_color(r, g, bb, al))
}

/// `oklch(L C H [/ A])` (CSS Color 4). C → a/b polar; resto como `oklab`.
pub(crate) fn parse_oklch_func(args: &str) -> Option<Color> {
    let (parts, alpha) = split_color_args(args)?;
    if parts.len() != 3 {
        return None;
    }
    let l = parse_num_or_pct(parts[0], 1.0)?;
    let c = parse_num_or_pct(parts[1], 0.4)?;
    let h = parse_hue(parts[2])?.to_radians();
    let al = match alpha {
        Some(s) => parse_alpha(s)?,
        None => 255,
    };
    let (r, g, bb) = oklab_to_linear_srgb(l, c * h.cos(), c * h.sin());
    Some(linear_srgb_to_color(r, g, bb, al))
}

/// CIE Lab (D50) → sRGB lineal, vía XYZ(D50), adaptación Bradford a D65 y
/// la matriz XYZ(D65)→sRGB-lineal (CSS Color 4, código de muestra).
fn lab_to_linear_srgb(l: f32, a: f32, b: f32) -> (f32, f32, f32) {
    const KAPPA: f32 = 24389.0 / 27.0;
    const EPS: f32 = 216.0 / 24389.0;
    // Blanco de referencia D50.
    const XN: f32 = 0.964_295_7;
    const YN: f32 = 1.0;
    const ZN: f32 = 0.825_104_6;
    let fy = (l + 16.0) / 116.0;
    let fx = fy + a / 500.0;
    let fz = fy - b / 200.0;
    let f_inv = |t: f32| {
        let t3 = t * t * t;
        if t3 > EPS { t3 } else { (116.0 * t - 16.0) / KAPPA }
    };
    let xr = f_inv(fx);
    let yr = if l > KAPPA * EPS { fy * fy * fy } else { l / KAPPA };
    let zr = f_inv(fz);
    let (x50, y50, z50) = (xr * XN, yr * YN, zr * ZN);
    // Bradford D50 → D65.
    let x = 0.955_473_45 * x50 - 0.023_098_537 * y50 + 0.063_259_31 * z50;
    let y = -0.028_369_707 * x50 + 1.009_995_46 * y50 + 0.021_041_399 * z50;
    let z = 0.012_314_002 * x50 - 0.020_507_697 * y50 + 1.330_366 * z50;
    // XYZ(D65) → sRGB lineal.
    (
        3.240_97 * x - 1.537_383_2 * y - 0.498_610_76 * z,
        -0.969_243_64 * x + 1.875_967_5 * y + 0.041_555_06 * z,
        0.055_630_08 * x - 0.203_976_96 * y + 1.056_971_5 * z,
    )
}

/// `lab(L a b [/ A])` (CSS Color 4). L 0..100 (o %), a/b número (o % de 125).
pub(crate) fn parse_lab_func(args: &str) -> Option<Color> {
    let (parts, alpha) = split_color_args(args)?;
    if parts.len() != 3 {
        return None;
    }
    let l = parse_num_or_pct(parts[0], 100.0)?;
    let a = parse_num_or_pct(parts[1], 125.0)?;
    let b = parse_num_or_pct(parts[2], 125.0)?;
    let al = match alpha {
        Some(s) => parse_alpha(s)?,
        None => 255,
    };
    let (r, g, bb) = lab_to_linear_srgb(l, a, b);
    Some(linear_srgb_to_color(r, g, bb, al))
}

/// `lch(L C H [/ A])` (CSS Color 4). C → a/b polar; resto como `lab`.
pub(crate) fn parse_lch_func(args: &str) -> Option<Color> {
    let (parts, alpha) = split_color_args(args)?;
    if parts.len() != 3 {
        return None;
    }
    let l = parse_num_or_pct(parts[0], 100.0)?;
    let c = parse_num_or_pct(parts[1], 150.0)?;
    let h = parse_hue(parts[2])?.to_radians();
    let al = match alpha {
        Some(s) => parse_alpha(s)?,
        None => 255,
    };
    let (r, g, bb) = lab_to_linear_srgb(l, c * h.cos(), c * h.sin());
    Some(linear_srgb_to_color(r, g, bb, al))
}

/// `color(<space> c1 c2 c3 [/ A])` (CSS Color 4). Soporta `srgb`,
/// `srgb-linear` y `display-p3`; otros espacios ⇒ `None` (degrada). Los
/// componentes son número 0..1 o porcentaje.
pub(crate) fn parse_color_func(args: &str) -> Option<Color> {
    let args = args.trim();
    // El primer token es el espacio; el resto, componentes (+ `/ alpha`).
    let (space, rest) = args.split_once(char::is_whitespace)?;
    let (parts, alpha) = split_color_args(rest.trim())?;
    if parts.len() != 3 {
        return None;
    }
    let comp = |s: &str| parse_num_or_pct(s, 1.0);
    let c0 = comp(parts[0])?;
    let c1 = comp(parts[1])?;
    let c2 = comp(parts[2])?;
    let al = match alpha {
        Some(s) => parse_alpha(s)?,
        None => 255,
    };
    let (r, g, b) = match space.trim().to_ascii_lowercase().as_str() {
        "srgb" => (srgb_to_linear(c0), srgb_to_linear(c1), srgb_to_linear(c2)),
        "srgb-linear" => (c0, c1, c2),
        "display-p3" => {
            // P3 con gamma sRGB → P3 lineal → matriz P3→sRGB lineal.
            let (lr, lg, lb) = (srgb_to_linear(c0), srgb_to_linear(c1), srgb_to_linear(c2));
            (
                1.224_940_2 * lr - 0.224_940_18 * lg,
                -0.042_056_974 * lr + 1.042_057 * lg,
                -0.019_636_242 * lr - 0.078_637_2 * lg + 1.098_273_4 * lb,
            )
        }
        _ => return None,
    };
    Some(linear_srgb_to_color(r, g, b, al))
}

/// sRGB lineal → OKLab (inversa de `oklab_to_linear_srgb`). Para mezclar
/// `color-mix(in oklab/oklch, ...)`.
fn linear_srgb_to_oklab(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let l = 0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b;
    let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
    let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;
    let (l_, m_, s_) = (l.cbrt(), m.cbrt(), s.cbrt());
    (
        0.210_454_26 * l_ + 0.793_617_8 * m_ - 0.004_072_047 * s_,
        1.977_998_5 * l_ - 2.428_592_2 * m_ + 0.450_593_7 * s_,
        0.025_904_037 * l_ + 0.782_771_77 * m_ - 0.808_675_77 * s_,
    )
}

/// `color-mix(in <space>, C1 [p1], C2 [p2])` (CSS Color 5). Soporta los
/// espacios de mezcla más usados en la web moderna: `srgb`, `srgb-linear`,
/// `oklab`, `oklch` (los demás degradan a `srgb`). El método de hue de
/// `oklch` es el default (arco más corto).
pub(crate) fn parse_color_mix(args: &str) -> Option<Color> {
    let segments = split_top_level_comma(args);
    if segments.len() != 3 {
        return None;
    }
    // Cabecera: `in <space>[ <hue-method>]` (el método de hue se ignora).
    let head = segments[0].trim();
    if head.len() < 3 || !head[..2].eq_ignore_ascii_case("in") {
        return None;
    }
    let after_in = head[2..].trim_start();
    let space = after_in.split_whitespace().next()?.to_ascii_lowercase();
    let (c1, p1) = parse_color_with_pct(segments[1].trim())?;
    let (c2, p2) = parse_color_with_pct(segments[2].trim())?;
    let (w1, w2) = mix_weights(p1, p2)?;
    Some(mix_colors(&space, c1, c2, w1, w2))
}

/// Un color de `color-mix` con su porcentaje opcional (antes o después del
/// color). `red`, `red 40%`, `40% red` → `(Color, Option<pct>)`.
fn parse_color_with_pct(s: &str) -> Option<(Color, Option<f32>)> {
    let s = s.trim();
    if let Some(c) = parse_color(s) {
        return Some((c, None));
    }
    // Porcentaje al final: `<color> 40%`.
    if let Some((rest, last)) = s.rsplit_once(char::is_whitespace) {
        if let Some(p) = last.trim().strip_suffix('%') {
            if let (Ok(v), Some(c)) = (p.trim().parse::<f32>(), parse_color(rest.trim())) {
                return Some((c, Some(v)));
            }
        }
    }
    // Porcentaje al principio: `40% <color>`.
    if let Some((first, rest)) = s.split_once(char::is_whitespace) {
        if let Some(p) = first.trim().strip_suffix('%') {
            if let (Ok(v), Some(c)) = (p.trim().parse::<f32>(), parse_color(rest.trim())) {
                return Some((c, Some(v)));
            }
        }
    }
    None
}

/// Pesos normalizados (suman 1) a partir de los porcentajes opcionales.
/// Ninguno ⇒ 50/50; uno ⇒ el otro completa a 100; ambos ⇒ se normalizan.
fn mix_weights(p1: Option<f32>, p2: Option<f32>) -> Option<(f32, f32)> {
    match (p1, p2) {
        (None, None) => Some((0.5, 0.5)),
        (Some(a), None) => Some((a / 100.0, 1.0 - a / 100.0)),
        (None, Some(b)) => Some((1.0 - b / 100.0, b / 100.0)),
        (Some(a), Some(b)) => {
            let sum = a + b;
            if sum <= 0.0 {
                return None;
            }
            Some((a / sum, b / sum))
        }
    }
}

/// Mezcla `c1`·w1 + `c2`·w2 en el espacio dado. El alpha se interpola
/// directo (no premultiplicado — aproximación para alphas distintos).
fn mix_colors(space: &str, c1: Color, c2: Color, w1: f32, w2: f32) -> Color {
    let a = (c1.a as f32 * w1 + c2.a as f32 * w2).round().clamp(0.0, 255.0) as u8;
    let to_u8 = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    let lin = |c: Color| {
        (
            srgb_to_linear(c.r as f32 / 255.0),
            srgb_to_linear(c.g as f32 / 255.0),
            srgb_to_linear(c.b as f32 / 255.0),
        )
    };
    match space {
        "srgb-linear" => {
            let (r1, g1, b1) = lin(c1);
            let (r2, g2, b2) = lin(c2);
            Color {
                r: to_u8(linear_to_srgb(r1 * w1 + r2 * w2)),
                g: to_u8(linear_to_srgb(g1 * w1 + g2 * w2)),
                b: to_u8(linear_to_srgb(b1 * w1 + b2 * w2)),
                a,
            }
        }
        "oklab" | "oklch" => {
            let (r1, g1, b1) = lin(c1);
            let (r2, g2, b2) = lin(c2);
            let (l1, a1, bb1) = linear_srgb_to_oklab(r1, g1, b1);
            let (l2, a2, bb2) = linear_srgb_to_oklab(r2, g2, b2);
            let (ml, ma, mb) = if space == "oklch" {
                // Polar: interpola L, C y H (arco más corto).
                let (cc1, h1) = (a1.hypot(bb1), bb1.atan2(a1).to_degrees());
                let (cc2, h2) = (a2.hypot(bb2), bb2.atan2(a2).to_degrees());
                let l = l1 * w1 + l2 * w2;
                let c = cc1 * w1 + cc2 * w2;
                let mut dh = h2 - h1;
                if dh > 180.0 {
                    dh -= 360.0;
                } else if dh < -180.0 {
                    dh += 360.0;
                }
                let h = (h1 + w2 * dh).to_radians();
                (l, c * h.cos(), c * h.sin())
            } else {
                (l1 * w1 + l2 * w2, a1 * w1 + a2 * w2, bb1 * w1 + bb2 * w2)
            };
            let (r, g, b) = oklab_to_linear_srgb(ml, ma, mb);
            let mut col = linear_srgb_to_color(r, g, b, a);
            col.a = a;
            col
        }
        // `srgb` y cualquier espacio no soportado → mezcla en sRGB con gamma.
        _ => Color {
            r: (c1.r as f32 * w1 + c2.r as f32 * w2).round().clamp(0.0, 255.0) as u8,
            g: (c1.g as f32 * w1 + c2.g as f32 * w2).round().clamp(0.0, 255.0) as u8,
            b: (c1.b as f32 * w1 + c2.b as f32 * w2).round().clamp(0.0, 255.0) as u8,
            a,
        },
    }
}

/// Parsea un value tipo `margin: <1..4 longitudes>`. Devuelve `None` si
/// algún token no es longitud válida o si hay menos de 1 / más de 4.
pub(crate) fn parse_sides(value: &str) -> Option<Sides<f32>> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    let parsed: Vec<f32> = parts
        .iter()
        .map(|t| parse_length_px(t))
        .collect::<Option<Vec<_>>>()?;
    Some(match parsed.as_slice() {
        [a] => Sides::all(*a),
        [v, h] => Sides { top: *v, right: *h, bottom: *v, left: *h },
        [t, h, b] => Sides { top: *t, right: *h, bottom: *b, left: *h },
        [t, r, b, l] => Sides { top: *t, right: *r, bottom: *b, left: *l },
        _ => return None,
    })
}

const NAMED_COLORS: &[(&str, Color)] = &[
    ("black", Color::BLACK),
    ("white", Color::WHITE),
    ("red", Color::rgb_const(255, 0, 0)),
    ("green", Color::rgb_const(0, 128, 0)),
    ("blue", Color::rgb_const(0, 0, 255)),
    ("gray", Color::rgb_const(128, 128, 128)),
    ("grey", Color::rgb_const(128, 128, 128)),
    ("silver", Color::rgb_const(192, 192, 192)),
    ("maroon", Color::rgb_const(128, 0, 0)),
    ("yellow", Color::rgb_const(255, 255, 0)),
    ("olive", Color::rgb_const(128, 128, 0)),
    ("lime", Color::rgb_const(0, 255, 0)),
    ("aqua", Color::rgb_const(0, 255, 255)),
    ("cyan", Color::rgb_const(0, 255, 255)),
    ("teal", Color::rgb_const(0, 128, 128)),
    ("navy", Color::rgb_const(0, 0, 128)),
    ("fuchsia", Color::rgb_const(255, 0, 255)),
    ("magenta", Color::rgb_const(255, 0, 255)),
    ("purple", Color::rgb_const(128, 0, 128)),
    ("orange", Color::rgb_const(255, 165, 0)),
    ("pink", Color::rgb_const(255, 192, 203)),
    ("brown", Color::rgb_const(165, 42, 42)),
    ("gold", Color::rgb_const(255, 215, 0)),
    ("indigo", Color::rgb_const(75, 0, 130)),
    ("violet", Color::rgb_const(238, 130, 238)),
    ("crimson", Color::rgb_const(220, 20, 60)),
    ("darkblue", Color::rgb_const(0, 0, 139)),
    ("darkgreen", Color::rgb_const(0, 100, 0)),
    ("darkred", Color::rgb_const(139, 0, 0)),
    ("darkgray", Color::rgb_const(169, 169, 169)),
    ("lightgray", Color::rgb_const(211, 211, 211)),
    ("lightblue", Color::rgb_const(173, 216, 230)),
    ("lightgreen", Color::rgb_const(144, 238, 144)),
    ("transparent", Color::TRANSPARENT),
];

pub(crate) fn parse_weight(s: &str) -> Option<u16> {
    match s.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(400),
        "bold" => Some(700),
        "lighter" => Some(300),
        "bolder" => Some(700),
        num => num.parse().ok(),
    }
}

pub(crate) fn parse_font_style(s: &str) -> Option<FontStyle> {
    // CSS spec: normal | italic | oblique [<angle>?]. Tratamos oblique
    // como italic — parley/fontique sintetizan si la fuente no tiene
    // oblique nativo.
    let v = s.trim().to_ascii_lowercase();
    if v == "normal" {
        Some(FontStyle::Normal)
    } else if v == "italic" || v.starts_with("oblique") {
        Some(FontStyle::Italic)
    } else {
        None
    }
}

pub(crate) fn parse_display(s: &str) -> Option<Display> {
    match s.trim().to_ascii_lowercase().as_str() {
        "block" => Some(Display::Block),
        "inline" => Some(Display::Inline),
        "inline-block" => Some(Display::InlineBlock),
        "flex" => Some(Display::Flex),
        "inline-flex" => Some(Display::InlineFlex),
        "grid" => Some(Display::Grid),
        "inline-grid" => Some(Display::InlineGrid),
        "none" => Some(Display::None),
        _ => None,
    }
}

pub(crate) fn parse_flex_direction(s: &str) -> Option<FlexDirection> {
    match s.trim().to_ascii_lowercase().as_str() {
        "row" => Some(FlexDirection::Row),
        "row-reverse" => Some(FlexDirection::RowReverse),
        "column" => Some(FlexDirection::Column),
        "column-reverse" => Some(FlexDirection::ColumnReverse),
        _ => None,
    }
}

pub(crate) fn parse_flex_wrap(s: &str) -> Option<FlexWrap> {
    match s.trim().to_ascii_lowercase().as_str() {
        "nowrap" => Some(FlexWrap::NoWrap),
        "wrap" => Some(FlexWrap::Wrap),
        "wrap-reverse" => Some(FlexWrap::WrapReverse),
        _ => None,
    }
}

pub(crate) fn parse_justify_content(s: &str) -> Option<JustifyContent> {
    match s.trim().to_ascii_lowercase().as_str() {
        "start" | "flex-start" | "left" => Some(JustifyContent::Start),
        "center" => Some(JustifyContent::Center),
        "end" | "flex-end" | "right" => Some(JustifyContent::End),
        "space-between" => Some(JustifyContent::SpaceBetween),
        "space-around" => Some(JustifyContent::SpaceAround),
        "space-evenly" => Some(JustifyContent::SpaceEvenly),
        _ => None,
    }
}

pub(crate) fn parse_align_items(s: &str) -> Option<AlignItems> {
    match s.trim().to_ascii_lowercase().as_str() {
        "start" | "flex-start" => Some(AlignItems::Start),
        "center" => Some(AlignItems::Center),
        "end" | "flex-end" => Some(AlignItems::End),
        "stretch" => Some(AlignItems::Stretch),
        "baseline" => Some(AlignItems::Baseline),
        _ => None,
    }
}

/// `align-content`. `normal` y `baseline` colapsan a `Normal` (default de
/// taffy ≈ stretch); el resto mapea directo. `start`/`end` aceptan también
/// la variante `flex-*`.
pub(crate) fn parse_align_content(s: &str) -> Option<AlignContent> {
    match s.trim().to_ascii_lowercase().as_str() {
        "normal" | "baseline" => Some(AlignContent::Normal),
        "start" | "flex-start" => Some(AlignContent::Start),
        "center" => Some(AlignContent::Center),
        "end" | "flex-end" => Some(AlignContent::End),
        "stretch" => Some(AlignContent::Stretch),
        "space-between" => Some(AlignContent::SpaceBetween),
        "space-around" => Some(AlignContent::SpaceAround),
        "space-evenly" => Some(AlignContent::SpaceEvenly),
        _ => None,
    }
}

/// `justify-items` (grid). Reusa el subset de `align-items` y agrega
/// `left`/`right` (que en escritura LTR equivalen a start/end). `normal`
/// se descarta → queda el default None. `auto`/`legacy` también.
pub(crate) fn parse_justify_items(s: &str) -> Option<AlignItems> {
    match s.trim().to_ascii_lowercase().as_str() {
        "left" => Some(AlignItems::Start),
        "right" => Some(AlignItems::End),
        other => parse_align_items(other),
    }
}

/// `justify-self` (grid item). Reusa `align-self` + `left`/`right`.
pub(crate) fn parse_justify_self(s: &str) -> Option<AlignSelf> {
    match s.trim().to_ascii_lowercase().as_str() {
        "left" => Some(AlignSelf::Start),
        "right" => Some(AlignSelf::End),
        other => parse_align_self(other),
    }
}

/// `place-content: <align-content> [<justify-content>]`. Un solo valor
/// setea ambos ejes. Cada mitad se valida con su parser propio; las que no
/// parsean se descartan (el otro eje igual se aplica).
pub(crate) fn parse_place_content_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let mut out = Vec::new();
    let mut it = value.split_whitespace();
    let Some(a) = it.next() else { return out };
    let b = it.next().unwrap_or(a);
    if let Some(ac) = parse_align_content(a) {
        out.push(Decl { kind: DeclKind::AlignContent(ac), important });
    }
    if let Some(jc) = parse_justify_content(b) {
        out.push(Decl { kind: DeclKind::JustifyContent(jc), important });
    }
    out
}

/// `place-items: <align-items> [<justify-items>]`. Un solo valor = ambos.
pub(crate) fn parse_place_items_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let mut out = Vec::new();
    let mut it = value.split_whitespace();
    let Some(a) = it.next() else { return out };
    let b = it.next().unwrap_or(a);
    if let Some(ai) = parse_align_items(a) {
        out.push(Decl { kind: DeclKind::AlignItems(ai), important });
    }
    if let Some(ji) = parse_justify_items(b) {
        out.push(Decl { kind: DeclKind::JustifyItems(ji), important });
    }
    out
}

/// `place-self: <align-self> [<justify-self>]`. Un solo valor = ambos.
pub(crate) fn parse_place_self_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let mut out = Vec::new();
    let mut it = value.split_whitespace();
    let Some(a) = it.next() else { return out };
    let b = it.next().unwrap_or(a);
    if let Some(asf) = parse_align_self(a) {
        out.push(Decl { kind: DeclKind::AlignSelf(asf), important });
    }
    if let Some(jsf) = parse_justify_self(b) {
        out.push(Decl { kind: DeclKind::JustifySelf(jsf), important });
    }
    out
}

/// `gap: V` ⇒ row=V, column=V. `gap: R C` ⇒ row=R, column=C. Coincide
/// con la semántica CSS shorthand (primer valor = row, segundo = column).
pub(crate) fn parse_gap(value: &str) -> Option<(f32, f32)> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    match parts.as_slice() {
        [v] => {
            let v = parse_length_px(v)?;
            Some((v, v))
        }
        [r, c] => Some((parse_length_px(r)?, parse_length_px(c)?)),
        _ => None,
    }
}

pub(crate) fn parse_box_sizing(s: &str) -> Option<BoxSizing> {
    match s.trim().to_ascii_lowercase().as_str() {
        "content-box" => Some(BoxSizing::ContentBox),
        "border-box" => Some(BoxSizing::BorderBox),
        _ => None,
    }
}

pub(crate) fn parse_overflow(s: &str) -> Option<Overflow> {
    match s.trim().to_ascii_lowercase().as_str() {
        "visible" => Some(Overflow::Visible),
        // hidden/clip/auto/scroll todos los tratamos como Hidden por
        // ahora (no soportamos scroll real; clip y hidden cortan igual).
        "hidden" | "clip" | "auto" | "scroll" => Some(Overflow::Hidden),
        _ => None,
    }
}

pub(crate) fn parse_white_space(s: &str) -> Option<WhiteSpace> {
    match s.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(WhiteSpace::Normal),
        "nowrap" => Some(WhiteSpace::NoWrap),
        "pre" => Some(WhiteSpace::Pre),
        "pre-wrap" => Some(WhiteSpace::PreWrap),
        "pre-line" => Some(WhiteSpace::PreLine),
        _ => None,
    }
}

pub(crate) fn parse_text_transform(s: &str) -> Option<TextTransform> {
    match s.trim().to_ascii_lowercase().as_str() {
        "none" => Some(TextTransform::None),
        "uppercase" => Some(TextTransform::Uppercase),
        "lowercase" => Some(TextTransform::Lowercase),
        "capitalize" => Some(TextTransform::Capitalize),
        _ => None,
    }
}

/// Acepta `0..1` o `0%..100%`. Clampa.
pub(crate) fn parse_opacity(s: &str) -> Option<f32> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix('%') {
        let pct: f32 = num.trim().parse().ok()?;
        return Some((pct / 100.0).clamp(0.0, 1.0));
    }
    s.parse::<f32>().ok().map(|v| v.clamp(0.0, 1.0))
}

pub(crate) fn parse_align_self(s: &str) -> Option<AlignSelf> {
    match s.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(AlignSelf::Auto),
        "start" | "flex-start" => Some(AlignSelf::Start),
        "center" => Some(AlignSelf::Center),
        "end" | "flex-end" => Some(AlignSelf::End),
        "stretch" => Some(AlignSelf::Stretch),
        "baseline" => Some(AlignSelf::Baseline),
        _ => None,
    }
}

/// `flex: <grow> [<shrink>] [<basis>]`. Casos especiales:
/// - `flex: none` → `0 0 auto`
/// - `flex: auto` → `1 1 auto`
/// - `flex: <number>` → `N 1 0%` (basis 0%, common preset)
/// Devuelve 3 decls atómicas (grow + shrink + basis).
/// Propiedades lógicas de caja (`margin-inline`/`margin-block`/`padding-*` y
/// sus `-start`/`-end`), mapeadas a las físicas asumiendo LTR + escritura
/// horizontal (el caso por defecto). `inline` ↔ left/right, `block` ↔
/// top/bottom; `start`=left/top, `end`=right/bottom. Las dos-lados aceptan
/// 1–2 valores (`margin-inline: 10px` o `10px 20px`). Devuelve `None` si el
/// nombre no es una propiedad lógica conocida. Fase 7.191.
pub(crate) fn parse_logical_box(prop: &str, value: &str, important: bool) -> Option<Vec<Decl>> {
    use DeclKind::{
        MarginBottom, MarginLeft, MarginRight, MarginTop, PaddingBottom, PaddingLeft,
        PaddingRight, PaddingTop,
    };
    use DeclKind::{InsetBottom, InsetLeft, InsetRight, InsetTop};
    let lower = prop.to_ascii_lowercase();
    // `inset-inline`/`inset-block` y sus `-start`/`-end`: usan `LengthVal`
    // (length/%/auto), no `f32` como margin/padding — firma aparte.
    let inset_two: Option<(fn(LengthVal) -> DeclKind, fn(LengthVal) -> DeclKind)> =
        match lower.as_str() {
            "inset-inline" => Some((InsetLeft, InsetRight)),
            "inset-block" => Some((InsetTop, InsetBottom)),
            _ => None,
        };
    if let Some((a, b)) = inset_two {
        let parts: Vec<&str> = value.split_whitespace().collect();
        let vals: Vec<LengthVal> =
            parts.iter().filter_map(|p| parse_length_or_pct_or_auto(p)).collect();
        if vals.is_empty() || vals.len() != parts.len() {
            return Some(Vec::new());
        }
        let (s, e) = if vals.len() == 1 { (vals[0], vals[0]) } else { (vals[0], vals[1]) };
        return Some(vec![
            Decl { kind: a(s), important },
            Decl { kind: b(e), important },
        ]);
    }
    let inset_single: Option<fn(LengthVal) -> DeclKind> = match lower.as_str() {
        "inset-inline-start" => Some(InsetLeft),
        "inset-inline-end" => Some(InsetRight),
        "inset-block-start" => Some(InsetTop),
        "inset-block-end" => Some(InsetBottom),
        _ => None,
    };
    if let Some(ctor) = inset_single {
        return Some(
            parse_length_or_pct_or_auto(value)
                .map(|v| vec![Decl { kind: ctor(v), important }])
                .unwrap_or_default(),
        );
    }
    // Lados emparejados (1–2 valores): (start_ctor, end_ctor).
    let two: Option<(fn(f32) -> DeclKind, fn(f32) -> DeclKind)> = match lower.as_str() {
        "margin-inline" => Some((MarginLeft, MarginRight)),
        "margin-block" => Some((MarginTop, MarginBottom)),
        "padding-inline" => Some((PaddingLeft, PaddingRight)),
        "padding-block" => Some((PaddingTop, PaddingBottom)),
        _ => None,
    };
    if let Some((a, b)) = two {
        let parts: Vec<&str> = value.split_whitespace().collect();
        let vals: Vec<f32> = parts.iter().filter_map(|p| parse_length_px(p)).collect();
        if vals.is_empty() || vals.len() != parts.len() {
            return Some(Vec::new());
        }
        let (s, e) = if vals.len() == 1 { (vals[0], vals[0]) } else { (vals[0], vals[1]) };
        return Some(vec![
            Decl { kind: a(s), important },
            Decl { kind: b(e), important },
        ]);
    }
    // Un solo lado (`-start`/`-end`).
    let single: Option<fn(f32) -> DeclKind> = match lower.as_str() {
        "margin-inline-start" => Some(MarginLeft),
        "margin-inline-end" => Some(MarginRight),
        "margin-block-start" => Some(MarginTop),
        "margin-block-end" => Some(MarginBottom),
        "padding-inline-start" => Some(PaddingLeft),
        "padding-inline-end" => Some(PaddingRight),
        "padding-block-start" => Some(PaddingTop),
        "padding-block-end" => Some(PaddingBottom),
        _ => None,
    };
    let ctor = single?;
    Some(
        parse_length_px(value)
            .map(|v| vec![Decl { kind: ctor(v), important }])
            .unwrap_or_default(),
    )
}

/// `inset: <t> [r] [b] [l]` — 1..4 valores con la distribución de `margin`
/// (1→todos, 2→TB/LR, 3→T/LR/B, 4→TRBL). Cada valor acepta length/%/auto.
/// Expande a los cuatro longhands `top`/`right`/`bottom`/`left`. Fase 7.189.
pub(crate) fn parse_inset_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    let vals: Vec<LengthVal> =
        parts.iter().filter_map(|p| parse_length_or_pct_or_auto(p)).collect();
    // Si algún token no parsea, descartamos el shorthand entero (CSS spec).
    if vals.is_empty() || vals.len() != parts.len() {
        return Vec::new();
    }
    let (t, r, b, l) = match vals.as_slice() {
        [a] => (*a, *a, *a, *a),
        [a, b2] => (*a, *b2, *a, *b2),
        [a, b2, c] => (*a, *b2, *c, *b2),
        [a, b2, c, d, ..] => (*a, *b2, *c, *d),
        [] => return Vec::new(),
    };
    vec![
        Decl { kind: DeclKind::InsetTop(t), important },
        Decl { kind: DeclKind::InsetRight(r), important },
        Decl { kind: DeclKind::InsetBottom(b), important },
        Decl { kind: DeclKind::InsetLeft(l), important },
    ]
}

/// `flex-flow: <direction> || <wrap>` (en cualquier orden) → `flex-direction`
/// + `flex-wrap`. Fase 7.189.
pub(crate) fn parse_flex_flow_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let mut out = Vec::new();
    for tok in value.split_whitespace() {
        if let Some(d) = parse_flex_direction(tok) {
            out.push(Decl { kind: DeclKind::FlexDirection(d), important });
        } else if let Some(w) = parse_flex_wrap(tok) {
            out.push(Decl { kind: DeclKind::FlexWrap(w), important });
        }
    }
    out
}

pub(crate) fn parse_flex_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let v = value.trim().to_ascii_lowercase();
    let (grow, shrink, basis) = if v == "none" {
        (0.0_f32, 0.0_f32, LengthVal::Auto)
    } else if v == "auto" {
        (1.0_f32, 1.0_f32, LengthVal::Auto)
    } else if v == "initial" {
        (0.0_f32, 1.0_f32, LengthVal::Auto)
    } else {
        let parts: Vec<&str> = value.split_whitespace().collect();
        match parts.as_slice() {
            [g] => {
                // `flex: 1` ⇒ `1 1 0%`
                let Some(g) = g.parse::<f32>().ok() else {
                    return Vec::new();
                };
                (g, 1.0, LengthVal::Pct(0.0))
            }
            [g, s_or_b] => {
                let Some(g) = g.parse::<f32>().ok() else {
                    return Vec::new();
                };
                // El segundo puede ser shrink (número solo) o basis (longitud).
                if let Some(b) = parse_length_or_pct(s_or_b) {
                    (g, 1.0, b)
                } else if let Some(s) = s_or_b.parse::<f32>().ok() {
                    (g, s, LengthVal::Pct(0.0))
                } else {
                    return Vec::new();
                }
            }
            [g, s, b] => {
                let Some(g) = g.parse::<f32>().ok() else {
                    return Vec::new();
                };
                let Some(s) = s.parse::<f32>().ok() else {
                    return Vec::new();
                };
                let Some(b) = parse_length_or_pct(b) else {
                    return Vec::new();
                };
                (g, s, b)
            }
            _ => return Vec::new(),
        }
    };
    vec![
        Decl { kind: DeclKind::FlexGrow(grow), important },
        Decl { kind: DeclKind::FlexShrink(shrink), important },
        Decl { kind: DeclKind::FlexBasis(basis), important },
    ]
}

/// `outline: <width> <style> <color>`. Tokens en cualquier orden.
pub(crate) fn parse_outline_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let mut width: Option<f32> = None;
    let mut color: Option<Color> = None;
    let mut current: bool = false;
    let mut style_active: Option<bool> = None;
    for tok in value.split_whitespace() {
        if !current && color.is_none() && is_current_color(tok) {
            current = true;
            continue;
        }
        if width.is_none() {
            if let Some(w) = parse_length_px(tok) {
                width = Some(w);
                continue;
            }
        }
        if style_active.is_none() {
            if let Some(active) = parse_border_style(tok) {
                style_active = Some(active);
                continue;
            }
        }
        if color.is_none() {
            if let Some(c) = parse_color(tok) {
                color = Some(c);
                continue;
            }
        }
    }
    let mut out = Vec::new();
    let active = style_active.unwrap_or(true);
    if !active {
        // `outline-style: none` apaga: width=0 + color=None.
        out.push(Decl { kind: DeclKind::OutlineStyle(false), important });
        return out;
    }
    if let Some(w) = width {
        out.push(Decl { kind: DeclKind::OutlineWidth(w), important });
    }
    if current {
        out.push(Decl { kind: DeclKind::CurrentColor(ColorTarget::Outline), important });
    } else if let Some(c) = color {
        out.push(Decl { kind: DeclKind::OutlineColor(c), important });
    }
    if style_active.is_some() {
        out.push(Decl { kind: DeclKind::OutlineStyle(true), important });
    }
    out
}

/// `background-image: linear-gradient(...)` o `none`. Devuelve un
/// `DeclKind` listo (Background o BackgroundGradient o None).
pub(crate) fn parse_background_image(value: &str) -> Option<DeclKind> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(DeclKind::BackgroundGradientNone);
    }
    if let Some(args) = strip_fn(v, "linear-gradient") {
        return parse_linear_gradient(args).map(DeclKind::BackgroundGradient);
    }
    if let Some(args) = strip_fn(v, "url") {
        // url('foo') / url("foo") / url(foo) — trimea comillas.
        let raw = args.trim();
        let unquoted = raw
            .strip_prefix('"').and_then(|s| s.strip_suffix('"'))
            .or_else(|| raw.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
            .unwrap_or(raw);
        let url = unquoted.trim();
        if url.is_empty() {
            return None;
        }
        return Some(DeclKind::BackgroundImageUrl(url.to_string()));
    }
    // Otros gradientes (`radial-gradient`, `conic-gradient`) o `cross-fade`
    // no soportados — silencio.
    None
}

/// `background-size`: `cover` | `contain` | `auto` | `<x> [<y>]`. Cada eje
/// acepta length/%/auto; un solo valor deja el segundo en `auto` (el chrome
/// deriva el otro por aspecto). Valores no reconocidos → None (decl ignorada).
pub(crate) fn parse_background_size(value: &str) -> Option<DeclKind> {
    let v = value.trim();
    match v.to_ascii_lowercase().as_str() {
        "cover" => return Some(DeclKind::BackgroundSize(BackgroundSize::Cover)),
        "contain" => return Some(DeclKind::BackgroundSize(BackgroundSize::Contain)),
        "auto" => return Some(DeclKind::BackgroundSize(BackgroundSize::Auto)),
        _ => {}
    }
    let toks: Vec<&str> = v.split_whitespace().collect();
    let sz = match toks.as_slice() {
        [x] => BackgroundSize::Explicit {
            x: parse_length_or_pct(x)?,
            y: LengthVal::Auto,
        },
        [x, y] => BackgroundSize::Explicit {
            x: parse_length_or_pct(x)?,
            y: parse_length_or_pct(y)?,
        },
        _ => return None,
    };
    Some(DeclKind::BackgroundSize(sz))
}

/// `background-repeat`: `repeat` | `no-repeat` | `repeat-x` | `repeat-y`, más
/// la sintaxis de dos valores (`repeat no-repeat` = sólo X, etc.). `space` y
/// `round` se aproximan a `repeat` (sin spacing/scaling fino).
pub(crate) fn parse_background_repeat(value: &str) -> Option<DeclKind> {
    let v = value.trim().to_ascii_lowercase();
    let r = match v.as_str() {
        "repeat" | "space" | "round" => BackgroundRepeat::Repeat,
        "no-repeat" => BackgroundRepeat::NoRepeat,
        "repeat-x" => BackgroundRepeat::RepeatX,
        "repeat-y" => BackgroundRepeat::RepeatY,
        other => {
            let axis = |t: &str| match t {
                "repeat" | "space" | "round" => Some(true),
                "no-repeat" => Some(false),
                _ => None,
            };
            let toks: Vec<&str> = other.split_whitespace().collect();
            match toks.as_slice() {
                [x, y] => match (axis(x)?, axis(y)?) {
                    (true, true) => BackgroundRepeat::Repeat,
                    (false, false) => BackgroundRepeat::NoRepeat,
                    (true, false) => BackgroundRepeat::RepeatX,
                    (false, true) => BackgroundRepeat::RepeatY,
                },
                _ => return None,
            }
        }
    };
    Some(DeclKind::BackgroundRepeat(r))
}

/// `background-origin`: `border-box` | `padding-box` | `content-box`.
pub(crate) fn parse_background_origin(value: &str) -> Option<DeclKind> {
    let o = match value.trim().to_ascii_lowercase().as_str() {
        "border-box" => BackgroundOrigin::BorderBox,
        "padding-box" => BackgroundOrigin::PaddingBox,
        "content-box" => BackgroundOrigin::ContentBox,
        _ => return None,
    };
    Some(DeclKind::BackgroundOrigin(o))
}

/// `background-clip`: `border-box` | `padding-box` | `content-box` | `text`.
/// `text` recorta el fondo a las glifos (Fase 7.208).
pub(crate) fn parse_background_clip(value: &str) -> Option<DeclKind> {
    let c = match value.trim().to_ascii_lowercase().as_str() {
        "border-box" => BackgroundClip::BorderBox,
        "padding-box" => BackgroundClip::PaddingBox,
        "content-box" => BackgroundClip::ContentBox,
        "text" => BackgroundClip::Text,
        _ => return None,
    };
    Some(DeclKind::BackgroundClip(c))
}

/// `background-position`: 1–2 valores. Keywords se mapean a %: `left`/`top`=0%,
/// `center`=50%, `right`/`bottom`=100%. Soporta el orden invertido por keyword
/// (`top left` ↔ `left top`); con lengths/% el orden es posicional (x, y). Un
/// solo valor deja el otro eje en `center` (50%).
pub(crate) fn parse_background_position(value: &str) -> Option<DeclKind> {
    // Devuelve (valor, Some(true)=keyword horizontal, Some(false)=vertical,
    // None=ambiguo: `center`, length o %).
    fn token(t: &str) -> Option<(LengthVal, Option<bool>)> {
        match t.to_ascii_lowercase().as_str() {
            "left" => Some((LengthVal::Pct(0.0), Some(true))),
            "right" => Some((LengthVal::Pct(100.0), Some(true))),
            "top" => Some((LengthVal::Pct(0.0), Some(false))),
            "bottom" => Some((LengthVal::Pct(100.0), Some(false))),
            "center" => Some((LengthVal::Pct(50.0), None)),
            other => parse_length_or_pct(other).map(|l| (l, None)),
        }
    }
    let toks: Vec<&str> = value.trim().split_whitespace().collect();
    let pos = match toks.as_slice() {
        [a] => {
            let (la, axis) = token(a)?;
            // Un keyword vertical solo (`top`/`bottom`) fija Y; el resto fija X.
            if axis == Some(false) {
                BackgroundPosition { x: LengthVal::Pct(50.0), y: la }
            } else {
                BackgroundPosition { x: la, y: LengthVal::Pct(50.0) }
            }
        }
        [a, b] => {
            let (la, aa) = token(a)?;
            let (lb, ab) = token(b)?;
            // Si los keywords explicitan ejes invertidos (`top left`, `center
            // right`), reordenar para que x sea siempre el horizontal.
            if aa == Some(false) || ab == Some(true) {
                BackgroundPosition { x: lb, y: la }
            } else {
                BackgroundPosition { x: la, y: lb }
            }
        }
        _ => return None,
    };
    Some(DeclKind::BackgroundPosition(pos))
}

/// Las piezas de UNA capa de `background` ya clasificadas. Los longhands
/// (color/image/size/position/repeat) salen de los value-parsers existentes;
/// `None` = la pieza no apareció en esa capa.
struct BgLayerParts {
    color: Option<Color>,
    /// `BackgroundImageUrl` | `BackgroundGradient` | `BackgroundGradientNone`.
    image: Option<DeclKind>,
    size: Option<BackgroundSize>,
    position: Option<BackgroundPosition>,
    repeat: Option<BackgroundRepeat>,
    /// `background-origin` (1ª caja del shorthand). `None` = no apareció.
    origin: Option<BackgroundOrigin>,
    /// `background-clip` (2ª caja del shorthand, o la 1ª si es la única).
    clip: Option<BackgroundClip>,
}

/// Clasifica los tokens de UNA capa de `background` (un segmento sin comas).
/// Tokeniza respetando paréntesis (`url(...)`/gradiente quedan enteros) y
/// separa el `/` (position / size) aunque venga pegado (`center/cover`).
/// `scroll`/`fixed`/`local` (attachment) y `*-box` (origin/clip) se aceptan y
/// se descartan (no se modelan). Lo que sobra se intenta parsear como color.
fn classify_background_layer(layer: &str) -> BgLayerParts {
    let mut tokens: Vec<String> = Vec::new();
    for t in split_top_level_ws(layer.trim()) {
        if t.contains('/') && !t.contains('(') {
            let mut buf = String::new();
            for ch in t.chars() {
                if ch == '/' {
                    if !buf.is_empty() {
                        tokens.push(std::mem::take(&mut buf));
                    }
                    tokens.push("/".to_string());
                } else {
                    buf.push(ch);
                }
            }
            if !buf.is_empty() {
                tokens.push(buf);
            }
        } else {
            tokens.push(t);
        }
    }

    let mut parts = BgLayerParts {
        color: None,
        image: None,
        size: None,
        position: None,
        repeat: None,
        origin: None,
        clip: None,
    };
    let mut pos_tokens: Vec<String> = Vec::new();
    let mut size_tokens: Vec<String> = Vec::new();
    // Cajas (`*-box`) en orden de aparición: la 1ª es origin, la 2ª clip.
    let mut box_tokens: Vec<String> = Vec::new();
    let mut after_slash = false;

    for t in &tokens {
        if t == "/" {
            after_slash = true;
            continue;
        }
        if after_slash {
            size_tokens.push(t.clone());
            continue;
        }
        let lt = t.to_ascii_lowercase();
        if lt.starts_with("url(") || lt.starts_with("linear-gradient(") || lt == "none" {
            if let Some(k) = parse_background_image(t) {
                parts.image = Some(k);
            }
            continue;
        }
        if matches!(
            lt.as_str(),
            "repeat" | "no-repeat" | "repeat-x" | "repeat-y" | "space" | "round"
        ) {
            if let Some(DeclKind::BackgroundRepeat(r)) = parse_background_repeat(t) {
                parts.repeat = Some(r);
            }
            continue;
        }
        // attachment (`scroll`/`fixed`/`local`) se acepta y descarta.
        if matches!(lt.as_str(), "scroll" | "fixed" | "local") {
            continue;
        }
        // `*-box` → origin (1ª) / clip (2ª). Se resuelven tras el loop.
        if matches!(lt.as_str(), "border-box" | "padding-box" | "content-box") {
            box_tokens.push(lt);
            continue;
        }
        if matches!(lt.as_str(), "left" | "right" | "top" | "bottom" | "center")
            || parse_length_or_pct(t).is_some()
        {
            pos_tokens.push(t.clone());
            continue;
        }
        if let Some(c) = parse_color(t) {
            parts.color = Some(c);
        }
    }
    if !pos_tokens.is_empty() {
        if let Some(DeclKind::BackgroundPosition(p)) =
            parse_background_position(&pos_tokens.join(" "))
        {
            parts.position = Some(p);
        }
    }
    if !size_tokens.is_empty() {
        if let Some(DeclKind::BackgroundSize(s)) = parse_background_size(&size_tokens.join(" ")) {
            parts.size = Some(s);
        }
    }
    // Cajas: 1ª = origin, 2ª = clip. Con una sola, fija ambas (spec). El
    // origin y el clip son enums distintos pero con los mismos 3 valores.
    if let Some(o) = box_tokens.first() {
        if let Some(DeclKind::BackgroundOrigin(v)) = parse_background_origin(o) {
            parts.origin = Some(v);
        }
        // La 2ª caja da el clip; si no hay 2ª, la 1ª también es el clip.
        let clip_tok = box_tokens.get(1).unwrap_or(o);
        if let Some(DeclKind::BackgroundClip(v)) = parse_background_clip(clip_tok) {
            parts.clip = Some(v);
        }
    }
    parts
}

/// Convierte las piezas de una capa EXTRA (índice ≥ 1) en un [`BackgroundLayer`].
/// Una capa sin imagen no pinta nada → `None` (se descarta). Los longhands
/// omitidos caen a sus defaults CSS (`auto` / `0% 0%` / `repeat`).
fn extra_layer_from_parts(parts: &BgLayerParts) -> Option<BackgroundLayer> {
    let image = match &parts.image {
        Some(DeclKind::BackgroundImageUrl(u)) => BackgroundImage::Url(u.clone()),
        Some(DeclKind::BackgroundGradient(g)) => BackgroundImage::Gradient(g.clone()),
        // `none` u otra cosa → sin imagen pintable.
        _ => return None,
    };
    Some(BackgroundLayer {
        image,
        size: parts.size.unwrap_or(BackgroundSize::Auto),
        position: parts.position.unwrap_or(BackgroundPosition {
            x: LengthVal::Pct(0.0),
            y: LengthVal::Pct(0.0),
        }),
        repeat: parts.repeat.unwrap_or(BackgroundRepeat::Repeat),
    })
}

/// Shorthand `background:` — expande a los longhands de cada capa. La PRIMERA
/// capa (la de más arriba en CSS) va a los campos `background_*` sueltos; las
/// capas 2..N (separadas por coma) a `BackgroundExtraLayers`. Reusa los
/// value-parsers de cada sub-propiedad. Cada capa: `<color> || <image> ||
/// <position> [ / <size> ] || <repeat> || <attachment> || <box>` (attachment y
/// origin/clip se aceptan y descartan). El color sólo cuenta en la última capa
/// (semántica CSS). Siempre emite `BackgroundExtraLayers` (posiblemente vacía)
/// para resetear las capas de una regla previa. Los demás longhands se emiten
/// sólo si la capa 0 los trae (igual que el shorthand `border`).
pub(crate) fn parse_background_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let layers = split_top_level_comma(value);
    let mut out = Vec::new();
    let mut extra: Vec<BackgroundLayer> = Vec::new();
    // El color de la última capa que lo declare (en CSS sólo la final lo lleva).
    let mut last_color: Option<Color> = None;

    for (i, layer) in layers.iter().enumerate() {
        let parts = classify_background_layer(layer);
        if let Some(c) = parts.color {
            last_color = Some(c);
        }
        if i == 0 {
            // Capa 0 → longhands sueltos.
            if let Some(k) = parts.image {
                out.push(Decl { kind: k, important });
            }
            if let Some(p) = parts.position {
                out.push(Decl { kind: DeclKind::BackgroundPosition(p), important });
            }
            if let Some(s) = parts.size {
                out.push(Decl { kind: DeclKind::BackgroundSize(s), important });
            }
            if let Some(r) = parts.repeat {
                out.push(Decl { kind: DeclKind::BackgroundRepeat(r), important });
            }
            if let Some(o) = parts.origin {
                out.push(Decl { kind: DeclKind::BackgroundOrigin(o), important });
            }
            if let Some(c) = parts.clip {
                out.push(Decl { kind: DeclKind::BackgroundClip(c), important });
            }
        } else if let Some(l) = extra_layer_from_parts(&parts) {
            extra.push(l);
        }
    }

    if let Some(c) = last_color {
        out.push(Decl { kind: DeclKind::Background(c), important });
    }
    // Siempre (incluso vacía) — resetea capas extra de una regla previa.
    out.push(Decl { kind: DeclKind::BackgroundExtraLayers(extra), important });
    out
}

/// Longhand `background-image: a, b, c` con varias capas. Capa 0 → su DeclKind
/// de imagen suelto; capas 2..N → `BackgroundExtraLayers` con size/position/
/// repeat por default (los longhands hermanos `background-size:`/etc. en lista
/// NO se zipean por capa todavía — sólo afectan la capa 0). Sólo se invoca
/// cuando el value trae ≥2 capas (sino cae al path normal de un solo valor).
pub(crate) fn parse_background_image_list(value: &str, important: bool) -> Vec<Decl> {
    let layers = split_top_level_comma(value);
    let mut out = Vec::new();
    let mut extra: Vec<BackgroundLayer> = Vec::new();
    for (i, layer) in layers.iter().enumerate() {
        let img = parse_background_image(layer.trim());
        if i == 0 {
            if let Some(k) = img {
                out.push(Decl { kind: k, important });
            }
        } else {
            let image = match img {
                Some(DeclKind::BackgroundImageUrl(u)) => BackgroundImage::Url(u),
                Some(DeclKind::BackgroundGradient(g)) => BackgroundImage::Gradient(g),
                _ => continue,
            };
            extra.push(BackgroundLayer {
                image,
                size: BackgroundSize::Auto,
                position: BackgroundPosition { x: LengthVal::Pct(0.0), y: LengthVal::Pct(0.0) },
                repeat: BackgroundRepeat::Repeat,
            });
        }
    }
    out.push(Decl { kind: DeclKind::BackgroundExtraLayers(extra), important });
    out
}

/// Parsea el contenido de `linear-gradient(...)`. Sintaxis aceptada:
/// - `linear-gradient(<angle>?, <stop>, <stop>, ...)`
/// - `linear-gradient(to <side>?, <stop>, <stop>, ...)`
/// `<angle>` en `Ndeg` o `Nturn` (turn × 360 = grados). Default 180
/// (top→bottom). `to right`=90, `to left`=270, `to top`=0, `to bottom`=180,
/// combinaciones diagonales (`to top right`=45) también. Stops: `<color>
/// <pos>?` donde pos es `N%` o `Npx`.
pub(crate) fn parse_linear_gradient(args: &str) -> Option<LinearGradient> {
    let parts: Vec<&str> = args.split(',').map(|s| s.trim()).collect();
    if parts.len() < 2 {
        return None;
    }
    let (angle_deg, stops_start) = parse_gradient_direction(parts[0]);
    let stops_start_idx = if angle_deg.is_some() { 1 } else { 0 };
    let angle_deg = angle_deg.unwrap_or(180.0);
    let mut stops: Vec<GradientStop> = Vec::new();
    for raw in &parts[stops_start_idx..] {
        if let Some(s) = parse_gradient_stop(raw) {
            stops.push(s);
        }
    }
    if stops.len() < 2 {
        return None;
    }
    let _ = stops_start;
    Some(LinearGradient { angle_deg, stops })
}

/// Si el token es una dirección/ángulo válido devuelve `(Some(deg),
/// true)`; si no encaja, `(None, false)` para que el caller lo trate
/// como stop.
pub(crate) fn parse_gradient_direction(s: &str) -> (Option<f32>, bool) {
    let s = s.trim();
    let lower = s.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("to ") {
        let deg = match rest.trim() {
            "top" => 0.0,
            "right" => 90.0,
            "bottom" => 180.0,
            "left" => 270.0,
            "top right" | "right top" => 45.0,
            "bottom right" | "right bottom" => 135.0,
            "bottom left" | "left bottom" => 225.0,
            "top left" | "left top" => 315.0,
            _ => return (None, false),
        };
        return (Some(deg), true);
    }
    if let Some(num) = lower.strip_suffix("deg") {
        if let Ok(v) = num.trim().parse::<f32>() {
            return (Some(v), true);
        }
    }
    if let Some(num) = lower.strip_suffix("turn") {
        if let Ok(v) = num.trim().parse::<f32>() {
            return (Some(v * 360.0), true);
        }
    }
    (None, false)
}

pub(crate) fn parse_gradient_stop(s: &str) -> Option<GradientStop> {
    let s = s.trim();
    let parts: Vec<&str> = s.split_whitespace().collect();
    match parts.as_slice() {
        [c] => Some(GradientStop { color: parse_color(c)?, pos: None }),
        [c, p] => {
            let color = parse_color(c)?;
            let pos = if let Some(pct) = p.strip_suffix('%') {
                pct.trim().parse::<f32>().ok().map(|v| (v / 100.0).clamp(0.0, 1.0))
            } else if let Some(px) = parse_length_px(p) {
                // Aproximación: tratamos px como 0..1 dividiendo por 100.
                // En el wild la mayoría usa %, así que esta heurística
                // raramente importa.
                Some((px / 100.0).clamp(0.0, 1.0))
            } else {
                None
            };
            Some(GradientStop { color, pos })
        }
        _ => None,
    }
}

/// Acepta `12px`, `1.5rem` (tratada como em*16), `0`. Sin unidad → px.
/// `Nvw`/`Nvh`/`Nvmin`/`Nvmax` resuelven contra el viewport activo
/// ([`resolve_viewport`]): el real bajo un `ViewportScope` (carga normal),
/// `DEFAULT_VIEWPORT` fuera de él (parsers sueltos en tests).
pub(crate) fn parse_length_px(s: &str) -> Option<f32> {
    let s = s.trim();
    if s == "0" {
        return Some(0.0);
    }
    if let Some(num) = s.strip_suffix("px") {
        return num.trim().parse().ok();
    }
    if let Some(num) = s.strip_suffix("rem") {
        let v: f32 = num.trim().parse().ok()?;
        return Some(v * 16.0);
    }
    if let Some(num) = s.strip_suffix("em") {
        let v: f32 = num.trim().parse().ok()?;
        return Some(v * 16.0);
    }
    if let Some(num) = s.strip_suffix("vmin") {
        let v: f32 = num.trim().parse().ok()?;
        let vp = resolve_viewport();
        return Some(v * vp.width.min(vp.height) / 100.0);
    }
    if let Some(num) = s.strip_suffix("vmax") {
        let v: f32 = num.trim().parse().ok()?;
        let vp = resolve_viewport();
        return Some(v * vp.width.max(vp.height) / 100.0);
    }
    if let Some(num) = s.strip_suffix("vw") {
        let v: f32 = num.trim().parse().ok()?;
        return Some(v * resolve_viewport().width / 100.0);
    }
    if let Some(num) = s.strip_suffix("vh") {
        let v: f32 = num.trim().parse().ok()?;
        return Some(v * resolve_viewport().height / 100.0);
    }
    s.parse().ok()
}

/// `length`, `%` o `auto`. Variante para insets que sí admiten `auto`.
pub(crate) fn parse_length_or_pct_or_auto(s: &str) -> Option<LengthVal> {
    parse_length_or_pct(s.trim())
}

pub(crate) fn parse_position(s: &str) -> Option<Position> {
    match s.trim().to_ascii_lowercase().as_str() {
        "static" => Some(Position::Static),
        "relative" => Some(Position::Relative),
        "absolute" => Some(Position::Absolute),
        "fixed" => Some(Position::Fixed),
        "sticky" => Some(Position::Sticky),
        _ => None,
    }
}

pub(crate) fn parse_vertical_align(s: &str) -> Option<VerticalAlign> {
    match s.trim().to_ascii_lowercase().as_str() {
        "baseline" => Some(VerticalAlign::Baseline),
        "top" | "text-top" => Some(VerticalAlign::Top),
        "middle" => Some(VerticalAlign::Middle),
        "bottom" | "text-bottom" => Some(VerticalAlign::Bottom),
        "super" => Some(VerticalAlign::Super),
        "sub" => Some(VerticalAlign::Sub),
        _ => None,
    }
}

pub(crate) fn parse_visibility(s: &str) -> Option<Visibility> {
    match s.trim().to_ascii_lowercase().as_str() {
        "visible" => Some(Visibility::Visible),
        // `collapse` lo tratamos igual que hidden (sólo aplica a
        // tablas/flex en CSS spec, aproximación segura).
        "hidden" | "collapse" => Some(Visibility::Hidden),
        _ => None,
    }
}

pub(crate) fn parse_pointer_events(s: &str) -> Option<PointerEvents> {
    match s.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(PointerEvents::Auto),
        "none" => Some(PointerEvents::None),
        _ => None,
    }
}

/// `text-shadow: <x> <y> [blur] <color>[, <x> <y> [blur] <color>]*`.
/// `none` → vector vacío. Devuelve None si ningún shadow es válido.
pub(crate) fn parse_text_shadows(value: &str) -> Option<Vec<TextShadow>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    for sh in v.split(',') {
        if let Some(s) = parse_one_text_shadow(sh) {
            out.push(s);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

pub(crate) fn parse_one_text_shadow(s: &str) -> Option<TextShadow> {
    let mut lengths: Vec<f32> = Vec::with_capacity(3);
    let mut color: Option<Color> = None;
    for tok in s.split_whitespace() {
        if let Some(l) = parse_length_px(tok) {
            lengths.push(l);
            continue;
        }
        if let Some(c) = parse_color(tok) {
            color = Some(c);
            continue;
        }
    }
    if lengths.len() < 2 {
        return None;
    }
    Some(TextShadow {
        offset_x: lengths[0],
        offset_y: lengths[1],
        blur_px: lengths.get(2).copied().unwrap_or(0.0),
        color: color.unwrap_or(Color::BLACK),
    })
}

/// `transform: none` o cadena de funciones (`rotate(45deg) scale(2)
/// translate(10px, 20px)`). Acepta `translate(x)`, `translate(x, y)`,
/// `translateX(x)`, `translateY(y)`, `scale(s)`, `scale(sx, sy)`,
/// `scaleX(sx)`, `scaleY(sy)`, `rotate(Ndeg|Nrad|Nturn)`.
pub(crate) fn parse_transforms(value: &str) -> Option<Vec<Transform>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    let mut rest = v;
    while !rest.trim().is_empty() {
        rest = rest.trim_start();
        let open = rest.find('(')?;
        let name = rest[..open].trim().to_ascii_lowercase();
        let mut depth = 1usize;
        let bytes = rest[open + 1..].as_bytes();
        let mut close = None;
        for (i, &c) in bytes.iter().enumerate() {
            match c {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let close = close?;
        let args = &rest[open + 1..open + 1 + close];
        let tr = parse_transform_fn(&name, args)?;
        out.push(tr);
        rest = &rest[open + 1 + close + 1..];
    }
    Some(out)
}

pub(crate) fn parse_transform_fn(name: &str, args: &str) -> Option<Transform> {
    let parts: Vec<&str> = args.split(',').map(|s| s.trim()).collect();
    match name {
        "translate" => match parts.as_slice() {
            [x] => Some(Transform::Translate(parse_length_px(x)?, 0.0)),
            [x, y] => Some(Transform::Translate(parse_length_px(x)?, parse_length_px(y)?)),
            _ => None,
        },
        "translatex" => Some(Transform::Translate(parse_length_px(parts[0])?, 0.0)),
        "translatey" => Some(Transform::Translate(0.0, parse_length_px(parts[0])?)),
        "scale" => match parts.as_slice() {
            [s] => {
                let v = s.parse::<f32>().ok()?;
                Some(Transform::Scale(v, v))
            }
            [sx, sy] => {
                Some(Transform::Scale(sx.parse().ok()?, sy.parse().ok()?))
            }
            _ => None,
        },
        "scalex" => Some(Transform::Scale(parts[0].parse().ok()?, 1.0)),
        "scaley" => Some(Transform::Scale(1.0, parts[0].parse().ok()?)),
        "rotate" => {
            let arg = parts[0];
            let deg = if let Some(n) = arg.strip_suffix("deg") {
                n.trim().parse::<f32>().ok()?
            } else if let Some(n) = arg.strip_suffix("rad") {
                let v: f32 = n.trim().parse().ok()?;
                v.to_degrees()
            } else if let Some(n) = arg.strip_suffix("turn") {
                let v: f32 = n.trim().parse().ok()?;
                v * 360.0
            } else {
                // Sin unidad: asumir deg.
                arg.parse::<f32>().ok()?
            };
            Some(Transform::Rotate(deg))
        }
        "skew" => match parts.as_slice() {
            [x] => Some(Transform::Skew(parse_hue(x)?, 0.0)),
            [x, y] => Some(Transform::Skew(parse_hue(x)?, parse_hue(y)?)),
            _ => None,
        },
        "skewx" => Some(Transform::Skew(parse_hue(parts[0])?, 0.0)),
        "skewy" => Some(Transform::Skew(0.0, parse_hue(parts[0])?)),
        "matrix" => match parts.as_slice() {
            [a, b, c, d, e, f] => Some(Transform::Matrix(
                a.parse().ok()?,
                b.parse().ok()?,
                c.parse().ok()?,
                d.parse().ok()?,
                e.parse().ok()?,
                f.parse().ok()?,
            )),
            _ => None,
        },
        _ => None,
    }
}

/// `grid-template-columns: <track-list>`. Subset soportado:
/// - `auto`
/// - `Npx` / `N%`
/// - `Nfr`
/// - `repeat(N, <track>)` con repeat de un solo track
/// Tokens separados por whitespace.
pub(crate) fn parse_grid_template(value: &str) -> Option<Vec<GridTrackSize>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    let mut out: Vec<GridTrackSize> = Vec::new();
    // Tokenize: respeta nesting de paréntesis para repeat(N, X).
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    for c in v.chars() {
        match c {
            '(' => {
                depth += 1;
                current.push(c);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    for tok in tokens {
        if let Some(inner) = strip_fn(&tok, "repeat") {
            let parts: Vec<&str> = inner.splitn(2, ',').collect();
            if parts.len() != 2 {
                continue;
            }
            let count: i32 = parts[0].trim().parse().ok()?;
            let track = parse_one_grid_track(parts[1].trim())?;
            for _ in 0..count.max(0) {
                out.push(track);
            }
        } else if let Some(t) = parse_one_grid_track(&tok) {
            out.push(t);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

pub(crate) fn parse_one_grid_track(s: &str) -> Option<GridTrackSize> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("auto") {
        return Some(GridTrackSize::Auto);
    }
    if let Some(num) = s.strip_suffix("fr") {
        let v: f32 = num.trim().parse().ok()?;
        return Some(GridTrackSize::Fr(v));
    }
    if let Some(lv) = parse_length_or_pct(s) {
        return Some(match lv {
            LengthVal::Px(v) => GridTrackSize::Px(v),
            LengthVal::Pct(v) => GridTrackSize::Pct(v),
            LengthVal::Auto => GridTrackSize::Auto,
        });
    }
    None
}

/// Evalúa una condición de `@media` contra el viewport por defecto. Subset:
/// `(max-width: Npx)`, `(min-width: Npx)`, encadenados por ` and `.
/// `screen`/`all` se ignoran (siempre true).
/// Evalúa una media query (`@media` en CSS y `window.matchMedia()` en JS) contra
/// el viewport actual. Soporta listas separadas por `,` (OR), `not`/`only`,
/// el combinador ` and `, tipos de media (`screen`/`all`/`print`/`speech`) y
/// las features: `min/max/exact-width`, `min/max/exact-height`, `orientation`
/// (portrait/landscape), `min/max/exact-resolution` (`Ndppx`/`Ndpi`/`Nx` vs
/// `vp.dpr`) y `prefers-color-scheme`/`prefers-reduced-motion` (reportamos
/// light / no-reduce). Features desconocidas se ignoran (no descalifican), igual
/// que el comportamiento previo, para no romper CSS que las use de forma
/// progresiva. Pública porque el chrome (`puriy-llimphi`) la reusa para resolver
/// `matchMedia` contra el viewport real de la ventana.
pub fn evaluate_media_query(condition: &str, vp: Viewport) -> bool {
    let cond = condition.trim().to_ascii_lowercase();
    if cond.is_empty() {
        return true;
    }
    // Media query LIST: separada por comas, matchea si CUALQUIER componente lo hace.
    if cond.contains(',') {
        return cond.split(',').any(|q| evaluate_media_query(q, vp));
    }
    // `not` a nivel de query invierte el resultado completo.
    if let Some(rest) = cond.strip_prefix("not ") {
        return !evaluate_media_query_terms(rest.trim(), vp);
    }
    evaluate_media_query_terms(&cond, vp)
}

/// Evalúa los términos unidos por ` and ` de una query ya sin `,`/`not` de tope.
pub(crate) fn evaluate_media_query_terms(cond: &str, vp: Viewport) -> bool {
    for part in cond.split(" and ").map(|s| s.trim()) {
        if part.is_empty() {
            continue;
        }
        // Tipos de media.
        if part == "all" || part == "screen" {
            continue;
        }
        if part == "print" || part == "speech" || part == "tty" {
            return false;
        }
        let part = part.strip_prefix("only ").unwrap_or(part).trim();
        // Esperamos `(feature)` o `(feature: value)`.
        let Some(inner) = part.strip_prefix('(').and_then(|s| s.strip_suffix(')')) else {
            // Token no reconocido (tipo de media raro): no matchea.
            return false;
        };
        if !evaluate_media_feature(inner.trim(), vp) {
            return false;
        }
    }
    true
}

/// Comparador de la sintaxis de rango de Media Queries 4.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum RangeCmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
}

impl RangeCmp {
    fn apply(self, lhs: f32, rhs: f32) -> bool {
        match self {
            RangeCmp::Lt => lhs < rhs,
            RangeCmp::Le => lhs <= rhs,
            RangeCmp::Gt => lhs > rhs,
            RangeCmp::Ge => lhs >= rhs,
            RangeCmp::Eq => (lhs - rhs).abs() < 0.5,
        }
    }
    /// Invierte el sentido (para `value op feature` → `feature flip(op) value`).
    fn flip(self) -> RangeCmp {
        match self {
            RangeCmp::Lt => RangeCmp::Gt,
            RangeCmp::Le => RangeCmp::Ge,
            RangeCmp::Gt => RangeCmp::Lt,
            RangeCmp::Ge => RangeCmp::Le,
            RangeCmp::Eq => RangeCmp::Eq,
        }
    }
}

/// Valor actual de una media feature de rango contra el viewport.
fn range_feature_current(name: &str, vp: Viewport) -> Option<f32> {
    match name {
        "width" | "inline-size" => Some(vp.width),
        "height" | "block-size" => Some(vp.height),
        "aspect-ratio" => Some(vp.width / vp.height),
        "resolution" => Some(vp.dpr),
        _ => None,
    }
}

/// Parsea el valor de comparación según la feature de rango.
fn range_feature_value(name: &str, val: &str) -> Option<f32> {
    match name {
        "width" | "inline-size" | "height" | "block-size" => parse_length_px(val),
        "aspect-ratio" => parse_aspect_ratio(val),
        "resolution" => parse_resolution_dppx(val),
        _ => None,
    }
}

/// Intenta evaluar la sintaxis de rango de MQ4: `(width >= 600px)`,
/// `(600px < width)`, `(400px <= width <= 800px)`. `None` si el `inner` no
/// es una expresión de rango (lo maneja el path `feature: value`).
pub(crate) fn try_eval_media_range(inner: &str, vp: Viewport) -> Option<bool> {
    // Sólo es rango si hay un comparador `<`/`>`/`=` (el path normal usa `:`).
    if !inner.contains(['<', '>', '=']) {
        return None;
    }
    // Tokeniza en palabras y comparadores (con o sin espacios).
    let mut words: Vec<String> = Vec::new();
    let mut ops: Vec<RangeCmp> = Vec::new();
    let mut order: Vec<bool> = Vec::new(); // true = word, false = op
    let bytes = inner.as_bytes();
    let mut i = 0;
    let mut cur = String::new();
    let flush = |cur: &mut String, words: &mut Vec<String>, order: &mut Vec<bool>| {
        let t = cur.trim();
        if !t.is_empty() {
            words.push(t.to_string());
            order.push(true);
        }
        cur.clear();
    };
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'<' || c == b'>' || c == b'=' {
            flush(&mut cur, &mut words, &mut order);
            let op = if (c == b'<' || c == b'>') && bytes.get(i + 1) == Some(&b'=') {
                i += 2;
                if c == b'<' { RangeCmp::Le } else { RangeCmp::Ge }
            } else {
                i += 1;
                match c {
                    b'<' => RangeCmp::Lt,
                    b'>' => RangeCmp::Gt,
                    _ => RangeCmp::Eq,
                }
            };
            ops.push(op);
            order.push(false);
            continue;
        }
        cur.push(c as char);
        i += 1;
    }
    flush(&mut cur, &mut words, &mut order);
    // Patrón válido: alterna word/op empezando y terminando en word.
    let alternating_ok = order.iter().enumerate().all(|(idx, is_word)| *is_word == (idx % 2 == 0));
    if !alternating_ok {
        return None;
    }
    match (words.as_slice(), ops.as_slice()) {
        // `feature op value` o `value op feature`.
        ([a, b], [op]) => {
            if let Some(cur) = range_feature_current(a, vp) {
                let v = range_feature_value(a, b)?;
                Some(op.apply(cur, v))
            } else if let Some(cur) = range_feature_current(b, vp) {
                let v = range_feature_value(b, a)?;
                Some(op.flip().apply(cur, v))
            } else {
                None
            }
        }
        // `v1 op1 feature op2 v2` (la feature está en el medio).
        ([v1, f, v2], [op1, op2]) => {
            let cur = range_feature_current(f, vp)?;
            let lo = range_feature_value(f, v1)?;
            let hi = range_feature_value(f, v2)?;
            Some(op1.flip().apply(cur, lo) && op2.apply(cur, hi))
        }
        _ => None,
    }
}

/// Evalúa UNA feature `(feature)` o `(feature: value)` contra el viewport.
pub(crate) fn evaluate_media_feature(inner: &str, vp: Viewport) -> bool {
    // Sintaxis de rango MQ4 (`width >= 600px`, `400px <= width <= 800px`).
    if let Some(r) = try_eval_media_range(inner, vp) {
        return r;
    }
    let Some((feature, val)) = inner.split_once(':').map(|(a, b)| (a.trim(), b.trim())) else {
        // Feature booleana (sin valor): matchea si la capacidad "existe".
        return matches!(inner, "color" | "grid" | "hover" | "pointer");
    };
    match feature {
        "max-width" => parse_length_px(val).is_some_and(|l| vp.width <= l),
        "min-width" => parse_length_px(val).is_some_and(|l| vp.width >= l),
        "width" => parse_length_px(val).is_some_and(|l| (vp.width - l).abs() < 0.5),
        "max-height" => parse_length_px(val).is_some_and(|l| vp.height <= l),
        "min-height" => parse_length_px(val).is_some_and(|l| vp.height >= l),
        "height" => parse_length_px(val).is_some_and(|l| (vp.height - l).abs() < 0.5),
        "orientation" => match val {
            "portrait" => vp.height >= vp.width,
            "landscape" => vp.width > vp.height,
            _ => false,
        },
        "min-resolution" => parse_resolution_dppx(val).is_some_and(|r| vp.dpr >= r),
        "max-resolution" => parse_resolution_dppx(val).is_some_and(|r| vp.dpr <= r),
        "resolution" => parse_resolution_dppx(val).is_some_and(|r| (vp.dpr - r).abs() < 0.01),
        "min-aspect-ratio" => {
            parse_aspect_ratio(val).is_some_and(|r| vp.width / vp.height >= r)
        }
        "max-aspect-ratio" => {
            parse_aspect_ratio(val).is_some_and(|r| vp.width / vp.height <= r)
        }
        "aspect-ratio" => {
            parse_aspect_ratio(val).is_some_and(|r| (vp.width / vp.height - r).abs() < 0.01)
        }
        // Preferencias del usuario: reportamos tema claro y sin reducción.
        "prefers-color-scheme" => val == "light" || val == "no-preference",
        "prefers-reduced-motion" => val == "no-preference",
        "prefers-contrast" => val == "no-preference",
        "hover" => val == "hover",
        "any-hover" => val == "hover",
        "pointer" => val == "fine",
        "any-pointer" => val == "fine",
        // Feature desconocida: no descalifica (comportamiento previo lenient).
        _ => true,
    }
}

/// Parsea un aspect-ratio de media query a un float `ancho/alto`. Acepta la
/// forma `W/H` (`16/9`) y el número suelto (`1.5`). `None` si no parsea o el
/// alto es cero.
pub(crate) fn parse_aspect_ratio(val: &str) -> Option<f32> {
    let v = val.trim();
    if let Some((w, h)) = v.split_once('/') {
        let w: f32 = w.trim().parse().ok()?;
        let h: f32 = h.trim().parse().ok()?;
        if h == 0.0 {
            return None;
        }
        Some(w / h)
    } else {
        v.parse::<f32>().ok()
    }
}

/// Parsea una resolución de media query a `dppx` (dots per px). Acepta
/// `Ndppx`, `Nx` (alias de dppx) y `Ndpi` (96dpi = 1dppx). `None` si no parsea.
pub(crate) fn parse_resolution_dppx(val: &str) -> Option<f32> {
    let v = val.trim();
    if let Some(n) = v.strip_suffix("dppx").or_else(|| v.strip_suffix('x')) {
        n.trim().parse::<f32>().ok()
    } else if let Some(n) = v.strip_suffix("dpi") {
        n.trim().parse::<f32>().ok().map(|d| d / 96.0)
    } else if let Some(n) = v.strip_suffix("dpcm") {
        n.trim().parse::<f32>().ok().map(|d| d / 96.0 * 2.54)
    } else {
        None
    }
}

/// Evalúa una condición `@supports`: una declaración `(prop: value)` es
/// soportada si nuestro parser la convierte a algún `DeclKind`. Soporta
/// `and`/`or`/`not`, agrupación con paréntesis y `selector(<sel>)`
/// (recursivo). Las keywords se reconocen en minúsculas.
pub(crate) fn evaluate_supports_query(condition: &str) -> bool {
    let cond = condition.trim();
    // `not <cond>`.
    if let Some(rest) = strip_supports_not(cond) {
        return !evaluate_supports_query(rest);
    }
    // `a and b and ...` (a nivel de paréntesis 0).
    let and_parts = split_supports(cond, "and");
    if and_parts.len() > 1 {
        return and_parts.iter().all(|p| evaluate_supports_query(p));
    }
    // `a or b or ...`.
    let or_parts = split_supports(cond, "or");
    if or_parts.len() > 1 {
        return or_parts.iter().any(|p| evaluate_supports_query(p));
    }
    // `selector(<sel>)` — soportado si el selector parsea.
    if let Some(sel) = cond
        .strip_prefix("selector(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return parse_selector(sel.trim()).is_some();
    }
    // Grupo o declaración entre paréntesis.
    if let Some(inner) = strip_supports_parens(cond) {
        let inner = inner.trim();
        if let Some((prop, val)) = split_top_colon(inner) {
            return decl_kind_from_pair(prop.trim(), val.trim()).is_some();
        }
        // Agrupación `( <cond> )`.
        return evaluate_supports_query(inner);
    }
    false
}

/// `not <cond>` / `not(<cond>)` (whitespace o `(` tras el keyword).
fn strip_supports_not(s: &str) -> Option<&str> {
    let rest = s.trim().strip_prefix("not")?;
    let c = rest.chars().next()?;
    (c.is_whitespace() || c == '(').then(|| rest.trim_start())
}

/// Divide `s` por ` kw ` (whitespace a ambos lados) a profundidad de
/// paréntesis 0. Devuelve `[s]` si no hay separador.
fn split_supports<'a>(s: &'a str, kw: &str) -> Vec<&'a str> {
    let bytes = s.as_bytes();
    let kwb = kw.as_bytes();
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b' ' if depth == 0 => {
                let j = i + 1;
                if bytes[j..].starts_with(kwb) && bytes.get(j + kwb.len()) == Some(&b' ') {
                    parts.push(s[start..i].trim());
                    i = j + kwb.len() + 1;
                    start = i;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(s[start..].trim());
    parts
}

/// Si `s` está envuelto por un par de paréntesis que se cierran al final
/// (no `(a) ... (b)`), devuelve el interior.
fn strip_supports_parens(s: &str) -> Option<&str> {
    let s = s.trim();
    let inner = s.strip_prefix('(')?.strip_suffix(')')?;
    let mut depth = 0i32;
    for c in inner.chars() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth < 0 {
                    return None; // se cierra antes del final → no envuelve todo
                }
            }
            _ => {}
        }
    }
    (depth == 0).then_some(inner)
}

/// Primer `:` a profundidad de paréntesis 0 → `(prop, value)`.
fn split_top_colon(s: &str) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    for (i, b) in s.bytes().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b':' if depth == 0 => return Some((&s[..i], &s[i + 1..])),
            _ => {}
        }
    }
    None
}

/// Indica que `cssparser` está enlazado aunque el subset actual no use
/// la API completa — la presencia del crate evita que `cargo` lo
/// pruebe y deja el camino abierto para Fase 3.
#[doc(hidden)]
pub fn _cssparser_anchor() {
    let _ = cssparser::ParserInput::new("");
}
