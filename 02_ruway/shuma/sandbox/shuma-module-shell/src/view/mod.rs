use llimphi_ui::llimphi_layout::taffy::prelude::auto;
use llimphi_ui::llimphi_layout::taffy::style::Position;
use super::*;

mod input;
mod tui;
mod ansi;
mod history_panel;
mod output_pane;
mod surface_view;
mod command_card;
mod output_line;
mod chrome;
#[cfg(test)]
mod gpu_grid_tests;

pub(crate) use input::*;
pub(crate) use tui::*;
pub(crate) use ansi::*;
pub(crate) use history_panel::*;
pub(crate) use output_pane::*;
pub(crate) use surface_view::*;
pub(crate) use command_card::*;
pub(crate) use output_line::*;
pub(crate) use chrome::*;

/// Vista pública del **input vivo** del shell, aislado del resto del shell. Lo
/// usan los frontends que quieren hospedar la línea de entrada en su propio
/// chasis (p. ej. la barra de pata: el cabezal de la barra ES este input, no un
/// placeholder). Comparte estado con [`body_view`] — los dos pintan distintas
/// partes del mismo `State` y se enrutan los `Msg` por el mismo `lift`.
pub fn input_view<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    shell_input_view(state, theme, lift)
}

/// Vista pública del **cuerpo** del shell sin el input: header + panel
/// principal (cards/PTY/TUI) + popups internos (completado, búsqueda de
/// historial, menú contextual). La usa pata para el drawer mientras el input
/// real vive en la barra.
pub fn body_view<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    let header = shell_header(state, theme);
    let main_panel: View<HostMsg> = if is_tui_fullscreen(state) {
        tui_panel::<HostMsg>(state, theme, lift.clone())
    } else if is_tui_active(state) {
        pty_lines_panel::<HostMsg>(state, theme)
    } else if terminal_surface_enabled() {
        output_pane_surface::<HostMsg>(state, theme, &lift)
    } else {
        output_pane::<HostMsg>(state, theme, &lift)
    };
    let body: View<HostMsg> = if !state.groups.is_empty() && !is_tui_active(state) {
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            flex_basis: length(0.0_f32),
            flex_grow: 1.0,
            min_size: Size {
                width: Dimension::auto(),
                height: length(0.0_f32),
            },
            gap: Size {
                width: length(8.0_f32),
                height: length(0.0_f32),
            },
            align_items: Some(AlignItems::Stretch),
            ..Default::default()
        })
        .children(vec![groups_panel::<HostMsg>(state, theme, &lift), main_panel])
    } else {
        main_panel
    };

    let mut children = vec![header, body];
    if state.history_search.is_some() {
        children.push(history_search_panel::<HostMsg>(state, theme));
    }
    if let Some(menu) = body_context_menu::<HostMsg>(state, theme, &lift) {
        children.push(menu);
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(8.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(children)
}

pub fn view<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    let header = shell_header(state, theme);
    // Render según la señal dura de alt-screen: pantalla completa (grid/vim)
    // sólo si el PTY entró a alternate screen; un PTY en modo líneas (p. ej.
    // `watch`) se lee como IDE-text; sin PTY, las cards de comandos.
    let main_panel: View<HostMsg> = if is_tui_fullscreen(state) {
        tui_panel::<HostMsg>(state, theme, lift.clone())
    } else if is_tui_active(state) {
        pty_lines_panel::<HostMsg>(state, theme)
    } else if terminal_surface_enabled() {
        // Experimental, detrás de SHUMA_TERMINAL_SURFACE (A/B con el viejo).
        output_pane_surface::<HostMsg>(state, theme, &lift)
    } else {
        output_pane::<HostMsg>(state, theme, &lift)
    };
    // Panel de grupos [RUN] a la izquierda (rescate del shell GPUI): cada
    // grupo guardado (`:save`) es una card clickable que lo ejecuta, con su
    // tecla F. Sólo aparece si hay grupos y no estamos en un TUI fullscreen.
    let body: View<HostMsg> = if !state.groups.is_empty() && !is_tui_active(state) {
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            flex_basis: length(0.0_f32),
            flex_grow: 1.0,
            min_size: Size {
                width: Dimension::auto(),
                height: length(0.0_f32),
            },
            gap: Size {
                width: length(8.0_f32),
                height: length(0.0_f32),
            },
            align_items: Some(AlignItems::Stretch),
            ..Default::default()
        })
        .children(vec![groups_panel::<HostMsg>(state, theme, &lift), main_panel])
    } else {
        main_panel
    };
    let input = shell_input_view(state, theme, lift.clone());

    let mut children = vec![header, body];
    // Banner de reprocess: el próximo comando recibe por stdin el stdout
    // del bloque armado. Click → cancela (toggle).
    if let Some(src) = state.reprocess_source {
        children.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(18.0_f32),
                },
                padding: Rect {
                    left: length(8.0_f32),
                    right: length(8.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                ..Default::default()
            })
            .fill(theme.bg_input)
            .radius(3.0)
            .hover_fill(theme.bg_row_hover)
            .on_click(lift(Msg::SetReprocess(src)))
            .text_aligned(
                format!("reprocesando la salida del bloque #{src} — Enter ejecuta · click cancela"),
                10.0,
                theme.accent,
                Alignment::Start,
            ),
        );
    }
    // Popup de completado: justo encima del input, candidatos con el
    // resaltado actual. Tab/flechas navegan, Enter acepta, Esc cierra.
    if let Some(popup) = completion_popup::<HostMsg>(state, theme) {
        children.push(popup);
    }
    if let Some(banner) = input_focus_banner::<HostMsg>(state, theme, &lift) {
        children.push(banner);
    }
    children.push(input);
    if state.history_search.is_some() {
        children.push(history_search_panel::<HostMsg>(state, theme));
    }
    // Menú contextual del output (click derecho): overlay por encima de todo,
    // sin clip — por eso va último en los children del root. Sus coords son del
    // nodo raíz (este mismo), así que el `anchor` cae donde se hizo click.
    if let Some(menu) = body_context_menu::<HostMsg>(state, theme, &lift) {
        children.push(menu);
    }

    let lift_menu = lift.clone();
    let lift_scale = lift.clone();
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(8.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    // Ctrl+rueda (o pinch de trackpad) sobre cualquier parte del shell = zoom
    // del texto. Va por `on_scale` —que el runtime resuelve ANTES que el
    // `on_scroll` de la superficie de output— para que la rueda con Ctrl no se
    // la coma el scroll del cuerpo (era el bug del "zoom con mouse que falta").
    // `factor` es el cambio multiplicativo incremental (>1 agranda); `ZoomBy`
    // lo aplica igual que el pinch.
    .on_scale(move |_phase, factor, _fx, _fy| Some(lift_scale(Msg::ZoomBy(factor))))
    // Click derecho en cualquier parte del output → menú contextual en `(x, y)`
    // (coords locales a este nodo raíz). El cuerpo IDE ya no captura el right-
    // click (lo delega acá) para que el menú gane.
    .on_right_click_at(move |x, y, _w, _h| Some(lift_menu(Msg::OpenBodyMenu { x, y })))
    .children(children)
}

/// Banner sobre la línea que avisa a qué comando vivo va el Enter (stdin),
/// cuando el input está dirigido a un job en vez de a la línea. `None` cuando
/// el foco es la línea (arrancar comandos). Click → vuelve a la línea.
pub(crate) fn input_focus_banner<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> Option<View<HostMsg>> {
    let block = state.input_focus?;
    // Sólo si el destino sigue vivo (si murió, el update ya limpió el foco; este
    // chequeo cubre el frame intermedio).
    let arc = state.job_by_block(block)?;
    let cmd = arc.lock().ok().map(|g| g.command.clone())?;
    let label = format!("→ Enter va al stdin de «{cmd}»  ·  click o mouse sobre la línea para volver a tipear comandos");
    Some(
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(18.0_f32),
            },
            flex_shrink: 0.0,
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_input_focus)
        .radius(3.0)
        .hover_fill(theme.bg_row_hover)
        .on_click(lift(Msg::FocusInput))
        .text_aligned(label, 10.0, theme.accent, Alignment::Start)
        .mono()
        .max_lines(1),
    )
}
