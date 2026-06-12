//! Modales: gestores de containers, hosts y disposiciones.

use super::super::*;
use super::widgets::*;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, Dimension, Style};
use llimphi_ui::llimphi_layout::taffy::{AlignItems, FlexDirection, JustifyContent, Rect, Size};
use llimphi_ui::View;
use llimphi_theme::Theme;
use llimphi_widget_text_input::{text_input_view, TextInputPalette};

// ─── Containers modal ───────────────────────────────────────────────

/// Diálogo bloqueante de containers.
pub(crate) fn containers_modal(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_widget_modal::{modal_view, ModalButton, ModalPalette, ModalSpec};
    modal_view(ModalSpec {
        title: "Containers".to_string(),
        body: containers_modal_body(model, theme),
        buttons: vec![ModalButton::cancel("Listo", Msg::CloseContainersModal)],
        size: (560.0, 600.0),
        viewport: model.viewport,
        on_dismiss: Msg::Noop,
        palette: ModalPalette::from_theme(theme),
    })
}

fn containers_modal_body(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;

    if let Some((host, user, port, engine)) = model.active_remote_target() {
        return remote_containers_body(model, &host, &user, port, &engine, theme);
    }

    let sub = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
        ..Default::default()
    })
    .text_aligned(
        "Elegí uno de la lista para editarlo, o «Nuevo». Los mounts se aplican al correr."
            .to_string(),
        11.0,
        theme.fg_muted,
        Alignment::Start,
    );

    let nuevo_btn = action_button_small("+ Nuevo", Msg::ContainerDraftNew, theme);
    let editor: Option<View<Msg>> =
        model.container_draft.as_ref().map(|d| container_draft_form(d, theme));
    let refresh = action_button_small("⟳ Refrescar lista", Msg::RefreshContainersFull, theme);

    let editing_name: Option<&str> = model
        .container_draft
        .as_ref()
        .and_then(|d| d.editing.as_deref());

    let mut rows: Vec<View<Msg>> = Vec::new();
    if !model.containers_full.is_empty() {
        rows.push(panel_label("Existentes", theme));
        for (i, c) in model.containers_full.iter().enumerate() {
            let selected = editing_name == Some(c.name.as_str());
            rows.push(container_row(i, c, selected, theme));
        }
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        gap: Size { width: length(0.0_f32), height: length(6.0_f32) },
        ..Default::default()
    })
    .children({
        let mut all = vec![sub, nuevo_btn];
        if let Some(ed) = editor {
            all.push(ed);
        }
        all.push(refresh);
        all.extend(rows);
        all
    })
}

/// Cuerpo del gestor cuando la sesión activa es **remota**.
fn remote_containers_body(
    model: &Model,
    host: &str,
    user: &str,
    port: u16,
    engine: &str,
    theme: &Theme,
) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;

    let sub = panel_note(
        &format!("Gestionando {user}@{host}:{port} ({engine}) por SSH."),
        theme,
    );

    let mut distro_btns: Vec<View<Msg>> = Vec::new();
    for d in Distro::ALL {
        let active = model.remote_new_distro == d;
        distro_btns.push(
            View::new(Style {
                flex_grow: 1.0,
                size: Size { width: Dimension::auto(), height: length(28.0_f32) },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .fill(if active { theme.accent } else { theme.bg_button })
            .hover_fill(if active { theme.accent } else { theme.bg_button_hover })
            .radius(4.0)
            .text_aligned(
                d.label().to_string(),
                11.0,
                if active { theme.bg_app } else { theme.fg_muted },
                Alignment::Center,
            )
            .on_click(Msg::SetRemoteNewDistro(d)),
        );
    }
    let distros = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(distro_btns);

    let crear = action_button_small("+ Crear en el host", Msg::CreateRemoteContainer, theme);
    let refresh = action_button_small("⟳ Refrescar lista", Msg::RefreshRemoteContainers, theme);

    let mut rows: Vec<View<Msg>> = Vec::new();
    if model.remote_containers.is_empty() {
        rows.push(panel_note(
            "Sin contenedores en el host (o no respondió aún). Refrescá.",
            theme,
        ));
    } else {
        rows.push(panel_label("En el host remoto", theme));
        for name in &model.remote_containers {
            rows.push(remote_container_row(name, theme));
        }
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        gap: Size { width: length(0.0_f32), height: length(6.0_f32) },
        ..Default::default()
    })
    .children({
        let mut all =
            vec![sub, panel_label("Crear nuevo", theme), distros, crear, refresh];
        all.extend(rows);
        all
    })
}

/// Una fila del gestor remoto: nombre + ▶ start · ■ stop · 🗑 rm.
fn remote_container_row(name: &str, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    let display = View::new(Style {
        size: Size { width: Dimension::auto(), height: length(18.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(name.to_string(), 12.0, theme.fg_text, Alignment::Start);
    let start = action_button_small("▶", Msg::RemoteStart(name.to_string()), theme);
    let stop = action_button_small("■", Msg::RemoteStop(name.to_string()), theme);
    let rm = action_button_small("🗑", Msg::RemoteRemove(name.to_string()), theme);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .radius(4.0)
    .hover_fill(theme.bg_row_hover)
    .children(vec![display, start, stop, rm])
}

/// Editor de contenedor: engine + distro + directorios montados.
fn container_draft_form(d: &ContainerDraft, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    let tpal = TextInputPalette::from_theme(theme);
    let editing = d.editing.is_some();

    let mk_radio = |label: String, active: bool, msg: Msg| {
        let v = View::new(Style {
            flex_grow: 1.0,
            size: Size { width: Dimension::auto(), height: length(28.0_f32) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(if active { theme.accent } else { theme.bg_button })
        .radius(4.0)
        .text_aligned(
            label,
            11.0,
            if active { theme.bg_app } else { theme.fg_muted },
            Alignment::Center,
        );
        if editing {
            v
        } else {
            v.hover_fill(if active { theme.accent } else { theme.bg_button_hover })
                .on_click(msg)
        }
    };

    let mut engine_btns: Vec<View<Msg>> = Vec::new();
    for (avail, name) in [
        (unshare_disponible(), "unshare"),
        (bwrap_disponible(), "bwrap"),
        (podman_disponible(), "podman"),
    ] {
        if avail && (!editing || d.engine == name) {
            engine_btns.push(mk_radio(
                name.to_string(),
                d.engine == name,
                Msg::ContainerDraftSetEngine(name.to_string()),
            ));
        }
    }
    if engine_btns.is_empty() {
        engine_btns.push(
            View::new(Style {
                flex_grow: 1.0,
                size: Size { width: Dimension::auto(), height: length(28.0_f32) },
                ..Default::default()
            })
            .text_aligned("—".to_string(), 11.0, theme.fg_muted, Alignment::Center),
        );
    }
    let engine_row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(engine_btns);

    let distro_row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(
        [Distro::Ubuntu, Distro::Debian, Distro::Alpine, Distro::Arch]
            .into_iter()
            .filter(|dd| !editing || d.distro == *dd)
            .map(|dd| {
                mk_radio(
                    dd.label().to_string(),
                    d.distro == dd,
                    Msg::ContainerDraftSetDistro(dd),
                )
            })
            .collect::<Vec<_>>(),
    );

    // Filas de mount.
    let mut mount_rows: Vec<View<Msg>> = Vec::new();
    for (i, md) in d.mounts.iter().enumerate() {
        let host_in = text_input_view(
            &md.host,
            "/home/usuario/proyecto",
            d.focus == Some((i, MountCol::Host)),
            &tpal,
            Msg::ContainerDraftFocusMount(i, MountCol::Host),
        );
        let arrow = View::new(Style {
            size: Size { width: length(16.0_f32), height: length(28.0_f32) },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .text_aligned("→".to_string(), 12.0, theme.fg_muted, Alignment::Center);
        let tgt_in = text_input_view(
            &md.target,
            "/work",
            d.focus == Some((i, MountCol::Target)),
            &tpal,
            Msg::ContainerDraftFocusMount(i, MountCol::Target),
        );
        let ro_label = if md.readonly { "ro" } else { "rw" };
        let ro_btn = View::new(Style {
            size: Size { width: length(34.0_f32), height: length(28.0_f32) },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(if md.readonly { theme.bg_button } else { theme.accent })
        .hover_fill(theme.bg_button_hover)
        .radius(4.0)
        .text_aligned(
            ro_label.to_string(),
            11.0,
            if md.readonly { theme.fg_text } else { theme.bg_app },
            Alignment::Center,
        )
        .on_click(Msg::ContainerDraftToggleMountRo(i));
        let rm_btn = action_button_small("🗑", Msg::ContainerDraftRemoveMount(i), theme);
        mount_rows.push(
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
                align_items: Some(AlignItems::Center),
                gap: Size { width: length(5.0_f32), height: length(0.0_f32) },
                ..Default::default()
            })
            .children(vec![host_in, arrow, tgt_in, ro_btn, rm_btn]),
        );
    }
    let add_mount = action_button_small("+ agregar directorio", Msg::ContainerDraftAddMount, theme);

    let save_label = if editing { "Guardar (Enter)" } else { "Crear (Enter)" };
    let buttons = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(36.0_f32) },
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(10.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![
        action_button_small(save_label, Msg::ContainerDraftSave, theme),
        action_button_small("Cancelar (Esc)", Msg::ContainerDraftCancel, theme),
    ]);

    let titulo = panel_label(
        if editing { "Editar contenedor" } else { "Nuevo contenedor" },
        theme,
    );
    let host_lbl = if d.host == "local" {
        "Host: Local".to_string()
    } else {
        format!("Host: {}", d.host)
    };
    let mut children = vec![
        titulo,
        panel_note(&host_lbl, theme),
        panel_label("Engine", theme),
        engine_row,
        panel_label("Distro", theme),
        distro_row,
        panel_label("Directorios montados (host → destino)", theme),
    ];
    children.extend(mount_rows);
    children.push(add_mount);
    children.push(buttons);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(12.0_f32),
            bottom: length(12.0_f32),
        },
        gap: Size { width: length(0.0_f32), height: length(6.0_f32) },
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(6.0_f32),
            bottom: length(6.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(6.0)
    .children(children)
}

fn container_row(idx: usize, c: &ContainerInfo, selected: bool, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    let running = c.status.starts_with("Up");
    let name_view = View::new(Style {
        size: Size { width: length(180.0_f32), height: length(18.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(c.name.clone(), 12.0, theme.fg_text, Alignment::Start);
    let status_view = View::new(Style {
        size: Size { width: Dimension::auto(), height: length(18.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(
        format!("{} · {}", c.status, c.image),
        11.0,
        if running { theme.accent } else { theme.fg_muted },
        Alignment::Start,
    );
    let mut children = vec![name_view, status_view];
    if c.rootfs {
        children.push(action_button_small("🗑", Msg::RemoveRootfs(c.name.clone()), theme));
    } else {
        children.push(action_button_small("▶", Msg::StartContainer(c.name.clone()), theme));
        children.push(action_button_small("■", Msg::StopContainer(c.name.clone()), theme));
        children.push(action_button_small("🗑", Msg::RemoveContainer(c.name.clone()), theme));
    }
    let mut row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(32.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .radius(4.0)
    .hover_fill(theme.bg_row_hover)
    .children(children);
    if selected {
        row = row.fill(theme.bg_panel_alt);
    }
    if c.rootfs {
        row = row.on_click(Msg::ContainerEdit(idx));
    }
    row
}

// ─── Hosts modal ────────────────────────────────────────────────────

/// Diálogo bloqueante de hosts remotos.
pub(crate) fn hosts_modal(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_widget_modal::{modal_view, ModalButton, ModalPalette, ModalSpec};
    modal_view(ModalSpec {
        title: "Hosts remotos".to_string(),
        body: hosts_modal_body(model, theme),
        buttons: vec![ModalButton::cancel("Listo", Msg::CloseHostsModal)],
        size: (520.0, 560.0),
        viewport: model.viewport,
        on_dismiss: Msg::Noop,
        palette: ModalPalette::from_theme(theme),
    })
}

fn hosts_modal_body(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;

    let sub = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
        ..Default::default()
    })
    .text_aligned(
        "Se guardan en ~/.config/shuma/hosts.json.".to_string(),
        11.0,
        theme.fg_muted,
        Alignment::Start,
    );

    let nuevo_btn = action_button_small("+ Nuevo", Msg::HostDraftStart, theme);
    let editor: Option<View<Msg>> =
        model.host_draft.as_ref().map(|d| host_draft_form(d, theme));
    let editing_name: Option<&str> = model
        .host_draft
        .as_ref()
        .and_then(|d| d.editing.as_deref());

    let mut rows: Vec<View<Msg>> = Vec::new();
    if !model.hosts.is_empty() {
        rows.push(panel_label("Guardados", theme));
        for (i, h) in model.hosts.iter().enumerate() {
            let selected = editing_name == Some(h.name.as_str());
            rows.push(host_row(i, h, selected, theme));
        }
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        gap: Size { width: length(0.0_f32), height: length(8.0_f32) },
        ..Default::default()
    })
    .children({
        let mut all = vec![sub, nuevo_btn];
        if let Some(ed) = editor {
            all.push(ed);
        }
        all.extend(rows);
        all
    })
}

fn host_row(idx: usize, h: &hosts::RemoteHost, selected: bool, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    let display = View::new(Style {
        size: Size { width: Dimension::auto(), height: length(18.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(
        format!("{} · {}", h.display(), h.auth.label()),
        12.0,
        theme.fg_text,
        Alignment::Start,
    );
    let rm_btn = action_button_small("🗑", Msg::HostDelete(idx), theme);
    let mut row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .radius(4.0)
    .hover_fill(theme.bg_row_hover)
    .children(vec![display, rm_btn])
    .on_click(Msg::HostEdit(idx));
    if selected {
        row = row.fill(theme.bg_panel_alt);
    }
    row
}

fn host_draft_form(d: &HostDraft, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    let tpal = TextInputPalette::from_theme(theme);
    let mut rows: Vec<View<Msg>> = Vec::new();
    rows.push(panel_label(
        if d.editing.is_some() { "Editar host" } else { "Nuevo host" },
        theme,
    ));
    rows.push(panel_label("Nombre", theme));
    rows.push(text_input_view(
        &d.name,
        "ejemplo",
        d.focused == Some(HostDraftField::Name),
        &tpal,
        Msg::HostDraftFocus(HostDraftField::Name),
    ));
    rows.push(panel_label("Host", theme));
    rows.push(text_input_view(
        &d.host,
        "1.2.3.4 o ejemplo.com",
        d.focused == Some(HostDraftField::Host),
        &tpal,
        Msg::HostDraftFocus(HostDraftField::Host),
    ));
    rows.push(panel_label("Usuario", theme));
    rows.push(text_input_view(
        &d.user,
        "root",
        d.focused == Some(HostDraftField::User),
        &tpal,
        Msg::HostDraftFocus(HostDraftField::User),
    ));
    rows.push(panel_label("Puerto", theme));
    rows.push(text_input_view(
        &d.port,
        "22",
        d.focused == Some(HostDraftField::Port),
        &tpal,
        Msg::HostDraftFocus(HostDraftField::Port),
    ));
    let auth_label = if d.use_password {
        "Contraseña (askpass al conectar)"
    } else {
        "Clave PEM"
    };
    rows.push(
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
            align_items: Some(AlignItems::Center),
            padding: Rect {
                left: length(4.0_f32),
                right: length(8.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .hover_fill(theme.bg_row_hover)
        .on_click(Msg::HostDraftToggleAuth)
        .text_aligned(
            format!("· Auth: {auth_label} (click cambia)"),
            11.0,
            theme.fg_text,
            Alignment::Start,
        ),
    );
    if !d.use_password {
        rows.push(panel_label("Path PEM", theme));
        rows.push(text_input_view(
            &d.pem_path,
            "/home/usuario/.ssh/id_rsa",
            d.focused == Some(HostDraftField::Pem),
            &tpal,
            Msg::HostDraftFocus(HostDraftField::Pem),
        ));
    }
    let save_label = if d.editing.is_some() { "Guardar (Enter)" } else { "Crear (Enter)" };
    let save = action_button_small(save_label, Msg::HostDraftSave, theme);
    let cancel = action_button_small("Cancelar (Esc)", Msg::HostDraftCancel, theme);
    let buttons = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(36.0_f32) },
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        align_items: Some(AlignItems::Center),
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(10.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![save, cancel]);
    rows.push(buttons);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(12.0_f32),
            bottom: length(12.0_f32),
        },
        gap: Size { width: length(0.0_f32), height: length(4.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(6.0)
    .children(rows)
}

// ─── Layouts modal ──────────────────────────────────────────────────

/// Diálogo bloqueante de disposiciones estilo tmux.
pub(crate) fn layouts_modal(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_widget_modal::{modal_view, ModalButton, ModalPalette, ModalSpec};
    modal_view(ModalSpec {
        title: "Disposiciones".to_string(),
        body: layouts_modal_body(model, theme),
        buttons: vec![ModalButton::cancel("Listo", Msg::CloseLayoutsModal)],
        size: (520.0, 520.0),
        viewport: model.viewport,
        on_dismiss: Msg::Noop,
        palette: ModalPalette::from_theme(theme),
    })
}

fn layouts_modal_body(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    let tpal = TextInputPalette::from_theme(theme);

    let sub = View::new(Style {
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        ..Default::default()
    })
    .text_aligned(
        "Una disposición guarda tus sesiones y la geometría de los paneles. Se guardan en ~/.config/shuma/layouts.json.".to_string(),
        11.0,
        theme.fg_muted,
        Alignment::Start,
    );

    let name_input = text_input_view(
        &model.layout_name,
        "nombre de la disposición",
        model.layout_name_focused,
        &tpal,
        Msg::LayoutNameFocus,
    );
    let save_btn = action_button_small("Guardar disposición actual", Msg::SaveLayout, theme);

    let mut rows: Vec<View<Msg>> = Vec::new();
    if !model.layouts.is_empty() {
        rows.push(panel_label("Guardadas", theme));
        for (i, l) in model.layouts.iter().enumerate() {
            rows.push(layout_row(i, &l.name, l.sessions.len(), theme));
        }
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        gap: Size { width: length(0.0_f32), height: length(8.0_f32) },
        ..Default::default()
    })
    .children({
        let mut all =
            vec![sub, panel_label("Guardar la actual", theme), name_input, save_btn];
        all.extend(rows);
        all
    })
}

fn layout_row(idx: usize, name: &str, n_sessions: usize, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    let plural = if n_sessions == 1 { "sesión" } else { "sesiones" };
    let display = View::new(Style {
        size: Size { width: Dimension::auto(), height: length(18.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(
        format!("{name} · {n_sessions} {plural}"),
        12.0,
        theme.fg_text,
        Alignment::Start,
    );
    let restore_btn = action_button_small("Restaurar", Msg::RestoreLayout(idx), theme);
    let rm_btn = action_button_small("🗑", Msg::DeleteLayout(idx), theme);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .radius(4.0)
    .hover_fill(theme.bg_row_hover)
    .children(vec![display, restore_btn, rm_btn])
}
