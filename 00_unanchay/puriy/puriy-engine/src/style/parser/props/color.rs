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
    // Fase 7.850 — `light-dark(<claro>, <oscuro>)` (CSS Color Adjustment 1).
    // La resolución correcta depende del color-scheme USADO del elemento, que
    // `parse_color` (context-free) no conoce. El motor reporta
    // `prefers-color-scheme: light` (ver `props/media.rs`), así que
    // resolvemos al primer argumento (esquema claro). El switch a oscuro no
    // está cableado todavía.
    if let Some(args) = strip_fn(s, "light-dark") {
        return parse_light_dark(args);
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
/// `light-dark(<claro>, <oscuro>)`. Resuelve al argumento claro (primero) —
/// ver la nota en [`parse_color`]. Exige exactamente 2 colores válidos; si
/// alguno no parsea, la declaración entera se descarta. Fase 7.850.
pub(crate) fn parse_light_dark(args: &str) -> Option<Color> {
    let segments = split_top_level_comma(args);
    if segments.len() != 2 {
        return None;
    }
    // Validamos ambos (el oscuro debe ser un color real aunque no lo usemos),
    // pero devolvemos el claro.
    let light = parse_color(segments[0].trim())?;
    let _dark = parse_color(segments[1].trim())?;
    Some(light)
}

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
    // Fase 7.847 — tokeniza respetando paréntesis para no partir `calc(…)`
    // por sus espacios internos; cada token acepta calc/min/max/clamp.
    let parts = split_top_level_ws(value);
    let parsed: Vec<f32> = parts
        .iter()
        .map(|t| parse_length_px_or_calc(t))
        .collect::<Option<Vec<_>>>()?;
    Some(match parsed.as_slice() {
        [a] => Sides::all(*a),
        [v, h] => Sides { top: *v, right: *h, bottom: *v, left: *h },
        [t, h, b] => Sides { top: *t, right: *h, bottom: *b, left: *h },
        [t, r, b, l] => Sides { top: *t, right: *r, bottom: *b, left: *l },
        _ => return None,
    })
}

// Tabla completa de color keywords CSS3 extendidos (147 nombres + `grey`
// y sus variantes + `transparent`). Antes sólo había 33 — un keyword
// ausente dropeaba la declaración entera y el elemento salía mal pintado.
// Fase 7.224. Valores de la spec CSS Color Module.
const NAMED_COLORS: &[(&str, Color)] = &[
    ("transparent", Color::TRANSPARENT),
    ("aliceblue", Color::rgb_const(240, 248, 255)),
    ("antiquewhite", Color::rgb_const(250, 235, 215)),
    ("aqua", Color::rgb_const(0, 255, 255)),
    ("aquamarine", Color::rgb_const(127, 255, 212)),
    ("azure", Color::rgb_const(240, 255, 255)),
    ("beige", Color::rgb_const(245, 245, 220)),
    ("bisque", Color::rgb_const(255, 228, 196)),
    ("black", Color::BLACK),
    ("blanchedalmond", Color::rgb_const(255, 235, 205)),
    ("blue", Color::rgb_const(0, 0, 255)),
    ("blueviolet", Color::rgb_const(138, 43, 226)),
    ("brown", Color::rgb_const(165, 42, 42)),
    ("burlywood", Color::rgb_const(222, 184, 135)),
    ("cadetblue", Color::rgb_const(95, 158, 160)),
    ("chartreuse", Color::rgb_const(127, 255, 0)),
    ("chocolate", Color::rgb_const(210, 105, 30)),
    ("coral", Color::rgb_const(255, 127, 80)),
    ("cornflowerblue", Color::rgb_const(100, 149, 237)),
    ("cornsilk", Color::rgb_const(255, 248, 220)),
    ("crimson", Color::rgb_const(220, 20, 60)),
    ("cyan", Color::rgb_const(0, 255, 255)),
    ("darkblue", Color::rgb_const(0, 0, 139)),
    ("darkcyan", Color::rgb_const(0, 139, 139)),
    ("darkgoldenrod", Color::rgb_const(184, 134, 11)),
    ("darkgray", Color::rgb_const(169, 169, 169)),
    ("darkgrey", Color::rgb_const(169, 169, 169)),
    ("darkgreen", Color::rgb_const(0, 100, 0)),
    ("darkkhaki", Color::rgb_const(189, 183, 107)),
    ("darkmagenta", Color::rgb_const(139, 0, 139)),
    ("darkolivegreen", Color::rgb_const(85, 107, 47)),
    ("darkorange", Color::rgb_const(255, 140, 0)),
    ("darkorchid", Color::rgb_const(153, 50, 204)),
    ("darkred", Color::rgb_const(139, 0, 0)),
    ("darksalmon", Color::rgb_const(233, 150, 122)),
    ("darkseagreen", Color::rgb_const(143, 188, 143)),
    ("darkslateblue", Color::rgb_const(72, 61, 139)),
    ("darkslategray", Color::rgb_const(47, 79, 79)),
    ("darkslategrey", Color::rgb_const(47, 79, 79)),
    ("darkturquoise", Color::rgb_const(0, 206, 209)),
    ("darkviolet", Color::rgb_const(148, 0, 211)),
    ("deeppink", Color::rgb_const(255, 20, 147)),
    ("deepskyblue", Color::rgb_const(0, 191, 255)),
    ("dimgray", Color::rgb_const(105, 105, 105)),
    ("dimgrey", Color::rgb_const(105, 105, 105)),
    ("dodgerblue", Color::rgb_const(30, 144, 255)),
    ("firebrick", Color::rgb_const(178, 34, 34)),
    ("floralwhite", Color::rgb_const(255, 250, 240)),
    ("forestgreen", Color::rgb_const(34, 139, 34)),
    ("fuchsia", Color::rgb_const(255, 0, 255)),
    ("gainsboro", Color::rgb_const(220, 220, 220)),
    ("ghostwhite", Color::rgb_const(248, 248, 255)),
    ("gold", Color::rgb_const(255, 215, 0)),
    ("goldenrod", Color::rgb_const(218, 165, 32)),
    ("gray", Color::rgb_const(128, 128, 128)),
    ("grey", Color::rgb_const(128, 128, 128)),
    ("green", Color::rgb_const(0, 128, 0)),
    ("greenyellow", Color::rgb_const(173, 255, 47)),
    ("honeydew", Color::rgb_const(240, 255, 240)),
    ("hotpink", Color::rgb_const(255, 105, 180)),
    ("indianred", Color::rgb_const(205, 92, 92)),
    ("indigo", Color::rgb_const(75, 0, 130)),
    ("ivory", Color::rgb_const(255, 255, 240)),
    ("khaki", Color::rgb_const(240, 230, 140)),
    ("lavender", Color::rgb_const(230, 230, 250)),
    ("lavenderblush", Color::rgb_const(255, 240, 245)),
    ("lawngreen", Color::rgb_const(124, 252, 0)),
    ("lemonchiffon", Color::rgb_const(255, 250, 205)),
    ("lightblue", Color::rgb_const(173, 216, 230)),
    ("lightcoral", Color::rgb_const(240, 128, 128)),
    ("lightcyan", Color::rgb_const(224, 255, 255)),
    ("lightgoldenrodyellow", Color::rgb_const(250, 250, 210)),
    ("lightgray", Color::rgb_const(211, 211, 211)),
    ("lightgrey", Color::rgb_const(211, 211, 211)),
    ("lightgreen", Color::rgb_const(144, 238, 144)),
    ("lightpink", Color::rgb_const(255, 182, 193)),
    ("lightsalmon", Color::rgb_const(255, 160, 122)),
    ("lightseagreen", Color::rgb_const(32, 178, 170)),
    ("lightskyblue", Color::rgb_const(135, 206, 250)),
    ("lightslategray", Color::rgb_const(119, 136, 153)),
    ("lightslategrey", Color::rgb_const(119, 136, 153)),
    ("lightsteelblue", Color::rgb_const(176, 196, 222)),
    ("lightyellow", Color::rgb_const(255, 255, 224)),
    ("lime", Color::rgb_const(0, 255, 0)),
    ("limegreen", Color::rgb_const(50, 205, 50)),
    ("linen", Color::rgb_const(250, 240, 230)),
    ("magenta", Color::rgb_const(255, 0, 255)),
    ("maroon", Color::rgb_const(128, 0, 0)),
    ("mediumaquamarine", Color::rgb_const(102, 205, 170)),
    ("mediumblue", Color::rgb_const(0, 0, 205)),
    ("mediumorchid", Color::rgb_const(186, 85, 211)),
    ("mediumpurple", Color::rgb_const(147, 112, 219)),
    ("mediumseagreen", Color::rgb_const(60, 179, 113)),
    ("mediumslateblue", Color::rgb_const(123, 104, 238)),
    ("mediumspringgreen", Color::rgb_const(0, 250, 154)),
    ("mediumturquoise", Color::rgb_const(72, 209, 204)),
    ("mediumvioletred", Color::rgb_const(199, 21, 133)),
    ("midnightblue", Color::rgb_const(25, 25, 112)),
    ("mintcream", Color::rgb_const(245, 255, 250)),
    ("mistyrose", Color::rgb_const(255, 228, 225)),
    ("moccasin", Color::rgb_const(255, 228, 181)),
    ("navajowhite", Color::rgb_const(255, 222, 173)),
    ("navy", Color::rgb_const(0, 0, 128)),
    ("oldlace", Color::rgb_const(253, 245, 230)),
    ("olive", Color::rgb_const(128, 128, 0)),
    ("olivedrab", Color::rgb_const(107, 142, 35)),
    ("orange", Color::rgb_const(255, 165, 0)),
    ("orangered", Color::rgb_const(255, 69, 0)),
    ("orchid", Color::rgb_const(218, 112, 214)),
    ("palegoldenrod", Color::rgb_const(238, 232, 170)),
    ("palegreen", Color::rgb_const(152, 251, 152)),
    ("paleturquoise", Color::rgb_const(175, 238, 238)),
    ("palevioletred", Color::rgb_const(219, 112, 147)),
    ("papayawhip", Color::rgb_const(255, 239, 213)),
    ("peachpuff", Color::rgb_const(255, 218, 185)),
    ("peru", Color::rgb_const(205, 133, 63)),
    ("pink", Color::rgb_const(255, 192, 203)),
    ("plum", Color::rgb_const(221, 160, 221)),
    ("powderblue", Color::rgb_const(176, 224, 230)),
    ("purple", Color::rgb_const(128, 0, 128)),
    ("rebeccapurple", Color::rgb_const(102, 51, 153)),
    ("red", Color::rgb_const(255, 0, 0)),
    ("rosybrown", Color::rgb_const(188, 143, 143)),
    ("royalblue", Color::rgb_const(65, 105, 225)),
    ("saddlebrown", Color::rgb_const(139, 69, 19)),
    ("salmon", Color::rgb_const(250, 128, 114)),
    ("sandybrown", Color::rgb_const(244, 164, 96)),
    ("seagreen", Color::rgb_const(46, 139, 87)),
    ("seashell", Color::rgb_const(255, 245, 238)),
    ("sienna", Color::rgb_const(160, 82, 45)),
    ("silver", Color::rgb_const(192, 192, 192)),
    ("skyblue", Color::rgb_const(135, 206, 235)),
    ("slateblue", Color::rgb_const(106, 90, 205)),
    ("slategray", Color::rgb_const(112, 128, 144)),
    ("slategrey", Color::rgb_const(112, 128, 144)),
    ("snow", Color::rgb_const(255, 250, 250)),
    ("springgreen", Color::rgb_const(0, 255, 127)),
    ("steelblue", Color::rgb_const(70, 130, 180)),
    ("tan", Color::rgb_const(210, 180, 140)),
    ("teal", Color::rgb_const(0, 128, 128)),
    ("thistle", Color::rgb_const(216, 191, 216)),
    ("tomato", Color::rgb_const(255, 99, 71)),
    ("turquoise", Color::rgb_const(64, 224, 208)),
    ("violet", Color::rgb_const(238, 130, 238)),
    ("wheat", Color::rgb_const(245, 222, 179)),
    ("white", Color::WHITE),
    ("whitesmoke", Color::rgb_const(245, 245, 245)),
    ("yellow", Color::rgb_const(255, 255, 0)),
    ("yellowgreen", Color::rgb_const(154, 205, 50)),
    // Fase 7.862 — colores de sistema (CSS Color 4 §System Colors). Sin tema
    // de UA real, los resolvemos a valores fijos de un esquema claro estándar.
    // Cubren los `<system-color>` que aparecen en hojas modernas y resets.
    ("canvas", Color::WHITE),
    ("canvastext", Color::BLACK),
    ("linktext", Color::rgb_const(0, 0, 238)),
    ("visitedtext", Color::rgb_const(85, 26, 139)),
    ("activetext", Color::rgb_const(255, 0, 0)),
    ("buttonface", Color::rgb_const(240, 240, 240)),
    ("buttontext", Color::BLACK),
    ("buttonborder", Color::rgb_const(118, 118, 118)),
    ("field", Color::WHITE),
    ("fieldtext", Color::BLACK),
    ("highlight", Color::rgb_const(51, 153, 255)),
    ("highlighttext", Color::WHITE),
    ("selecteditem", Color::rgb_const(51, 153, 255)),
    ("selecteditemtext", Color::WHITE),
    ("mark", Color::rgb_const(255, 255, 0)),
    ("marktext", Color::BLACK),
    ("graytext", Color::rgb_const(128, 128, 128)),
    ("accentcolor", Color::rgb_const(51, 153, 255)),
    ("accentcolortext", Color::WHITE),
    ("windowtext", Color::BLACK),
    ("window", Color::WHITE),
];
