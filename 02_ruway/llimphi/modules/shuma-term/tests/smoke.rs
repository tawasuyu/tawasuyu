//! Smoke test del terminal: spawnea un shell, le tipea `echo hola`,
//! drena hasta ver el output, y verifica que el contenido del screen
//! contenga "hola". Cierre con SIGTERM se valida por el Drop.
//!
//! Requiere `/bin/sh` y un sistema Linux real (no corre en sandbox
//! puro). Es razonable porque shuma-exec ya lo asume.

use std::time::{Duration, Instant};

use llimphi_module_shuma_term::{self as term, ShumaTermAction, ShumaTermMsg};

#[test]
fn echo_a_traves_del_pty_aparece_en_el_screen() {
    let mut state = term::spawn_with(
        "/tmp".to_string(),
        "/bin/sh".to_string(),
        Vec::new(),
        80,
        24,
    );

    // El shell escribe su prompt al arrancar; lo drenamos sin asumir
    // su contenido (cambia por distro).
    spin_drain(&mut state, Duration::from_millis(200));

    // Tipeamos el comando. Sin Llimphi alrededor llamamos a write_input
    // directamente — el módulo permite hacerlo via KeyInput, pero
    // construir KeyEvents acá es ruido para este test.
    write_raw(&mut state, b"echo hola_del_test\n");

    // Esperamos hasta 2s a que el output llegue al screen.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut visto = false;
    while Instant::now() < deadline {
        spin_drain(&mut state, Duration::from_millis(50));
        if state.screen_contents().contains("hola_del_test") {
            visto = true;
            break;
        }
    }
    assert!(
        visto,
        "esperaba ver 'hola_del_test' en el screen, contenido actual:\n{}",
        state.screen_contents()
    );
}

#[test]
fn ctrl_shift_w_emite_action_close_sin_pasar_al_pty() {
    use llimphi_ui::{Key, KeyEvent, KeyState, Modifiers};

    let mut state = term::spawn_with(
        "/tmp".to_string(),
        "/bin/sh".to_string(),
        Vec::new(),
        80,
        24,
    );
    spin_drain(&mut state, Duration::from_millis(100));

    let ev = KeyEvent {
        key: Key::Character("w".into()),
        state: KeyState::Pressed,
        text: Some("w".into()),
        modifiers: Modifiers { ctrl: true, shift: true, ..Modifiers::default() },
        repeat: false,
    };
    let action = term::apply(&mut state, ShumaTermMsg::KeyInput(ev));
    assert_eq!(action, ShumaTermAction::Close);
}

#[test]
fn key_to_bytes_mapea_los_casos_canonicos() {
    use llimphi_ui::{Key, KeyEvent, KeyState, Modifiers, NamedKey};

    let mk = |key: Key, mods: Modifiers, text: Option<&str>| KeyEvent {
        key,
        state: KeyState::Pressed,
        text: text.map(|s| s.to_string()),
        modifiers: mods,
        repeat: false,
    };

    // Enter → CR (no LF — el driver del PTY lo expande).
    assert_eq!(
        term::key_to_bytes(&mk(Key::Named(NamedKey::Enter), Modifiers::default(), None)),
        b"\r"
    );
    // Backspace → DEL.
    assert_eq!(
        term::key_to_bytes(&mk(
            Key::Named(NamedKey::Backspace),
            Modifiers::default(),
            None
        )),
        vec![0x7f]
    );
    // ArrowUp → CSI A.
    assert_eq!(
        term::key_to_bytes(&mk(
            Key::Named(NamedKey::ArrowUp),
            Modifiers::default(),
            None
        )),
        b"\x1b[A"
    );
    // Ctrl+C → 0x03.
    let ctrl = Modifiers { ctrl: true, ..Modifiers::default() };
    assert_eq!(
        term::key_to_bytes(&mk(Key::Character("c".into()), ctrl, Some("c"))),
        vec![0x03]
    );
    // Texto plano (con shift aplicado por el backend) → ese mismo texto.
    assert_eq!(
        term::key_to_bytes(&mk(Key::Character("A".into()), Modifiers::default(), Some("A"))),
        b"A"
    );
    // Alt+x → ESC + x.
    let alt = Modifiers { alt: true, ..Modifiers::default() };
    assert_eq!(
        term::key_to_bytes(&mk(Key::Character("x".into()), alt, Some("x"))),
        vec![0x1b, b'x']
    );
}

// ---------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------

/// Pequeño polling: dispara Tick varias veces durante `total` para que
/// el módulo drene los bytes que el reader thread haya emitido.
fn spin_drain(state: &mut llimphi_module_shuma_term::ShumaTermState, total: Duration) {
    let deadline = Instant::now() + total;
    while Instant::now() < deadline {
        term::apply(state, ShumaTermMsg::Tick);
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Atajo: enviar bytes crudos al PTY sin construir un KeyEvent. Usa la
/// API pública via un truco — convertimos a un KeyEvent "texto" para
/// evitar exponer write_input crudo en el contrato.
fn write_raw(state: &mut llimphi_module_shuma_term::ShumaTermState, bytes: &[u8]) {
    use llimphi_ui::{Key, KeyEvent, KeyState, Modifiers};
    // Texto entero (incluyendo el LF) en un solo KeyInput. apply() lo
    // copia tal cual al PTY via la rama `text`.
    let s = std::str::from_utf8(bytes).expect("test usa ascii");
    let ev = KeyEvent {
        // Key::Character vacío para que no entremos por la rama ctrl/alt.
        key: Key::Character("".into()),
        state: KeyState::Pressed,
        text: Some(s.to_string()),
        modifiers: Modifiers::default(),
        repeat: false,
    };
    term::apply(state, ShumaTermMsg::KeyInput(ev));
}
