use super::*;

/// Acepta `12px`, `1.5rem` (tratada como em*16), `0`. Sin unidad → px.
///
/// Modelo: separa el número de la unidad alfabética del sufijo (en vez de
/// probar `strip_suffix` por unidad, que no escala — `svh` colisiona con
/// `vh`, `cqmin` con `in`, etc.). Si el sufijo no es una unidad conocida,
/// cae a `s.parse()` para no regresar números crudos / notación científica.
///
/// Cobertura de unidades (Fase 7.852 — lote data-driven de cobertura CSS):
/// - **Absolutas**: `px`, `cm`, `mm`, `q`, `in`, `pt`, `pc` (1in = 96px).
/// - **Font-relativas**: `em`/`rem` (em fijo a 16px — simplificación previa),
///   `ch`/`ex` (≈ 0.5em = 8px; sin métricas de fuente reales).
/// - **Viewport**: `vw`/`vh`/`vmin`/`vmax` y las variantes de UA dinámica
///   `svw|lvw|dvw` / `svh|lvh|dvh` / `svmin|…` / `svmax|…`. Sin barras de UI
///   que aparezcan/desaparezcan, small=large=dynamic colapsan al viewport.
/// - **Container query**: `cqw|cqh|cqi|cqb|cqmin|cqmax`. Sin un container
///   real en este punto del pipeline, se resuelven contra el viewport
///   (`cqi`=inline≈ancho, `cqb`=block≈alto) — aproximación documentada.
///
/// Resuelven contra el viewport activo ([`resolve_viewport`]): el real bajo
/// un `ViewportScope` (carga normal), `DEFAULT_VIEWPORT` fuera de él.
pub(crate) fn parse_length_px(s: &str) -> Option<f32> {
    let s = s.trim();
    if s == "0" {
        return Some(0.0);
    }
    // La unidad es el sufijo alfabético; el número, todo lo previo.
    let unit_start = s.char_indices().find(|(_, c)| c.is_ascii_alphabetic());
    let Some((idx, _)) = unit_start else {
        // Sin unidad alfabética: número crudo (px) o notación científica.
        return s.parse().ok();
    };
    let num: f32 = match s[..idx].trim().parse() {
        Ok(n) => n,
        // Prefijo no numérico (p.ej. notación científica `1e3`): reintenta
        // el string entero antes de rendirse.
        Err(_) => return s.parse().ok(),
    };
    let vp = resolve_viewport();
    let v = match s[idx..].trim().to_ascii_lowercase().as_str() {
        "px" => num,
        "rem" | "em" => num * 16.0,
        "ch" | "ex" => num * 8.0,
        // Fase 7.898 — unidades font-relativas restantes (CSS Values 4). Sin
        // métricas reales de fuente, se aproximan contra el em=16:
        //  · rex/rch = ex/ch del root (mismo 16px) ≈ 0.5em = 8px.
        //  · cap/rcap = altura de mayúsculas ≈ 0.7em = 11.2px.
        //  · ic/ric = avance ideográfico (glifo CJK full-width) ≈ 1em = 16px.
        //  · lh/rlh = altura de línea `normal` ≈ 1.2em = 19.2px.
        "rex" | "rch" => num * 8.0,
        "cap" | "rcap" => num * 11.2,
        "ic" | "ric" => num * 16.0,
        "lh" | "rlh" => num * 19.2,
        "cm" => num * 96.0 / 2.54,
        "mm" => num * 96.0 / 25.4,
        "q" => num * 96.0 / 25.4 / 4.0, // 1q = 1/40 cm
        "in" => num * 96.0,
        "pt" => num * 96.0 / 72.0,
        "pc" => num * 16.0, // 1pc = 12pt = 16px
        "vw" | "svw" | "lvw" | "dvw" => num * vp.width / 100.0,
        "vh" | "svh" | "lvh" | "dvh" => num * vp.height / 100.0,
        // Fase 7.898 — viewport inline/block. En modo de escritura horizontal
        // (el único acá) inline = ancho y block = alto.
        "vi" | "svi" | "lvi" | "dvi" => num * vp.width / 100.0,
        "vb" | "svb" | "lvb" | "dvb" => num * vp.height / 100.0,
        "vmin" | "svmin" | "lvmin" | "dvmin" => num * vp.width.min(vp.height) / 100.0,
        "vmax" | "svmax" | "lvmax" | "dvmax" => num * vp.width.max(vp.height) / 100.0,
        // Container query units → viewport (sin container real acá).
        "cqw" | "cqi" => num * vp.width / 100.0,
        "cqh" | "cqb" => num * vp.height / 100.0,
        "cqmin" => num * vp.width.min(vp.height) / 100.0,
        "cqmax" => num * vp.width.max(vp.height) / 100.0,
        _ => return s.parse().ok(),
    };
    Some(v)
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
        // Fase 7.832 — `-webkit-sticky` alias vendor legacy (Safari) de `sticky`.
        "sticky" | "-webkit-sticky" => Some(Position::Sticky),
        _ => None,
    }
}

pub(crate) fn parse_vertical_align(s: &str) -> Option<VerticalAlign> {
    match s.trim().to_ascii_lowercase().as_str() {
        "baseline" => Some(VerticalAlign::Baseline),
        "top" | "text-top" => Some(VerticalAlign::Top),
        // Fase 7.842 — `-webkit-baseline-middle` (alias legacy WebKit) ≈ middle.
        "middle" | "-webkit-baseline-middle" => Some(VerticalAlign::Middle),
        "bottom" | "text-bottom" => Some(VerticalAlign::Bottom),
        "super" => Some(VerticalAlign::Super),
        "sub" => Some(VerticalAlign::Sub),
        // Fase 7.861 — `vertical-align: <length>|<percentage>` es un corrimiento
        // numérico de la baseline. El modelo es enum de keywords (mapea a
        // alignment de taffy), sin offset numérico — colapsa a `Baseline` para
        // no descartar la declaración (divergencia documentada).
        other => {
            let is_pct = other.strip_suffix('%').is_some_and(|n| n.trim().parse::<f32>().is_ok());
            (parse_length_px(other).is_some() || is_pct).then_some(VerticalAlign::Baseline)
        }
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
        "none" => Some(PointerEvents::None),
        // Fase 7.841 — los valores SVG (`visiblePainted`/`painted`/`fill`/
        // `stroke`/`all`/`visible`…) significan todos "recibe eventos de
        // puntero"; en nuestro modelo binario (Auto|None) colapsan a Auto.
        // Fase 7.876 — `bounding-box` (SVG2) también recibe eventos → Auto.
        "auto" | "all" | "visible" | "visiblepainted" | "visiblefill"
        | "visiblestroke" | "painted" | "fill" | "stroke" | "bounding-box" => {
            Some(PointerEvents::Auto)
        }
        _ => None,
    }
}

/// `object-fit: fill | contain | cover | none | scale-down`. Fase 7.230.
pub(crate) fn parse_object_fit(s: &str) -> Option<ObjectFit> {
    match s.trim().to_ascii_lowercase().as_str() {
        "fill" => Some(ObjectFit::Fill),
        "contain" => Some(ObjectFit::Contain),
        "cover" => Some(ObjectFit::Cover),
        "none" => Some(ObjectFit::None),
        "scale-down" => Some(ObjectFit::ScaleDown),
        _ => None,
    }
}

/// Indica que `cssparser` está enlazado aunque el subset actual no use
/// la API completa — la presencia del crate evita que `cargo` lo
/// pruebe y deja el camino abierto para Fase 3.
#[doc(hidden)]
pub fn _cssparser_anchor() {
    let _ = cssparser::ParserInput::new("");
}
