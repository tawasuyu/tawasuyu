//! Parser de intenciones del prompt de shuma.
//!
//! Una "intención" es una línea del prompt: etapas separadas por `|`.
//! Cada etapa es un comando a ejecutar, o un token de referencia a un
//! resultado previo de la sesión (`%cN` un comando, `%pN` un buffer
//! intermedio). Ej: `ssh nodo 'cat data.json' | %p1 | sort`.

use serde::{Deserialize, Serialize};

/// Referencia a un resultado de la sesión.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ref {
    /// `%cN` — un comando registrado de la sesión.
    Command(u32),
    /// `%pN` — un buffer intermedio producido por un comando.
    Buffer(u32),
}

impl Ref {
    /// Parsea un token aislado `%c3` / `%p12`. `None` si no es un token.
    pub fn parse(token: &str) -> Option<Ref> {
        let rest = token.trim().strip_prefix('%')?;
        let mut chars = rest.chars();
        let kind = chars.next()?;
        let num: u32 = chars.as_str().parse().ok()?;
        match kind {
            'c' => Some(Ref::Command(num)),
            'p' => Some(Ref::Buffer(num)),
            _ => None,
        }
    }
}

/// Una etapa del pipeline de una intención.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Stage {
    /// Comando a ejecutar (texto crudo; puede ser `ssh host '...'`).
    Exec(String),
    /// Inyección de un resultado previo de la sesión.
    Inject(Ref),
}

/// Una intención parseada: etapas conectadas por pipes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Intention {
    pub stages: Vec<Stage>,
}

impl Intention {
    /// Parsea una línea del prompt. Las etapas se separan por `|`; una
    /// etapa que es exactamente un token `%pN`/`%cN` es `Inject`, el
    /// resto es `Exec`. Las etapas vacías se descartan.
    pub fn parse(line: &str) -> Intention {
        let stages = line
            .split('|')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| match Ref::parse(s) {
                Some(r) => Stage::Inject(r),
                None => Stage::Exec(s.to_string()),
            })
            .collect();
        Intention { stages }
    }

    /// `true` si la intención no tiene etapas (línea vacía).
    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }

    /// Todas las referencias que la intención consume.
    pub fn refs(&self) -> Vec<Ref> {
        self.stages
            .iter()
            .filter_map(|s| match s {
                Stage::Inject(r) => Some(*r),
                Stage::Exec(_) => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ref_tokens() {
        assert_eq!(Ref::parse("%c3"), Some(Ref::Command(3)));
        assert_eq!(Ref::parse("%p12"), Some(Ref::Buffer(12)));
        assert_eq!(Ref::parse(" %p1 "), Some(Ref::Buffer(1)));
        assert_eq!(Ref::parse("sort"), None);
        assert_eq!(Ref::parse("%x9"), None);
        assert_eq!(Ref::parse("%p"), None);
    }

    #[test]
    fn parses_the_spec_example() {
        // ssh nodo 'cat data.json' | %p1 | sort
        let i = Intention::parse("ssh nodo 'cat data.json' | %p1 | sort");
        assert_eq!(i.stages.len(), 3);
        assert_eq!(i.stages[0], Stage::Exec("ssh nodo 'cat data.json'".into()));
        assert_eq!(i.stages[1], Stage::Inject(Ref::Buffer(1)));
        assert_eq!(i.stages[2], Stage::Exec("sort".into()));
    }

    #[test]
    fn refs_extracts_only_injections() {
        let i = Intention::parse("cat x | %p1 | %c2 | wc -l");
        assert_eq!(i.refs(), vec![Ref::Buffer(1), Ref::Command(2)]);
    }

    #[test]
    fn empty_line_is_empty_intention() {
        assert!(Intention::parse("   ").is_empty());
        assert!(Intention::parse("| |").is_empty());
    }
}
