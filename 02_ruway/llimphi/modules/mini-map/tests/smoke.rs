//! Smoke tests del minimap. Sin backend grafico — solo `apply`,
//! `on_key`, `open_shortcut` y la conversion y->line.

use llimphi_module_mini_map::{
    self as minimap, MiniMapAction, MiniMapMsg, MiniMapState,
};
use llimphi_ui::{Key, KeyEvent, KeyState, Modifiers};

fn key_with(ctrl: bool, shift: bool, ch: &str) -> KeyEvent {
    KeyEvent {
        key: Key::Character(ch.into()),
        state: KeyState::Pressed,
        text: Some(ch.into()),
        modifiers: Modifiers { ctrl, shift, ..Modifiers::default() },
        repeat: false,
    }
}

#[test]
fn open_shortcut_es_ctrl_shift_m() {
    assert!(minimap::open_shortcut(&key_with(true, true, "m")));
    assert!(minimap::open_shortcut(&key_with(true, true, "M")));
    assert!(!minimap::open_shortcut(&key_with(true, false, "m")));
    assert!(!minimap::open_shortcut(&key_with(false, true, "m")));
}

#[test]
fn jump_emite_jumpto() {
    let mut s = MiniMapState::new();
    let action = minimap::apply(&mut s, MiniMapMsg::Jump(42));
    assert_eq!(action, MiniMapAction::JumpTo(42));
}

#[test]
fn close_emite_close() {
    let mut s = MiniMapState::new();
    let action = minimap::apply(&mut s, MiniMapMsg::Close);
    assert_eq!(action, MiniMapAction::Close);
}

#[test]
fn y_to_line_proporcional() {
    // 100 lineas, panel de 200 px → cada linea ocupa 2 px.
    assert_eq!(minimap::y_to_line(0.0, 200.0, 100), 0);
    assert_eq!(minimap::y_to_line(100.0, 200.0, 100), 50);
    assert_eq!(minimap::y_to_line(200.0, 200.0, 100), 99);
    // Clamping fuera de rango.
    assert_eq!(minimap::y_to_line(-50.0, 200.0, 100), 0);
    assert_eq!(minimap::y_to_line(500.0, 200.0, 100), 99);
}

#[test]
fn y_to_line_buffer_vacio_no_paniquea() {
    assert_eq!(minimap::y_to_line(0.0, 100.0, 0), 0);
    assert_eq!(minimap::y_to_line(50.0, 100.0, 0), 0);
}

#[test]
fn on_key_es_pasivo() {
    let s = MiniMapState::new();
    let ev = key_with(false, false, "a");
    assert!(minimap::on_key(&s, &ev).is_none());
}
