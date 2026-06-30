//! Overlays del shell nahual: barra de menú, menú contextual y modales
//! (prompt de nombre, confirmación de borrado, renombrado por lote). Movido
//! de `main.rs` en el split de 2026-06-12 (puro movimiento de código).

use std::sync::Arc;

use crate::modelo::*;
use crate::helpers::viewport_of;
use crate::state::Label;
use llimphi_ui::{Handle, View};
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_theme::Theme;
use app_bus::{AppMenu, Menu, MenuItem};
use llimphi_widget_menubar::{MenuBarSpec, DEFAULT_HEIGHT as MENU_H};
use llimphi_widget_context_menu::{ContextMenuItem, ContextMenuPalette, ContextMenuSpec};

/// Etiqueta de la entrada seleccionada para el header del contextual.
pub(crate) fn etiqueta_seleccion(m: &Model) -> String {
    m.cur()
        .selected_node()
        .map(|n| n.name.clone())
        .unwrap_or_else(|| rimay_localize::t("nahual-shell-entry"))
}

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
pub(crate) fn menubar_spec<'a>(menu: &'a AppMenu, model: &Model, theme: &'a Theme) -> MenuBarSpec<'a, Msg> {
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme,
        viewport: viewport_of(model),
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

/// El menú principal del shell. Sólo comandos que mapean a `Msg` reales:
/// navegación (abrir/subir), montaje de fuentes no-POSIX (nouser/minga),
/// desmontar, tema. Sin "Editar": el shell no tiene campos de texto
/// editables — el panel derecho son visores de sólo lectura.
pub(crate) fn app_menu(model: &Model) -> AppMenu {
    let montado = model.is_foreign();
    // Montar sólo aplica desde POSIX (no anidamos fuentes); desmontar sólo
    // cuando hay una fuente activa. Reflejamos eso en gris.
    let mut mount_nouser = MenuItem::new(rimay_localize::t("nahual-shell-mount-nouser"), "file.mount_nouser")
        .shortcut("m")
        .separated();
    let mut mount_minga = MenuItem::new(rimay_localize::t("nahual-shell-mount-minga"), "file.mount_minga").shortcut("g");
    let mut mount_disp =
        MenuItem::new(rimay_localize::t("nahual-shell-mount-dispositivos"), "file.mount_dispositivos");
    let mut unmount = MenuItem::new(rimay_localize::t("nahual-shell-unmount"), "file.unmount").separated();
    if montado {
        mount_nouser = mount_nouser.disabled();
        mount_minga = mount_minga.disabled();
        mount_disp = mount_disp.disabled();
    } else {
        unmount = unmount.disabled();
    }
    // Operaciones de archivo (Fase 4.3): sólo sobre POSIX escribible. Sobre una
    // fuente montada read-only salen en gris.
    let editable = model.can_edit();
    let mut newdir = MenuItem::new(rimay_localize::t("nahual-shell-new-dir"), "file.newdir").shortcut("F7").separated();
    let mut newfile = MenuItem::new(rimay_localize::t("nahual-shell-new-file"), "file.newfile");
    let mut rename = MenuItem::new(rimay_localize::t("nahual-shell-rename"), "file.rename").shortcut("F2");
    let mut delete = MenuItem::new(rimay_localize::t("nahual-shell-delete"), "file.delete").shortcut("Supr");
    if !editable {
        newdir = newdir.disabled();
        newfile = newfile.disabled();
        rename = rename.disabled();
        delete = delete.disabled();
    }
    AppMenu::new()
        .menu(
            Menu::new(rimay_localize::t("nahual-shell-grp-file"))
                .item(MenuItem::new(rimay_localize::t("open"), "file.open").shortcut("Enter"))
                .item(MenuItem::new(rimay_localize::t("nahual-shell-parent"), "file.parent").shortcut("Backspace"))
                .item(newdir)
                .item(newfile)
                .item(rename)
                .item(delete)
                .item(mount_nouser)
                .item(mount_minga)
                .item(mount_disp)
                .item(unmount)
                .item(MenuItem::new(rimay_localize::t("exit"), "file.quit").shortcut("Ctrl+Q").separated()),
        )
        .menu(etiqueta_menu(editable))
        .menu(Menu::new(rimay_localize::t("nahual-shell-menu-view")).item(MenuItem::new(rimay_localize::t("cycle-theme"), "view.theme")))
        .menu(Menu::new(rimay_localize::t("help")).item(MenuItem::new(rimay_localize::t("about"), "help.about")))
}

/// El menú "Etiqueta": los siete colores + "Sin etiqueta". Aplica a la marca
/// múltiple o, si no hay, al nodo bajo el cursor. Gris si la fuente no es POSIX.
pub(crate) fn etiqueta_menu(editable: bool) -> Menu {
    let mut menu = Menu::new(rimay_localize::t("nahual-shell-grp-label"));
    for label in Label::ALL {
        // Un punto del color como prefijo del nombre (el menubar pinta texto).
        let mut it = MenuItem::new(format!("● {}", label.name()), label_cmd(label));
        if !editable {
            it = it.disabled();
        }
        menu = menu.item(it);
    }
    let mut sin = MenuItem::new(rimay_localize::t("nahual-shell-label-none"), "label.none").separated();
    if !editable {
        sin = sin.disabled();
    }
    menu.item(sin)
}

/// El command id del menú para cada color.
pub(crate) fn label_cmd(label: Label) -> &'static str {
    match label {
        Label::Red => "label.red",
        Label::Orange => "label.orange",
        Label::Yellow => "label.yellow",
        Label::Green => "label.green",
        Label::Blue => "label.blue",
        Label::Purple => "label.purple",
        Label::Gray => "label.gray",
    }
}

/// Inversa de [`label_cmd`]: el `Label` (o `None` para "Sin etiqueta") que un
/// command id de etiqueta denota.
pub(crate) fn label_from_cmd(cmd: &str) -> Option<Option<Label>> {
    match cmd {
        "label.red" => Some(Some(Label::Red)),
        "label.orange" => Some(Some(Label::Orange)),
        "label.yellow" => Some(Some(Label::Yellow)),
        "label.green" => Some(Some(Label::Green)),
        "label.blue" => Some(Some(Label::Blue)),
        "label.purple" => Some(Some(Label::Purple)),
        "label.gray" => Some(Some(Label::Gray)),
        "label.none" => Some(None),
        _ => None,
    }
}

/// Traduce un command id del menú principal al `Msg`/efecto real.
pub(crate) fn handle_menu_command(model: Model, cmd: &str, handle: &Handle<Msg>) -> Model {
    match cmd {
        "file.open" => handle.dispatch(Msg::OpenSelected),
        "file.parent" => handle.dispatch(Msg::Parent),
        "file.newdir" => handle.dispatch(Msg::NewDirPrompt),
        "file.newfile" => handle.dispatch(Msg::NewFilePrompt),
        "file.rename" => handle.dispatch(Msg::RenamePrompt),
        "file.delete" => handle.dispatch(Msg::DeleteSelection),
        "file.mount_nouser" => handle.dispatch(Msg::MountNouser),
        "file.mount_minga" => handle.dispatch(Msg::MountMinga),
        "file.mount_dispositivos" => handle.dispatch(Msg::MountDispositivos),
        "file.unmount" => handle.dispatch(Msg::Unmount),
        "file.quit" => std::process::exit(0),
        "view.theme" => handle.dispatch(Msg::CycleTheme),
        // Etiquetas: cada color (o "Sin etiqueta") despacha su Msg.
        _ if label_from_cmd(cmd).is_some() => match label_from_cmd(cmd).unwrap() {
            Some(label) => handle.dispatch(Msg::SetLabel(label)),
            None => handle.dispatch(Msg::ClearLabel),
        },
        // "help.about" y desconocidos: no-op (sin diálogo todavía).
        _ => {}
    }
    model
}

/// Arma el `ContextMenuSpec` del menú contextual sobre la entrada
/// seleccionada. Las acciones son las navegaciones/montajes que ya existen
/// como `Msg` — no inventamos edición (no hay campos de texto).
pub(crate) fn context_menu_spec(model: &Model, x: f32, y: f32) -> ContextMenuSpec<Msg> {
    let montado = model.is_foreign();
    // Construimos la lista de (item, msg) según el contexto, para que el
    // índice del `on_pick` y el item visible siempre coincidan.
    let mut acciones: Vec<(ContextMenuItem, Msg)> = vec![
        (ContextMenuItem::action(rimay_localize::t("open")), Msg::OpenSelected),
        (ContextMenuItem::action(rimay_localize::t("nahual-shell-parent")), Msg::Parent),
    ];
    // Operaciones de archivo (Fase 4.3): sólo sobre POSIX escribible.
    if model.can_edit() {
        acciones.push((ContextMenuItem::action(rimay_localize::t("nahual-shell-new-dir")), Msg::NewDirPrompt));
        acciones.push((ContextMenuItem::action(rimay_localize::t("nahual-shell-new-file")), Msg::NewFilePrompt));
        if model.cur().selected_node().is_some() {
            acciones.push((ContextMenuItem::action(rimay_localize::t("nahual-shell-rename")), Msg::RenamePrompt));
            acciones.push((ContextMenuItem::action(rimay_localize::t("nahual-shell-delete")), Msg::DeleteSelection));
        }
        if !model.cur_pane().marked.is_empty() {
            acciones.push((
                ContextMenuItem::action(rimay_localize::t("nahual-shell-batch-rename")),
                Msg::BatchRenameStart,
            ));
        }
        // Renombrar por contenido con IA (marca o cursor).
        if model.cur().selected_node().is_some() || !model.cur_pane().marked.is_empty() {
            acciones.push((
                ContextMenuItem::action(rimay_localize::t("nahual-shell-ai-rename-ctx")),
                Msg::AiRename,
            ));
        }
        acciones.push((ContextMenuItem::action(rimay_localize::t("nahual-shell-add-favorite-ctx")), Msg::AddPlace));
        if model.dual {
            acciones.push((ContextMenuItem::action(rimay_localize::t("nahual-shell-copy-other")), Msg::CopyToOther));
            acciones.push((ContextMenuItem::action(rimay_localize::t("nahual-shell-move-other")), Msg::MoveToOther));
        }
    }
    // Edición del grafo de Mónadas (sólo con un grafo nouser montado): las
    // mismas ops que la command palette, contextuales a la selección.
    if model.cur().monad_graph().is_some() {
        let cursor_es_monada =
            model.cur().selected_node().is_some_and(|n| n.id.starts_with("m:"));
        let dentro_monada = model.cur().current_id().starts_with("m:");
        let hay_sel =
            model.cur().selected_node().is_some() || !model.cur_pane().marked.is_empty();
        if dentro_monada && hay_sel {
            acciones.push((
                ContextMenuItem::action(rimay_localize::t("nahual-shell-submonadize")),
                Msg::SubmonadizePrompt,
            ));
        }
        if cursor_es_monada {
            acciones.push((
                ContextMenuItem::action(rimay_localize::t("nahual-shell-rename-monad")),
                Msg::RenameMonadPrompt,
            ));
            // Fusionar: requiere otras Mónadas marcadas para traer adentro.
            if model.cur_pane().marked.iter().any(|id| id.starts_with("m:")) {
                acciones.push((
                    ContextMenuItem::action(rimay_localize::t("nahual-shell-merge-monads")),
                    Msg::MergeMonads,
                ));
            }
            acciones.push((
                ContextMenuItem::action(rimay_localize::t("nahual-shell-delete-monad")),
                Msg::DeleteMonad,
            ));
        }
    }
    if montado {
        acciones.push((ContextMenuItem::action(rimay_localize::t("nahual-shell-unmount")), Msg::Unmount));
    } else {
        acciones.push((
            ContextMenuItem::action(rimay_localize::t("nahual-shell-mount-nouser")),
            Msg::MountNouser,
        ));
        acciones.push((
            ContextMenuItem::action(rimay_localize::t("nahual-shell-mount-minga")),
            Msg::MountMinga,
        ));
        acciones.push((
            ContextMenuItem::action(rimay_localize::t("nahual-shell-mount-dispositivos")),
            Msg::MountDispositivos,
        ));
    }
    // Open-with (AppBus): si la selección es un archivo, ofrecé abrirlo con
    // cada app de la suite que declara su mime, más "editar" y "terminal".
    if model.ctx_target.is_some() {
        for (id, label) in &model.ctx_open_with {
            acciones.push((
                ContextMenuItem::action(rimay_localize::t_args(
                    "nahual-shell-open-with",
                    &[("app", label.clone().into())],
                )),
                Msg::OpenWith(id.clone()),
            ));
        }
        acciones.push((ContextMenuItem::action(rimay_localize::t("nahual-shell-ai-ask-ctx")), Msg::AiAsk));
        acciones.push((ContextMenuItem::action(rimay_localize::t("nahual-shell-edit-in-nada")), Msg::EditSelected));
        acciones.push((
            ContextMenuItem::action(rimay_localize::t("nahual-shell-terminal-here")),
            Msg::TerminalHere,
        ));
    }
    let msgs: Vec<Msg> = acciones.iter().map(|(_, m)| m.clone()).collect();
    let items: Vec<ContextMenuItem> = acciones.into_iter().map(|(it, _)| it).collect();
    let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> =
        Arc::new(move |i: usize| msgs.get(i).cloned().unwrap_or(Msg::CloseMenus));
    ContextMenuSpec {
        anchor: (x, y),
        viewport: viewport_of(model),
        header: Some(etiqueta_seleccion(model)),
        items,
        active: usize::MAX,
        on_pick,
        on_dismiss: Msg::CloseMenus,
        palette: ContextMenuPalette::from_theme(&model.theme),
    }
}

/// Rect de padding uniforme — atajo para los modales/panel de la cola.
pub(crate) fn pad(v: f32) -> Rect<llimphi_ui::llimphi_layout::taffy::LengthPercentage> {
    Rect { left: length(v), right: length(v), top: length(v), bottom: length(v) }
}

/// Rect de padding sólo horizontal (top/bottom 0).
pub(crate) fn pad_h(v: f32) -> Rect<llimphi_ui::llimphi_layout::taffy::LengthPercentage> {
    Rect { left: length(v), right: length(v), top: length(0.0), bottom: length(0.0) }
}

/// Una fila de alto fijo, ancho total, contenido centrado verticalmente.
pub(crate) fn fila(h: f32) -> Style {
    Style {
        size: Size { width: percent(1.0_f32), height: length(h) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    }
}

/// Envuelve una `card` en un scrim full-screen centrado; un click fuera
/// dispatcha `dismiss`.
pub(crate) fn modal_scrim(card: View<Msg>, dismiss: Msg) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::from_rgba8(0, 0, 0, 130))
    .on_click(dismiss)
    .children(vec![card])
}

/// Overlay del prompt de nombre (nueva carpeta/archivo, renombrar): card
/// centrada con el título, el texto en edición y los atajos.
pub(crate) fn prompt_overlay(p: &Prompt, theme: &Theme) -> View<Msg> {
    let input = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(34.0) },
        padding: pad(8.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_app)
    .radius(6.0)
    .border(1.0, theme.fg_muted)
    .text(format!("{}_", p.text), 15.0, theme.fg_text);

    let card = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(440.0_f32), height: length(160.0_f32) },
        padding: pad(18.0),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(10.0)
    .border(1.0, theme.accent)
    .children(vec![
        View::new(fila(30.0)).text(p.title(), 16.0, theme.fg_text),
        input,
        View::new(fila(26.0)).text(rimay_localize::t("nahual-shell-prompt-hint"), 12.0, theme.fg_muted),
    ]);
    modal_scrim(card, Msg::PromptCancel)
}

/// Overlay de confirmación de borrado: lista los nombres a borrar y botones
/// Borrar / Cancelar. El click en el scrim cancela.
pub(crate) fn confirm_overlay(targets: &[(nahual_source_core::NodeId, String)], theme: &Theme) -> View<Msg> {
    let nombres: Vec<&str> = targets.iter().map(|(_, n)| n.as_str()).collect();
    let resumen = if nombres.len() == 1 {
        rimay_localize::t_args("nahual-shell-confirm-one", &[("name", nombres[0].to_string().into())])
    } else {
        rimay_localize::t_args("nahual-shell-confirm-many", &[("n", nombres.len().to_string().into())])
    };
    let detalle = {
        let muestra: Vec<&str> = nombres.iter().take(4).copied().collect();
        let mut s = muestra.join(", ");
        if nombres.len() > 4 {
            s.push_str(&format!(", … (+{})", nombres.len() - 4));
        }
        s
    };

    let boton_borrar = View::new(Style {
        size: Size { width: length(120.0_f32), height: length(34.0_f32) },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        margin: Rect { left: length(0.0), right: length(10.0), top: length(0.0), bottom: length(0.0) },
        ..Default::default()
    })
    .fill(theme.fg_destructive)
    .radius(6.0)
    .on_click(Msg::ConfirmDelete)
    .text(rimay_localize::t("nahual-shell-delete-btn"), 14.0, theme.bg_app);

    let boton_cancelar = View::new(Style {
        size: Size { width: length(120.0_f32), height: length(34.0_f32) },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_app)
    .radius(6.0)
    .border(1.0, theme.fg_muted)
    .on_click(Msg::CancelConfirm)
    .text(rimay_localize::t("nahual-shell-cancel-btn"), 14.0, theme.fg_text);

    let botones = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(40.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![boton_borrar, boton_cancelar]);

    let card = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(460.0_f32), height: length(180.0_f32) },
        padding: pad(18.0),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(10.0)
    .border(1.0, theme.fg_destructive)
    .children(vec![
        View::new(fila(32.0)).text(resumen, 16.0, theme.fg_text),
        View::new(fila(40.0)).text(detalle, 12.0, theme.fg_muted),
        botones,
    ]);
    modal_scrim(card, Msg::CancelConfirm)
}

/// Overlay del **renombrado por lote** (Fase 4.5): patrón en edición + tabla de
/// previsualización `viejo → nuevo`. Las colisiones (dos objetivos al mismo
/// nombre nuevo) se tiñen en rojo para avisar antes de aplicar.
pub(crate) fn batch_overlay(b: &BatchRename, theme: &Theme) -> View<Msg> {
    let total = b.targets.len();
    // Pre-calcula los nuevos nombres y cuenta colisiones entre ellos.
    let nuevos: Vec<String> = (0..total).map(|i| b.nuevo_nombre(i)).collect();
    let mut conteo: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for nn in &nuevos {
        *conteo.entry(nn.as_str()).or_insert(0) += 1;
    }

    // En modo IA el "input" es informativo (los nombres ya vienen propuestos);
    // en modo patrón es el patrón en edición con cursor.
    let input_txt = if b.es_ia() {
        rimay_localize::t("nahual-shell-batch-ai-input")
    } else {
        format!("{}_", b.pattern)
    };
    let input = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(34.0_f32) },
        padding: pad(8.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_app)
    .radius(6.0)
    .border(1.0, theme.accent)
    .text(input_txt, 15.0, if b.es_ia() { theme.fg_muted } else { theme.fg_text });

    // Filas de preview (hasta 12 visibles).
    let filas: Vec<View<Msg>> = (0..total)
        .take(12)
        .map(|i| {
            let original = &b.targets[i].1;
            let nuevo = &nuevos[i];
            let colision = conteo.get(nuevo.as_str()).copied().unwrap_or(0) > 1;
            let color = if colision {
                theme.fg_destructive
            } else if nuevo == original {
                theme.fg_muted
            } else {
                theme.fg_text
            };
            let marca = if colision { "⚠ " } else { "" };
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(22.0_f32) },
                padding: pad_h(4.0),
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text(format!("{marca}{original}  →  {nuevo}"), 13.0, color)
        })
        .collect();
    let oculto = total.saturating_sub(12);
    let mut hijos_lista = filas;
    if oculto > 0 {
        hijos_lista.push(
            View::new(fila(20.0)).text(
                rimay_localize::t_args("nahual-shell-more", &[("n", oculto.to_string().into())]),
                12.0,
                theme.fg_muted,
            ),
        );
    }
    let lista = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(300.0_f32) },
        padding: pad(8.0),
        ..Default::default()
    })
    .fill(theme.bg_app)
    .radius(6.0)
    .children(hijos_lista);

    let card = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(640.0_f32), height: length(470.0_f32) },
        padding: pad(18.0),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(10.0)
    .border(1.0, theme.accent)
    .children(vec![
        View::new(fila(30.0)).text(
            if b.es_ia() {
                rimay_localize::t_args("nahual-shell-batch-ai-title", &[("n", total.to_string().into())])
            } else {
                rimay_localize::t_args("nahual-shell-batch-title", &[("n", total.to_string().into())])
            },
            16.0,
            theme.fg_text,
        ),
        View::new(fila(22.0)).text(
            if b.es_ia() {
                rimay_localize::t("nahual-shell-batch-ai-sub")
            } else {
                rimay_localize::t("nahual-shell-batch-sub")
            },
            12.0,
            theme.fg_muted,
        ),
        input,
        View::new(fila(24.0)).text(rimay_localize::t("nahual-shell-preview"), 13.0, theme.fg_muted),
        lista,
        View::new(fila(26.0)).text(rimay_localize::t("nahual-shell-batch-hint"), 12.0, theme.fg_muted),
    ]);
    modal_scrim(card, Msg::BatchCancel)
}
