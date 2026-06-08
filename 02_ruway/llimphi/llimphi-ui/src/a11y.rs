//! Traducción del árbol Llimphi a un árbol [AccessKit](https://accesskit.dev)
//! para alimentar lectores de pantalla y otras tecnologías de asistencia.
//!
//! Cada frame el runtime llama a [`build_tree`] con el árbol montado +
//! `ComputedLayout` + el id de foco actual. La función produce un
//! `accesskit::TreeUpdate` que el adapter (`accesskit_winit::Adapter`) empuja
//! al sistema operativo.
//!
//! ## Mapeo de identidades
//!
//! Cada `MountedNode` recibe un `NodeId(idx + ROOT_OFFSET)` derivado de su
//! índice en `Mounted::nodes` — estable dentro de un frame, no necesariamente
//! entre frames (si la app re-renderiza un árbol distinto, los ids cambian).
//! `ROOT_NODE_ID` queda reservado para el nodo raíz sintético que envuelve
//! todo el árbol. Los nodos sin semántica declarada igual aparecen en el árbol
//! si contienen texto, son `focusable` o tienen `on_click` — los lectores los
//! anuncian aunque el caller no haya marcado un rol explícito.
//!
//! ## Acciones soportadas (v1)
//!
//! - `Action::Focus`: mueve el foco de Llimphi a ese nodo (vía
//!   [`crate::App::on_focus`]).
//! - `Action::Click` / `Default`: ejecuta el `on_click` del nodo si existe;
//!   los handlers `*_at` se ignoran en v1 (no tienen una posición sintética
//!   coherente — la documentamos como limitación).

use accesskit::{Action, Node, NodeId, Rect as AkRect, Role as AkRole, Tree, TreeId, TreeUpdate};
use llimphi_compositor::{Mounted, Role as LRole, SemanticsSpec};
use llimphi_layout::ComputedLayout;

/// NodeId reservado para la raíz sintética del árbol. El nodo App es siempre
/// el padre lógico de todos los `MountedNode` que producimos.
pub const ROOT_NODE_ID: NodeId = NodeId(1);

/// Offset desde el cual numeramos los `MountedNode`. Deja el rango [0, OFFSET)
/// para ids reservados (root y futuros nodos sintéticos como overlays).
const MOUNTED_OFFSET: u64 = 1000;

/// `NodeId` AccessKit asignado al `MountedNode` con índice `idx`. La función
/// es inversa de [`mounted_idx_for`].
pub fn node_id_for(idx: usize) -> NodeId {
    NodeId(MOUNTED_OFFSET + idx as u64)
}

/// Recupera el índice del `MountedNode` que corresponde a un `NodeId`
/// AccessKit, o `None` si el id está fuera del rango de nodos montados.
pub fn mounted_idx_for(id: NodeId) -> Option<usize> {
    let v = id.0;
    if v >= MOUNTED_OFFSET {
        Some((v - MOUNTED_OFFSET) as usize)
    } else {
        None
    }
}

/// Construye el árbol AccessKit completo para el frame actual.
///
/// `focused_idx` es el índice del `MountedNode` enfocado (si lo hay) —
/// resolvelo desde `state.focused: Option<u64>` mapeando contra el campo
/// `focusable` de cada MountedNode.
pub fn build_tree<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    focused_idx: Option<usize>,
    app_name: &str,
    tree_id: TreeId,
) -> TreeUpdate {
    let mut nodes: Vec<(NodeId, Node)> = Vec::with_capacity(mounted.nodes.len() + 1);

    // 1) Raíz sintética: lista los hijos top-level (los nodos cuya posición en
    // el array es 0 o queda fuera de cualquier subárbol previo). En la práctica
    // el `MountedNode` con índice 0 es la raíz del View — la usamos directo.
    let mut root = Node::new(AkRole::Window);
    root.set_label(app_name.to_string());
    if !mounted.nodes.is_empty() {
        root.set_children(vec![node_id_for(0)]);
    }
    nodes.push((ROOT_NODE_ID, root));

    // 2) Un Node AccessKit por cada MountedNode. Hijos = los hijos directos en
    // el árbol pre-orden (subtree_end nos da el rango).
    for (idx, mn) in mounted.nodes.iter().enumerate() {
        let mut node = Node::new(map_role(&mn.semantics, mn));

        // Bounds del nodo (rect absoluto del layout). Sin esto el lector no
        // sabe dónde está visualmente y la navegación por reading order
        // degrada a "como vinieron".
        if let Some(r) = computed.get(mn.id) {
            node.set_bounds(AkRect {
                x0: r.x as f64,
                y0: r.y as f64,
                x1: (r.x + r.w) as f64,
                y1: (r.y + r.h) as f64,
            });
        }

        // Label / value / description. Si la app declaró semantics, mandamos
        // esos. Si no, intentamos derivar un label del texto visible — los
        // lectores leen igualmente texto sin rol, pero un label explícito es
        // más claro.
        if let Some(spec) = &mn.semantics {
            apply_semantics(&mut node, spec);
            // Si declaró rol pero no label y hay texto plano en el nodo,
            // caemos al texto: cubre widgets como `app-header` que setean
            // `.role(Heading)` sobre un nodo con `text_aligned("Título", …)`
            // sin duplicar el string en `.aria_label(...)`.
            if spec.label.is_none() {
                if let Some(t) = &mn.text {
                    node.set_label(t.content.clone());
                }
            }
        } else if let Some(t) = &mn.text {
            node.set_label(t.content.clone());
        }

        // Acciones: declaramos las que el adapter va a recibir y ejecutar.
        // `Focus` para cualquier nodo enfocable; `Click` para cualquier nodo
        // con `on_click`. El handler del runtime las despacha en `act` (ver
        // eventloop.rs).
        if mn.focusable.is_some() {
            node.add_action(Action::Focus);
        }
        if mn.on_click.is_some() || mn.on_click_at.is_some() {
            node.add_action(Action::Click);
        }

        // Hijos: rango [idx+1, subtree_end) — pero acá necesitamos sólo los
        // hijos DIRECTOS, no descendientes. Los hijos directos son los nodos
        // cuyo padre es este: en el orden pre-orden con `subtree_end`, los
        // hijos directos del nodo idx son los nodos h tales que h.parent == idx.
        // Lo computamos: empezamos desde idx+1 y saltamos por subtree_end de
        // cada hijo, hasta salir del rango.
        let children = direct_children(mounted, idx);
        if !children.is_empty() {
            node.set_children(children.into_iter().map(node_id_for).collect::<Vec<_>>());
        }

        nodes.push((node_id_for(idx), node));
    }

    let focus = focused_idx
        .map(node_id_for)
        .unwrap_or(ROOT_NODE_ID);

    TreeUpdate {
        nodes,
        tree: Some(Tree::new(ROOT_NODE_ID)),
        focus,
        tree_id,
    }
}

/// Índices de los hijos directos del MountedNode `parent_idx`. Asume el
/// recorrido pre-orden estándar del `mount`: el primer hijo está en
/// `parent_idx + 1`; los siguientes se obtienen saltando por `subtree_end`.
fn direct_children<Msg>(mounted: &Mounted<Msg>, parent_idx: usize) -> Vec<usize> {
    let parent = &mounted.nodes[parent_idx];
    let mut out = Vec::new();
    let mut cursor = parent_idx + 1;
    while cursor < parent.subtree_end {
        out.push(cursor);
        cursor = mounted.nodes[cursor].subtree_end;
    }
    out
}

/// Aplica los campos de un `SemanticsSpec` sobre un `accesskit::Node` recién
/// creado (rol ya fijado). Mapea flags ARIA → setters AccessKit.
fn apply_semantics(node: &mut Node, spec: &SemanticsSpec) {
    if let Some(label) = &spec.label {
        node.set_label(label.to_string());
    }
    if let Some(desc) = &spec.description {
        node.set_description(desc.to_string());
    }
    if let Some(value) = &spec.value {
        node.set_value(value.to_string());
    }
    // Flags. AccessKit usa `toggled` para checked/pressed (mismo enum); para
    // expanded hay `set_expanded(bool)`; disabled = is_disabled flag.
    if let Some(checked) = spec.flags.checked.or(spec.flags.pressed) {
        node.set_toggled(if checked {
            accesskit::Toggled::True
        } else {
            accesskit::Toggled::False
        });
    }
    if let Some(expanded) = spec.flags.expanded {
        node.set_expanded(expanded);
    }
    if spec.flags.disabled == Some(true) {
        node.set_disabled();
    }
    if spec.flags.readonly == Some(true) {
        node.set_read_only();
    }
    if spec.flags.required == Some(true) {
        node.set_required();
    }
}

/// Mapea un `Role` de Llimphi a un `accesskit::Role`. Para nodos sin
/// `semantics` declarado, fallback a `Role::GenericContainer` (un grupo
/// transparente que no aporta semántica propia pero permite que la jerarquía
/// se navegue).
fn map_role<Msg>(spec: &Option<SemanticsSpec>, _mn: &llimphi_compositor::MountedNode<Msg>) -> AkRole {
    let Some(role) = spec.as_ref().and_then(|s| s.role) else {
        return AkRole::GenericContainer;
    };
    match role {
        LRole::Button => AkRole::Button,
        LRole::TextInput => AkRole::TextInput,
        LRole::Heading => AkRole::Heading,
        LRole::Checkbox => AkRole::CheckBox,
        LRole::Label => AkRole::Label,
        LRole::Link => AkRole::Link,
        LRole::MenuItem => AkRole::MenuItem,
        LRole::Tab => AkRole::Tab,
        LRole::Image => AkRole::Image,
        LRole::Slider => AkRole::Slider,
        LRole::Group => AkRole::Group,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llimphi_compositor::{mount, Role as LRole, View};
    use llimphi_layout::taffy::prelude::length;
    use llimphi_layout::taffy::Size;
    use llimphi_layout::{LayoutTree, Style};

    /// Monta un árbol con un nodo botón + un texto plano. Devuelve mounted +
    /// computed contra un viewport razonable.
    fn arbol_simple() -> (llimphi_compositor::Mounted<()>, ComputedLayout) {
        let boton = View::<()>::new(Style {
            size: Size { width: length(80.0_f32), height: length(40.0_f32) },
            ..Default::default()
        })
        .role(LRole::Button)
        .aria_label("Guardar");
        let texto = View::<()>::new(Style::default()).text(
            "Hola",
            14.0,
            llimphi_raster::peniko::Color::WHITE,
        );
        let raiz = View::<()>::new(Style::default()).children(vec![boton, texto]);
        let mut layout = LayoutTree::new();
        let mounted = mount(&mut layout, raiz);
        let computed = layout
            .compute(mounted.root, (1000.0_f32, 1000.0_f32))
            .expect("layout");
        (mounted, computed)
    }

    #[test]
    fn build_tree_arma_raiz_y_un_node_por_mounted() {
        let (m, c) = arbol_simple();
        let tree = build_tree(&m, &c, None, "tawasuyu-test", TreeId(uuid::Uuid::nil()));
        // root + 3 nodos (raíz View, boton, texto).
        assert_eq!(tree.nodes.len(), 1 + m.nodes.len());
        assert_eq!(tree.nodes[0].0, ROOT_NODE_ID);
        // El segundo Node es el primer MountedNode (raíz del View).
        assert_eq!(tree.nodes[1].0, node_id_for(0));
        // Foco fallback = root sintético.
        assert_eq!(tree.focus, ROOT_NODE_ID);
    }

    #[test]
    fn boton_con_label_se_traduce_a_role_button() {
        let (m, c) = arbol_simple();
        let tree = build_tree(&m, &c, None, "test", TreeId(uuid::Uuid::nil()));
        // El nodo con role=Button debería tener rol Button en accesskit.
        let boton_node = tree
            .nodes
            .iter()
            .find(|(_, n)| n.role() == AkRole::Button)
            .expect("hay un Button");
        assert_eq!(boton_node.1.label().as_deref(), Some("Guardar"));
    }

    #[test]
    fn texto_sin_semantica_se_lee_como_label_del_node_generico() {
        let (m, c) = arbol_simple();
        let tree = build_tree(&m, &c, None, "test", TreeId(uuid::Uuid::nil()));
        // Algún nodo con label "Hola" (el texto plano).
        assert!(
            tree.nodes
                .iter()
                .any(|(_, n)| n.label().as_deref() == Some("Hola")),
            "el texto plano debería aparecer como label"
        );
    }

    #[test]
    fn foco_explicito_se_refleja_en_treeupdate_focus() {
        let (m, c) = arbol_simple();
        let tree = build_tree(&m, &c, Some(1), "test", TreeId(uuid::Uuid::nil()));
        assert_eq!(tree.focus, node_id_for(1));
    }

    #[test]
    fn mounted_idx_for_invierte_node_id_for() {
        for i in 0..16 {
            let nid = node_id_for(i);
            assert_eq!(mounted_idx_for(nid), Some(i));
        }
        assert_eq!(mounted_idx_for(ROOT_NODE_ID), None);
    }
}
