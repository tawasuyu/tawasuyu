//! **LayoutBuilder** — el 4º seam de PARIDAD-FLUTTER: construir un subárbol
//! sensible al **tamaño del slot** del nodo (no de la ventana — para eso
//! alcanza `on_resize` + el Model). Flutter `LayoutBuilder`.
//!
//! El modelo de Llimphi corre `view → mount → compute → paint`: el `View` se
//! arma ANTES de conocer el layout, así que "construir distinto según el espacio
//! disponible" exige diferir. La solución, sin tocar `mount`/`paint`, es una
//! **resolución en dos pasadas** orquestada por el runtime:
//!
//! 1. Montar el árbol tal cual ([`crate::View::layout_builder`] queda como
//!    **hoja** — no tiene `children` estáticos) y computar el layout. Ahora cada
//!    builder tiene su rect resuelto por su `Style`/contexto flex.
//! 2. [`collect_builder_constraints`] lee esos rects (en pre-orden), se pide un
//!    `view()` fresco y [`expand_layout_builders`] invoca cada closure con sus
//!    [`crate::Constraints`] para producir el subárbol real. Ese árbol expandido
//!    se monta y pinta normalmente.
//!
//! [`has_layout_builder`] hace que todo esto sea **coste cero** cuando ningún
//! nodo usa el builder (el caso de la abrumadora mayoría de frames): es un
//! simple walk que corta el camino de dos pasadas.
//!
//! **Correspondencia de orden.** `collect_builder_constraints` recorre
//! `Mounted::nodes` (pre-orden, padre antes que hijos — el orden en que `mount`
//! los pushea) filtrando `is_layout_builder`; `expand_layout_builders` recorre
//! el `View` fresco en el MISMO pre-orden asignando un índice por builder. Como
//! ambos árboles salen del mismo `view(model)` determinista, el i-ésimo builder
//! de uno corresponde al i-ésimo del otro — por eso alcanza con un `Vec`
//! ordenado, sin keys.
//!
//! **Límite v1**: sin anidamiento. Un builder cuyo subárbol producido contiene
//! otro `layout_builder` no resuelve el interno (no existía en la pasada 1):
//! queda como hoja. El anidamiento requeriría iterar la resolución; se difiere.

use crate::{Constraints, ComputedLayout, Mounted, View};

/// `true` si `view` o algún descendiente declara un [`crate::View::layout_builder`].
/// El runtime lo usa para decidir si vale la pena la resolución en dos pasadas;
/// cuando es `false` (lo normal) el camino diferido se evita por completo.
pub fn has_layout_builder<Msg>(view: &View<Msg>) -> bool {
    view.layout_builder.is_some() || view.children.iter().any(has_layout_builder)
}

/// Lee las [`Constraints`] (tamaño del slot) de cada nodo `is_layout_builder`
/// del árbol montado, en pre-orden. El runtime las pasa a
/// [`expand_layout_builders`]. Un nodo sin rect computado (fuera del layout)
/// cae a `0×0`.
pub fn collect_builder_constraints<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
) -> Vec<Constraints> {
    mounted
        .nodes
        .iter()
        .filter(|n| n.is_layout_builder)
        .map(|n| {
            computed
                .get(n.id)
                .map(|r| Constraints { max_width: r.w, max_height: r.h })
                .unwrap_or(Constraints { max_width: 0.0, max_height: 0.0 })
        })
        .collect()
}

/// Expande los `layout_builder` de `view` (pre-orden) usando `cons` — una
/// [`Constraints`] por builder, en el orden que produjo
/// [`collect_builder_constraints`]. Cada builder se reemplaza por un nodo
/// contenedor (su mismo `Style`) cuyo único hijo es lo que devolvió la closure
/// invocada con sus constraints. Builders sin constraint correspondiente (más
/// builders que `cons`, p. ej. uno anidado recién producido) caen a `0×0` y se
/// resuelven igual, pero su tamaño será nulo (límite v1: sin anidamiento).
/// Consume `view`.
pub fn expand_layout_builders<Msg>(view: View<Msg>, cons: &[Constraints]) -> View<Msg> {
    let mut idx = 0;
    expand_rec(view, cons, &mut idx)
}

fn expand_rec<Msg>(mut view: View<Msg>, cons: &[Constraints], idx: &mut usize) -> View<Msg> {
    if let Some(builder) = view.layout_builder.take() {
        let c = cons
            .get(*idx)
            .copied()
            .unwrap_or(Constraints { max_width: 0.0, max_height: 0.0 });
        *idx += 1;
        // El builder posee los hijos: descartamos cualquier `children` estático
        // y ponemos lo que produjo la closure. NO recursamos en el resultado
        // (v1 sin anidamiento — un builder interno queda como hoja al montarse).
        let child = builder(c);
        view.children = vec![child];
        view
    } else {
        let children = std::mem::take(&mut view.children);
        view.children = children
            .into_iter()
            .map(|c| expand_rec(c, cons, idx))
            .collect();
        view
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{mount, Constraints};
    use llimphi_layout::taffy::prelude::*;
    use llimphi_layout::{LayoutTree, Style};

    /// Árbol sin builders → `has_layout_builder` falso y expand es no-op.
    #[test]
    fn sin_builder_es_noop() {
        let v = View::<()>::new(Style::default())
            .children(vec![View::<()>::new(Style::default())]);
        assert!(!has_layout_builder(&v));
        let v = expand_layout_builders(v, &[]);
        assert_eq!(v.children.len(), 1);
    }

    #[test]
    fn detecta_builder_anidado_en_hijos() {
        let v = View::<()>::new(Style::default()).children(vec![
            View::<()>::new(Style::default()),
            View::<()>::new(Style::default()).layout_builder(|_c| View::<()>::new(Style::default())),
        ]);
        assert!(has_layout_builder(&v));
    }

    /// El builder recibe las constraints y produce su subárbol; el nodo deja de
    /// ser builder y queda como contenedor con el hijo producido.
    #[test]
    fn expand_invoca_closure_con_constraints() {
        // Dos columnas a percent(0.5) del root 400px → cada slot = 200px. La de
        // la izquierda es un builder que mete 1 hijo si es angosta (<300) o 2 si
        // es ancha. A 200px mete 1.
        let build_col = |c: Constraints| {
            let n = if c.max_width < 300.0 { 1 } else { 2 };
            View::<()>::new(Style::default())
                .children((0..n).map(|_| View::<()>::new(Style::default())).collect())
        };
        let root = View::<()>::new(Style {
            size: Size { width: length(400.0), height: length(100.0) },
            flex_direction: FlexDirection::Row,
            ..Default::default()
        })
        .children(vec![
            View::<()>::new(Style {
                size: Size { width: percent(0.5), height: percent(1.0) },
                ..Default::default()
            })
            .layout_builder(build_col),
            View::<()>::new(Style {
                size: Size { width: percent(0.5), height: percent(1.0) },
                ..Default::default()
            }),
        ]);

        // Pasada 1: montar (builder como hoja) y computar.
        let mut l1 = LayoutTree::new();
        let m1 = mount(&mut l1, root);
        let c1 = l1.compute(m1.root, (400.0, 100.0)).expect("layout");
        let cons = collect_builder_constraints(&m1, &c1);
        assert_eq!(cons.len(), 1, "un solo builder");
        assert!((cons[0].max_width - 200.0).abs() < 1.0, "slot 200px: {:?}", cons[0]);

        // Pasada 2: árbol fresco (mismo Style) + expand.
        let root2 = View::<()>::new(Style {
            size: Size { width: length(400.0), height: length(100.0) },
            flex_direction: FlexDirection::Row,
            ..Default::default()
        })
        .children(vec![
            View::<()>::new(Style {
                size: Size { width: percent(0.5), height: percent(1.0) },
                ..Default::default()
            })
            .layout_builder(build_col),
            View::<()>::new(Style {
                size: Size { width: percent(0.5), height: percent(1.0) },
                ..Default::default()
            }),
        ]);
        let expanded = expand_layout_builders(root2, &cons);
        // El nodo builder (hijo 0 del root) ya no es builder y tiene 1 hijo
        // producido (slot 200 < 300 → angosto → 1 columna).
        let col_izq = &expanded.children[0];
        assert!(col_izq.layout_builder.is_none(), "ya expandido");
        assert_eq!(col_izq.children.len(), 1, "200px angosto → 1 hijo");
    }

    /// Con un slot ancho el mismo builder produce 2 hijos — verifica que la
    /// rama de decisión depende de las constraints reales.
    #[test]
    fn slot_ancho_produce_mas_hijos() {
        let build_col = |c: Constraints| {
            let n = if c.max_width < 300.0 { 1 } else { 2 };
            View::<()>::new(Style::default())
                .children((0..n).map(|_| View::<()>::new(Style::default())).collect())
        };
        // Constraint inyectada directo: 500px → ancho. El builder devuelve UN
        // contenedor (hijo único del nodo) con 2 columnas adentro.
        let v = View::<()>::new(Style::default()).layout_builder(build_col);
        let expanded = expand_layout_builders(v, &[Constraints { max_width: 500.0, max_height: 100.0 }]);
        assert_eq!(expanded.children.len(), 1, "el builder produce 1 contenedor");
        assert_eq!(expanded.children[0].children.len(), 2, "ancho → 2 columnas");
    }

    /// Pre-orden: dos builders hermanos reciben sus constraints en orden.
    #[test]
    fn dos_builders_reciben_constraints_en_preorden() {
        let mk = |w: f32| {
            move |_c: Constraints| {
                View::<()>::new(Style {
                    size: Size { width: length(w), height: length(10.0) },
                    ..Default::default()
                })
            }
        };
        let root = View::<()>::new(Style::default()).children(vec![
            View::<()>::new(Style::default()).layout_builder(mk(1.0)),
            View::<()>::new(Style::default()).layout_builder(mk(2.0)),
        ]);
        let cons = vec![
            Constraints { max_width: 111.0, max_height: 0.0 },
            Constraints { max_width: 222.0, max_height: 0.0 },
        ];
        let expanded = expand_layout_builders(root, &cons);
        // Ambos expandidos, en orden (verificamos vía el ancho del hijo producido
        // que NO depende de la constraint acá — sólo confirmamos que se invocaron
        // los dos y que ninguno quedó como builder).
        assert!(expanded.children[0].layout_builder.is_none());
        assert!(expanded.children[1].layout_builder.is_none());
        assert_eq!(expanded.children[0].children.len(), 1);
        assert_eq!(expanded.children[1].children.len(), 1);
    }
}
