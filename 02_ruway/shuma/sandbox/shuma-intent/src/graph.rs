//! Grafo de contexto de una sesión de shuma.
//!
//! Registra cada intención ejecutada como un nodo `%cN`; al terminar, el
//! nodo expone su buffer de salida `%pN`. El grafo permite resolver las
//! referencias del prompt, validar intenciones nuevas antes de ejecutar,
//! y colapsar nodos exitosos para la quietud visual.

use crate::parse::{Intention, Ref};
use serde::{Deserialize, Serialize};

/// Estado de un nodo del grafo de contexto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeStatus {
    Running,
    Ok,
    Failed,
}

/// Un comando registrado en la sesión.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandNode {
    /// Identificador `%cN`.
    pub id: u32,
    /// Texto de la intención original.
    pub intention: String,
    /// Buffer `%pN` producido como salida, si el comando ya terminó.
    pub output_buffer: Option<u32>,
    pub status: NodeStatus,
    /// Colapsado en la UI (nodo exitoso retraído por quietud visual).
    pub collapsed: bool,
    /// Bytes del buffer de salida (para dimensionar el grafo visual).
    pub output_bytes: u64,
}

/// Grafo de intenciones y flujos de una sesión de shell.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionGraph {
    commands: Vec<CommandNode>,
    next_command: u32,
    next_buffer: u32,
}

impl Default for SessionGraph {
    fn default() -> Self {
        Self { commands: Vec::new(), next_command: 1, next_buffer: 1 }
    }
}

impl SessionGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.commands.len()
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    pub fn commands(&self) -> &[CommandNode] {
        &self.commands
    }

    /// Registra una intención nueva en estado `Running`. Devuelve su `%cN`.
    pub fn record(&mut self, intention: impl Into<String>) -> u32 {
        let id = self.next_command;
        self.next_command += 1;
        self.commands.push(CommandNode {
            id,
            intention: intention.into(),
            output_buffer: None,
            status: NodeStatus::Running,
            collapsed: false,
            output_bytes: 0,
        });
        id
    }

    /// Marca un comando como terminado y le asigna un buffer de salida.
    /// Devuelve el `%pN` asignado, o `None` si el `%cN` no existe.
    pub fn complete(&mut self, command_id: u32, ok: bool, output_bytes: u64) -> Option<u32> {
        let buffer = self.next_buffer;
        let node = self.commands.iter_mut().find(|c| c.id == command_id)?;
        node.status = if ok { NodeStatus::Ok } else { NodeStatus::Failed };
        node.output_bytes = output_bytes;
        node.output_buffer = Some(buffer);
        self.next_buffer += 1;
        Some(buffer)
    }

    /// Resuelve una referencia a su nodo de comando.
    pub fn resolve(&self, r: Ref) -> Option<&CommandNode> {
        match r {
            Ref::Command(n) => self.commands.iter().find(|c| c.id == n),
            Ref::Buffer(n) => self.commands.iter().find(|c| c.output_buffer == Some(n)),
        }
    }

    /// Referencias de la intención que NO se pueden resolver en esta
    /// sesión. Vacío = la intención es ejecutable (validación previa
    /// del prompt).
    pub fn dangling_refs(&self, intention: &Intention) -> Vec<Ref> {
        intention
            .refs()
            .into_iter()
            .filter(|r| self.resolve(*r).is_none())
            .collect()
    }

    /// Colapsa los nodos exitosos (quietud visual: los flujos que ya
    /// funcionaron se retraen).
    pub fn collapse_succeeded(&mut self) {
        for c in &mut self.commands {
            if c.status == NodeStatus::Ok {
                c.collapsed = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_assigns_increasing_command_ids() {
        let mut g = SessionGraph::new();
        assert_eq!(g.record("cat a"), 1);
        assert_eq!(g.record("cat b"), 2);
        assert_eq!(g.len(), 2);
    }

    #[test]
    fn complete_assigns_buffer_and_status() {
        let mut g = SessionGraph::new();
        let c1 = g.record("cat data.json");
        let buf = g.complete(c1, true, 2_400_000).expect("c1 existe");
        assert_eq!(buf, 1);
        let node = g.resolve(Ref::Command(c1)).unwrap();
        assert_eq!(node.status, NodeStatus::Ok);
        assert_eq!(node.output_buffer, Some(1));
        assert_eq!(node.output_bytes, 2_400_000);
        // Se resuelve también por su buffer.
        assert!(g.resolve(Ref::Buffer(1)).is_some());
    }

    #[test]
    fn dangling_refs_validates_an_intention() {
        let mut g = SessionGraph::new();
        let c1 = g.record("cat data.json");
        g.complete(c1, true, 100).unwrap(); // produce %p1

        // `%p1` existe, `%p9` no.
        let ok = Intention::parse("sort | %p1");
        assert!(g.dangling_refs(&ok).is_empty());

        let bad = Intention::parse("sort | %p9");
        assert_eq!(g.dangling_refs(&bad), vec![Ref::Buffer(9)]);
    }

    #[test]
    fn collapse_only_retracts_successful_nodes() {
        let mut g = SessionGraph::new();
        let c1 = g.record("ok cmd");
        let c2 = g.record("fail cmd");
        let _c3 = g.record("running cmd");
        g.complete(c1, true, 0).unwrap();
        g.complete(c2, false, 0).unwrap();
        g.collapse_succeeded();
        assert!(g.resolve(Ref::Command(c1)).unwrap().collapsed);
        assert!(!g.resolve(Ref::Command(c2)).unwrap().collapsed);
    }
}
