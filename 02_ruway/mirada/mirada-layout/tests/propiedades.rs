//! Fuzzing por propiedades del motor de teselado.
//!
//! Los 74 tests unitarios cubren casos concretos; esto barre el espacio de
//! entrada con miles de combinaciones aleatorias buscando la cola larga: un
//! modo × cuenta × tamaño de pantalla que viole un invariante o paniquee.
//!
//! Invariantes que **deben** sostenerse para cualquier entrada:
//!  - `tile` devuelve exactamente `count` rects.
//!  - Ningún rect es degenerado (ancho/alto negativos).
//!  - Cada rect cae dentro de la pantalla (con pantalla de área positiva).
//!  - `Workspace` nunca paniquea ante cualquier secuencia de operaciones, y
//!    el foco siempre apunta a una ventana viva (o a ninguna).

use mirada_layout::{tile, LayoutMode, LayoutParams, Rect, Workspace};
use proptest::prelude::*;

/// Los siete modos de teselado.
fn modo() -> impl Strategy<Value = LayoutMode> {
    prop_oneof![
        Just(LayoutMode::MasterStack),
        Just(LayoutMode::Monocle),
        Just(LayoutMode::Grid),
        Just(LayoutMode::Columns),
        Just(LayoutMode::Rows),
        Just(LayoutMode::CenteredMaster),
        Just(LayoutMode::Spiral),
    ]
}

/// Una pantalla de área positiva en un rango realista (incluye orígenes
/// negativos: en multi-monitor un output puede estar a la izquierda del 0).
fn pantalla() -> impl Strategy<Value = Rect> {
    (-2000i32..2000, -2000i32..2000, 1i32..6000, 1i32..4000)
        .prop_map(|(x, y, w, h)| Rect::new(x, y, w, h))
}

fn parametros() -> impl Strategy<Value = LayoutParams> {
    (modo(), 0.0f32..1.0, 1usize..6, 0i32..64).prop_map(|(mode, master_ratio, master_count, gap)| {
        LayoutParams {
            mode,
            master_ratio,
            master_count,
            gap,
        }
    })
}

proptest! {
    /// `tile` respeta cuenta, no-degeneración y contención para todo modo.
    #[test]
    fn tile_es_consistente(screen in pantalla(), count in 0usize..24, params in parametros()) {
        let rects = tile(screen, count, &params);

        // 1 · Cuenta exacta (contrato documentado de `tile`).
        prop_assert_eq!(rects.len(), count);

        for r in &rects {
            // 2 · Nada degenerado.
            prop_assert!(r.w >= 0, "ancho negativo: {:?}", r);
            prop_assert!(r.h >= 0, "alto negativo: {:?}", r);

            // 3 · Contención: el rect no se sale de la pantalla. Sólo se
            //     exige para celdas visibles (las de área 0 colapsan en un
            //     borde y la comparación deja de ser informativa).
            if r.is_visible() {
                prop_assert!(r.x >= screen.x, "x fuera por izquierda: {:?} en {:?}", r, screen);
                prop_assert!(r.y >= screen.y, "y fuera por arriba: {:?} en {:?}", r, screen);
                prop_assert!(
                    r.x + r.w <= screen.x + screen.w,
                    "se sale por derecha: {:?} en {:?}", r, screen
                );
                prop_assert!(
                    r.y + r.h <= screen.y + screen.h,
                    "se sale por abajo: {:?} en {:?}", r, screen
                );
            }
        }
    }
}

/// Una operación sobre el `Workspace` — el alfabeto del fuzzing.
#[derive(Debug, Clone)]
enum Op {
    Add(u64),
    Remove(u64),
    Focus(u64),
    FocusNext,
    FocusPrev,
    MoveForward,
    MoveBackward,
    Promote,
    Swap(u64, u64),
    SetMode(LayoutMode),
    Fullscreen(Option<u64>),
    Float(u64, Option<Rect>),
}

fn operacion() -> impl Strategy<Value = Op> {
    // IDs en un rango chico → se reutilizan (abrir/cerrar/enfocar la misma
    // ventana), que es donde aparecen los bugs de estado, no con IDs únicos.
    let id = 0u64..8;
    prop_oneof![
        id.clone().prop_map(Op::Add),
        id.clone().prop_map(Op::Remove),
        id.clone().prop_map(Op::Focus),
        Just(Op::FocusNext),
        Just(Op::FocusPrev),
        Just(Op::MoveForward),
        Just(Op::MoveBackward),
        Just(Op::Promote),
        (id.clone(), id.clone()).prop_map(|(a, b)| Op::Swap(a, b)),
        modo().prop_map(Op::SetMode),
        proptest::option::of(id.clone()).prop_map(Op::Fullscreen),
        (id.clone(), proptest::option::of(Just(Rect::new(0, 0, 100, 100))))
            .prop_map(|(w, r)| Op::Float(w, r)),
    ]
}

proptest! {
    /// Ninguna secuencia de operaciones tumba el `Workspace`, y tras
    /// aplicarla el foco apunta a una ventana presente (o a ninguna), y todo
    /// rect del layout pertenece a una ventana presente y no es degenerado.
    #[test]
    fn workspace_mantiene_invariantes(ops in proptest::collection::vec(operacion(), 0..200)) {
        let mut ws = Workspace::new(LayoutParams::default());
        for op in ops {
            match op {
                Op::Add(w) => ws.add(w),
                Op::Remove(w) => { ws.remove(w); }
                Op::Focus(w) => { ws.focus_window(w); }
                Op::FocusNext => ws.focus_next(),
                Op::FocusPrev => ws.focus_prev(),
                Op::MoveForward => ws.move_focused_forward(),
                Op::MoveBackward => ws.move_focused_backward(),
                Op::Promote => ws.promote_focused(),
                Op::Swap(a, b) => { ws.swap(a, b); }
                Op::SetMode(m) => ws.set_mode(m),
                Op::Fullscreen(w) => ws.set_fullscreen(w),
                Op::Float(w, r) => ws.set_floating(w, r),
            }
        }

        let presentes: Vec<u64> = ws.windows().to_vec();

        // El foco apunta a algo vivo (o a nada).
        if let Some(f) = ws.focused() {
            prop_assert!(presentes.contains(&f), "foco {} ausente de {:?}", f, presentes);
        }

        // Cada ventana del layout existe y tiene rect no degenerado.
        let screen = Rect::new(0, 0, 1920, 1080);
        for (id, rect) in ws.layout(screen) {
            prop_assert!(presentes.contains(&id), "layout pinta ausente {}", id);
            prop_assert!(rect.w >= 0 && rect.h >= 0, "rect degenerado {:?}", rect);
        }
    }
}
