//! Smoke tests del fuzzy match y del flujo `Open → KeyInput → Apply`.
//! No requieren backend gráfico — sólo el reducer puro y `refilter`.

use llimphi_module_command_palette::{
    self as palette, Command, PaletteAction, PaletteMsg, PaletteState,
};
use llimphi_ui::{Key, KeyEvent, KeyState, Modifiers};

fn seed() -> Vec<Command> {
    vec![
        Command::new("editor.save", "Save File", "Editor").with_shortcut("Ctrl+S"),
        Command::new("editor.open", "Open File", "Editor").with_shortcut("Ctrl+P"),
        Command::new("editor.findInFiles", "Find in Files", "Editor")
            .with_shortcut("Ctrl+Shift+F"),
        Command::new("terminal.open", "Open Terminal", "Terminal")
            .with_shortcut("Ctrl+`"),
        Command::new("lsp.format", "Format Document", "LSP")
            .with_shortcut("Ctrl+Alt+L"),
        Command::new("lsp.goto", "Go to Definition", "LSP").with_shortcut("F12"),
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
fn estado_vacio_lista_todos_los_comandos() {
    let cmds = seed();
    let s = PaletteState::new(&cmds);
    assert_eq!(s.results.len(), cmds.len());
    assert_eq!(s.selected, 0);
}

#[test]
fn fuzzy_match_acerca_el_comando_correcto_al_top() {
    let cmds = seed();
    let mut s = PaletteState::new(&cmds);

    // Tipear "term" debería rankear "Open Terminal" o "Terminal" arriba.
    for ch in ["t", "e", "r", "m"] {
        let action = palette::apply(&mut s, PaletteMsg::KeyInput(key_char(ch)), &cmds);
        assert_eq!(action, PaletteAction::None);
    }
    let top = s.results.first().expect("debe haber al menos un match");
    assert_eq!(
        cmds[*top].id, "terminal.open",
        "esperaba terminal.open al top, vi {:?}",
        cmds[*top].title
    );
}

#[test]
fn enter_emite_invoke_con_el_id_seleccionado() {
    let cmds = seed();
    let mut s = PaletteState::new(&cmds);

    for ch in ["s", "a", "v"] {
        palette::apply(&mut s, PaletteMsg::KeyInput(key_char(ch)), &cmds);
    }
    let action = palette::apply(&mut s, PaletteMsg::Apply, &cmds);
    assert_eq!(action, PaletteAction::Invoke("editor.save".into()));
}

#[test]
fn nav_circula_por_los_resultados() {
    let cmds = seed();
    let mut s = PaletteState::new(&cmds);
    assert_eq!(s.selected, 0);

    palette::apply(&mut s, PaletteMsg::Nav(1), &cmds);
    assert_eq!(s.selected, 1);

    // Saltar al final desde la cima con -1 (wrap-around).
    let mut s = PaletteState::new(&cmds);
    palette::apply(&mut s, PaletteMsg::Nav(-1), &cmds);
    assert_eq!(s.selected, cmds.len() - 1);
}

#[test]
fn escape_emite_close() {
    let cmds = seed();
    let mut s = PaletteState::new(&cmds);
    let action = palette::apply(&mut s, PaletteMsg::Close, &cmds);
    assert_eq!(action, PaletteAction::Close);
}

#[test]
fn open_shortcut_es_ctrl_shift_p() {
    use llimphi_ui::Modifiers;
    let mk = |ctrl: bool, shift: bool, c: &str| KeyEvent {
        key: Key::Character(c.into()),
        state: KeyState::Pressed,
        text: Some(c.into()),
        modifiers: Modifiers { ctrl, shift, ..Modifiers::default() },
        repeat: false,
    };
    assert!(palette::open_shortcut(&mk(true, true, "p")));
    assert!(palette::open_shortcut(&mk(true, true, "P")));
    // Sin shift no — ese es Ctrl+P del file-picker.
    assert!(!palette::open_shortcut(&mk(true, false, "p")));
    // Sin ctrl no.
    assert!(!palette::open_shortcut(&mk(false, true, "p")));
    // Otra letra no.
    assert!(!palette::open_shortcut(&mk(true, true, "q")));
}

#[test]
fn busqueda_por_grupo_funciona() {
    let cmds = seed();
    let mut s = PaletteState::new(&cmds);
    // "lsp" debería traer Format y Goto Definition (ambos del grupo LSP).
    for ch in ["l", "s", "p"] {
        palette::apply(&mut s, PaletteMsg::KeyInput(key_char(ch)), &cmds);
    }
    let ids: Vec<&str> = s.results.iter().map(|&i| cmds[i].id.as_str()).collect();
    assert!(ids.contains(&"lsp.format"), "esperaba lsp.format en {ids:?}");
    assert!(ids.contains(&"lsp.goto"), "esperaba lsp.goto en {ids:?}");
}
