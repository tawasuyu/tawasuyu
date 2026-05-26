//! Smoke tests del modulo bookmarks: toggle, jump-next/prev,
//! shortcuts, fuzzy refilter del overlay.

use std::path::PathBuf;

use llimphi_module_bookmarks::{
    self as bm, BookmarksAction, BookmarksMsg, BookmarksOverlay, BookmarksState,
};
use llimphi_ui::{Key, KeyEvent, KeyState, Modifiers};

fn key_with(ctrl: bool, alt: bool, shift: bool, ch: &str) -> KeyEvent {
    KeyEvent {
        key: Key::Character(ch.into()),
        state: KeyState::Pressed,
        text: Some(ch.into()),
        modifiers: Modifiers { ctrl, alt, shift, ..Modifiers::default() },
        repeat: false,
    }
}

#[test]
fn toggle_agrega_y_remueve() {
    let mut s = BookmarksState::new();
    let p = PathBuf::from("/x/foo.rs");
    let a1 = bm::apply(&mut s, BookmarksMsg::ToggleAt { path: p.clone(), line: 5 });
    assert!(matches!(a1, BookmarksAction::SetStatus(_)));
    assert!(s.contains(&p, 5));
    let a2 = bm::apply(&mut s, BookmarksMsg::ToggleAt { path: p.clone(), line: 5 });
    assert!(matches!(a2, BookmarksAction::SetStatus(_)));
    assert!(!s.contains(&p, 5));
}

#[test]
fn jump_next_wraparound() {
    let mut s = BookmarksState::new();
    let a = PathBuf::from("/x/a.rs");
    let b = PathBuf::from("/x/b.rs");
    s.toggle(a.clone(), 10);
    s.toggle(b.clone(), 20);
    s.toggle(a.clone(), 30);
    // Estamos en (a, 10) - next debe ser (b, 20).
    let action = bm::apply(&mut s, BookmarksMsg::JumpNext { current_path: a.clone(), current_line: 10 });
    assert_eq!(action, BookmarksAction::JumpTo { path: b.clone(), line: 20 });
    // Estamos en (a, 30) - next wrappea a (a, 10).
    let action = bm::apply(&mut s, BookmarksMsg::JumpNext { current_path: a.clone(), current_line: 30 });
    assert_eq!(action, BookmarksAction::JumpTo { path: a.clone(), line: 10 });
}

#[test]
fn jump_prev_wraparound() {
    let mut s = BookmarksState::new();
    let a = PathBuf::from("/x/a.rs");
    s.toggle(a.clone(), 10);
    s.toggle(a.clone(), 20);
    s.toggle(a.clone(), 30);
    // Estamos en (a, 10) - prev wrappea a (a, 30).
    let action = bm::apply(&mut s, BookmarksMsg::JumpPrev { current_path: a.clone(), current_line: 10 });
    assert_eq!(action, BookmarksAction::JumpTo { path: a.clone(), line: 30 });
}

#[test]
fn jump_sin_marks_es_setstatus() {
    let mut s = BookmarksState::new();
    let action = bm::apply(&mut s, BookmarksMsg::JumpNext { current_path: PathBuf::from("/x"), current_line: 0 });
    assert!(matches!(action, BookmarksAction::SetStatus(_)));
}

#[test]
fn shortcuts_distinguibles() {
    assert!(bm::toggle_shortcut(&key_with(true, true, false, "b")));
    assert!(!bm::toggle_shortcut(&key_with(true, true, true, "b"))); // ctrl+alt+shift+b no
    assert!(bm::open_shortcut(&key_with(true, false, true, "b")));
    assert!(bm::next_shortcut(&key_with(true, true, false, "n")));
    assert!(bm::prev_shortcut(&key_with(true, true, false, "p")));
}

#[test]
fn refilter_con_query_vacio_lista_todos() {
    let mut s = BookmarksState::new();
    s.toggle(PathBuf::from("/x/a.rs"), 1);
    s.toggle(PathBuf::from("/x/b.rs"), 2);
    s.overlay = Some(BookmarksOverlay::new());
    bm::refilter_overlay(&mut s);
    assert_eq!(s.overlay.as_ref().unwrap().results.len(), 2);
}

#[test]
fn clear_all_vacia_marks() {
    let mut s = BookmarksState::new();
    s.toggle(PathBuf::from("/x"), 1);
    s.toggle(PathBuf::from("/y"), 2);
    let _ = bm::apply(&mut s, BookmarksMsg::ClearAll);
    assert!(s.marks.is_empty());
}
