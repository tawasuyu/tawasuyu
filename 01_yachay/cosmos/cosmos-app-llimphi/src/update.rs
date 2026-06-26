//! Helpers de transición del modelo y cuerpo de `App::update`.
//!
//! Las funciones de este módulo son llamadas tanto desde `update`
//! como desde `nav_ops`, `dialog_ops` y `rectify_ops`.

use std::sync::Arc;

use crate::astroview::compute_astro;
use crate::chrome::MenuCmd;
use crate::dialog_ops::{
    dialog_cal_pick, dialog_cal_view, dialog_confirm, dialog_focus, dialog_pick_city,
    dialog_pick_contact, dialog_set_kind, dialog_time_step, dialog_toggle_calendar,
    dialog_toggle_kind, open_chart_dialog, open_contact_dialog,
};
use crate::model::{MenuKind, Model, Msg, OverlayKind, WheelOpt};
use crate::nav_ops::{
    activate_tab, close_chart_tab, commit_rename, delete_selected, do_duplicar, do_export_group,
    do_guardar, do_import_group, do_nueva, nav_click, new_group, open_add_hoy, open_hoy_chart,
    paste_node, refresh_nav, start_rename,
};
use crate::persist::{load_chart_from_disk, save_chart_to_disk, save_ui_state, UiState};
use crate::rectify_ops::{apply_rectify, compute_triggers, run_rectify};
use crate::{chrome, library, model, tools};

use llimphi_ui::Handle;

/// Abre el diálogo de creación adecuado al nodo seleccionado: sobre la rama
/// «Hoy» agrega una carta del día por coordenadas; en cualquier otro lado,
/// el diálogo normal de «Nueva carta».
fn open_new_chart_or_hoy(m: &mut Model) {
    let on_hoy = m
        .nav_selected
        .as_deref()
        .map(|k| k == library::HOY_CONTACT_KEY || library::is_hoy_chart_key(k))
        .unwrap_or(false);
    if on_hoy {
        open_add_hoy(m);
    } else {
        open_chart_dialog(m);
    }
}

// =====================================================================
// Helpers compartidos de recomputo
// =====================================================================

/// Recomputa el render de TODAS las cartas abiertas (mosaico siempre
/// consistente al cambiar capas/armónico) y refresca `m.render` con el de
/// la pestaña activa. Las cartas abiertas son pocas; el costo es marginal.
pub(crate) fn recompute_chart(m: &mut Model) {
    let off = m.rectify_offset_min;
    if m.open.is_empty() {
        let (render, error) = crate::engine::compute(&m.chart, &m.overlays, m.harmonic, m.cfg.minor_aspects, off);
        m.render = render;
        m.error = error;
        return;
    }
    let overlays = m.overlays.clone();
    let (h, minor) = (m.harmonic, m.cfg.minor_aspects);
    let active = m.active_tab.min(m.open.len() - 1);
    for i in 0..m.open.len() {
        let (render, error) = crate::engine::compute(&m.open[i].chart, &overlays, h, minor, off);
        m.open[i].render = render;
        if i == active {
            m.render = m.open[i].render.clone();
            m.error = error;
        }
    }
}

// El cómputo astronómico es el pesado (144 muestras × 10 cuerpos): NO corre
// en el hilo de UI. Esto sólo marca sucio; el despacho a un worker ocurre al
// final de `update` (que tiene el Handle). El render de la carta sí es barato
// y queda síncrono (ver `recompute_chart`).
pub(crate) fn recompute_astro(m: &mut Model) {
    m.astro_dirty = true;
}

/// Persiste el UI-state completo a disco.
pub(crate) fn save_ui(m: &Model) {
    save_ui_state(&UiState {
        overlays: m.overlays.clone(),
        harmonic: m.harmonic,
        cfg: m.cfg.clone(),
        nav_w: m.nav_w,
        tools_w: m.tools_w,
        nav_open: m.nav_open,
        tools_open: m.tools_open,
        chart_view: m.chart_view,
        tool_cat: m.tool_cat,
        expanded_panels: m.expanded_panels.clone(),
        tile_mode: m.tile_mode,
        dock_left: m.dock_left.clone(),
        dock_right: m.dock_right.clone(),
        sphere_yaw: crate::model::orient_to_yaw_pitch(m.sphere_orient).0,
        sphere_pitch: crate::model::orient_to_yaw_pitch(m.sphere_orient).1,
        sky_nadir: m.sky_nadir,
    });
}

// =====================================================================
// Helpers de transición
// =====================================================================

pub(crate) fn set_harmonic(m: &mut Model, h: u32) {
    if m.harmonic != h {
        m.harmonic = h;
        recompute_chart(m);
    }
}

pub(crate) fn apply_overlay(m: &mut Model, k: OverlayKind) {
    if let Some(idx) = m.overlays.iter().position(|x| *x == k) {
        m.overlays.remove(idx);
    } else {
        m.overlays.push(k);
    }
    recompute_chart(m);
}

pub(crate) fn toggle_wheel(m: &mut Model, opt: WheelOpt) {
    match opt {
        WheelOpt::MinorAspects => {
            m.cfg.minor_aspects = !m.cfg.minor_aspects;
            // Los menores deben calcularse para poder dibujarse.
            recompute_chart(m);
        }
        WheelOpt::CoordLabels => m.cfg.coord_labels = !m.cfg.coord_labels,
        WheelOpt::Dial3d => m.cfg.dial_3d = !m.cfg.dial_3d,
        WheelOpt::AscCross => m.cfg.asc_cross = !m.cfg.asc_cross,
    }
}

/// Aplica una selección del segmented de tema (0 = Oscuro, 1 = Claro,
/// 2 = Impresión) y refleja el `Theme` activo en el modelo.
pub(crate) fn set_theme_mode(m: &mut Model, idx: usize) {
    m.cfg.set_theme_idx(idx);
    m.theme = m.cfg.active_theme();
}

/// Rasteriza la hoja imprimible (rueda + cabecera + aspectos) a un PNG de
/// alta resolución y la abre en el visor de imágenes del SO para imprimir.
pub(crate) fn do_imprimir(m: &mut Model) {
    match crate::print::imprimir_carta(m) {
        Ok(path) => {
            m.status_note = Some(format!("Hoja rasterizada y abierta para imprimir ({})", path.display()));
        }
        Err(e) => m.error = Some(format!("imprimir: {e}")),
    }
}

pub(crate) fn do_recargar(m: &mut Model) {
    if let Some(c) = load_chart_from_disk() {
        m.chart = c;
        recompute_chart(m);
        recompute_astro(m);
        m.status_note = Some("Carta recargada de disco".into());
    }
}

pub(crate) fn do_eliminar(m: &mut Model) {
    delete_selected(m);
}

// =====================================================================
// apply_cmd / apply_nav_act
// =====================================================================

pub(crate) fn apply_cmd(m: &mut Model, cmd: MenuCmd) {
    match cmd {
        MenuCmd::Sep => {}
        MenuCmd::Nueva => do_nueva(m),
        MenuCmd::Guardar => do_guardar(m),
        MenuCmd::Theme(idx) => set_theme_mode(m, idx),
        MenuCmd::Imprimir => do_imprimir(m),
        MenuCmd::Duplicar => do_duplicar(m),
        MenuCmd::Recargar => do_recargar(m),
        MenuCmd::Eliminar => do_eliminar(m),
        MenuCmd::SetChartView(cv) => m.chart_view = cv,
        MenuCmd::GoToolCat(tc) => {
            // Activa la categoría en el sidebar donde vive (o la trae al
            // derecho si no está acoplada en ningún lado).
            let item = model::DockItem::from_tool_cat(tc);
            if m.dock_left.contains(&item) {
                m.active_left = Some(item);
            } else {
                m.dock_move(item, model::DockSide::Right);
            }
            m.tools_open = true;
        }
        MenuCmd::ToggleNav => m.nav_open = !m.nav_open,
        MenuCmd::ToggleTools => m.tools_open = !m.tools_open,
        MenuCmd::Overlay(k) => apply_overlay(m, k),
        MenuCmd::Harmonic(h) => set_harmonic(m, h),
        MenuCmd::AcercaDe => {
            m.status_note =
                Some("cosmos · astronomía + astrología sobre Llimphi (wgpu + vello + taffy)".into())
        }
        MenuCmd::Wheel(opt) => toggle_wheel(m, opt),
        MenuCmd::Deselect => m.selected_body = None,
    }
}

/// Ejecuta una acción del menú contextual del árbol sobre el nodo ya
/// seleccionado (lo dejó `OpenNavCtx`).
pub(crate) fn apply_nav_act(m: &mut Model, act: chrome::NavAct) {
    use chrome::NavAct;
    match act {
        NavAct::NewGroup => new_group(m),
        NavAct::NewContact => open_contact_dialog(m),
        NavAct::NewChart => open_chart_dialog(m),
        NavAct::Rename => {
            if let Some(key) = m.nav_selected.clone() {
                start_rename(m, key);
            }
        }
        NavAct::Cut => {
            m.nav_cut = m.nav_selected.clone();
            if m.nav_cut.is_some() {
                m.status_note = Some("Cortado — elegí un grupo destino y pegá".into());
            }
        }
        NavAct::Paste => paste_node(m),
        NavAct::Duplicate => do_duplicar(m),
        NavAct::Delete => delete_selected(m),
    }
}

// =====================================================================
// Función update del bucle Elm — llamada desde `impl App for Cosmos`
// =====================================================================

/// Refleja en el rail hospedado de pata qué diente tiene cosmos desplegado: el
/// `DockItem` del lado expandido (`dock_expanded`), o `None` si está en puro
/// lienzo. Sólo manda `SetActive` cuando cambia respecto del último reportado
/// (`host_active_synced`), para no escribir el socket en cada `update`. No-op si
/// cosmos no delega (sin `_host`).
fn sync_host_active(m: &mut Model) {
    let active = m
        .dock_expanded
        .and_then(|side| m.dock_active(side))
        .map(|item| item.to_u64() as u32);
    if active == m.host_active_synced {
        return;
    }
    m.host_active_synced = active;
    if let Some(h) = m._host.as_mut() {
        h.set_active(active);
    }
}

/// Re-publica los dientes al rail de pata cuando el dock se reordenó. Los dientes
/// de cosmos SON sus `DockItem`s (izquierda+derecha); al moverlos (`dock_move`)
/// la lista/orden cambia y el `Register` inicial queda viejo. Sólo manda
/// `HostClient::update` cuando la firma del dock cambió respecto de lo último
/// publicado (`host_teeth_synced`). No-op sin `_host` (no delegado).
fn sync_host_teeth(m: &mut Model) {
    let teeth: Vec<pata_host::HostedTooth> = m
        .dock_left
        .iter()
        .chain(&m.dock_right)
        .map(|i| crate::dock_item_tooth(*i))
        .collect();
    let sig: Vec<u32> = teeth.iter().map(|t| t.id).collect();
    if sig == m.host_teeth_synced {
        return;
    }
    m.host_teeth_synced = sig;
    if let Some(h) = m._host.as_mut() {
        h.update(teeth);
    }
}

pub(crate) fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
    let mut m = model;
    let mut persist = false;
    // Cualquier interacción que no sea abrir un menú limpia la nota
    // efímera de estado. El resultado del worker (AstroComputed) tampoco
    // la toca: es un evento de fondo, no una acción del usuario.
    match &msg {
        Msg::OpenMenu(_) | Msg::MenuTick | Msg::WawaConfigChanged(_) | Msg::AstroComputed(..) => {}
        _ => m.status_note = None,
    }
    match msg {
        Msg::WawaConfigChanged(cfg) => {
            // El modo impresión ignora el tinte del SO: la hoja es B/N.
            if !m.cfg.print_mode {
                m.theme = wawa_config_llimphi::theme_from_wawa(&cfg, &m.theme);
            }
            if cfg.lang != rimay_localize::current_locale() {
                let _ = rimay_localize::set_locale(&cfg.lang);
            }
        }
        // multi-carta (tabs del centro)
        Msg::ActivateChartTab(i) => activate_tab(&mut m, i),
        Msg::CloseChartTab(i) => close_chart_tab(&mut m, i),
        Msg::ToggleTileMode => {
            m.tile_mode = !m.tile_mode;
            persist = true;
        }
        Msg::SphereRotate(dx, dy) => {
            // Arcball: el delta de arrastre rota la orientación sobre los ejes
            // FIJOS de cámara (X derecha, Y arriba) → sin gimbal, sin topes, sin
            // cambio de eje. `dx`/`dy` son píxeles; ~0.005 rad/px. Agarre tipo
            // globo: la esfera sigue al cursor. Sin persistir (el drag es denso).
            let k = 0.005_f32;
            use llimphi_3d::glam::{Quat, Vec3};
            let dq = Quat::from_axis_angle(Vec3::Y, dx * k)
                * Quat::from_axis_angle(Vec3::X, dy * k);
            m.sphere_orient = (dq * m.sphere_orient).normalize();
        }
        Msg::SphereReset => {
            m.sphere_orient = crate::model::default_orient();
            persist = true;
        }
        Msg::SphereSetOrient(q) => {
            m.sphere_orient = q.normalize();
            persist = true;
        }
        Msg::WheelPan(dx, dy) => {
            m.wheel_pan.0 += dx;
            m.wheel_pan.1 += dy;
        }
        Msg::WheelZoom(factor) => {
            m.wheel_zoom = (m.wheel_zoom * factor).clamp(0.25, 8.0);
        }
        Msg::WheelResetView => {
            m.wheel_zoom = 1.0;
            m.wheel_pan = (0.0, 0.0);
            m.dial_rot = 0.0;
        }
        Msg::DialRotate(dx) => {
            // 1 px de arrastre ≈ 0.2° del dial; envuelve en [0,90).
            m.dial_rot = (m.dial_rot + dx * 0.2).rem_euclid(90.0);
        }
        Msg::WheelSetView(z, px, py) => {
            m.wheel_zoom = z;
            m.wheel_pan = (px, py);
        }
        Msg::ToggleSkyNadir => {
            m.sky_nadir = !m.sky_nadir;
            persist = true;
        }
        Msg::Resized(w, h) => m.viewport = (w, h),
        Msg::ToolsScroll(delta) => {
            // El panel de herramientas que scrollea es la categoría
            // activa (derecha primero, si no izquierda).
            let cat = m
                .dock_active(model::DockSide::Right)
                .and_then(|i| i.tool_cat())
                .or_else(|| m.dock_active(model::DockSide::Left).and_then(|i| i.tool_cat()));
            let content = cat.map(|c| tools::tools_content_h(c, &m)).unwrap_or(0.0);
            let viewport = tools::tools_viewport_h(&m);
            m.tools_scroll = llimphi_widget_scroll::clamp_offset(
                m.tools_scroll + delta,
                content,
                viewport,
            );
        }
        // navegación
        Msg::ToggleNavNode(key) => m.toggle_nav(key),
        Msg::NavClick(key) => nav_click(&mut m, key),
        Msg::NewGroup => new_group(&mut m),
        Msg::DeleteSelected => delete_selected(&mut m),
        Msg::CutNode => {
            m.nav_cut = m.nav_selected.clone();
            if m.nav_cut.is_some() {
                m.status_note = Some("Cortado — seleccioná un grupo destino y pegá".into());
            }
        }
        Msg::PasteNode => {
            paste_node(&mut m);
            persist = true;
        }
        Msg::RenameStart => {
            if let Some(key) = m.nav_selected.clone() {
                start_rename(&mut m, key);
            }
        }
        Msg::RenameKey(ev) => {
            if m.nav_rename.is_some() {
                m.rename_input.apply_key(&ev);
            }
        }
        Msg::RenameCommit => commit_rename(&mut m),
        Msg::RenameCancel => m.nav_rename = None,
        Msg::ChartFileChanged => {
            if let Some(c) = load_chart_from_disk() {
                m.chart = c.clone();
                // Reflejar la edición externa en la pestaña activa.
                if let Some(t) = m.open.get_mut(m.active_tab) {
                    t.chart = c;
                }
                recompute_chart(&mut m);
                recompute_astro(&mut m);
            }
        }
        Msg::SelectBody(sel) => {
            m.selected_body = if m.selected_body == sel { None } else { sel };
        }
        // capas / armónico / configuración
        Msg::ToggleOverlay(k) => {
            apply_overlay(&mut m, k);
            persist = true;
        }
        Msg::SetHarmonic(n) => {
            set_harmonic(&mut m, n);
            persist = true;
        }
        Msg::SetThemeMode(idx) => {
            set_theme_mode(&mut m, idx);
            persist = true;
        }
        Msg::PrintSheet => do_imprimir(&mut m),
        Msg::ToggleWheelOpt(opt) => {
            toggle_wheel(&mut m, opt);
            persist = true;
        }
        Msg::SetRotOffset(dv) => {
            m.cfg.rot_offset_deg = (m.cfg.rot_offset_deg + dv).rem_euclid(360.0);
            persist = true;
        }
        Msg::SetUseNow(b) => {
            m.cfg.use_now = b;
            recompute_astro(&mut m);
            persist = true;
        }
        // menú principal
        Msg::OpenMenu(k) => {
            m.menu_open = if m.menu_open == Some(k) { None } else { Some(k) };
            m.menu_active = usize::MAX;
            m.ctx_open = None;
            // Animación de aparición/swap: cada vez que se abre (o se
            // cambia de) menú, el dropdown se funde+desliza de nuevo.
            if m.menu_open.is_some() {
                m.menu_anim = llimphi_motion::Tween::new(
                    0.0,
                    1.0,
                    llimphi_motion::motion::FAST,
                    llimphi_motion::motion::ease_out_cubic,
                );
                llimphi_motion::animate(handle, llimphi_motion::motion::FAST, || Msg::MenuTick);
            }
        }
        Msg::MenuPick(kind, idx) => {
            m.menu_open = None;
            m.menu_active = usize::MAX;
            let cmd = chrome::menu_entries(kind, &m).get(idx).map(|e| e.cmd);
            if let Some(cmd) = cmd {
                apply_cmd(&mut m, cmd);
                persist = true;
            }
        }
        Msg::MenuNav(dir) => {
            if let Some(kind) = m.menu_open {
                let entries = chrome::menu_entries(kind, &m);
                let items: Vec<_> = entries.iter().map(chrome::MenuEntry::to_item).collect();
                m.menu_active =
                    llimphi_widget_context_menu::step_active(&items, m.menu_active, dir);
            }
        }
        Msg::MenuActivate => {
            if let Some(kind) = m.menu_open {
                let idx = m.menu_active;
                let cmd = chrome::menu_entries(kind, &m).get(idx).map(|e| e.cmd);
                m.menu_open = None;
                m.menu_active = usize::MAX;
                if let Some(cmd) = cmd {
                    apply_cmd(&mut m, cmd);
                    persist = true;
                }
            }
        }
        Msg::MenuTick => {}
        Msg::CloseMenu => {
            m.menu_open = None;
            m.menu_active = usize::MAX;
        }
        // menú contextual
        Msg::OpenCanvasCtx(x, y) => {
            m.ctx_open = Some((x, y));
            m.menu_open = None;
        }
        Msg::CtxPick(idx) => {
            m.ctx_open = None;
            let cmd = chrome::ctx_entries(&m).get(idx).map(|e| e.cmd);
            if let Some(cmd) = cmd {
                apply_cmd(&mut m, cmd);
                persist = true;
            }
        }
        Msg::CloseCtx => {
            m.ctx_open = None;
            m.nav_ctx = None;
        }
        // menú contextual del árbol de datos
        Msg::OpenNavCtx(key) => {
            m.nav_selected = Some(key.clone());
            m.nav_ctx = Some(key);
            m.ctx_open = None;
            m.menu_open = None;
        }
        Msg::NavCtxPick(idx) => {
            let act = m
                .nav_ctx
                .as_ref()
                .map(|k| chrome::nav_ctx_entries(&m, k))
                .and_then(|entries| entries.get(idx).and_then(|e| e.act));
            m.nav_ctx = None;
            if let Some(act) = act {
                apply_nav_act(&mut m, act);
                persist = true;
            }
        }
        Msg::NavScroll(delta) => {
            let content = chrome::nav_content_h(&m);
            let viewport = chrome::nav_viewport_h(&m);
            m.nav_scroll =
                llimphi_widget_scroll::clamp_offset(m.nav_scroll + delta, content, viewport);
        }
        Msg::PrintScroll(delta) => {
            let content = chrome::print_sheet_h(&m.render);
            let viewport = chrome::print_viewport_h(&m);
            m.print_scroll =
                llimphi_widget_scroll::clamp_offset(m.print_scroll + delta, content, viewport);
        }
        Msg::ImportGroup => do_import_group(&mut m),
        Msg::ExportGroup => do_export_group(&mut m),
        Msg::AddHoyChart => open_add_hoy(&mut m),
        Msg::AnimTick => {
            // ~80 ms por tick. Envolvemos por la duración del loop para que el
            // acumulador no crezca sin techo (precisión f32 estable en sesiones
            // largas). `frame_at_time` ya hace su propio módulo igual.
            m.anim_t += 0.08;
            if let Some(anim) = &m.empty_anim {
                let dur = anim.duration_secs() as f32;
                if dur > 0.0 && m.anim_t > dur {
                    m.anim_t -= dur;
                }
            }
        }
        Msg::HoyTick => {
            // Refresca la carta «Hoy» mostrada al instante actual.
            if let Some(key) = m.hoy_active.clone() {
                let loc = if key == library::HOY_USER_KEY {
                    m.cfg.user_location.clone()
                } else {
                    library::parse_hoy_loc_key(&key)
                        .and_then(|i| m.cfg.hoy_locations.get(i).cloned())
                };
                if let Some(loc) = loc {
                    open_hoy_chart(&mut m, &key, &loc);
                }
            }
        }
        // rectificador de hora
        Msg::RectifyNudge(d) => {
            m.rectify_offset_min += d;
            recompute_chart(&mut m);
            recompute_astro(&mut m);
        }
        Msg::RectifyResetOffset => {
            m.rectify_offset_min = 0;
            recompute_chart(&mut m);
            recompute_astro(&mut m);
        }
        Msg::RectifyAddEvent => m.rectify_events.push(25.0),
        Msg::RectifyEventDelta(i, d) => {
            if let Some(e) = m.rectify_events.get_mut(i) {
                *e = (*e + d).clamp(0.0, 120.0);
            }
        }
        Msg::RectifyRemoveEvent(i) => {
            if i < m.rectify_events.len() {
                m.rectify_events.remove(i);
            }
        }
        Msg::RectifyRun => run_rectify(&mut m),
        Msg::RectifyApply => apply_rectify(&mut m),
        Msg::RectifySetKey(naibod) => {
            m.rectify_naibod = naibod;
            if !m.rectify_triggers.is_empty() {
                compute_triggers(&mut m);
            }
        }
        Msg::RectifyAgeDelta(d) => {
            m.rectify_age = (m.rectify_age + d).clamp(0.0, 120.0);
        }
        Msg::RectifyTriggers => compute_triggers(&mut m),
        // diálogos modales
        Msg::OpenNewContactDialog => open_contact_dialog(&mut m),
        Msg::OpenNewChartDialog => open_new_chart_or_hoy(&mut m),
        Msg::NavAdd(key) => {
            // El «+»/«Nueva» de una fila ancla la creación a ese nodo.
            m.nav_selected = Some(key);
            open_new_chart_or_hoy(&mut m);
        }
        Msg::DialogFocus(f) => dialog_focus(&mut m, f),
        Msg::DialogClickAt(f, x) => {
            // Enfocar el campo (carga su valor en el buffer vivo) y registrar
            // el click para caret/selección (doble = palabra, triple = todo).
            dialog_focus(&mut m, f);
            m.dialog_input.pointer_click(x, 13.0);
        }
        Msg::DialogKey(ev) => {
            m.dialog_input.apply_key(&ev);
            let txt = m.dialog_input.text();
            let f = m.dialog_field;
            if let Some(d) = m.dialog.as_mut() {
                d.set_field(f, txt);
            }
        }
        Msg::DialogPickCity(idx) => dialog_pick_city(&mut m, idx),
        Msg::DialogPickContact(id) => dialog_pick_contact(&mut m, id),
        Msg::DialogSetKind(k) => dialog_set_kind(&mut m, k),
        Msg::DialogToggleKind => dialog_toggle_kind(&mut m),
        Msg::DialogToggleCalendar => dialog_toggle_calendar(&mut m),
        Msg::DialogCalPick(y, mo, d) => dialog_cal_pick(&mut m, y, mo, d),
        Msg::DialogCalView(y, mo) => dialog_cal_view(&mut m, y, mo),
        Msg::DialogTimeStep(hours, delta) => dialog_time_step(&mut m, hours, delta),
        Msg::DialogConfirm => {
            dialog_confirm(&mut m);
            persist = true;
        }
        Msg::DialogCancel => m.dialog = None,
        Msg::DialogNop => {}
        // layout guardable
        Msg::SetNavWidth(dx) => m.nudge_nav(dx),
        Msg::SetToolsWidth(dx) => m.nudge_tools(dx),
        Msg::PersistLayout => persist = true,
        // panel de herramientas
        Msg::ToggleToolPanel(p) => {
            m.toggle_panel(p);
            persist = true;
        }
        // dock
        Msg::DockActivate(side, item) => {
            // Clic en el diente activo del lado ya desplegado → colapsa
            // (toggle, estilo web); cualquier otro → activa + despliega.
            let toggle_off = m.dock_active(side) == Some(item)
                && m.dock_expanded == Some(side);
            match side {
                model::DockSide::Left => m.active_left = Some(item),
                model::DockSide::Right => m.active_right = Some(item),
            }
            m.dock_expanded = if toggle_off { None } else { Some(side) };
            persist = true;
        }
        Msg::DockDrop(side, payload) => {
            if let Some(item) = model::DockItem::from_u64(payload) {
                // Sólo mover si cambia de lado — evita el reordenado
                // molesto al soltar (o al hacer clic) en el mismo lado.
                let already = match side {
                    model::DockSide::Left => m.dock_left.contains(&item),
                    model::DockSide::Right => m.dock_right.contains(&item),
                };
                if !already {
                    m.dock_move(item, side);
                    persist = true;
                }
            }
        }
        // Rail hospedado: pata reenvió el clic de un diente prestado.
        Msg::HostActivate(id) => {
            if let Some(item) = model::DockItem::from_u64(id as u64) {
                let side = if m.dock_left.contains(&item) {
                    model::DockSide::Left
                } else {
                    model::DockSide::Right
                };
                let toggle_off =
                    m.dock_active(side) == Some(item) && m.dock_expanded == Some(side);
                match side {
                    model::DockSide::Left => m.active_left = Some(item),
                    model::DockSide::Right => m.active_right = Some(item),
                }
                m.dock_expanded = if toggle_off { None } else { Some(side) };
                persist = true;
            }
        }
        // tipo de gráfica
        Msg::SetChartView(v) => {
            m.chart_view = v;
            persist = true;
        }
        // Resultado del worker astronómico.
        Msg::AstroComputed(gen, astro) => {
            if gen == m.astro_gen {
                m.astro = Some(Arc::try_unwrap(astro).unwrap_or_else(|a| (*a).clone()));
            }
        }
    }
    if persist {
        save_ui(&m);
    }
    // Cómputo astronómico FUERA del hilo de UI.
    if m.astro_dirty {
        m.astro_dirty = false;
        m.astro_gen = m.astro_gen.wrapping_add(1);
        let gen = m.astro_gen;
        let (c, use_now) = (m.chart.clone(), m.cfg.use_now);
        handle.spawn(move || Msg::AstroComputed(gen, Arc::new(compute_astro(&c, use_now))));
    }
    // Refleja en el rail de pata el estado del dock (si delegamos): primero la
    // lista de dientes (si se reordenó), luego cuál quedó desplegado.
    sync_host_teeth(&mut m);
    sync_host_active(&mut m);
    m
}
