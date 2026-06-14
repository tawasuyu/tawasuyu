use super::*;

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
    if let Some(args) = strip_fn(v, "radial-gradient") {
        return parse_radial_gradient(args).map(DeclKind::BackgroundGradient);
    }
    if let Some(args) = strip_fn(v, "conic-gradient") {
        return parse_conic_gradient(args).map(DeclKind::BackgroundGradient);
    }
    // `repeating-*-gradient(...)`: mismo parser, con el flag `repeating`.
    if let Some(args) = strip_fn(v, "repeating-linear-gradient") {
        return parse_linear_gradient(args).map(mark_repeating).map(DeclKind::BackgroundGradient);
    }
    if let Some(args) = strip_fn(v, "repeating-radial-gradient") {
        return parse_radial_gradient(args).map(mark_repeating).map(DeclKind::BackgroundGradient);
    }
    if let Some(args) = strip_fn(v, "repeating-conic-gradient") {
        return parse_conic_gradient(args).map(mark_repeating).map(DeclKind::BackgroundGradient);
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
    // Fase 7.870 — `image-set(...)` (elige por resolución/tipo) y
    // `cross-fade(...)` (mezcla de imágenes). Sin pipeline para elegir/mezclar,
    // tomamos la PRIMERA `url(...)` de los argumentos (el candidato base).
    for fname in ["image-set", "-webkit-image-set", "cross-fade", "-webkit-cross-fade"] {
        if let Some(args) = strip_fn(v, fname) {
            if let Some(url) = first_url_in(args) {
                return Some(DeclKind::BackgroundImageUrl(url));
            }
        }
    }
    // `paint()`/`element()` no soportados — silencio.
    None
}

/// Extrae la 1ª `url(...)` que aparezca dentro de `s` (desquotada). `None` si
/// no hay ninguna. Usada por `image-set`/`cross-fade`. Fase 7.870.
/// Fase 7.900 — `image-set("a.png" 1x, …)` admite el URL como string pelado
/// sin `url(...)`; si no hay `url(`, tomamos la 1ª string entrecomillada.
fn first_url_in(s: &str) -> Option<String> {
    if let Some(start) = s.find("url(") {
        let after = &s[start + 4..];
        let close = after.find(')')?;
        let raw = after[..close].trim();
        let unquoted = raw
            .strip_prefix('"').and_then(|x| x.strip_suffix('"'))
            .or_else(|| raw.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')))
            .unwrap_or(raw)
            .trim();
        return (!unquoted.is_empty()).then(|| unquoted.to_string());
    }
    // Sin `url(...)`: 1ª string entrecomillada (sintaxis moderna de image-set).
    for quote in ['"', '\''] {
        if let Some(open) = s.find(quote) {
            let after = &s[open + 1..];
            if let Some(close) = after.find(quote) {
                let inner = after[..close].trim();
                if !inner.is_empty() {
                    return Some(inner.to_string());
                }
            }
        }
    }
    None
}

/// Marca un gradiente como `repeating-*` (Fase 7.228).
fn mark_repeating(mut g: LinearGradient) -> LinearGradient {
    g.repeating = true;
    g
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
    // Fase 7.877 — tokeniza respetando paréntesis (calc).
    let toks_owned = split_top_level_ws(v);
    let toks: Vec<&str> = toks_owned.iter().map(String::as_str).collect();
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
    // Fase 7.877 — tokeniza respetando paréntesis (calc con espacios internos).
    let toks_owned = split_top_level_ws(value.trim());
    let toks: Vec<&str> = toks_owned.iter().map(String::as_str).collect();
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
        // Fase 7.860 — forma de 3-4 tokens con offset por borde
        // (`right 10px bottom 20px`, `left 25% top`). Cada eje es un keyword
        // de borde + un offset opcional.
        _ => return parse_bg_position_edges(&toks),
    };
    Some(DeclKind::BackgroundPosition(pos))
}

/// `<position>` de 3-4 tokens: `[ left|right|center ] [<len-pct>]? &&
/// [ top|bottom|center ] [<len-pct>]?`. El offset desde un borde a `100%`
/// (`right`/`bottom`): si es `%` se invierte (`right 20%` → `80%`); si es
/// `px` no es representable como un único `LengthVal` (haría falta
/// `calc(100% - px)`) y se aproxima al borde. Fase 7.860.
fn parse_bg_position_edges(toks: &[&str]) -> Option<DeclKind> {
    fn edge_val(at_end: bool, off: Option<LengthVal>) -> LengthVal {
        match off {
            None => LengthVal::Pct(if at_end { 100.0 } else { 0.0 }),
            Some(LengthVal::Pct(p)) => LengthVal::Pct(if at_end { 100.0 - p } else { p }),
            // Offset px desde left/top → directo; desde right/bottom → borde.
            Some(l) if !at_end => l,
            Some(_) => LengthVal::Pct(100.0),
        }
    }
    let lower: Vec<String> = toks.iter().map(|t| t.to_ascii_lowercase()).collect();
    let mut x: Option<LengthVal> = None;
    let mut y: Option<LengthVal> = None;
    let mut center_pending = 0u8;
    let mut i = 0;
    while i < lower.len() {
        match lower[i].as_str() {
            "left" | "right" => {
                let at_end = lower[i] == "right";
                let off = toks.get(i + 1).and_then(|t| parse_length_or_pct(t));
                if off.is_some() {
                    i += 1;
                }
                x = Some(edge_val(at_end, off));
            }
            "top" | "bottom" => {
                let at_end = lower[i] == "bottom";
                let off = toks.get(i + 1).and_then(|t| parse_length_or_pct(t));
                if off.is_some() {
                    i += 1;
                }
                y = Some(edge_val(at_end, off));
            }
            "center" => center_pending += 1,
            _ => return None, // length suelta en forma de 4 valores → inválido
        }
        i += 1;
    }
    // Los `center` rellenan los ejes que quedaron libres.
    if center_pending > 0 && x.is_none() {
        x = Some(LengthVal::Pct(50.0));
    }
    if center_pending > 0 && y.is_none() {
        y = Some(LengthVal::Pct(50.0));
    }
    Some(DeclKind::BackgroundPosition(BackgroundPosition {
        x: x.unwrap_or(LengthVal::Pct(50.0)),
        y: y.unwrap_or(LengthVal::Pct(50.0)),
    }))
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
        push_gradient_stops(raw, false, &mut stops);
    }
    if stops.len() < 2 {
        return None;
    }
    let _ = stops_start;
    Some(LinearGradient { geometry: GradientGeometry::Linear { angle_deg }, stops, repeating: false })
}

/// Parsea el contenido de `radial-gradient(...)`. Sintaxis aceptada (MVP):
/// `radial-gradient([<shape> || <size>]? [at <position>]?, <stop>, <stop>…)`.
/// `<shape>` = `circle`/`ellipse` (no se distingue en el render), `<size>` =
/// `closest-side`/`closest-corner`/`farthest-side`/`farthest-corner`,
/// `<position>` reutiliza el parser de `background-position`. Si el primer
/// segmento no es un prelude válido, se trata como el primer stop (default
/// `farthest-corner at center`). Fase 7.226.
pub(crate) fn parse_radial_gradient(args: &str) -> Option<LinearGradient> {
    let parts: Vec<&str> = args.split(',').map(|s| s.trim()).collect();
    if parts.len() < 2 {
        return None;
    }
    let (spec, stops_start) = match parse_radial_prelude(parts[0]) {
        Some(spec) => (spec, 1),
        None => (RadialSpec::default(), 0),
    };
    let mut stops: Vec<GradientStop> = Vec::new();
    for raw in &parts[stops_start..] {
        push_gradient_stops(raw, false, &mut stops);
    }
    if stops.len() < 2 {
        return None;
    }
    Some(LinearGradient { geometry: GradientGeometry::Radial(spec), stops, repeating: false })
}

/// Parsea el contenido de `conic-gradient(...)`. Sintaxis aceptada (MVP):
/// `conic-gradient([from <angle>]? [at <position>]?, <stop>, <stop>…)`.
/// `<angle>` en `deg`/`turn`/`rad`/`grad` (default 0 = up). `<position>`
/// reutiliza el parser de `background-position` (default center). Los stops
/// se reparten 0..1 sobre el barrido (no parseamos posiciones angulares por
/// stop todavía). Fase 7.227.
pub(crate) fn parse_conic_gradient(args: &str) -> Option<LinearGradient> {
    let parts: Vec<&str> = args.split(',').map(|s| s.trim()).collect();
    if parts.len() < 2 {
        return None;
    }
    let (geom, stops_start) = match parse_conic_prelude(parts[0]) {
        Some(g) => (g, 1),
        None => (GradientGeometry::Conic {
            from_deg: 0.0,
            cx: LengthVal::Pct(50.0),
            cy: LengthVal::Pct(50.0),
        }, 0),
    };
    let mut stops: Vec<GradientStop> = Vec::new();
    for raw in &parts[stops_start..] {
        push_gradient_stops(raw, true, &mut stops);
    }
    if stops.len() < 2 {
        return None;
    }
    Some(LinearGradient { geometry: geom, stops, repeating: false })
}

/// Interpreta el primer segmento de un `conic-gradient` como prelude
/// (`from <angle>` y/o `at <position>`). `None` si parece un color stop.
fn parse_conic_prelude(s: &str) -> Option<GradientGeometry> {
    let lc = s.to_ascii_lowercase();
    let mut from_deg = 0.0_f32;
    let mut cx = LengthVal::Pct(50.0);
    let mut cy = LengthVal::Pct(50.0);
    let mut matched = false;
    // Separa el `at <position>` del head (`from <angle>`).
    let (head, pos_part) = match lc.split_once(" at ") {
        Some((h, p)) => (h.trim().to_string(), Some(p.trim().to_string())),
        None => {
            if let Some(p) = lc.strip_prefix("at ") {
                (String::new(), Some(p.trim().to_string()))
            } else {
                (lc.clone(), None)
            }
        }
    };
    if let Some(rest) = head.strip_prefix("from ") {
        from_deg = parse_angle_deg(rest.trim())?;
        matched = true;
    } else if !head.is_empty() {
        // Head no vacío que no es `from …` → no es prelude (es un stop).
        return None;
    }
    if let Some(p) = pos_part {
        if let Some(DeclKind::BackgroundPosition(bp)) = parse_background_position(&p) {
            cx = bp.x;
            cy = bp.y;
            matched = true;
        } else {
            return None;
        }
    }
    matched.then_some(GradientGeometry::Conic { from_deg, cx, cy })
}

/// Parsea un `<angle>` CSS a grados: `deg`/`turn`/`rad`/`grad` o número crudo
/// (= grados). Fase 7.227.
fn parse_angle_deg(s: &str) -> Option<f32> {
    let s = s.trim();
    if let Some(n) = s.strip_suffix("deg") {
        n.trim().parse().ok()
    } else if let Some(n) = s.strip_suffix("turn") {
        n.trim().parse::<f32>().ok().map(|t| t * 360.0)
    } else if let Some(n) = s.strip_suffix("grad") {
        n.trim().parse::<f32>().ok().map(|g| g * 0.9)
    } else if let Some(n) = s.strip_suffix("rad") {
        n.trim().parse::<f32>().ok().map(|r| r.to_degrees())
    } else {
        s.parse().ok()
    }
}

/// Interpreta el primer segmento de un `radial-gradient` como prelude
/// (shape/size/`at <pos>`). `None` si parece un color stop (para que el
/// caller lo trate como tal).
fn parse_radial_prelude(s: &str) -> Option<RadialSpec> {
    let lc = s.to_ascii_lowercase();
    let mut spec = RadialSpec::default();
    let mut matched = false;
    // Separa el `at <position>` del head (shape/size).
    let (head, pos_part) = match lc.split_once(" at ") {
        Some((h, p)) => (h.trim().to_string(), Some(p.trim().to_string())),
        None => {
            if let Some(p) = lc.strip_prefix("at ") {
                (String::new(), Some(p.trim().to_string()))
            } else {
                (lc.clone(), None)
            }
        }
    };
    for tok in head.split_whitespace() {
        match tok {
            "circle" | "ellipse" => matched = true,
            "closest-side" => {
                spec.size = RadialSize::ClosestSide;
                matched = true;
            }
            "closest-corner" => {
                spec.size = RadialSize::ClosestCorner;
                matched = true;
            }
            "farthest-side" => {
                spec.size = RadialSize::FarthestSide;
                matched = true;
            }
            "farthest-corner" => {
                spec.size = RadialSize::FarthestCorner;
                matched = true;
            }
            // Token desconocido en el head → no es prelude (es un stop).
            _ => return None,
        }
    }
    if let Some(p) = pos_part {
        if let Some(DeclKind::BackgroundPosition(bp)) = parse_background_position(&p) {
            spec.cx = bp.x;
            spec.cy = bp.y;
            matched = true;
        } else {
            return None;
        }
    }
    matched.then_some(spec)
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

/// Parsea una posición de stop: `40%` → `Pct(40)`. En un gradiente lineal o
/// radial el resto es una longitud (`10px`/`1rem`/`0` → `Px(...)`, px reales,
/// no la vieja heurística `/100`); en uno **cónico** es un ángulo
/// (`90deg`/`0.25turn` → `Px(grados)`, ya que el render trata el eje cónico
/// como 360°). Devuelve `None` si el token no es una posición válida.
fn parse_stop_pos(p: &str, conic: bool) -> Option<LengthVal> {
    if let Some(pct) = p.strip_suffix('%') {
        pct.trim().parse::<f32>().ok().map(LengthVal::Pct)
    } else if conic {
        parse_angle_deg(p).map(LengthVal::Px)
    } else {
        parse_length_px(p).map(LengthVal::Px)
    }
}

/// Parsea un stop de gradiente y empuja 1 o 2 stops a `out`. Acepta:
/// `<color>` (sin posición), `<color> <pos>` (una posición) y
/// `<color> <pos> <pos>` (doble posición CSS — atajo `#ccc 0 10px` que
/// equivale a dos stops del mismo color; omnipresente en franjas
/// `repeating-*`). Devuelve `false` si el token no es un stop válido.
fn push_gradient_stops(raw: &str, conic: bool, out: &mut Vec<GradientStop>) -> bool {
    let parts: Vec<&str> = raw.split_whitespace().collect();
    match parts.as_slice() {
        [c] => match parse_color(c) {
            Some(color) => {
                out.push(GradientStop { color, pos: None });
                true
            }
            None => false,
        },
        [c, p] => match parse_color(c) {
            Some(color) => {
                out.push(GradientStop { color, pos: parse_stop_pos(p, conic) });
                true
            }
            None => false,
        },
        [c, p1, p2] => match parse_color(c) {
            Some(color) => {
                out.push(GradientStop { color, pos: parse_stop_pos(p1, conic) });
                out.push(GradientStop { color, pos: parse_stop_pos(p2, conic) });
                true
            }
            None => false,
        },
        _ => false,
    }
}

/// Compat: parsea un único stop lineal/radial (sin doble posición). Usado por tests.
#[cfg(test)]
pub(crate) fn parse_gradient_stop(s: &str) -> Option<GradientStop> {
    let mut v = Vec::new();
    if push_gradient_stops(s, false, &mut v) {
        v.into_iter().next()
    } else {
        None
    }
}
