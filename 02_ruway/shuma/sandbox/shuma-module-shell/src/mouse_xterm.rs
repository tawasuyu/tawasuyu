//! Codificación xterm de eventos de mouse para PTY/vt100.
//!
//! Convierte un evento de Llimphi (click/wheel sobre el panel TUI) en los
//! bytes que el programa bajo el PTY espera leer cuando habilitó alguna
//! variante del **xterm mouse protocol**. El parser [`vt100`] expone qué modo
//! y qué encoding pidió el programa: este módulo sólo emite la secuencia
//! adecuada — no decide si el mouse está habilitado (eso lo chequea el
//! caller con `screen.mouse_protocol_mode()`).
//!
//! ## Codings soportados
//!
//! - **Default** (X10/“legacy”): `\x1b[M Cb Cx Cy` con cada byte ASCII
//!   (cols/rows limitados a 1..=223).
//! - **SGR** (`DECSET 1006`): `\x1b[< Cb ; Cx ; Cy M` para press / `m` para
//!   release. Soporta cols/rows arbitrarios y release distinguible.
//! - **UTF-8** (`DECSET 1005`): variante intermedia — emitida como Default
//!   por simplicidad (los TUIs modernos negocian SGR).
//!
//! ## Modos
//!
//! - `Press` (X10): sólo press, sin release ni motion.
//! - `PressRelease` (VT200), `ButtonMotion`, `AnyMotion`: además de press,
//!   reporta release (y motion si está en modo motion — no implementado).

use vt100::{MouseProtocolEncoding, MouseProtocolMode};

/// Botón del mouse en términos xterm (Cb base, sin modificadores).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XBtn {
    Left = 0,
    Middle = 1,
    Right = 2,
    /// Rueda hacia arriba (button 4 = Cb 64 en Default; 64 en SGR).
    WheelUp = 64,
    /// Rueda hacia abajo (button 5).
    WheelDown = 65,
}

/// Fase del evento.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XPhase {
    Press,
    Release,
}

/// Encodea un evento de mouse para el `(mode, encoding)` activos. Devuelve
/// la secuencia de bytes a escribir al stdin del PTY, o `None` si el modo no
/// reporta este tipo de evento (p. ej. `Press` (X10) sólo reporta press).
///
/// `col`/`row` son 1-based (la celda *visible*; el caller las clampa al
/// tamaño del grid antes de llamar).
pub fn encode(
    mode: MouseProtocolMode,
    encoding: MouseProtocolEncoding,
    btn: XBtn,
    phase: XPhase,
    col: u16,
    row: u16,
) -> Option<Vec<u8>> {
    // Mouse deshabilitado: nada que mandar.
    if matches!(mode, MouseProtocolMode::None) {
        return None;
    }
    // X10 sólo reporta press. Release/motion se filtran en origen.
    if matches!(mode, MouseProtocolMode::Press) && phase == XPhase::Release {
        return None;
    }
    match encoding {
        MouseProtocolEncoding::Sgr => Some(encode_sgr(btn, phase, col, row)),
        MouseProtocolEncoding::Default | MouseProtocolEncoding::Utf8 => {
            Some(encode_default(btn, phase, col, row))
        }
    }
}

/// SGR (`DECSET 1006`): `\x1b[< Cb ; Cx ; Cy M` para press, `m` para release.
/// Sin limites de col/row.
fn encode_sgr(btn: XBtn, phase: XPhase, col: u16, row: u16) -> Vec<u8> {
    let cb = btn as u32;
    let terminator = if matches!(phase, XPhase::Release) { 'm' } else { 'M' };
    format!("\x1b[<{};{};{}{}", cb, col, row, terminator).into_bytes()
}

/// Default/X10: `\x1b[M Cb Cx Cy` con cada coord = pos+32 (offset por 1, base
/// 1). En release el bit bajo del Cb se setea a 3 (button-release ambiguo).
fn encode_default(btn: XBtn, phase: XPhase, col: u16, row: u16) -> Vec<u8> {
    let mut cb = btn as u32;
    if matches!(phase, XPhase::Release) {
        // En X10 default, release usa el código 3 en los bits bajos (cualquier
        // botón). vt100 entiende esto.
        cb = (cb & !0b11) | 0b11;
    }
    // Clampea coords a 1..=223 (cabe en un byte ASCII tras el offset de 32).
    let c = (col.clamp(1, 223) as u32) + 32;
    let r = (row.clamp(1, 223) as u32) + 32;
    let cb_byte = (cb + 32).min(255) as u8;
    let c_byte = c.min(255) as u8;
    let r_byte = r.min(255) as u8;
    vec![0x1b, b'[', b'M', cb_byte, c_byte, r_byte]
}

/// Helper: convierte `(lx, ly, rect_w, rect_h, grid_cols, grid_rows)` en
/// `(col, row)` 1-based clampeado al tamaño del grid. Replica el cálculo de
/// `cell_w`/`cell_h` del painter del `generic_grid_panel` para mantener la
/// hit-test consistente con lo pintado.
pub fn local_to_cell(
    lx: f32,
    ly: f32,
    rect_w: f32,
    rect_h: f32,
    grid_cols: u16,
    grid_rows: u16,
) -> (u16, u16) {
    // Padding interior del painter (debe seguir a `tui_panel::generic_grid_panel`).
    const PAD: f32 = 6.0;
    let avail_w = (rect_w - PAD * 2.0).max(1.0);
    let avail_h = (rect_h - PAD * 2.0).max(1.0);
    let cell_w = (avail_w / grid_cols as f32).max(1.0);
    let cell_h = (avail_h / grid_rows as f32).max(1.0);
    let lx_in = (lx - PAD).max(0.0);
    let ly_in = (ly - PAD).max(0.0);
    // Clampea como f32 ANTES de castear a u16 (un click muy lejos del rect
    // —p.ej. coords basura— no debe overflowear el cast). El `+1` que sigue
    // queda dentro de los límites del grid.
    let col_f = (lx_in / cell_w).clamp(0.0, grid_cols.max(1) as f32 - 1.0);
    let row_f = (ly_in / cell_h).clamp(0.0, grid_rows.max(1) as f32 - 1.0);
    let col = (col_f as u16) + 1;
    let row = (row_f as u16) + 1;
    (col, row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sgr_press_left_at_5_3() {
        let bytes = encode_sgr(XBtn::Left, XPhase::Press, 5, 3);
        assert_eq!(bytes, b"\x1b[<0;5;3M");
    }

    #[test]
    fn sgr_release_right_at_10_7() {
        let bytes = encode_sgr(XBtn::Right, XPhase::Release, 10, 7);
        assert_eq!(bytes, b"\x1b[<2;10;7m");
    }

    #[test]
    fn sgr_wheel_up_y_down() {
        let up = encode_sgr(XBtn::WheelUp, XPhase::Press, 1, 1);
        let down = encode_sgr(XBtn::WheelDown, XPhase::Press, 1, 1);
        assert_eq!(up, b"\x1b[<64;1;1M");
        assert_eq!(down, b"\x1b[<65;1;1M");
    }

    #[test]
    fn default_press_left_at_1_1() {
        let bytes = encode_default(XBtn::Left, XPhase::Press, 1, 1);
        // Cb = 0+32 = 32 (' '), col = 1+32 = 33 ('!'), row = 1+32 = 33 ('!').
        assert_eq!(bytes, b"\x1b[M !!");
    }

    #[test]
    fn default_release_marca_button_release() {
        let bytes = encode_default(XBtn::Left, XPhase::Release, 1, 1);
        // Cb = (0&!3)|3 = 3; 3+32 = 35 ('#').
        assert_eq!(bytes, b"\x1b[M#!!");
    }

    #[test]
    fn x10_mode_filtra_release() {
        // En X10 los release no se reportan — el `encode` devuelve None.
        assert!(
            encode(MouseProtocolMode::Press, MouseProtocolEncoding::Sgr, XBtn::Left, XPhase::Release, 1, 1)
                .is_none()
        );
    }

    #[test]
    fn modo_none_filtra_todo() {
        assert!(
            encode(MouseProtocolMode::None, MouseProtocolEncoding::Sgr, XBtn::Left, XPhase::Press, 1, 1)
                .is_none()
        );
    }

    #[test]
    fn vt200_reporta_press_y_release() {
        assert!(encode(
            MouseProtocolMode::PressRelease,
            MouseProtocolEncoding::Sgr,
            XBtn::Left,
            XPhase::Press,
            5,
            3,
        )
        .is_some());
        assert!(encode(
            MouseProtocolMode::PressRelease,
            MouseProtocolEncoding::Sgr,
            XBtn::Left,
            XPhase::Release,
            5,
            3,
        )
        .is_some());
    }

    #[test]
    fn cell_centrada_cae_en_la_celda_correcta() {
        // Grid 10x5 en un rect 100x50 con padding 6: cell_w = (100-12)/10 = 8.8
        // cell_h = (50-12)/5 = 7.6. Click en (6+8.8*2 + 4, 6+7.6*1 + 2) ≈ (27.6, 15.6).
        // Esperado: col 3, row 2 (1-based).
        let (c, r) = local_to_cell(27.6, 15.6, 100.0, 50.0, 10, 5);
        assert_eq!(c, 3);
        assert_eq!(r, 2);
    }

    #[test]
    fn cell_clampa_al_grid() {
        // Click muy lejos del rect: la última celda.
        let (c, r) = local_to_cell(1e6, 1e6, 100.0, 50.0, 10, 5);
        assert_eq!(c, 10);
        assert_eq!(r, 5);
    }

    #[test]
    fn cell_pad_va_a_la_primera_celda() {
        // Click dentro del padding (esquina superior-izquierda): cae a (1,1).
        let (c, r) = local_to_cell(0.0, 0.0, 100.0, 50.0, 10, 5);
        assert_eq!(c, 1);
        assert_eq!(r, 1);
    }
}
