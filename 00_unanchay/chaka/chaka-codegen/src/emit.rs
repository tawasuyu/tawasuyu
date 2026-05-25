//! `Emitter` — un acumulador de código Rust con control de sangría.

/// Acumula líneas de código Rust generado, llevando la sangría actual.
pub(crate) struct Emitter {
    out: String,
    depth: usize,
}

impl Emitter {
    pub(crate) fn new() -> Self {
        Self {
            out: String::new(),
            depth: 0,
        }
    }

    /// Escribe una línea con la sangría actual.
    pub(crate) fn line(&mut self, s: &str) {
        for _ in 0..self.depth {
            self.out.push_str("    ");
        }
        self.out.push_str(s);
        self.out.push('\n');
    }

    /// Una línea en blanco.
    pub(crate) fn blank(&mut self) {
        self.out.push('\n');
    }

    /// Aumenta un nivel de sangría.
    pub(crate) fn indent(&mut self) {
        self.depth += 1;
    }

    /// Reduce un nivel de sangría.
    pub(crate) fn dedent(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// Entrega el código acumulado.
    pub(crate) fn finish(self) -> String {
        self.out
    }
}
