//! Smoke tests del fuzzy match y el routing de teclas. Sin backend
//! gráfico — sólo `apply` + `refilter`.

use llimphi_module_symbol_outline::{
    self as outline, OutlineAction, OutlineMsg, OutlineState, SymbolItem,
};
use llimphi_ui::{Key, KeyEvent, KeyState, Modifiers};

fn seed() -> Vec<SymbolItem> {
    vec![
        SymbolItem {
            name: "Model".into(),
            kind: "struct".into(),
            line: 100,
            col: 0,
            container: None,
            depth: 0,
        },
        SymbolItem {
            name: "init".into(),
            kind: "fn".into(),
            line: 110,
            col: 4,
            container: Some("Model".into()),
            depth: 1,
        },
        SymbolItem {
            name: "update".into(),
            kind: "fn".into(),
            line: 200,
            col: 4,
            container: Some("Model".into()),
            depth: 1,
        },
        SymbolItem {
            name: "Renderer".into(),
            kind: "struct".into(),
            line: 300,
            col: 0,
            container: None,
            depth: 0,
        },
        SymbolItem {
            name: "draw".into(),
            kind: "fn".into(),
            line: 310,
            col: 4,
            container: Some("Renderer".into()),
            depth: 1,
        },
    ]
}

fn key_char(c: &str) -> KeyEvent {
    KeyEvent {
        key: Key::Character(c.into()),
        state: KeyState::Pressed,
        text: Some(c.into()),
        modifiers: Modifiers::default(),
        repeat: false,
    }
}

#[test]
fn estado_vacio_lista_todos_los_simbolos() {
    let items = seed();
    let s = OutlineState::new(&items);
    assert_eq!(s.results.len(), items.len());
}

#[test]
fn fuzzy_match_filtra_por_nombre_de_clase_contenedora() {
    // Tipear "render" debería traer `draw` (su container es "Renderer")
    // gracias a que refilter incluye container en la haystack.
    let items = seed();
    let mut s = OutlineState::new(&items);
    for ch in ["r", "e", "n", "d", "e", "r"] {
        outline::apply(&mut s, OutlineMsg::KeyInput(key_char(ch)), &items);
    }
    let names: Vec<&str> = s.results.iter().map(|&i| items[i].name.as_str()).collect();
    assert!(
        names.contains(&"draw") || names.contains(&"Renderer"),
        "esperaba draw o Renderer en {names:?}"
    );
}

#[test]
fn apply_emite_goto_con_line_col_del_item_seleccionado() {
    let items = seed();
    let mut s = OutlineState::new(&items);
    // Filtrar "update".
    for ch in ["u", "p", "d", "a", "t", "e"] {
        outline::apply(&mut s, OutlineMsg::KeyInput(key_char(ch)), &items);
    }
    let action = outline::apply(&mut s, OutlineMsg::Apply, &items);
    assert_eq!(action, OutlineAction::GoTo { line: 200, col: 4 });
}

#[test]
fn nav_wrap_around() {
    let items = seed();
    let mut s = OutlineState::new(&items);
    assert_eq!(s.selected, 0);
    outline::apply(&mut s, OutlineMsg::Nav(-1), &items);
    assert_eq!(s.selected, items.len() - 1);
}

#[test]
fn open_shortcut_es_ctrl_shift_o() {
    let mk = |ctrl: bool, shift: bool, c: &str| KeyEvent {
        key: Key::Character(c.into()),
        state: KeyState::Pressed,
        text: Some(c.into()),
        modifiers: Modifiers { ctrl, shift, ..Modifiers::default() },
        repeat: false,
    };
    assert!(outline::open_shortcut(&mk(true, true, "o")));
    assert!(outline::open_shortcut(&mk(true, true, "O")));
    assert!(!outline::open_shortcut(&mk(true, false, "o")));
    assert!(!outline::open_shortcut(&mk(false, true, "o")));
}

#[test]
fn items_vacios_no_paniquean() {
    let items: Vec<SymbolItem> = Vec::new();
    let mut s = OutlineState::new(&items);
    assert!(s.results.is_empty());
    let action = outline::apply(&mut s, OutlineMsg::Apply, &items);
    assert_eq!(action, OutlineAction::None);
}
