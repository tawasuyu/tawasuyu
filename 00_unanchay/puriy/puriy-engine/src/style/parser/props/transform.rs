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
        // Fase 7.839 — funciones 3D proyectadas al plano 2D (no hay pipeline
        // de perspectiva real). Antes, una sola de éstas en la cadena tiraba el
        // `transform` entero. Aproximamos: se conserva la componente que SÍ vive
        // en 2D y las puramente fuera-de-plano quedan en identidad.
        "translate3d" => match parts.as_slice() {
            [x, y, _z] => {
                Some(Transform::Translate(parse_length_px(x)?, parse_length_px(y)?))
            }
            _ => None,
        },
        "translatez" => parse_length_px(parts.first()?).map(|_| Transform::Translate(0.0, 0.0)),
        "scale3d" => match parts.as_slice() {
            [sx, sy, _sz] => Some(Transform::Scale(sx.parse().ok()?, sy.parse().ok()?)),
            _ => None,
        },
        "scalez" => parts.first()?.parse::<f32>().ok().map(|_| Transform::Scale(1.0, 1.0)),
        // Rotación fuera del plano (X/Y): validamos el ángulo pero sin giro 2D.
        "rotatex" | "rotatey" => parse_hue(parts.first()?).map(|_| Transform::Rotate(0.0)),
        "rotatez" => parse_hue(parts.first()?).map(Transform::Rotate),
        "rotate3d" => match parts.as_slice() {
            [x, y, z, a] => {
                let (x, y, z) =
                    (x.parse::<f32>().ok()?, y.parse::<f32>().ok()?, z.parse::<f32>().ok()?);
                let deg = parse_hue(a)?;
                // Sólo el eje Z gira en el plano; X/Y → identidad.
                if x == 0.0 && y == 0.0 && z != 0.0 {
                    Some(Transform::Rotate(deg))
                } else {
                    Some(Transform::Rotate(0.0))
                }
            }
            _ => None,
        },
        // `perspective(<len>|none)` sola no produce matriz 2D → identidad
        // (validamos el argumento para no tragar basura).
        "perspective" => {
            let a = parts.first()?.trim();
            if a.eq_ignore_ascii_case("none") || parse_length_px(a).is_some() {
                Some(Transform::Scale(1.0, 1.0))
            } else {
                None
            }
        }
        // `matrix3d(<16>)`: proyección de la 4×4 a la afín 2D (m11,m12,m21,m22
        // y la traslación x/y de la 4ª columna; column-major).
        "matrix3d" => {
            if parts.len() == 16 {
                let p = |i: usize| parts[i].parse::<f32>().ok();
                Some(Transform::Matrix(p(0)?, p(1)?, p(4)?, p(5)?, p(12)?, p(13)?))
            } else {
                None
            }
        }
        _ => None,
    }
}
