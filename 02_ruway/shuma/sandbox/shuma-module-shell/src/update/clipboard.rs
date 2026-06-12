use super::*;
use crate::view::{VIM_CHAR_W, VIM_LINE_H, vim_px_to_cell};

/// Lee el clipboard del SO (vía `arboard`). Devuelve `None` si no hay
/// display server, está vacío, o el contenido no es texto. No cachea —
/// el sistema tiene su propio TTL.
pub(crate) fn read_clipboard() -> Option<String> {
    let mut clip = arboard::Clipboard::new().ok()?;
    clip.get_text().ok()
}

/// Limpia texto pegado al editor de línea. A diferencia del shell GPUI
/// (que colapsaba todo a una línea unida por `; `), este input es
/// **multilínea** —editar construcciones abiertas, pegar scripts—, así que
/// los saltos se **preservan**. Lo que sí hacemos:
///
/// - normalizar `\r\n` y `\r` a `\n` (pastes de Windows / terminales),
/// - tab → espacio (el line editor no tabula columnas),
/// - descartar caracteres de control peligrosos (ESC, BEL, …) que un paste
///   de terminal puede arrastrar y que corromperían el render del input,
/// - recortar **un** salto final, para que pegar `"ls -la\n"` no deje una
///   línea vacía colgando bajo el comando.
pub(crate) fn sanitize_paste(s: &str) -> String {
    let normalized = s.replace("\r\n", "\n").replace('\r', "\n");
    let cleaned: String = normalized
        .chars()
        .map(|c| if c == '\t' { ' ' } else { c })
        .filter(|c| *c == '\n' || !c.is_control())
        .collect();
    cleaned
        .strip_suffix('\n')
        .map(str::to_string)
        .unwrap_or(cleaned)
}

/// Escribe texto al clipboard del SO. No-op silencioso sin display server.
pub(crate) fn set_clipboard(text: &str) {
    if let Ok(mut clip) = arboard::Clipboard::new() {
        let _ = clip.set_text(text.to_string());
    }
}

/// Extrae el texto de la selección del card de vim sobre el screen
/// actual del PTY y lo copia al clipboard. Selección lineal por filas
/// (estilo terminal), cada fila recortada de espacios al final.
pub(crate) fn copy_vim_selection(s: &State) {
    let Some(vs) = s.vim_sel else { return };
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
    let (rows, cols) = screen.size();
    let mut grid: Vec<Vec<char>> = Vec::with_capacity(rows as usize);
    for r in 0..rows {
        let mut line: Vec<char> = Vec::with_capacity(cols as usize);
        for c in 0..cols {
            let ch = match screen.cell(r, c) {
                Some(cell) if cell.has_contents() => cell.contents().chars().next().unwrap_or(' '),
                _ => ' ',
            };
            line.push(ch);
        }
        grid.push(line);
    }
    let (cw, lh) = match s.vim_metrics.lock() {
        Ok(g) if g.0 > 1.0 && g.1 > 1.0 => (g.0 as f64, g.1 as f64),
        _ => (VIM_CHAR_W, VIM_LINE_H),
    };
    let (r0, c0) = vim_px_to_cell(vs.ax as f64, vs.ay as f64, cw, lh);
    let (r1, c1) = vim_px_to_cell(vs.hx as f64, vs.hy as f64, cw, lh);
    let (sr, sc, er, ec) = if (r0, c0) <= (r1, c1) {
        (r0, c0, r1, c1)
    } else {
        (r1, c1, r0, c0)
    };
    if sr >= grid.len() {
        return;
    }
    let er = er.min(grid.len() - 1);
    let mut out = String::new();
    for r in sr..=er {
        let line = &grid[r];
        let lo = if r == sr { sc.min(line.len()) } else { 0 };
        let hi = if r == er {
            (ec + 1).min(line.len())
        } else {
            line.len()
        };
        if hi > lo {
            let seg: String = line[lo..hi].iter().collect();
            out.push_str(seg.trim_end());
        }
        if r != er {
            out.push('\n');
        }
    }
    if !out.trim().is_empty() {
        set_clipboard(&out);
    }
}
