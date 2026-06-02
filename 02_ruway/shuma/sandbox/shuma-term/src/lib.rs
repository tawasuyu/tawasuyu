//! `shuma-term` — emulador de terminal sync, agnóstico de UI.
//!
//! Toma los bytes crudos de un PTY (los que llegan por el `<card_id>.sock` de
//! sandokan) y mantiene un **grid de celdas** con atributos: el modelo que el
//! front Llimphi pinta. El parsing de secuencias VT/ANSI lo hace `vte`; acá
//! traducimos sus eventos a movimientos de cursor, escritura de celdas,
//! borrados y atributos (SGR: bold + 16 colores).
//!
//! Es deliberadamente un subconjunto (sin scrollback histórico, sin modos
//! alternos, sin DEC private completos): cubre lo que un shell típico emite
//! (prompt, `ls --color`, edición de línea, `clear`). Suficiente para una
//! sesión usable; los huecos se llenan cuando un caso real los pida.

#![forbid(unsafe_code)]

/// Color de celda: índice ANSI 0..15, o [`Color::DEFAULT`] (color del tema).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub u8);

impl Color {
    /// Sentinela "usar el color por defecto del tema" (no es un índice ANSI).
    pub const DEFAULT: Color = Color(0xFF);
    /// `true` si es el color por defecto (el front usa el del tema).
    pub fn is_default(self) -> bool {
        self.0 == 0xFF
    }
}

/// Una celda del grid: un carácter + sus atributos visuales.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: Color::DEFAULT,
            bg: Color::DEFAULT,
            bold: false,
        }
    }
}

/// Emulador de terminal: parser VT + pantalla. Alimentá bytes con
/// [`Terminal::feed`] y leé el grid con [`Terminal::row`]/[`Terminal::cursor`].
pub struct Terminal {
    parser: vte::Parser,
    screen: Screen,
}

impl Terminal {
    /// Crea un terminal de `cols`×`rows` celdas (mínimo 1×1).
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            parser: vte::Parser::new(),
            screen: Screen::new(cols.max(1), rows.max(1)),
        }
    }

    /// Procesa un chunk de bytes del PTY, mutando la pantalla.
    pub fn feed(&mut self, bytes: &[u8]) {
        let Self { parser, screen } = self;
        parser.advance(screen, bytes);
    }

    /// Redimensiona la pantalla (al cambiar el tamaño de la ventana). Conserva
    /// el contenido que cabe; el resto se recorta/rellena con celdas vacías.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.screen.resize(cols.max(1), rows.max(1));
    }

    pub fn cols(&self) -> usize {
        self.screen.cols
    }
    pub fn rows(&self) -> usize {
        self.screen.rows
    }

    /// Celdas de la fila `y` (0 = arriba). Vacío si `y` está fuera de rango.
    pub fn row(&self, y: usize) -> &[Cell] {
        if y < self.screen.rows {
            &self.screen.cells[y * self.screen.cols..(y + 1) * self.screen.cols]
        } else {
            &[]
        }
    }

    /// Posición del cursor `(col, row)`.
    pub fn cursor(&self) -> (usize, usize) {
        (self.screen.cx, self.screen.cy)
    }
}

struct Screen {
    cols: usize,
    rows: usize,
    cells: Vec<Cell>,
    cx: usize,
    cy: usize,
    fg: Color,
    bg: Color,
    bold: bool,
}

impl Screen {
    fn new(cols: usize, rows: usize) -> Self {
        Self {
            cols,
            rows,
            cells: vec![Cell::default(); cols * rows],
            cx: 0,
            cy: 0,
            fg: Color::DEFAULT,
            bg: Color::DEFAULT,
            bold: false,
        }
    }

    fn resize(&mut self, cols: usize, rows: usize) {
        let mut next = vec![Cell::default(); cols * rows];
        for y in 0..rows.min(self.rows) {
            for x in 0..cols.min(self.cols) {
                next[y * cols + x] = self.cells[y * self.cols + x];
            }
        }
        self.cells = next;
        self.cols = cols;
        self.rows = rows;
        self.cx = self.cx.min(cols.saturating_sub(1));
        self.cy = self.cy.min(rows.saturating_sub(1));
    }

    fn put(&mut self, c: char) {
        if self.cx >= self.cols {
            self.cx = 0;
            self.line_feed();
        }
        let i = self.cy * self.cols + self.cx;
        self.cells[i] = Cell {
            ch: c,
            fg: self.fg,
            bg: self.bg,
            bold: self.bold,
        };
        self.cx += 1;
    }

    fn line_feed(&mut self) {
        self.cy += 1;
        if self.cy >= self.rows {
            self.scroll_up();
            self.cy = self.rows - 1;
        }
    }

    fn scroll_up(&mut self) {
        // Tira la primera fila y agrega una vacía abajo.
        self.cells.drain(0..self.cols);
        self.cells
            .extend(std::iter::repeat(Cell::default()).take(self.cols));
    }

    fn fill(&mut self, range: std::ops::Range<usize>) {
        for c in &mut self.cells[range] {
            *c = Cell::default();
        }
    }

    fn sgr(&mut self, code: u16) {
        match code {
            0 => {
                self.fg = Color::DEFAULT;
                self.bg = Color::DEFAULT;
                self.bold = false;
            }
            1 => self.bold = true,
            22 => self.bold = false,
            30..=37 => self.fg = Color((code - 30) as u8),
            39 => self.fg = Color::DEFAULT,
            40..=47 => self.bg = Color((code - 40) as u8),
            49 => self.bg = Color::DEFAULT,
            90..=97 => self.fg = Color((code - 90 + 8) as u8),
            100..=107 => self.bg = Color((code - 100 + 8) as u8),
            _ => {}
        }
    }
}

/// Primer valor del param `idx` (subparam 0), o `default`.
fn param_at(params: &vte::Params, idx: usize, default: u16) -> u16 {
    params
        .iter()
        .nth(idx)
        .and_then(|p| p.first().copied())
        .filter(|&v| v != 0)
        .unwrap_or(default)
}

impl vte::Perform for Screen {
    fn print(&mut self, c: char) {
        self.put(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' | 0x0b | 0x0c => self.line_feed(), // LF, VT, FF
            b'\r' => self.cx = 0,
            0x08 => self.cx = self.cx.saturating_sub(1), // BS
            b'\t' => self.cx = ((self.cx / 8) + 1) * 8,  // siguiente tab stop
            _ => {}
        }
        if self.cx > self.cols.saturating_sub(1) {
            self.cx = self.cols.saturating_sub(1);
        }
    }

    fn csi_dispatch(&mut self, params: &vte::Params, _inter: &[u8], _ignore: bool, action: char) {
        let n = || param_at(params, 0, 1) as usize;
        match action {
            'A' => self.cy = self.cy.saturating_sub(n()),
            'B' => self.cy = (self.cy + n()).min(self.rows - 1),
            'C' => self.cx = (self.cx + n()).min(self.cols - 1),
            'D' => self.cx = self.cx.saturating_sub(n()),
            'H' | 'f' => {
                let row = param_at(params, 0, 1) as usize;
                let col = param_at(params, 1, 1) as usize;
                self.cy = row.saturating_sub(1).min(self.rows - 1);
                self.cx = col.saturating_sub(1).min(self.cols - 1);
            }
            'J' => {
                // Erase in Display: 0 = del cursor abajo, 1 = arriba, 2 = todo.
                let cur = self.cy * self.cols + self.cx;
                let total = self.cells.len();
                match param_at(params, 0, 0) {
                    1 => self.fill(0..(cur + 1).min(total)),
                    2 => self.fill(0..total),
                    _ => self.fill(cur..total),
                }
            }
            'K' => {
                // Erase in Line: 0 = a la derecha, 1 = a la izquierda, 2 = toda.
                let start = self.cy * self.cols;
                let end = start + self.cols;
                let cur = start + self.cx;
                match param_at(params, 0, 0) {
                    1 => self.fill(start..(cur + 1).min(end)),
                    2 => self.fill(start..end),
                    _ => self.fill(cur..end),
                }
            }
            'm' => {
                // SGR: cada param es un atributo. Sin params = reset.
                if params.iter().next().is_none() {
                    self.sgr(0);
                } else {
                    for p in params.iter() {
                        self.sgr(p.first().copied().unwrap_or(0));
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(t: &Terminal, y: usize) -> String {
        t.row(y).iter().map(|c| c.ch).collect::<String>()
    }

    #[test]
    fn prints_text_and_wraps_newline() {
        let mut t = Terminal::new(10, 4);
        t.feed(b"hello\r\nworld");
        assert_eq!(line(&t, 0).trim_end(), "hello");
        assert_eq!(line(&t, 1).trim_end(), "world");
        assert_eq!(t.cursor(), (5, 1));
    }

    #[test]
    fn autowraps_at_right_edge() {
        let mut t = Terminal::new(3, 4);
        t.feed(b"abcd"); // "abc" llena la fila 0, "d" cae a la fila 1
        assert_eq!(line(&t, 0), "abc");
        assert_eq!(line(&t, 1).trim_end(), "d");
    }

    #[test]
    fn cursor_position_then_overwrite() {
        let mut t = Terminal::new(10, 4);
        t.feed(b"\x1b[2;3HX"); // CUP fila 2 col 3 (1-based) → (col2,row1)
        assert_eq!(t.row(1)[2].ch, 'X');
        assert_eq!(t.cursor(), (3, 1));
    }

    #[test]
    fn sgr_sets_color_and_bold() {
        let mut t = Terminal::new(10, 2);
        t.feed(b"\x1b[1;31mR\x1b[0mn");
        let r = t.row(0);
        assert_eq!(r[0].ch, 'R');
        assert_eq!(r[0].fg, Color(1)); // 31 → rojo
        assert!(r[0].bold);
        // Tras reset, la 'n' vuelve a default.
        assert_eq!(r[1].ch, 'n');
        assert!(r[1].fg.is_default());
        assert!(!r[1].bold);
    }

    #[test]
    fn erase_line_to_right() {
        let mut t = Terminal::new(6, 2);
        t.feed(b"abcdef\x1b[1;3H\x1b[K"); // cursor a col3, borra a la derecha
        assert_eq!(line(&t, 0).trim_end(), "ab");
    }

    #[test]
    fn scrolls_when_past_bottom() {
        let mut t = Terminal::new(4, 2);
        t.feed(b"a\r\nb\r\nc"); // 3 líneas en 2 filas → 'a' se va con el scroll
        assert_eq!(line(&t, 0).trim_end(), "b");
        assert_eq!(line(&t, 1).trim_end(), "c");
        assert_eq!(t.cursor().1, 1); // cursor en la última fila
    }

    #[test]
    fn clear_screen_blanks_all() {
        let mut t = Terminal::new(5, 3);
        t.feed(b"junk\r\nmore");
        t.feed(b"\x1b[2J");
        for y in 0..3 {
            assert_eq!(line(&t, y).trim_end(), "");
        }
    }
}
