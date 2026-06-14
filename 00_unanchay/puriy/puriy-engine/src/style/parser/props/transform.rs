use super::*;

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
        out.extend(parse_transform_fn(&name, args)?);
        rest = &rest[open + 1 + close + 1..];
    }
    Some(out)
}

/// Parsea un eje de `translate` (`<length-percentage>`) en `(px, pct)`:
/// exactamente uno es no-cero (o ambos cero). El `%` no es una longitud, así
/// que `parse_length_px` lo rechaza — lo capturamos primero. `pct` en %
/// (50.0 = 50%). `None` si no parsea.
fn translate_axis(s: &str) -> Option<(f32, f32)> {
    let s = s.trim();
    if let Some(p) = s.strip_suffix('%') {
        return p.trim().parse::<f32>().ok().map(|n| (0.0, n));
    }
    parse_length_px(s).map(|px| (px, 0.0))
}

/// Construye los `Transform` de un `translate` a partir de sus componentes px
/// y %: como las traslaciones conmutan, separar la parte px (`Translate`) de
/// la % (`TranslatePct`) en dos entradas adyacentes es equivalente a la
/// traslación combinada. `translate(0,0)` emite un `Translate(0,0)` (no-op).
fn build_translate((px, py): (f32, f32), (pctx, pcty): (f32, f32)) -> Vec<Transform> {
    let mut v = Vec::new();
    if pctx != 0.0 || pcty != 0.0 {
        v.push(Transform::TranslatePct(pctx, pcty));
    }
    if px != 0.0 || py != 0.0 || v.is_empty() {
        v.push(Transform::Translate(px, py));
    }
    v
}

/// Devuelve 0..N `Transform` para una función (translate puede dar 2; el resto
/// 1). `parse_transforms` aplana con `extend`.
pub(crate) fn parse_transform_fn(name: &str, args: &str) -> Option<Vec<Transform>> {
    let parts: Vec<&str> = args.split(',').map(|s| s.trim()).collect();
    let one = |t: Transform| Some(vec![t]);
    match name {
        "translate" => match parts.as_slice() {
            [x] => {
                let (xpx, xpct) = translate_axis(x)?;
                Some(build_translate((xpx, 0.0), (xpct, 0.0)))
            }
            [x, y] => {
                let (xpx, xpct) = translate_axis(x)?;
                let (ypx, ypct) = translate_axis(y)?;
                Some(build_translate((xpx, ypx), (xpct, ypct)))
            }
            _ => None,
        },
        "translatex" => {
            let (px, pct) = translate_axis(parts[0])?;
            Some(build_translate((px, 0.0), (pct, 0.0)))
        }
        "translatey" => {
            let (px, pct) = translate_axis(parts[0])?;
            Some(build_translate((0.0, px), (0.0, pct)))
        }
        "scale" => match parts.as_slice() {
            [s] => {
                let v = s.parse::<f32>().ok()?;
                one(Transform::Scale(v, v))
            }
            [sx, sy] => one(Transform::Scale(sx.parse().ok()?, sy.parse().ok()?)),
            _ => None,
        },
        "scalex" => one(Transform::Scale(parts[0].parse().ok()?, 1.0)),
        "scaley" => one(Transform::Scale(1.0, parts[0].parse().ok()?)),
        // Fase 7.875 — `parse_hue` cubre deg/rad/grad/turn, `none`, sin-unidad
        // (→deg) y ahora `calc()`. Reemplaza el strip manual.
        "rotate" => one(Transform::Rotate(parse_hue(parts[0])?)),
        "skew" => match parts.as_slice() {
            [x] => one(Transform::Skew(parse_hue(x)?, 0.0)),
            [x, y] => one(Transform::Skew(parse_hue(x)?, parse_hue(y)?)),
            _ => None,
        },
        "skewx" => one(Transform::Skew(parse_hue(parts[0])?, 0.0)),
        "skewy" => one(Transform::Skew(0.0, parse_hue(parts[0])?)),
        "matrix" => match parts.as_slice() {
            [a, b, c, d, e, f] => one(Transform::Matrix(
                a.parse().ok()?,
                b.parse().ok()?,
                c.parse().ok()?,
                d.parse().ok()?,
                e.parse().ok()?,
                f.parse().ok()?,
            )),
            _ => None,
        },
        // Fase 7.839 — funciones 3D proyectadas al plano 2D (no hay pipeline
        // de perspectiva real). Antes, una sola de éstas en la cadena tiraba el
        // `transform` entero. Aproximamos: se conserva la componente que SÍ vive
        // en 2D y las puramente fuera-de-plano quedan en identidad.
        "translate3d" => match parts.as_slice() {
            [x, y, _z] => {
                let (xpx, xpct) = translate_axis(x)?;
                let (ypx, ypct) = translate_axis(y)?;
                Some(build_translate((xpx, ypx), (xpct, ypct)))
            }
            _ => None,
        },
        "translatez" => parse_length_px(parts.first()?).map(|_| vec![Transform::Translate(0.0, 0.0)]),
        "scale3d" => match parts.as_slice() {
            [sx, sy, _sz] => one(Transform::Scale(sx.parse().ok()?, sy.parse().ok()?)),
            _ => None,
        },
        "scalez" => parts.first()?.parse::<f32>().ok().map(|_| vec![Transform::Scale(1.0, 1.0)]),
        // Rotación fuera del plano (X/Y): validamos el ángulo pero sin giro 2D.
        "rotatex" | "rotatey" => parse_hue(parts.first()?).map(|_| vec![Transform::Rotate(0.0)]),
        "rotatez" => one(Transform::Rotate(parse_hue(parts.first()?)?)),
        "rotate3d" => match parts.as_slice() {
            [x, y, z, a] => {
                let (x, y, z) =
                    (x.parse::<f32>().ok()?, y.parse::<f32>().ok()?, z.parse::<f32>().ok()?);
                let deg = parse_hue(a)?;
                // Sólo el eje Z gira en el plano; X/Y → identidad.
                if x == 0.0 && y == 0.0 && z != 0.0 {
                    one(Transform::Rotate(deg))
                } else {
                    one(Transform::Rotate(0.0))
                }
            }
            _ => None,
        },
        // `perspective(<len>|none)` sola no produce matriz 2D → identidad
        // (validamos el argumento para no tragar basura).
        "perspective" => {
            let a = parts.first()?.trim();
            if a.eq_ignore_ascii_case("none") || parse_length_px(a).is_some() {
                one(Transform::Scale(1.0, 1.0))
            } else {
                None
            }
        }
        // `matrix3d(<16>)`: proyección de la 4×4 a la afín 2D (m11,m12,m21,m22
        // y la traslación x/y de la 4ª columna; column-major).
        "matrix3d" => {
            if parts.len() == 16 {
                let p = |i: usize| parts[i].parse::<f32>().ok();
                one(Transform::Matrix(p(0)?, p(1)?, p(4)?, p(5)?, p(12)?, p(13)?))
            } else {
                None
            }
        }
        _ => None,
    }
}
