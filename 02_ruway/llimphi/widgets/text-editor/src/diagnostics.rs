//! Diagnósticos del editor — espejo minimal del shape de `lsp-types`
//! sin depender del crate. Pensado para que un client LSP (rust-analyzer,
//! pylsp, etc.) lo poble desde fuera; el render del editor los pinta
//! como subrayado bajo el rango.
//!
//! El client real vive aparte (proceso + JSON-RPC) — este módulo sólo
//! define el shape de los datos y el helper para renderizarlos.

use crate::cursor::Pos;

/// Severidad — mismos valores y orden que en LSP (1 = Error es el más alto).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Information,
    Hint,
}

/// Rango cerrado de un diagnostic. `end` exclusivo en `col`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagnosticRange {
    pub start: Pos,
    pub end: Pos,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub range: DiagnosticRange,
    pub severity: Severity,
    /// Mensaje humano corto — el render lo trunca para mostrar al hover/
    /// en una mini popup futura. En esta versión solo se usa el rango.
    pub message: String,
    /// Source del diagnostic — "rust-analyzer", "pylsp", "clippy", etc.
    /// `None` si no se conoce.
    pub source: Option<String>,
}

impl Diagnostic {
    pub fn error(line_start: usize, col_start: usize, line_end: usize, col_end: usize, message: impl Into<String>) -> Self {
        Self {
            range: DiagnosticRange {
                start: Pos::new(line_start, col_start),
                end: Pos::new(line_end, col_end),
            },
            severity: Severity::Error,
            message: message.into(),
            source: None,
        }
    }
    pub fn warning(line_start: usize, col_start: usize, line_end: usize, col_end: usize, message: impl Into<String>) -> Self {
        Self {
            range: DiagnosticRange {
                start: Pos::new(line_start, col_start),
                end: Pos::new(line_end, col_end),
            },
            severity: Severity::Warning,
            message: message.into(),
            source: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_funcionan() {
        let e = Diagnostic::error(1, 2, 1, 5, "boom");
        assert_eq!(e.severity, Severity::Error);
        assert_eq!(e.range.start, Pos::new(1, 2));
        assert_eq!(e.range.end, Pos::new(1, 5));

        let w = Diagnostic::warning(0, 0, 0, 10, "ojo");
        assert_eq!(w.severity, Severity::Warning);
    }
}
