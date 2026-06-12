//! Dispatch principal de propiedades CSS: `decl_kind_from_pair`.
//! La mega-`match` original se partió en cuatro grupos contiguos
//! (`dispatch_a..d`) encadenados por `or_else` — las props son únicas,
//! así que el orden y el resultado son idénticos al match monolítico.
use super::*;

/// Keyword CSS-wide (`inherit`/`initial`/`unset`/`revert`). `revert` se
/// aproxima como `unset`. Fase 7.225.
fn wide_keyword(value: &str) -> Option<WideKw> {
    match value.trim().to_ascii_lowercase().as_str() {
        "inherit" => Some(WideKw::Inherit),
        "initial" => Some(WideKw::Initial),
        "unset" => Some(WideKw::Unset),
        "revert" | "revert-layer" => Some(WideKw::Unset),
        _ => None,
    }
}

/// Mapea una propiedad longhand al `WideProp` del subset curado. `None` para
/// las no soportadas (su keyword wide se dropea). Fase 7.225.
fn wide_prop(prop: &str) -> Option<WideProp> {
    Some(match prop.to_ascii_lowercase().as_str() {
        "color" => WideProp::Color,
        "background-color" => WideProp::Background,
        "font-size" => WideProp::FontSize,
        "font-weight" => WideProp::FontWeight,
        "font-style" => WideProp::FontStyle,
        "font-family" => WideProp::FontFamily,
        "line-height" => WideProp::LineHeight,
        "text-align" => WideProp::TextAlign,
        "text-decoration" | "text-decoration-line" => WideProp::TextDecoration,
        "visibility" => WideProp::Visibility,
        "display" => WideProp::Display,
        "box-sizing" => WideProp::BoxSizing,
        "border-color" => WideProp::BorderColor,
        _ => return None,
    })
}

pub(crate) fn decl_kind_from_pair(prop: &str, value: &str) -> Option<DeclKind> {
    // Keywords CSS-wide (inherit/initial/unset/revert) sobre el subset
    // curado de propiedades — se resuelven luego contra padre/default.
    if let Some(kw) = wide_keyword(value) {
        return wide_prop(prop).map(|prop| DeclKind::Wide { prop, kw });
    }
    // El match original despachaba sobre `prop.to_ascii_lowercase()`.
    let p = prop.to_ascii_lowercase();
    let p = p.as_str();
    dispatch_a(p, value)
        .or_else(|| dispatch_b(p, value))
        .or_else(|| dispatch_c(p, value))
        .or_else(|| dispatch_d(p, value))
}
