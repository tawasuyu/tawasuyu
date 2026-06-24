use super::*;
use crate::mouse_xterm::{encode, local_to_cell, XBtn, XPhase};

/// `true` si hay un `ActiveRun` con PTY vivo. Las teclas van al stdin del
/// PTY mientras esto sea cierto (el programa es interactivo, esté o no en
/// pantalla completa). El **render** en cambio sigue a [`is_tui_fullscreen`].
///
/// **No-blocking**: usa `try_lock`. Si el lector del PTY tiene el mutex en
/// este instante (drenando una ráfaga grande de output, p. ej. `ls -alR`),
/// volvemos `false` antes que pasmar el thread de pintura: pintar `false`
/// un frame de más es indistinguible de "todavía no llegó el dato", pero
/// bloquear el render durante una ráfaga deja la pantalla negra.
pub(crate) fn is_tui_active(s: &State) -> bool {
    let Some(arc) = s.running.as_ref() else {
        return false;
    };
    let g = match arc.try_lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    g.tui.is_some()
}

/// `true` si el PTY vivo entró a **alternate screen** (`ESC[?1049h`) — la
/// señal dura de una app TUI de pantalla completa (vim, htop, less, man…).
/// Es lo que decide pintar el panel full-screen (grid/vim) en vez de las
/// líneas. Al salir del alt-screen (`ESC[?1049l`) vuelve a modo líneas.
///
/// Misma política `try_lock` que [`is_tui_active`]: ante contienda, `false`
/// — el render cae al pane de cards (que sí usa data ya volcada a
/// `state.output`) y nunca se pasma esperando al lector del PTY.
pub(crate) fn is_tui_fullscreen(s: &State) -> bool {
    let Some(arc) = s.running.as_ref() else {
        return false;
    };
    let g = match arc.try_lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    g.tui
        .as_ref()
        .map(|t| t.parser.screen().alternate_screen())
        .unwrap_or(false)
}

/// El `AppSkin` del run vivo (si hay PTY/TUI), para el aviso visual. Misma
/// política `try_lock` que [`is_tui_fullscreen`]: ante contienda, `None`.
pub(crate) fn running_skin(s: &State) -> Option<crate::AppSkin> {
    let arc = s.running.as_ref()?;
    let g = arc.try_lock().ok()?;
    g.tui.as_ref().map(|t| t.skin)
}

/// Contenido de la pantalla del PTY vivo cuando está en **modo líneas**
/// (PTY presente, sin alt-screen). Devuelve las filas como texto (sin
/// formato), recortando las filas vacías del final. `None` si no hay PTY
/// o está en pantalla completa (ese caso lo pinta el panel full-screen).
/// Las salidas de programas que no toman la pantalla (p. ej. `watch`) se
/// leen así como texto normal en vez de una grilla apretada.
pub(crate) fn pty_line_text(s: &State) -> Option<Vec<String>> {
    let arc = s.running.as_ref()?;
    let g = match arc.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    let tui = g.tui.as_ref()?;
    let screen = tui.parser.screen();
    if screen.alternate_screen() {
        return None;
    }
    Some(screen_to_lines(screen))
}

/// Filas de un `vt100::Screen` como texto sin formato, recortando las
/// filas vacías del final. Pura (sin State) para poder testearla con un
/// parser construido a mano.
pub(crate) fn screen_to_lines(screen: &vt100::Screen) -> Vec<String> {
    let (_rows, cols) = screen.size();
    let mut lines: Vec<String> = screen
        .rows(0, cols)
        .map(|r| r.trim_end().to_string())
        .collect();
    while lines.last().map(|l| l.is_empty()).unwrap_or(false) {
        lines.pop();
    }
    lines
}

/// Traduce una tecla a su secuencia de bytes para el PTY (xterm-compat).
/// Las TUIs esperan estos códigos.
pub(crate) fn key_to_pty_bytes(ev: &KeyEvent) -> Vec<u8> {
    match &ev.key {
        Key::Named(NamedKey::Enter) => b"\r".to_vec(),
        Key::Named(NamedKey::Tab) => b"\t".to_vec(),
        Key::Named(NamedKey::Backspace) => b"\x7f".to_vec(),
        Key::Named(NamedKey::Escape) => b"\x1b".to_vec(),
        Key::Named(NamedKey::ArrowUp) => b"\x1b[A".to_vec(),
        Key::Named(NamedKey::ArrowDown) => b"\x1b[B".to_vec(),
        Key::Named(NamedKey::ArrowRight) => b"\x1b[C".to_vec(),
        Key::Named(NamedKey::ArrowLeft) => b"\x1b[D".to_vec(),
        Key::Named(NamedKey::Home) => b"\x1b[H".to_vec(),
        Key::Named(NamedKey::End) => b"\x1b[F".to_vec(),
        Key::Named(NamedKey::PageUp) => b"\x1b[5~".to_vec(),
        Key::Named(NamedKey::PageDown) => b"\x1b[6~".to_vec(),
        Key::Named(NamedKey::Delete) => b"\x1b[3~".to_vec(),
        Key::Named(NamedKey::Space) => b" ".to_vec(),
        _ => {
            // Ctrl-<x>: codifica el byte 0x01..0x1a para letras.
            if ev.modifiers.ctrl {
                if let Key::Character(c) = &ev.key {
                    if let Some(ch) = c.chars().next() {
                        let lo = ch.to_ascii_lowercase();
                        if ('a'..='z').contains(&lo) {
                            return vec![(lo as u8) - b'a' + 1];
                        }
                    }
                }
            }
            ev.text.as_deref().unwrap_or("").as_bytes().to_vec()
        }
    }
}

/// Manda los bytes de la tecla al PTY del run activo. No-op si no hay
/// tui activo.
pub(crate) fn forward_key_to_pty(s: &State, ev: &KeyEvent) {
    let Some(arc) = s.running.as_ref() else {
        return;
    };
    let bytes = key_to_pty_bytes(ev);
    if bytes.is_empty() {
        return;
    }
    let guard = match arc.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    guard.handle.write_input(bytes);
}

/// Pega el contenido del clipboard en el PTY del run activo. Si el TUI
/// hijo está en bracketed-paste mode (DECSET 2004), envuelve la
/// secuencia en `\x1b[200~...\x1b[201~` para que vim, less y emacs
/// distingan "tipeé esto" de "pegué esto" (auto-indent, paste-mode,
/// etc.). No-op silencioso si no hay TUI o el clipboard está vacío.
pub(crate) fn forward_paste_to_pty(s: &State) {
    let Some(arc) = s.running.as_ref() else {
        return;
    };
    let Some(text) = read_clipboard() else {
        return;
    };
    if text.is_empty() {
        return;
    }
    let guard = match arc.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    let bracketed = guard
        .tui
        .as_ref()
        .map(|t| t.parser.screen().bracketed_paste())
        .unwrap_or(false);
    let payload: Vec<u8> = if bracketed {
        let mut buf: Vec<u8> = b"\x1b[200~".to_vec();
        buf.extend_from_slice(text.as_bytes());
        buf.extend_from_slice(b"\x1b[201~");
        buf
    } else {
        text.into_bytes()
    };
    guard.handle.write_input(payload);
}

/// Convierte un click sobre el panel TUI en bytes xterm-mouse y los manda
/// al PTY del run activo. No-op si el programa no habilitó mouse
/// (`MouseProtocolMode::None`) o no hay TUI. Para modos que reportan
/// release (VT200/ButtonMotion/AnyMotion), encadena Press + Release en una
/// sola escritura — los TUIs (vim/htop/btop) los procesan en ese orden.
pub(crate) fn forward_tui_click_to_pty(
    s: &State,
    button: u8,
    lx: f32,
    ly: f32,
    rect_w: f32,
    rect_h: f32,
) {
    let Some(arc) = s.running.as_ref() else {
        return;
    };
    let guard = match arc.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    let Some(tui) = guard.tui.as_ref() else {
        return;
    };
    let screen = tui.parser.screen();
    let mode = screen.mouse_protocol_mode();
    if matches!(mode, vt100::MouseProtocolMode::None) {
        return;
    }
    let encoding = screen.mouse_protocol_encoding();
    let btn = match button {
        0 => XBtn::Left,
        1 => XBtn::Middle,
        2 => XBtn::Right,
        _ => return,
    };
    let (col, row) = local_to_cell(lx, ly, rect_w, rect_h, tui.cols, tui.rows);
    let mut payload: Vec<u8> = Vec::new();
    if let Some(b) = encode(mode, encoding, btn, XPhase::Press, col, row) {
        payload.extend_from_slice(&b);
    }
    if let Some(b) = encode(mode, encoding, btn, XPhase::Release, col, row) {
        payload.extend_from_slice(&b);
    }
    if !payload.is_empty() {
        guard.handle.write_input(payload);
    }
}

/// Convierte un tick de rueda sobre el panel TUI en eventos xterm-mouse
/// (button 4 = arriba, button 5 = abajo) y los manda al PTY. Emite tantos
/// "press" como ticks lógicos (ceil de `|dy|`) — la rueda no tiene release
/// en xterm. No-op si el programa no habilitó mouse o no hay TUI.
pub(crate) fn forward_tui_wheel_to_pty(
    s: &State,
    dy: f32,
    lx: f32,
    ly: f32,
    rect_w: f32,
    rect_h: f32,
) {
    let Some(arc) = s.running.as_ref() else {
        return;
    };
    let guard = match arc.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    let Some(tui) = guard.tui.as_ref() else {
        return;
    };
    let screen = tui.parser.screen();
    let mode = screen.mouse_protocol_mode();
    if matches!(mode, vt100::MouseProtocolMode::None) {
        return;
    }
    let encoding = screen.mouse_protocol_encoding();
    let btn = if dy > 0.0 { XBtn::WheelUp } else { XBtn::WheelDown };
    let ticks = dy.abs().ceil() as u32;
    if ticks == 0 {
        return;
    }
    let (col, row) = local_to_cell(lx, ly, rect_w, rect_h, tui.cols, tui.rows);
    let mut payload: Vec<u8> = Vec::new();
    for _ in 0..ticks.min(8) {
        if let Some(b) = encode(mode, encoding, btn, XPhase::Press, col, row) {
            payload.extend_from_slice(&b);
        }
    }
    if !payload.is_empty() {
        guard.handle.write_input(payload);
    }
}
