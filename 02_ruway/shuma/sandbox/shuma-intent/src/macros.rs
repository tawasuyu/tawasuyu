//! Macros del shell — la barra de ejecución [RUN].
//!
//! Una macro es una secuencia de intenciones nombrada y opcionalmente
//! mapeada a una tecla física (F1-F3...). Son serializables: la spec
//! pide que sean compartibles entre sesiones y entre usuarios.

use serde::{Deserialize, Serialize};

/// Una macro: un nombre, una tecla opcional y las intenciones que dispara.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Macro {
    pub name: String,
    /// Tecla física que la dispara (`"F1"`, `"F2"`, ...). `None` = sin atajo.
    pub key: Option<String>,
    /// Líneas de prompt que ejecuta, en orden.
    pub intentions: Vec<String>,
}

impl Macro {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), key: None, intentions: Vec::new() }
    }

    /// Builder: asigna una tecla.
    pub fn bind(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Builder: agrega una intención.
    pub fn step(mut self, intention: impl Into<String>) -> Self {
        self.intentions.push(intention.into());
        self
    }
}

/// Colección de macros de la barra [RUN]. Serializable para compartir.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MacroBook {
    macros: Vec<Macro>,
}

impl MacroBook {
    pub fn new() -> Self {
        Self::default()
    }

    /// Agrega (o reemplaza por nombre) una macro.
    pub fn insert(&mut self, m: Macro) {
        if let Some(slot) = self.macros.iter_mut().find(|x| x.name == m.name) {
            *slot = m;
        } else {
            self.macros.push(m);
        }
    }

    pub fn len(&self) -> usize {
        self.macros.len()
    }

    pub fn is_empty(&self) -> bool {
        self.macros.is_empty()
    }

    pub fn all(&self) -> &[Macro] {
        &self.macros
    }

    /// Macro mapeada a una tecla física dada.
    pub fn by_key(&self, key: &str) -> Option<&Macro> {
        self.macros.iter().find(|m| m.key.as_deref() == Some(key))
    }

    /// Macro por nombre exacto.
    pub fn by_name(&self, name: &str) -> Option<&Macro> {
        self.macros.iter().find(|m| m.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macro_builder_composes() {
        let m = Macro::new("deploy")
            .bind("F2")
            .step("cargo build --release")
            .step("scp target/release/app host:/srv");
        assert_eq!(m.key.as_deref(), Some("F2"));
        assert_eq!(m.intentions.len(), 2);
    }

    #[test]
    fn book_lookup_by_key_and_name() {
        let mut book = MacroBook::new();
        book.insert(Macro::new("build").bind("F1").step("cargo build"));
        book.insert(Macro::new("clean").bind("F3").step("cargo clean"));
        assert_eq!(book.len(), 2);
        assert_eq!(book.by_key("F1").unwrap().name, "build");
        assert_eq!(book.by_key("F3").unwrap().name, "clean");
        assert!(book.by_key("F9").is_none());
        assert!(book.by_name("clean").is_some());
    }

    #[test]
    fn insert_replaces_by_name() {
        let mut book = MacroBook::new();
        book.insert(Macro::new("x").step("v1"));
        book.insert(Macro::new("x").step("v2"));
        assert_eq!(book.len(), 1);
        assert_eq!(book.by_name("x").unwrap().intentions, vec!["v2"]);
    }
}
