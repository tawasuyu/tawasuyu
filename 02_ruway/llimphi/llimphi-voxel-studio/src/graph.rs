//! `graph` — **puente entre la ecuación de una ley y un grafo de nodos**: la segunda
//! superficie de autoría (la primera es la barra de fórmula). Ambas son vistas del
//! **mismo AST** [`Expr`]: el grafo se deriva de las fórmulas al entrar en modo nodos
//! ([`EqGraph::from_fuentes`]) y se recompila a fórmulas en cada edición
//! ([`EqGraph::to_fuentes`]) — ida y vuelta sin una segunda fuente de verdad (el texto
//! sigue siendo canónico y persistente; el grafo es estado de UI, no se guarda).
//!
//! Modelo: un nodo por sub‑expresión. Las **hojas** (constante, campo, parámetro,
//! `dt`, y los muestreos de vecinos `lap`/`avg`/`abajo`… que llevan su campo adentro)
//! no tienen entradas; los **operadores** (unario/binario/clamp) tienen 1/2/3 entradas;
//! y cada campo remata en un **sink** `Δcampoₖ` (1 entrada, sin salida). Convertir el
//! grafo a `Expr` es seguir el cable de cada sink hacia atrás.

use llimphi_voxel::{BinOp, Dir, Expr, Reduce, Symbols, UnOp};
use llimphi_widget_nodegraph::{NodeId, NodeSpec, PinIdx, Wire};

/// La **operación** de un nodo = un constructor de [`Expr`] (o el sink de un campo).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NodeOp {
    Const(f32),
    Field(u16),
    Param(u16),
    Dt,
    Lap(u16),
    Vecinos(Reduce, u16),
    Dir(Dir, u16),
    Un(UnOp),
    Bin(BinOp),
    Clamp,
    /// Remate del campo `k`: `Δcampoₖ/dt = <entrada>`.
    Sink(u16),
}

impl NodeOp {
    /// Etiquetas de los pins de entrada (su cantidad = aridad).
    pub fn inputs(&self) -> Vec<String> {
        match self {
            NodeOp::Un(_) => vec!["x".into()],
            NodeOp::Bin(_) => vec!["a".into(), "b".into()],
            NodeOp::Clamp => vec!["x".into(), "lo".into(), "hi".into()],
            NodeOp::Sink(_) => vec!["=".into()],
            _ => vec![],
        }
    }
    /// Etiquetas de los pins de salida (el sink no tiene).
    pub fn outputs(&self) -> Vec<String> {
        match self {
            NodeOp::Sink(_) => vec![],
            _ => vec!["".into()],
        }
    }
    /// Texto del nodo (usa los nombres de campos/params de `sym`).
    pub fn label(&self, sym: &Symbols) -> String {
        let campo = |f: u16| sym.campos.get(f as usize).cloned().unwrap_or_else(|| format!("c{f}"));
        match self {
            NodeOp::Const(c) => fmt_num(*c),
            NodeOp::Field(f) => campo(*f),
            NodeOp::Param(p) => sym.params.get(*p as usize).cloned().unwrap_or_else(|| format!("p{p}")),
            NodeOp::Dt => "dt".into(),
            NodeOp::Lap(f) => format!("lap({})", campo(*f)),
            NodeOp::Vecinos(r, f) => format!("{}({})", reduce_name(*r), campo(*f)),
            NodeOp::Dir(d, f) => format!("{}({})", dir_name(*d), campo(*f)),
            NodeOp::Un(u) => un_name(*u).into(),
            NodeOp::Bin(b) => bin_name(*b).into(),
            NodeOp::Clamp => "clamp".into(),
            NodeOp::Sink(k) => format!("Δ{}", campo(*k)),
        }
    }
}

/// Un nodo del grafo: id + operación + posición en el lienzo.
#[derive(Debug, Clone)]
pub struct EqNode {
    pub id: NodeId,
    pub op: NodeOp,
    pub x: f32,
    pub y: f32,
}

/// Grafo editable de una ley: nodos + cables + los sinks (uno por campo, en orden).
/// Estado de UI transitorio; se compila a `fuentes` en cada cambio.
#[derive(Debug, Clone, Default)]
pub struct EqGraph {
    pub nodes: Vec<EqNode>,
    pub wires: Vec<Wire>,
    pub sinks: Vec<NodeId>,
    next_id: NodeId,
}

const COL_W: f32 = 150.0;
const ROW_H: f32 = 64.0;
const CAMPO_GAP: f32 = 90.0;
const ORIGIN_X: f32 = 40.0;
const ORIGIN_Y: f32 = 40.0;

impl EqGraph {
    /// Construye el grafo a partir de las fórmulas de una ley (una por campo). Las que
    /// no parsean caen a `Const(0)`. Auto‑distribuye los nodos (hojas a la izquierda,
    /// sink a la derecha; un bloque por campo, apilados).
    pub fn from_fuentes(fuentes: &[String], sym: &Symbols) -> EqGraph {
        let mut g = EqGraph::default();
        let mut base_y = ORIGIN_Y;
        for (k, src) in fuentes.iter().enumerate() {
            let expr = Expr::parse(src, sym).unwrap_or(Expr::Const(0.0));
            let mut cursor = base_y;
            let (root, depth) = g.build_expr(&expr, &mut cursor);
            // Sink a la derecha del árbol.
            let sink = g.push(NodeOp::Sink(k as u16), 0.0, 0.0);
            let sink_depth = depth + 1;
            g.set_pos(sink, sink_depth, (base_y + cursor - ROW_H) * 0.5);
            g.wires.push(Wire { from_node: root, from_output: 0, to_node: sink, to_input: 0 });
            g.sinks.push(sink);
            base_y = cursor + CAMPO_GAP;
        }
        // x se fijó por profundidad relativa a la izquierda; recolocar con columnas
        // globales para que el sink quede a la derecha del más profundo.
        g.reflow();
        g
    }

    fn push(&mut self, op: NodeOp, x: f32, y: f32) -> NodeId {
        let id = self.next_id;
        self.next_id += 1;
        self.nodes.push(EqNode { id, op, x, y });
        id
    }

    /// `x` provisorio = profundidad (columnas desde la izquierda); se reacomoda en
    /// [`reflow`]. `y` = fila. Guarda la profundidad en `x` como entero flotante.
    fn set_pos(&mut self, id: NodeId, depth: usize, y: f32) {
        if let Some(n) = self.nodes.iter_mut().find(|n| n.id == id) {
            n.x = depth as f32;
            n.y = y;
        }
    }

    /// Crea los nodos de una `Expr` (post‑orden). Devuelve `(id_raíz, profundidad)`.
    /// `cursor` avanza una fila por hoja para apilar sin solaparse.
    fn build_expr(&mut self, e: &Expr, cursor: &mut f32) -> (NodeId, usize) {
        match e {
            Expr::Const(c) => (self.leaf(NodeOp::Const(*c), cursor), 0),
            Expr::Field(f) => (self.leaf(NodeOp::Field(*f), cursor), 0),
            Expr::Param(p) => (self.leaf(NodeOp::Param(*p), cursor), 0),
            Expr::Dt => (self.leaf(NodeOp::Dt, cursor), 0),
            Expr::Laplacian(f) => (self.leaf(NodeOp::Lap(*f), cursor), 0),
            Expr::Vecinos(r, f) => (self.leaf(NodeOp::Vecinos(*r, *f), cursor), 0),
            Expr::Dir(d, f) => (self.leaf(NodeOp::Dir(*d, *f), cursor), 0),
            Expr::Un(op, a) => {
                let (ca, da) = self.build_expr(a, cursor);
                let id = self.push(NodeOp::Un(*op), 0.0, 0.0);
                let y = self.y_of(ca);
                self.set_pos(id, da + 1, y);
                self.wires.push(Wire { from_node: ca, from_output: 0, to_node: id, to_input: 0 });
                (id, da + 1)
            }
            Expr::Bin(op, a, b) => {
                let (ca, da) = self.build_expr(a, cursor);
                let (cb, db) = self.build_expr(b, cursor);
                let id = self.push(NodeOp::Bin(*op), 0.0, 0.0);
                let y = (self.y_of(ca) + self.y_of(cb)) * 0.5;
                self.set_pos(id, da.max(db) + 1, y);
                self.wires.push(Wire { from_node: ca, from_output: 0, to_node: id, to_input: 0 });
                self.wires.push(Wire { from_node: cb, from_output: 0, to_node: id, to_input: 1 });
                (id, da.max(db) + 1)
            }
            Expr::Clamp(x, lo, hi) => {
                let (cx, dx) = self.build_expr(x, cursor);
                let (cl, dl) = self.build_expr(lo, cursor);
                let (ch, dh) = self.build_expr(hi, cursor);
                let id = self.push(NodeOp::Clamp, 0.0, 0.0);
                let y = self.y_of(cl);
                self.set_pos(id, dx.max(dl).max(dh) + 1, y);
                self.wires.push(Wire { from_node: cx, from_output: 0, to_node: id, to_input: 0 });
                self.wires.push(Wire { from_node: cl, from_output: 0, to_node: id, to_input: 1 });
                self.wires.push(Wire { from_node: ch, from_output: 0, to_node: id, to_input: 2 });
                (id, dx.max(dl).max(dh) + 1)
            }
        }
    }

    fn leaf(&mut self, op: NodeOp, cursor: &mut f32) -> NodeId {
        let y = *cursor;
        *cursor += ROW_H;
        self.push(op, 0.0, y)
    }

    fn y_of(&self, id: NodeId) -> f32 {
        self.nodes.iter().find(|n| n.id == id).map(|n| n.y).unwrap_or(0.0)
    }

    /// Convierte las profundidades guardadas en `x` a coordenadas de lienzo: la
    /// columna 0 es la más profunda (izquierda), y crece hacia la derecha.
    fn reflow(&mut self) {
        let maxd = self.nodes.iter().map(|n| n.x as usize).max().unwrap_or(0);
        for n in &mut self.nodes {
            let depth = n.x as usize;
            n.x = ORIGIN_X + (maxd - depth) as f32 * COL_W;
        }
    }

    // --- Grafo → fórmulas ---------------------------------------------------

    /// Fuente de cada campo, reconstruida siguiendo el cable de su sink hacia atrás.
    /// Campo sin sink o con la entrada del sink desconectada → `"0"`.
    pub fn to_fuentes(&self, n_campos: usize, sym: &Symbols) -> Vec<String> {
        (0..n_campos)
            .map(|k| {
                let sink = self.sinks.get(k).copied();
                let root = sink.and_then(|s| self.source_into(s, 0));
                match root {
                    Some(r) => self.build_expr_from(r, 0).to_source(sym),
                    None => "0".to_string(),
                }
            })
            .collect()
    }

    /// Nodo cuyo output alimenta la entrada `pin` del nodo `node` (o `None`).
    pub fn source_into(&self, node: NodeId, pin: PinIdx) -> Option<NodeId> {
        self.wires
            .iter()
            .find(|w| w.to_node == node && w.to_input == pin)
            .map(|w| w.from_node)
    }

    /// Reconstruye la `Expr` de un nodo (entradas faltantes → `Const(0)`). Corta a
    /// profundidad 128 por si un rewire malicioso arma un ciclo.
    fn build_expr_from(&self, node: NodeId, depth: u32) -> Expr {
        if depth > 128 {
            return Expr::Const(0.0);
        }
        let Some(n) = self.nodes.iter().find(|n| n.id == node) else {
            return Expr::Const(0.0);
        };
        let arg = |i: PinIdx| -> Expr {
            match self.source_into(node, i) {
                Some(src) => self.build_expr_from(src, depth + 1),
                None => Expr::Const(0.0),
            }
        };
        match n.op {
            NodeOp::Const(c) => Expr::Const(c),
            NodeOp::Field(f) => Expr::Field(f),
            NodeOp::Param(p) => Expr::Param(p),
            NodeOp::Dt => Expr::Dt,
            NodeOp::Lap(f) => Expr::Laplacian(f),
            NodeOp::Vecinos(r, f) => Expr::Vecinos(r, f),
            NodeOp::Dir(d, f) => Expr::Dir(d, f),
            NodeOp::Un(op) => Expr::Un(op, Box::new(arg(0))),
            NodeOp::Bin(op) => Expr::Bin(op, Box::new(arg(0)), Box::new(arg(1))),
            NodeOp::Clamp => Expr::Clamp(Box::new(arg(0)), Box::new(arg(1)), Box::new(arg(2))),
            NodeOp::Sink(_) => arg(0),
        }
    }

    // --- Edición ------------------------------------------------------------

    /// Agrega un nodo suelto (para el paladar de nodos). Devuelve su id.
    pub fn add(&mut self, op: NodeOp, x: f32, y: f32) -> NodeId {
        self.push(op, x, y)
    }

    /// Mueve un nodo (arrastre; layout).
    pub fn drag(&mut self, id: NodeId, dx: f32, dy: f32) {
        if let Some(n) = self.nodes.iter_mut().find(|n| n.id == id) {
            n.x = (n.x + dx).max(0.0);
            n.y = (n.y + dy).max(0.0);
        }
    }

    /// Conecta `from`(salida 0) → entrada `to_pin` de `to`, reemplazando lo que
    /// hubiera. Rechaza auto‑lazo y conexiones que crearían un ciclo.
    pub fn connect(&mut self, from: NodeId, to: NodeId, to_pin: PinIdx) -> bool {
        if from == to || self.reachable(to, from) {
            return false; // ciclo
        }
        self.wires.retain(|w| !(w.to_node == to && w.to_input == to_pin));
        self.wires.push(Wire { from_node: from, from_output: 0, to_node: to, to_input: to_pin });
        true
    }

    /// Borra un nodo y todos sus cables. Un sink no se borra (define el campo).
    pub fn delete(&mut self, id: NodeId) {
        if self.sinks.contains(&id) {
            return;
        }
        self.nodes.retain(|n| n.id != id);
        self.wires.retain(|w| w.from_node != id && w.to_node != id);
    }

    /// ¿Se llega de `start` a `target` siguiendo cables hacia adelante? (anti‑ciclo).
    fn reachable(&self, start: NodeId, target: NodeId) -> bool {
        let mut stack = vec![start];
        let mut seen = Vec::new();
        while let Some(n) = stack.pop() {
            if n == target {
                return true;
            }
            if seen.contains(&n) {
                continue;
            }
            seen.push(n);
            for w in self.wires.iter().filter(|w| w.from_node == n) {
                stack.push(w.to_node);
            }
        }
        false
    }

    /// Especificaciones para el widget (label por `sym`).
    pub fn node_specs(&self, sym: &Symbols) -> Vec<NodeSpec> {
        self.nodes
            .iter()
            .map(|n| NodeSpec {
                id: n.id,
                label: n.op.label(sym),
                x: n.x,
                y: n.y,
                inputs: n.op.inputs(),
                outputs: n.op.outputs(),
            })
            .collect()
    }
}

// --- Nombres de operadores (para labels) ------------------------------------

fn reduce_name(r: Reduce) -> &'static str {
    match r {
        Reduce::Promedio => "avg",
        Reduce::Suma => "sum6",
        Reduce::Min => "min6",
        Reduce::Max => "max6",
    }
}
fn dir_name(d: Dir) -> &'static str {
    match d {
        Dir::Abajo => "abajo",
        Dir::Arriba => "arriba",
        Dir::Este => "este",
        Dir::Oeste => "oeste",
        Dir::Norte => "norte",
        Dir::Sur => "sur",
    }
}
fn un_name(u: UnOp) -> &'static str {
    match u {
        UnOp::Neg => "−x",
        UnOp::Abs => "abs",
        UnOp::Exp => "exp",
        UnOp::Sqrt => "sqrt",
    }
}
fn bin_name(b: BinOp) -> &'static str {
    match b {
        BinOp::Add => "+",
        BinOp::Sub => "−",
        BinOp::Mul => "×",
        BinOp::Div => "÷",
        BinOp::Min => "min",
        BinOp::Max => "max",
        BinOp::Gt => ">",
        BinOp::Lt => "<",
    }
}
fn fmt_num(c: f32) -> String {
    if c.fract() == 0.0 && c.abs() < 1e7 {
        format!("{}", c as i64)
    } else {
        format!("{c}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym() -> Symbols {
        Symbols {
            campos: vec!["u".into(), "v".into()],
            params: vec!["Du".into(), "Dv".into(), "F".into(), "k".into()],
        }
    }

    #[test]
    fn round_trip_expr_grafo_expr() {
        let sym = sym();
        let fuentes = vec![
            "Du * lap(u) - u * v * v + F * (1 - u)".to_string(),
            "Dv * lap(v) + u * v * v - (F + k) * v".to_string(),
        ];
        let g = EqGraph::from_fuentes(&fuentes, &sym);
        let back = g.to_fuentes(2, &sym);
        for i in 0..2 {
            let a = Expr::parse(&fuentes[i], &sym).unwrap();
            let b = Expr::parse(&back[i], &sym).unwrap();
            assert_eq!(a, b, "campo {i}: {} → {}", fuentes[i], back[i]);
        }
    }

    #[test]
    fn round_trip_terminos_variados() {
        let sym = Symbols { campos: vec!["c".into()], params: vec!["k".into()] };
        for src in [
            "clamp(c + dt, 0, 1)",
            "min(avg(c), max6(c)) - abajo(c)",
            "k * lap(c) + abs(c) * exp(k)",
            "sqrt(c) < k",
        ] {
            let g = EqGraph::from_fuentes(&[src.to_string()], &sym);
            let back = g.to_fuentes(1, &sym);
            let a = Expr::parse(src, &sym).unwrap();
            let b = Expr::parse(&back[0], &sym).unwrap();
            assert_eq!(a, b, "{src} → {}", back[0]);
        }
    }

    #[test]
    fn rewire_cambia_la_formula() {
        let sym = Symbols { campos: vec!["c".into()], params: vec!["k".into()] };
        // c + k  → reconectar la entrada `b` del `+` a un nodo `dt`.
        let mut g = EqGraph::from_fuentes(&["c + k".to_string()], &sym);
        // encontrar el nodo `+` (Bin(Add)).
        let plus = g.nodes.iter().find(|n| matches!(n.op, NodeOp::Bin(BinOp::Add))).unwrap().id;
        let dt = g.add(NodeOp::Dt, 0.0, 0.0);
        assert!(g.connect(dt, plus, 1), "reconecta la entrada b");
        let back = g.to_fuentes(1, &sym);
        let got = Expr::parse(&back[0], &sym).unwrap();
        let want = Expr::parse("c + dt", &sym).unwrap();
        assert_eq!(got, want, "quedó: {}", back[0]);
    }

    #[test]
    fn connect_rechaza_ciclo() {
        let sym = Symbols { campos: vec!["c".into()], params: vec![] };
        let mut g = EqGraph::from_fuentes(&["c + c".to_string()], &sym);
        let plus = g.nodes.iter().find(|n| matches!(n.op, NodeOp::Bin(BinOp::Add))).unwrap().id;
        // intentar alimentar el `+` con su propia salida → ciclo, rechazado.
        assert!(!g.connect(plus, plus, 0), "auto-lazo rechazado");
    }

    #[test]
    fn sink_no_se_borra() {
        let sym = Symbols { campos: vec!["c".into()], params: vec![] };
        let mut g = EqGraph::from_fuentes(&["c".to_string()], &sym);
        let sink = g.sinks[0];
        g.delete(sink);
        assert!(g.nodes.iter().any(|n| n.id == sink), "el sink sobrevive");
    }
}
