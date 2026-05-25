//! Pipeline — la línea descompuesta en sus etapas separadas por `|`.
//!
//! Procesar los pipes es el primer paso para que el shell sea inteligente
//! con la línea: saber cuántas etapas hay, cuál es el comando de cada
//! una y qué argumentos lleva.

use serde::{Deserialize, Serialize};

use crate::token::{Token, TokenKind};

/// Una etapa del pipeline — un comando y sus argumentos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stage {
    /// Nombre del comando, si la etapa lo tiene.
    pub command: Option<String>,
    /// Argumentos y flags, en orden de aparición.
    pub args: Vec<String>,
    /// Todos los tokens de la etapa (sin la `|` que la separa).
    pub tokens: Vec<Token>,
}

impl Stage {
    fn from_tokens(tokens: Vec<Token>) -> Self {
        let mut command = None;
        let mut args = Vec::new();
        for t in &tokens {
            match t.kind {
                TokenKind::Command => command = Some(t.text.clone()),
                TokenKind::Argument | TokenKind::Flag => args.push(t.text.clone()),
                _ => {}
            }
        }
        Self { command, args, tokens }
    }
}

/// La línea completa descompuesta en etapas de pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Pipeline {
    pub stages: Vec<Stage>,
}

impl Pipeline {
    /// Cantidad de etapas.
    pub fn len(&self) -> usize {
        self.stages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }

    /// `true` si la línea encadena dos o más comandos por `|`.
    pub fn is_piped(&self) -> bool {
        self.stages.len() > 1
    }
}

/// Descompone los tokens clasificados en etapas separadas por `|`.
/// El espacio en blanco a los lados se conserva dentro de cada etapa;
/// una etapa vacía (p. ej. la línea termina en `|`) también cuenta.
pub fn split_pipeline(tokens: &[Token]) -> Pipeline {
    if tokens.is_empty() {
        return Pipeline::default();
    }
    let mut stages = Vec::new();
    let mut current: Vec<Token> = Vec::new();
    for t in tokens {
        if t.kind == TokenKind::Pipe {
            stages.push(Stage::from_tokens(std::mem::take(&mut current)));
        } else {
            current.push(t.clone());
        }
    }
    stages.push(Stage::from_tokens(current));
    Pipeline { stages }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialect::Dialect;
    use crate::lexer::tokenize;

    fn pipeline(line: &str) -> Pipeline {
        split_pipeline(&tokenize(line, Dialect::Bash))
    }

    #[test]
    fn single_command_is_one_stage() {
        let p = pipeline("ls -la");
        assert_eq!(p.len(), 1);
        assert!(!p.is_piped());
        assert_eq!(p.stages[0].command.as_deref(), Some("ls"));
        assert_eq!(p.stages[0].args, vec!["-la"]);
    }

    #[test]
    fn pipe_creates_two_stages() {
        let p = pipeline("cat data.json | grep error");
        assert_eq!(p.len(), 2);
        assert!(p.is_piped());
        assert_eq!(p.stages[0].command.as_deref(), Some("cat"));
        assert_eq!(p.stages[1].command.as_deref(), Some("grep"));
        assert_eq!(p.stages[1].args, vec!["error"]);
    }

    #[test]
    fn three_stage_pipeline() {
        let p = pipeline("cat f | sort | uniq -c");
        assert_eq!(p.len(), 3);
        assert_eq!(p.stages[2].command.as_deref(), Some("uniq"));
        assert_eq!(p.stages[2].args, vec!["-c"]);
    }

    #[test]
    fn trailing_pipe_leaves_an_empty_stage() {
        let p = pipeline("ls |");
        assert_eq!(p.len(), 2);
        assert_eq!(p.stages[1].command, None);
    }

    #[test]
    fn empty_line_has_no_stages() {
        assert!(pipeline("").is_empty());
    }
}
