//! Smoke tests del cómputo de filas y el routing de teclas. Sin
//! backend gráfico — pruebas puras sobre `compute_rows` y `apply`.

use llimphi_module_diff_viewer::{
    self as diff, DiffAction, DiffKind, DiffMsg, DiffState,
};

#[test]
fn diff_basico_inserts_y_deletes() {
    let before = "a\nb\nc\n";
    let after = "a\nB\nc\nd\n";
    let (rows, stats) = diff::compute_rows(before, after);

    // El diff esperado:
    //   = a / a
    //   - b
    //   + B
    //   = c / c
    //   + d
    assert_eq!(stats.equals, 2);
    assert_eq!(stats.deletes, 1);
    assert_eq!(stats.inserts, 2);

    assert_eq!(rows[0].kind, DiffKind::Equal);
    assert_eq!(rows[0].left.as_ref().unwrap().text, "a");
    assert_eq!(rows[0].right.as_ref().unwrap().text, "a");

    // El primer cambio debe ser un Delete o Insert (similar agrupa);
    // verificamos que B aparezca y b no.
    let texts_left: Vec<&str> = rows
        .iter()
        .filter_map(|r| r.left.as_ref().map(|c| c.text.as_str()))
        .collect();
    let texts_right: Vec<&str> = rows
        .iter()
        .filter_map(|r| r.right.as_ref().map(|c| c.text.as_str()))
        .collect();
    assert!(texts_left.contains(&"b"));
    assert!(texts_right.contains(&"B"));
    assert!(texts_right.contains(&"d"));
}

#[test]
fn numeros_de_linea_son_correctos() {
    let before = "alpha\nbeta\ngamma\n";
    let after = "alpha\nBETA\ngamma\ndelta\n";
    let (rows, _) = diff::compute_rows(before, after);

    // alpha en línea 1 de ambos.
    let alpha_row = rows.iter().find(|r| {
        r.left.as_ref().map(|c| c.text == "alpha").unwrap_or(false)
    }).unwrap();
    assert_eq!(alpha_row.left.as_ref().unwrap().line_no, 1);
    assert_eq!(alpha_row.right.as_ref().unwrap().line_no, 1);

    // beta (delete) en línea 2 izquierda.
    let beta_row = rows.iter().find(|r| {
        r.left.as_ref().map(|c| c.text == "beta").unwrap_or(false)
    }).unwrap();
    assert_eq!(beta_row.left.as_ref().unwrap().line_no, 2);
    assert!(beta_row.right.is_none());

    // delta (insert) en línea 4 derecha.
    let delta_row = rows.iter().find(|r| {
        r.right.as_ref().map(|c| c.text == "delta").unwrap_or(false)
    }).unwrap();
    assert_eq!(delta_row.right.as_ref().unwrap().line_no, 4);
    assert!(delta_row.left.is_none());
}

#[test]
fn textos_identicos_solo_equal() {
    let text = "uno\ndos\ntres\n";
    let (rows, stats) = diff::compute_rows(text, text);
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|r| r.kind == DiffKind::Equal));
    assert_eq!(stats.inserts, 0);
    assert_eq!(stats.deletes, 0);
    assert_eq!(stats.equals, 3);
}

#[test]
fn scroll_no_excede_los_limites() {
    let before = (0..50).map(|i| i.to_string()).collect::<Vec<_>>().join("\n");
    let after = before.clone(); // identical → 50 Equal rows
    let mut state = DiffState::new("a", "b", &before, &after);
    assert_eq!(state.scroll, 0);

    // Scroll grande hacia abajo: tope = 50 - visible_rows.
    diff::apply(&mut state, DiffMsg::Scroll(1000), 10);
    assert_eq!(state.scroll, 40);

    // Scroll arriba: tope mínimo 0.
    diff::apply(&mut state, DiffMsg::Scroll(-1000), 10);
    assert_eq!(state.scroll, 0);
}

#[test]
fn next_hunk_salta_a_la_proxima_diferencia() {
    // 20 líneas iguales + 2 cambios + 20 más. visible_rows=5 deja
    // espacio real para scrollear.
    let mut before = String::new();
    let mut after = String::new();
    for i in 0..20 {
        before.push_str(&format!("eq{i}\n"));
        after.push_str(&format!("eq{i}\n"));
    }
    before.push_str("DEL\n");
    after.push_str("INS\n");
    for i in 20..40 {
        before.push_str(&format!("eq{i}\n"));
        after.push_str(&format!("eq{i}\n"));
    }
    let mut state = DiffState::new("a", "b", &before, &after);
    assert_eq!(state.scroll, 0);

    diff::apply(&mut state, DiffMsg::NextHunk, 5);
    assert!(state.scroll > 0, "scroll quedó en 0 — no saltó al hunk");
    let row = &state.rows[state.scroll];
    assert!(
        !matches!(row.kind, DiffKind::Equal),
        "esperaba aterrizar en un hunk, vi {:?}",
        row.kind
    );

    // PrevHunk: vuelve al inicio (no hay hunk antes del primer cambio).
    diff::apply(&mut state, DiffMsg::PrevHunk, 5);
    // Puede quedarse en el mismo hunk si era el único accesible hacia
    // atrás, o saltar más arriba. Lo único que verificamos es que no
    // hubo panic ni scroll fuera de rango.
    assert!(state.scroll < state.rows.len());
}

#[test]
fn escape_cierra() {
    let mut state = DiffState::new("a", "b", "x\n", "y\n");
    let action = diff::apply(&mut state, DiffMsg::Close, 10);
    assert_eq!(action, DiffAction::Close);
}

#[test]
fn open_shortcut_es_ctrl_shift_d() {
    use llimphi_ui::{Key, KeyEvent, KeyState, Modifiers};
    let mk = |ctrl: bool, shift: bool, c: &str| KeyEvent {
        key: Key::Character(c.into()),
        state: KeyState::Pressed,
        text: Some(c.into()),
        modifiers: Modifiers { ctrl, shift, ..Modifiers::default() },
        repeat: false,
    };
    assert!(diff::open_shortcut(&mk(true, true, "d")));
    assert!(diff::open_shortcut(&mk(true, true, "D")));
    assert!(!diff::open_shortcut(&mk(true, false, "d")));
    assert!(!diff::open_shortcut(&mk(false, true, "d")));
}
