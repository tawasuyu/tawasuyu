//! `grafo` — modelo de un grafo de fuerzas pairwise editable visualmente.
//!
//! Cada nodo representa un fragmento de la expresión `F_over_r` que se
//! evalúa por par `(i, j)` (ver convención en `tinkuy-dsl/examples/*.tnk`).
//! El grafo se "lifta" a un [`tinkuy_dsl::Expr`] caminando desde el nodo
//! [`NodeKind::Output`] hacia atrás por los cables — luego se pasa por
//! [`tinkuy_dsl::optimize`] y [`tinkuy_dsl::compile`] para producir el
//! `Bytecode` que `DslForce` ejecuta.
//!
//! Reglas duras:
//!
//! - Hay **exactamente un** nodo [`NodeKind::Output`] por grafo. Es la
//!   raíz del `Expr` final.
//! - Cada pin de entrada de un nodo debe estar conectado por un cable a
//!   un pin de salida de otro nodo; si falta alguno, el `lift` falla con
//!   [`LiftError::PinDesconectado`].
//! - Sin ciclos. El `lift` los detecta con DFS + `Vec<bool>` de visited.

use alloc_compat::{Box, String, Vec};
use llimphi_widget_nodegraph::{NodeId, NodeSpec, PinIdx, Wire};
use rimay_localize::t;

use tinkuy_dsl::{BinOp, Expr, Func, Var};

/// Pequeño shim para no exponer `alloc::` en `lib.rs`; este crate usa `std`
/// pero las firmas que cruzan a `tinkuy-dsl` (que es `no_std + alloc`)
/// hablan en `Box`/`String`/`Vec` de la stdlib. Acá renombramos por claridad.
mod alloc_compat {
    pub use std::boxed::Box;
    pub use std::string::String;
    pub use std::vec::Vec;
}

/// Identidad del tipo de un nodo del grafo. Determina cuántos pins de
/// entrada y salida lo rodean y cómo se compila a [`Expr`].
#[derive(Clone, Debug)]
pub enum NodeKind {
    /// Variable de entorno bindeada por el evaluador. Sin entradas; una
    /// salida.
    Var(Var),
    /// Literal numérico embebido. Sin entradas; una salida.
    Num(f32),
    /// Operación binaria: dos entradas (`a`, `b`), una salida.
    Bin(BinOp),
    /// Función reservada del DSL. La aridad varía:
    ///   - `Pow` → 2 entradas (`base`, `exp`)
    ///   - `Inv` → 1 entrada (`x`)
    ///   - `Sqrt` → 1 entrada (`x`)
    Func(Func),
    /// Negación unaria. Una entrada (`x`), una salida.
    Neg,
    /// Salida del grafo entero. Una entrada (`F/r`), sin salidas. Su
    /// entrada determina la `Expr` raíz que se compila a `Bytecode`.
    Output,
}

impl NodeKind {
    /// Cantidad de pins de entrada que expone este nodo.
    pub fn n_inputs(&self) -> usize {
        match self {
            NodeKind::Var(_) | NodeKind::Num(_) => 0,
            NodeKind::Bin(_) => 2,
            NodeKind::Func(f) => f.arity(),
            NodeKind::Neg => 1,
            NodeKind::Output => 1,
        }
    }

    /// Cantidad de pins de salida que expone este nodo.
    pub fn n_outputs(&self) -> usize {
        match self {
            NodeKind::Output => 0,
            _ => 1,
        }
    }

    /// Labels cortos para cada pin de entrada (en orden).
    pub fn input_labels(&self) -> Vec<String> {
        match self {
            NodeKind::Var(_) | NodeKind::Num(_) => Vec::new(),
            NodeKind::Bin(_) => vec!["a".into(), "b".into()],
            NodeKind::Func(Func::Pow) => vec!["base".into(), "exp".into()],
            NodeKind::Func(Func::Inv) => vec!["x".into()],
            NodeKind::Func(Func::Sqrt) => vec!["x".into()],
            NodeKind::Neg => vec!["x".into()],
            NodeKind::Output => vec!["F/r".into()],
        }
    }

    pub fn output_labels(&self) -> Vec<String> {
        match self {
            NodeKind::Output => Vec::new(),
            _ => vec!["out".into()],
        }
    }

    /// Etiqueta humana corta para la title bar.
    pub fn title(&self) -> String {
        match self {
            NodeKind::Var(v) => format!("var · {}", var_name(*v)),
            NodeKind::Num(n) => format!("num · {}", n),
            NodeKind::Bin(op) => match op {
                BinOp::Add => "+".into(),
                BinOp::Sub => "−".into(),
                BinOp::Mul => "×".into(),
                BinOp::Div => "÷".into(),
            },
            NodeKind::Func(f) => match f {
                Func::Pow => "pow".into(),
                Func::Inv => "inv".into(),
                Func::Sqrt => "sqrt".into(),
            },
            NodeKind::Neg => "neg".into(),
            NodeKind::Output => t("tinkuy-node-output"),
        }
    }
}

fn var_name(v: Var) -> &'static str {
    match v {
        Var::R => "r",
        Var::R2 => "r2",
        Var::Eps => "eps",
        Var::Sigma => "sigma",
        Var::Qi => "qi",
        Var::Qj => "qj",
        Var::Mi => "mi",
        Var::Mj => "mj",
        Var::Dx => "dx",
        Var::Dy => "dy",
        Var::Dz => "dz",
    }
}

/// Un nodo posicionado en el lienzo. La `id` es opaca; el grafo se asegura
/// de que sea única dentro de su `Vec<GraphNode>`.
#[derive(Clone, Debug)]
pub struct GraphNode {
    pub id: NodeId,
    pub kind: NodeKind,
    pub x: f32,
    pub y: f32,
}

/// Grafo completo. Se almacena en el `Model` y se "lifta" a una expresión
/// al recompilar.
#[derive(Clone, Debug, Default)]
pub struct ForceGraph {
    pub nodes: Vec<GraphNode>,
    pub wires: Vec<Wire>,
    /// Próxima `NodeId` libre. Monótonamente creciente; no se reusa cuando
    /// un nodo se elimina (mantiene wires estables si futuro deletes).
    next_id: NodeId,
}

/// Errores del lift `ForceGraph → Expr`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LiftError {
    /// El grafo no contiene un nodo `Output`.
    SinSalida,
    /// El grafo contiene más de un nodo `Output`.
    SalidaDuplicada,
    /// Falta cablear la entrada `pin` del nodo `node`.
    PinDesconectado { node: NodeId, pin: PinIdx },
    /// Se detectó un ciclo durante el DFS.
    Ciclo,
}

impl ForceGraph {
    pub fn new() -> Self {
        Self::default()
    }

    fn spawn(&mut self, kind: NodeKind, x: f32, y: f32) -> NodeId {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.nodes.push(GraphNode { id, kind, x, y });
        id
    }

    fn connect(&mut self, from_node: NodeId, from_output: PinIdx, to_node: NodeId, to_input: PinIdx) {
        self.wires.push(Wire {
            from_node,
            from_output,
            to_node,
            to_input,
        });
    }

    /// Reemplaza el cable que apunta al pin `(to_node, to_input)` por uno
    /// nuevo que sale de `(from_node, from_output)`. Si no había cable
    /// previo, simplemente lo añade. Política: cada pin de entrada recibe
    /// **un solo** cable (el último gana) — análogo a `nakui`/`takiy`.
    pub fn rewire_input(
        &mut self,
        from_node: NodeId,
        from_output: PinIdx,
        to_node: NodeId,
        to_input: PinIdx,
    ) {
        self.wires
            .retain(|w| !(w.to_node == to_node && w.to_input == to_input));
        self.connect(from_node, from_output, to_node, to_input);
    }

    /// Mueve el nodo `id` por `(dx, dy)` pixels. Si el id no existe, es
    /// un no-op silencioso (caller pudo persistir un id stale).
    pub fn move_node(&mut self, id: NodeId, dx: f32, dy: f32) {
        if let Some(n) = self.nodes.iter_mut().find(|n| n.id == id) {
            n.x += dx;
            n.y += dy;
        }
    }

    /// Construye un grafo pre-poblado con la fórmula Lennard-Jones canónica:
    /// `24·ε·(2·(σ/r)¹² − (σ/r)⁶)·(1/r²)`. Es la misma expresión que el
    /// archivo `tinkuy-dsl/examples/lj.tnk` y produce el mismo bytecode que
    /// el kernel nativo. Las coordenadas asumen un lienzo de ~720×420 pixels.
    pub fn lennard_jones_default() -> Self {
        let mut g = Self::new();

        // Capa 0 — variables / literales (izquierda).
        let n_sigma = g.spawn(NodeKind::Var(Var::Sigma), 20.0, 30.0);
        let n_r = g.spawn(NodeKind::Var(Var::R), 20.0, 110.0);
        let n_r2 = g.spawn(NodeKind::Var(Var::R2), 20.0, 190.0);
        let n_eps = g.spawn(NodeKind::Var(Var::Eps), 20.0, 270.0);
        let n_c24 = g.spawn(NodeKind::Num(24.0), 20.0, 350.0);
        let n_c2 = g.spawn(NodeKind::Num(2.0), 20.0, 430.0);

        // Capa 1 — σ/r y 1/r² (núcleo geométrico).
        let n_sr = g.spawn(NodeKind::Bin(BinOp::Div), 220.0, 70.0);
        let n_invr2 = g.spawn(NodeKind::Func(Func::Inv), 220.0, 190.0);

        // Capa 2 — (σ/r)⁶ y (σ/r)¹².
        let n_sr6 = g.spawn(NodeKind::Func(Func::Pow), 420.0, 30.0);
        let n_p6 = g.spawn(NodeKind::Num(6.0), 240.0, -20.0);
        let n_sr12 = g.spawn(NodeKind::Func(Func::Pow), 420.0, 130.0);
        let n_p12 = g.spawn(NodeKind::Num(12.0), 240.0, 250.0);

        // Capa 3 — 2·sr12 y 24·ε.
        let n_two_sr12 = g.spawn(NodeKind::Bin(BinOp::Mul), 620.0, 110.0);
        let n_24eps = g.spawn(NodeKind::Bin(BinOp::Mul), 220.0, 310.0);

        // Capa 4 — 2·sr12 − sr6, 24·ε·dif.
        let n_diff = g.spawn(NodeKind::Bin(BinOp::Sub), 820.0, 70.0);
        let n_24eps_diff = g.spawn(NodeKind::Bin(BinOp::Mul), 1020.0, 170.0);

        // Capa 5 — multiplicar por 1/r².
        let n_final = g.spawn(NodeKind::Bin(BinOp::Mul), 1220.0, 230.0);

        // Salida.
        let n_out = g.spawn(NodeKind::Output, 1420.0, 230.0);

        // Cables. Por convención del widget: el pin 0 de output de cualquier
        // nodo común es "out"; los pins de entrada se indexan 0..n.
        g.connect(n_sigma, 0, n_sr, 0); // σ → sr.a
        g.connect(n_r, 0, n_sr, 1); //     r → sr.b
        g.connect(n_r2, 0, n_invr2, 0); //  r2 → inv.x

        g.connect(n_sr, 0, n_sr6, 0); //    sr → pow6.base
        g.connect(n_p6, 0, n_sr6, 1); //    6  → pow6.exp
        g.connect(n_sr, 0, n_sr12, 0); //   sr → pow12.base
        g.connect(n_p12, 0, n_sr12, 1); //  12 → pow12.exp

        g.connect(n_c2, 0, n_two_sr12, 0); // 2     → mul.a
        g.connect(n_sr12, 0, n_two_sr12, 1); // sr12 → mul.b

        g.connect(n_c24, 0, n_24eps, 0); //  24 → mul.a
        g.connect(n_eps, 0, n_24eps, 1); //  ε  → mul.b

        g.connect(n_two_sr12, 0, n_diff, 0);
        g.connect(n_sr6, 0, n_diff, 1);

        g.connect(n_24eps, 0, n_24eps_diff, 0);
        g.connect(n_diff, 0, n_24eps_diff, 1);

        g.connect(n_24eps_diff, 0, n_final, 0);
        g.connect(n_invr2, 0, n_final, 1);

        g.connect(n_final, 0, n_out, 0);

        g
    }

    /// "Lifta" el grafo a una `Expr` arrastrando desde el `Output` por los
    /// cables. Detecta ciclos y pins desconectados.
    pub fn lift_to_expr(&self) -> Result<Expr, LiftError> {
        // Localiza el (único) nodo Output.
        let mut salidas = self.nodes.iter().filter(|n| matches!(n.kind, NodeKind::Output));
        let salida = salidas.next().ok_or(LiftError::SinSalida)?;
        if salidas.next().is_some() {
            return Err(LiftError::SalidaDuplicada);
        }

        // DFS con stack de visited para detección de ciclos.
        let mut visiting = vec![false; self.next_id as usize];
        build_expr_for_input(self, salida.id, 0, &mut visiting)
    }
}

/// Para el pin de entrada `pin_in` del nodo `node_id`, encuentra el cable
/// que llega y construye recursivamente la `Expr` del proveedor. Marca
/// `visiting[node_id]` durante el descenso para detectar ciclos.
fn build_expr_for_input(
    g: &ForceGraph,
    node_id: NodeId,
    pin_in: PinIdx,
    visiting: &mut [bool],
) -> Result<Expr, LiftError> {
    // Hallar el cable que termina en (node_id, pin_in).
    let wire = g
        .wires
        .iter()
        .find(|w| w.to_node == node_id && w.to_input == pin_in)
        .ok_or(LiftError::PinDesconectado {
            node: node_id,
            pin: pin_in,
        })?;
    build_expr_for_node(g, wire.from_node, visiting)
}

fn build_expr_for_node(
    g: &ForceGraph,
    node_id: NodeId,
    visiting: &mut [bool],
) -> Result<Expr, LiftError> {
    if visiting[node_id as usize] {
        return Err(LiftError::Ciclo);
    }
    visiting[node_id as usize] = true;

    let node = g
        .nodes
        .iter()
        .find(|n| n.id == node_id)
        // Cable apunta a un nodo inexistente: tratamos como ciclo/grafo roto.
        .ok_or(LiftError::Ciclo)?;

    let expr = match &node.kind {
        NodeKind::Var(v) => Expr::Var(*v),
        NodeKind::Num(n) => Expr::Num(*n),
        NodeKind::Neg => {
            let inner = build_expr_for_input(g, node_id, 0, visiting)?;
            Expr::Neg(Box::new(inner))
        }
        NodeKind::Bin(op) => {
            let a = build_expr_for_input(g, node_id, 0, visiting)?;
            let b = build_expr_for_input(g, node_id, 1, visiting)?;
            Expr::Bin(*op, Box::new(a), Box::new(b))
        }
        NodeKind::Func(f) => match f.arity() {
            1 => {
                let x = build_expr_for_input(g, node_id, 0, visiting)?;
                Expr::Call(*f, vec![x])
            }
            2 => {
                let a = build_expr_for_input(g, node_id, 0, visiting)?;
                let b = build_expr_for_input(g, node_id, 1, visiting)?;
                Expr::Call(*f, vec![a, b])
            }
            // Las funciones del DSL hoy tienen aridad ∈ {1, 2}; si alguien
            // agrega una de aridad 3+ a `tinkuy-dsl`, este match falla
            // explícitamente para que el grafo se actualice junto.
            other => panic!("aridad inesperada para Func: {other}"),
        },
        NodeKind::Output => {
            // Solo debería alcanzarse vía `lift_to_expr` con `pin_in = 0` —
            // jamás como proveedor de otro nodo. Si llegó acá hay un cable
            // que sale del Output (debería ser imposible: tiene 0 outputs).
            return Err(LiftError::Ciclo);
        }
    };

    visiting[node_id as usize] = false;
    Ok(expr)
}

/// Convierte un [`ForceGraph`] a la lista de `NodeSpec`s que consume el
/// widget. Las posiciones, labels e inputs/outputs salen directamente del
/// `NodeKind`. Las wires viajan tal cual.
pub fn render_nodes(g: &ForceGraph) -> Vec<NodeSpec> {
    g.nodes
        .iter()
        .map(|n| NodeSpec {
            id: n.id,
            label: n.kind.title(),
            x: n.x,
            y: n.y,
            inputs: n.kind.input_labels(),
            outputs: n.kind.output_labels(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tinkuy_dsl::{compile, eval_with_stack, optimize, VarBindings};

    /// El grafo LJ default debe liftarse, compilarse y producir el mismo
    /// valor que la fórmula nativa para un par dado.
    #[test]
    fn lj_default_matches_native() {
        let g = ForceGraph::lennard_jones_default();
        let expr = g.lift_to_expr().expect("lift LJ default");
        let expr_opt = optimize(expr);
        let bc = compile(&expr_opt).expect("compile LJ default");

        // Par de prueba: σ=1, ε=1, r=1.2 (zona repulsiva fuerte).
        let r = 1.2_f32;
        let sigma = 1.0_f32;
        let eps = 1.0_f32;
        let r2 = r * r;
        let bindings = VarBindings {
            r,
            r2,
            eps,
            sigma,
            qi: 0.0,
            qj: 0.0,
            mi: 1.0,
            mj: 1.0,
            dx: r,
            dy: 0.0,
            dz: 0.0,
        };
        let mut stack = vec![0.0f32; bc.stack_depth as usize];
        let val = eval_with_stack(&bc, &bindings, &mut stack).unwrap();

        let sr = sigma / r;
        let sr6 = sr.powi(6);
        let sr12 = sr.powi(12);
        let native = 24.0 * eps * (2.0 * sr12 - sr6) / r2;

        let rel = (val - native).abs() / native.abs().max(1.0);
        assert!(rel < 1.0e-3, "DSL {} vs nativo {} (rel {})", val, native, rel);
    }

    #[test]
    fn lift_falla_si_falta_salida() {
        let g = ForceGraph::new();
        assert_eq!(g.lift_to_expr(), Err(LiftError::SinSalida));
    }

    #[test]
    fn lift_falla_si_pin_desconectado() {
        let mut g = ForceGraph::new();
        let _out = g.spawn(NodeKind::Output, 0.0, 0.0);
        // Sin cables → entrada del Output desconectada.
        match g.lift_to_expr() {
            Err(LiftError::PinDesconectado { .. }) => {}
            other => panic!("esperaba PinDesconectado, fue {:?}", other),
        }
    }

    #[test]
    fn detecta_ciclo() {
        // out ← a (Neg), a ← b (Neg), b ← a (Neg) — cable a.0 ← b.out.
        let mut g = ForceGraph::new();
        let a = g.spawn(NodeKind::Neg, 0.0, 0.0);
        let b = g.spawn(NodeKind::Neg, 0.0, 0.0);
        let out = g.spawn(NodeKind::Output, 0.0, 0.0);
        g.connect(a, 0, out, 0);
        g.connect(b, 0, a, 0);
        g.connect(a, 0, b, 0); // a → b → a: ciclo.
        assert_eq!(g.lift_to_expr(), Err(LiftError::Ciclo));
    }
}
