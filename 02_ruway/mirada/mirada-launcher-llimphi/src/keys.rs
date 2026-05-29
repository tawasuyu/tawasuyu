//! Parser mínimo `string -> Key` para los hotkeys del TOML.
//!
//! No cubre modifiers (`Ctrl+Space`) en MVP — sólo teclas únicas:
//! `F1..F12`, `Escape/Esc`, `Enter/Return`, `Tab`, `Space`, o un carácter
//! (`a`, `/`).

use llimphi_ui::{Key, NamedKey};

/// Convierte una etiqueta tipo `"F12"` al `Key` correspondiente.
/// Si no matchea ninguna conocida y tiene un único `char`, lo trata
/// como tecla de carácter.
pub fn parse(label: &str) -> Option<Key> {
    let s = label.trim();
    if s.is_empty() {
        return None;
    }
    let named = match s {
        "F1" => NamedKey::F1,
        "F2" => NamedKey::F2,
        "F3" => NamedKey::F3,
        "F4" => NamedKey::F4,
        "F5" => NamedKey::F5,
        "F6" => NamedKey::F6,
        "F7" => NamedKey::F7,
        "F8" => NamedKey::F8,
        "F9" => NamedKey::F9,
        "F10" => NamedKey::F10,
        "F11" => NamedKey::F11,
        "F12" => NamedKey::F12,
        "Escape" | "Esc" => NamedKey::Escape,
        "Enter" | "Return" => NamedKey::Enter,
        "Tab" => NamedKey::Tab,
        "Space" => NamedKey::Space,
        "Backspace" => NamedKey::Backspace,
        _ => {
            if s.chars().count() == 1 {
                return Some(Key::Character(s.into()));
            }
            return None;
        }
    };
    Some(Key::Named(named))
}

/// `true` si el `KeyEvent` corresponde a la etiqueta dada. Falla cerrado
/// (devuelve `false`) si el label no se pudo parsear.
pub fn matches(label: &str, event_key: &Key) -> bool {
    match parse(label) {
        Some(parsed) => &parsed == event_key,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_function_keys() {
        assert_eq!(parse("F1"), Some(Key::Named(NamedKey::F1)));
        assert_eq!(parse("F12"), Some(Key::Named(NamedKey::F12)));
    }

    #[test]
    fn parses_named_aliases() {
        assert_eq!(parse("Esc"), Some(Key::Named(NamedKey::Escape)));
        assert_eq!(parse("Escape"), Some(Key::Named(NamedKey::Escape)));
        assert_eq!(parse("Return"), Some(Key::Named(NamedKey::Enter)));
    }

    #[test]
    fn parses_single_chars() {
        assert!(matches!(parse("/"), Some(Key::Character(_))));
    }

    #[test]
    fn rejects_unknown_multichar() {
        assert!(parse("MetaSuper").is_none());
        assert!(parse("").is_none());
    }
}
