use super::*;

pub(crate) fn build_banners(model: &Model) -> Vec<View<Msg>> {
    let mut out: Vec<View<Msg>> = Vec::new();
    if let Some(t) = &model.toast {
        out.push(
            banner_view::<Msg>(t.kind, t.text.clone()).on_click(Msg::DismissToast),
        );
    }
    if let Some(msg) = &model.initial_toast {
        out.push(banner_view::<Msg>(BannerKind::Info, msg.clone()));
    }
    if let Some(msg) = &model.load_error {
        out.push(banner_view::<Msg>(BannerKind::Error, msg.clone()));
    }
    out
}

pub(crate) fn build_sidebar(model: &Model, theme: &Theme) -> View<Msg> {
    let palette = ListPalette::from_theme(theme);

    // Sección 1: lista de módulos. Cada módulo lleva un ícono VECTORIAL derivado
    // determinísticamente de su `id` (identicon de tullpu-icon) — color y forma
    // estables por módulo, en toda máquina, sin glifos de fuente.
    let module_rows: Vec<IconRow<Msg>> = model
        .modules
        .iter()
        .enumerate()
        .map(|(i, m)| IconRow {
            icon: Some(tullpu_icon_llimphi::spec_view(
                tullpu_icon_core::derivar_spec(&m.id),
                theme.fg_text,
            )),
            label: m.label.clone(),
            selected: model.selected_module == Some(i),
            on_click: Msg::SelectModule(i),
        })
        .collect();

    let modules_panel = icon_list_view(IconListSpec {
        rows: module_rows,
        total: model.modules.len(),
        caption: Some(rimay_localize::t_args(
            "nakui-sidebar-modules",
            &[("count", model.modules.len().to_string().into())],
        )),
        truncated_hint: None,
        row_height: ROW_HEIGHT,
        palette,
    });

    // Sección 2: menú del módulo activo.
    let menu_panel = match model.selected_module {
        Some(mod_idx) => {
            let m = &model.modules[mod_idx];
            let rows: Vec<ListRow<Msg>> = m
                .menu
                .iter()
                .enumerate()
                .map(|(i, item)| ListRow {
                    label: match &item.icon {
                        Some(ic) => format!("{ic}  {}", item.label),
                        None => item.label.clone(),
                    },
                    selected: model.selected_menu == Some(i),
                    on_click: Msg::SelectMenu(i),
                })
                .collect();
            list_view(ListSpec {
                rows,
                total: m.menu.len(),
                caption: Some(rimay_localize::t("nakui-sidebar-menu")),
                truncated_hint: None,
                row_height: ROW_HEIGHT,
                palette,
            })
        }
        None => empty_panel(theme, &rimay_localize::t("nakui-empty-no-modules")),
    };

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(vec![modules_panel, menu_panel])
}

/// Hash estable de una cadena → `key` de animación: la misma escena
/// produce siempre la misma key entre rebuilds, escenas distintas keys
/// distintas.
fn key_of(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// `key` estable de la escena ERP activa (form / ficha de detalle / vista
/// del menú). Cambia sólo al conmutar de modo —dispara la transición de
/// entrada del cuerpo— y permanece estable mientras se opera dentro de la
/// misma escena (tipear, paginar, ordenar) para no re-animar en cada frame.
fn erp_scene_key(model: &Model) -> u64 {
    if let Some(form) = &model.form {
        match form.editing {
            Some(id) => key_of(&format!("form:edit:{}:{id}", form.entity)),
            None => key_of(&format!("form:new:{}", form.entity)),
        }
    } else if let Some(d) = &model.detail {
        key_of(&format!("detail:{}:{}", d.entity, d.id))
    } else {
        match (model.selected_module, model.selected_menu) {
            (Some(mi), Some(mj)) => key_of(&format!("view:{mi}:{mj}")),
            _ => key_of("empty"),
        }
    }
}

pub(crate) fn build_main(model: &Model, theme: &Theme) -> View<Msg> {
    // Prioridad del área principal: form > ficha de detalle > vista
    // seleccionada en el menú.
    let inner = if let Some(form) = &model.form {
        build_form_panel(model, form, theme)
    } else if let Some(detail) = &model.detail {
        build_detail_panel(model, detail, theme)
    } else {
        match (model.selected_module, model.selected_menu) {
            (Some(mod_idx), Some(menu_idx)) => {
                let m = &model.modules[mod_idx];
                let item = &m.menu[menu_idx];
                match m.views.get(&item.view) {
                    Some(view) => build_view_panel(model, mod_idx, &item.view, view, theme),
                    None => empty_panel(
                        theme,
                        &format!("vista '{}' no existe en el manifest del módulo", item.view),
                    ),
                }
            }
            (Some(_), None) => empty_panel(theme, &rimay_localize::t("nakui-empty-pick-menu")),
            _ => empty_panel(theme, &rimay_localize::t("nakui-empty-pick-module")),
        }
    };

    // Transición de escena: al conmutar entre List / Form / Detail /
    // Dashboard la `scene_key` cambia y el cuerpo entra con fade + un
    // breve slide-up en vez de saltar de golpe. Estable dentro de la misma
    // escena → no se re-anima al editar/paginar.
    let inner = inner.animated_enter_from(
        erp_scene_key(model),
        llimphi_theme::motion::SLOW,
        Affine::translate((0.0, 24.0)),
    );

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(16.0_f32),
            right: length(16.0_f32),
            top: length(12.0_f32),
            bottom: length(12.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(8.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![inner])
}

/// Clave del Form view dentro del módulo (para `Msg::OpenForm`).
pub(crate) fn form_view_key(module: &Module, fv: &FormView) -> String {
    module
        .views
        .iter()
        .find_map(|(k, v)| match v {
            ModuleView::Form(f) if f.entity == fv.entity && f.title == fv.title => {
                Some(k.clone())
            }
            _ => None,
        })
        .unwrap_or_default()
}
