use super::*;

/// Menú contextual del output (click derecho): Copiar selección · Copiar todo ·
/// Seleccionar todo. `None` si no está abierto. Las acciones operan sobre el
/// bloque guardado en `state.body_menu`. "Copiar" se deshabilita sin selección.
pub(crate) fn body_context_menu<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> Option<View<HostMsg>> {
    use llimphi_widget_context_menu::{
        context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
    };
    let (x, y, block) = state.body_menu?;
    let mut copiar = ContextMenuItem::action("Copiar").with_shortcut("Ctrl+C");
    if !menu_has_selection(state, block) {
        copiar = copiar.disabled();
    }
    let items = vec![
        copiar,
        ContextMenuItem::action("Copiar todo"),
        ContextMenuItem::action("Seleccionar todo"),
    ];
    let lift_pick = lift.clone();
    let menu = context_menu_view(ContextMenuSpec {
        anchor: (x, y),
        viewport: (1280.0, 800.0),
        header: None,
        items,
        active: usize::MAX,
        on_pick: std::sync::Arc::new(move |i| lift_pick(Msg::BodyMenuPick(i))),
        on_dismiss: lift(Msg::BodyMenuDismiss),
        palette: ContextMenuPalette::from_theme(theme),
    });
    // El menú (con su scrim full-screen) está hecho para `view_overlay`; acá lo
    // hospedamos en el flujo del shell, así que lo sacamos del layout flex con
    // un contenedor `Position::Absolute` (si no, el scrim aplasta el output).
    Some(
        View::new(Style {
            position: Position::Absolute,
            inset: Rect {
                left: length(0.0_f32),
                top: length(0.0_f32),
                right: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .children(vec![menu]),
    )
}
