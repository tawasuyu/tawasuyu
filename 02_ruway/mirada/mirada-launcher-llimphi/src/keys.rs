//! Parser `string -> Hotkey` para los hotkeys del TOML.
//!
//! Acepta una tecla única (`F1..F12`, `Escape/Esc`, `Enter/Return`, `Tab`,
//! `Space`, `Backspace`, o un carácter como `a` / `/`) y, opcionalmente,
//! modificadores antepuestos con `+`:
//!
//! - `Ctrl` / `Control`
//! - `Shift`
//! - `Alt`
//! - `Super` / `Meta` / `Logo` / `Win` / `Cmd`
//!
//! Ejemplos: `"F12"`, `"Ctrl+Space"`, `"Super+d"`, `"Ctrl+Shift+F1"`. El
//! orden de los modificadores no importa; un token de modificador
//! desconocido invalida el combo (falla cerrado). El carácter `+` como
//! tecla no es expresable (se usa como separador) — usá su nombre si hace
//! falta.

use llimphi_ui::{Key, KeyEvent, NamedKey};

/// Un atajo parseado: la tecla y qué modificadores deben estar exactamente
/// presentes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hotkey {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
    pub key: Key,
}

/// Convierte la parte de tecla (sin modificadores) tipo `"F12"` al `Key`
/// correspondiente. Si no matchea ninguna conocida y tiene un único `char`,
/// lo trata como tecla de carácter.
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

/// Parsea un combo completo (`"Ctrl+Space"`) a [`Hotkey`]. El último token es
/// la tecla; los anteriores, modificadores. `None` si la tecla no parsea o
/// algún modificador es desconocido.
pub fn parse_combo(label: &str) -> Option<Hotkey> {
    let mut tokens: Vec<&str> = label.split('+').map(str::trim).filter(|t| !t.is_empty()).collect();
    let key_tok = tokens.pop()?;
    let key = parse(key_tok)?;
    let mut hk = Hotkey { ctrl: false, shift: false, alt: false, meta: false, key };
    for t in tokens {
        match t.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => hk.ctrl = true,
            "shift" => hk.shift = true,
            "alt" => hk.alt = true,
            "super" | "meta" | "logo" | "win" | "cmd" => hk.meta = true,
            _ => return None, // modificador desconocido: fallar cerrado
        }
    }
    Some(hk)
}

/// `true` si el `KeyEvent` corresponde a la etiqueta dada (tecla + el mismo
/// juego exacto de modificadores). Falla cerrado (devuelve `false`) si el
/// label no se pudo parsear.
pub fn matches(label: &str, event: &KeyEvent) -> bool {
    let Some(hk) = parse_combo(label) else {
        return false;
    };
    let m = &event.modifiers;
    m.ctrl == hk.ctrl
        && m.shift == hk.shift
        && m.alt == hk.alt
        && m.meta == hk.meta
        && key_eq(&hk.key, &event.key)
}

/// Igualdad de teclas tolerante a mayúsculas para caracteres (con Shift, el
/// `Key::Character` puede llegar en mayúscula).
fn key_eq(a: &Key, b: &Key) -> bool {
    match (a, b) {
        (Key::Character(x), Key::Character(y)) => x.eq_ignore_ascii_case(y),
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llimphi_ui::{KeyState, Modifiers};

    fn ev(key: Key, modifiers: Modifiers) -> KeyEvent {
        KeyEvent { key, state: KeyState::Pressed, text: None, modifiers, repeat: false }
    }

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

    #[test]
    fn plain_key_needs_no_modifiers() {
        let none = Modifiers::default();
        assert!(matches("F12", &ev(Key::Named(NamedKey::F12), none)));
        // Con un modificador colgado, "F12" pelado NO matchea.
        let ctrl = Modifiers { ctrl: true, ..Default::default() };
        assert!(!matches("F12", &ev(Key::Named(NamedKey::F12), ctrl)));
    }

    #[test]
    fn combo_requires_exact_modifiers() {
        let ctrl = Modifiers { ctrl: true, ..Default::default() };
        assert!(matches("Ctrl+Space", &ev(Key::Named(NamedKey::Space), ctrl)));
        // Sin Ctrl no matchea.
        assert!(!matches("Ctrl+Space", &ev(Key::Named(NamedKey::Space), Modifiers::default())));
        // Con Ctrl+Shift tampoco (juego exacto).
        let ctrl_shift = Modifiers { ctrl: true, shift: true, ..Default::default() };
        assert!(!matches("Ctrl+Space", &ev(Key::Named(NamedKey::Space), ctrl_shift)));
    }

    #[test]
    fn super_aliases_and_char_case_insensitive() {
        let meta = Modifiers { meta: true, ..Default::default() };
        assert!(matches("Super+d", &ev(Key::Character("d".into()), meta)));
        // El alias Meta es equivalente y el carácter en mayúscula matchea.
        assert!(matches("Meta+d", &ev(Key::Character("D".into()), meta)));
    }

    #[test]
    fn order_of_modifiers_is_irrelevant() {
        let cs = Modifiers { ctrl: true, shift: true, ..Default::default() };
        assert!(matches("Shift+Ctrl+F1", &ev(Key::Named(NamedKey::F1), cs)));
        assert!(matches("Ctrl+Shift+F1", &ev(Key::Named(NamedKey::F1), cs)));
    }

    #[test]
    fn unknown_modifier_fails_closed() {
        assert!(parse_combo("Hyper+a").is_none());
    }
}
