//! Command palette del shell nahual (Ctrl+Shift+P, también Ctrl+P): el
//! catálogo de comandos accionables, el mapeo `id → Msg` y el routing del
//! módulo `llimphi-module-command-palette`. Mismo patrón canónico que `nada`:
//! el módulo es agnóstico (sólo presenta + rankea por fuzzy); acá vive todo lo
//! que es de la app.

use llimphi_module_command_palette::{self as palette, Command as PaletteCommand, PaletteAction, PaletteMsg, PaletteState};
use llimphi_ui::Handle;

use crate::modelo::{Model, Msg, WheelMode};
use crate::state::Label;
use nahual_source_core::ViewMode;

/// Catálogo de comandos que el palette muestra. Estático: lo construimos una
/// sola vez en `init` y vive en `Model.palette_commands`. Cada `id` debe estar
/// mapeado en [`palette_id_to_msg`] para que el invoke encuentre su `Msg`.
/// Títulos y grupos en español — el fuzzy matchea sobre `"título · grupo"`,
/// así que "ver" encuentra todo el grupo Vista.
pub(crate) fn build_command_catalog() -> Vec<PaletteCommand> {
    use rimay_localize::t;
    // Nombres de grupo localizados (el fuzzy matchea sobre "título · grupo",
    // así que el grupo también debe quedar en el idioma activo).
    let g_nav = t("nahual-shell-grp-navigate");
    let g_view = t("nahual-shell-grp-view");
    let g_file = t("nahual-shell-grp-file");
    let g_sel = t("nahual-shell-grp-selection");
    let g_label = t("nahual-shell-grp-label");
    let g_source = t("nahual-shell-grp-source");
    let g_session = t("nahual-shell-grp-session");
    let g_ai = t("nahual-shell-grp-ai");
    let g_tools = t("nahual-shell-grp-tools");
    vec![
        // ---- Navegación ----
        PaletteCommand::new("nav.open", t("nahual-shell-open-selection"), g_nav.clone()).with_shortcut("Enter"),
        PaletteCommand::new("nav.parent", t("nahual-shell-parent"), g_nav.clone()).with_shortcut("⌫"),
        PaletteCommand::new("nav.back", t("nahual-shell-back"), g_nav.clone()),
        PaletteCommand::new("nav.forward", t("nahual-shell-forward"), g_nav.clone()),
        PaletteCommand::new("nav.filter", t("nahual-shell-filter-folder"), g_nav).with_shortcut("/"),
        // ---- Vista ----
        PaletteCommand::new("view.list", t("nahual-shell-view-list"), g_view.clone()),
        PaletteCommand::new("view.details", t("nahual-shell-view-details"), g_view.clone()),
        PaletteCommand::new("view.icons", t("nahual-shell-view-icons"), g_view.clone()),
        PaletteCommand::new("view.gallery", t("nahual-shell-view-gallery"), g_view.clone()),
        PaletteCommand::new("view.toggleDual", t("nahual-shell-dual-panel"), g_view.clone()).with_shortcut("d"),
        PaletteCommand::new("view.togglePreview", t("nahual-shell-preview-panel"), g_view.clone()),
        PaletteCommand::new("view.wheelZoom", t("nahual-shell-wheel-zoom"), g_view.clone()),
        PaletteCommand::new("view.wheelList", t("nahual-shell-wheel-list"), g_view.clone()),
        PaletteCommand::new("view.cycleTheme", t("cycle-theme"), g_view),
        // ---- Archivo ----
        PaletteCommand::new("file.newDir", t("nahual-shell-new-dir"), g_file.clone()).with_shortcut("F7"),
        PaletteCommand::new("file.newFile", t("nahual-shell-new-file"), g_file.clone()),
        PaletteCommand::new("file.rename", t("nahual-shell-rename"), g_file.clone()).with_shortcut("F2"),
        PaletteCommand::new("file.delete", t("nahual-shell-delete"), g_file.clone()).with_shortcut("Supr"),
        PaletteCommand::new("file.batchRename", t("nahual-shell-batch-rename"), g_file.clone()),
        PaletteCommand::new("file.mark", t("nahual-shell-toggle-mark"), g_file.clone()).with_shortcut("Ins"),
        PaletteCommand::new("file.copyToOther", t("nahual-shell-copy-other"), g_file.clone()).with_shortcut("F5"),
        PaletteCommand::new("file.moveToOther", t("nahual-shell-move-other"), g_file.clone()).with_shortcut("F6"),
        PaletteCommand::new("file.addFavorite", t("nahual-shell-add-favorite"), g_file.clone()),
        // Organización del grafo de Mónadas (sólo activa con nouser montado).
        PaletteCommand::new("monad.submonadize", t("nahual-shell-submonadize"), g_file),
        // ---- Selección ----
        PaletteCommand::new("select.all", t("nahual-shell-select-all"), g_sel.clone()).with_shortcut("Ctrl+A"),
        PaletteCommand::new("select.none", t("nahual-shell-select-none"), g_sel.clone()),
        PaletteCommand::new("select.invert", t("nahual-shell-invert-selection"), g_sel.clone()).with_shortcut("*"),
        PaletteCommand::new("select.pattern", t("nahual-shell-select-pattern"), g_sel),
        // ---- Etiquetas ----
        PaletteCommand::new("label.red", t("nahual-shell-label-red"), g_label.clone()),
        PaletteCommand::new("label.orange", t("nahual-shell-label-orange"), g_label.clone()),
        PaletteCommand::new("label.yellow", t("nahual-shell-label-yellow"), g_label.clone()),
        PaletteCommand::new("label.green", t("nahual-shell-label-green"), g_label.clone()),
        PaletteCommand::new("label.blue", t("nahual-shell-label-blue"), g_label.clone()),
        PaletteCommand::new("label.purple", t("nahual-shell-label-purple"), g_label.clone()),
        PaletteCommand::new("label.gray", t("nahual-shell-label-gray"), g_label.clone()),
        PaletteCommand::new("label.none", t("nahual-shell-label-clear"), g_label),
        // ---- Fuentes (montaje) ----
        PaletteCommand::new("source.mountNouser", t("nahual-shell-mount-nouser"), g_source.clone()).with_shortcut("m"),
        PaletteCommand::new("source.mountMinga", t("nahual-shell-mount-minga"), g_source.clone()).with_shortcut("g"),
        PaletteCommand::new("source.unmount", t("nahual-shell-unmount"), g_source),
        // ---- Sesiones ----
        PaletteCommand::new("session.new", t("nahual-shell-new-session"), g_session),
        // ---- IA ----
        PaletteCommand::new("ai.ask", t("nahual-shell-ai-ask"), g_ai.clone()).with_shortcut("Ctrl+I"),
        PaletteCommand::new("ai.rename", t("nahual-shell-ai-rename"), g_ai.clone()),
        PaletteCommand::new("ai.index", t("nahual-shell-ai-index"), g_ai),
        // ---- Herramientas ----
        PaletteCommand::new("tools.find", t("nahual-shell-find"), g_tools.clone()).with_shortcut("Ctrl+F"),
        PaletteCommand::new("tools.terminalHere", t("nahual-shell-terminal-here"), g_tools.clone()),
        PaletteCommand::new("tools.editInNada", t("nahual-shell-edit-in-nada"), g_tools),
    ]
}

/// Traduce un id de comando del catálogo al `Msg` correspondiente. Si el id es
/// desconocido devuelve `None`. Mantener en sync con [`build_command_catalog`].
pub(crate) fn palette_id_to_msg(id: &str) -> Option<Msg> {
    Some(match id {
        // Navegación.
        "nav.open" => Msg::OpenSelected,
        "nav.parent" => Msg::Parent,
        "nav.back" => Msg::NavBack,
        "nav.forward" => Msg::NavForward,
        "nav.filter" => Msg::NavFilterStart,
        // Vista.
        "view.list" => Msg::SetViewMode(ViewMode::List),
        "view.details" => Msg::SetViewMode(ViewMode::Details),
        "view.icons" => Msg::SetViewMode(ViewMode::Icons),
        "view.gallery" => Msg::SetViewMode(ViewMode::Gallery),
        "view.toggleDual" => Msg::ToggleDual,
        "view.togglePreview" => Msg::TogglePreviewPanel,
        "view.wheelZoom" => Msg::SetWheelMode(WheelMode::Zoom),
        "view.wheelList" => Msg::SetWheelMode(WheelMode::Lista),
        "view.cycleTheme" => Msg::CycleTheme,
        // Archivo.
        "file.newDir" => Msg::NewDirPrompt,
        "file.newFile" => Msg::NewFilePrompt,
        "file.rename" => Msg::RenamePrompt,
        "file.delete" => Msg::DeleteSelection,
        "file.batchRename" => Msg::BatchRenameStart,
        "file.mark" => Msg::ToggleMark,
        "file.copyToOther" => Msg::CopyToOther,
        "file.moveToOther" => Msg::MoveToOther,
        "file.addFavorite" => Msg::AddPlace,
        "monad.submonadize" => Msg::SubmonadizePrompt,
        // Selección.
        "select.all" => Msg::SelectAll,
        "select.none" => Msg::SelectNone,
        "select.invert" => Msg::InvertSelection,
        "select.pattern" => Msg::SelectByPattern,
        // Etiquetas.
        "label.red" => Msg::SetLabel(Label::Red),
        "label.orange" => Msg::SetLabel(Label::Orange),
        "label.yellow" => Msg::SetLabel(Label::Yellow),
        "label.green" => Msg::SetLabel(Label::Green),
        "label.blue" => Msg::SetLabel(Label::Blue),
        "label.purple" => Msg::SetLabel(Label::Purple),
        "label.gray" => Msg::SetLabel(Label::Gray),
        "label.none" => Msg::ClearLabel,
        // Fuentes.
        "source.mountNouser" => Msg::MountNouser,
        "source.mountMinga" => Msg::MountMinga,
        "source.unmount" => Msg::Unmount,
        // Sesiones.
        "session.new" => Msg::SessionNew,
        // IA.
        "ai.ask" => Msg::AiAsk,
        "ai.rename" => Msg::AiRename,
        "ai.index" => Msg::SemIndexBuild,
        // Herramientas.
        "tools.find" => Msg::FindOpen,
        "tools.terminalHere" => Msg::TerminalHere,
        "tools.editInNada" => Msg::EditSelected,
        _ => return None,
    })
}

/// Routea un `PaletteMsg` al módulo. Lazy-init en `Open` (pre-pobla con todos
/// los comandos). En `Invoke(id)` cierra el palette y dispatcha el `Msg`
/// correspondiente — el comando se ejecuta en el siguiente turno del loop.
pub(crate) fn apply_palette(model: Model, pm: PaletteMsg, handle: &Handle<Msg>) -> Model {
    let mut m = model;
    if matches!(pm, PaletteMsg::Open) && m.palette.is_none() {
        m.palette = Some(PaletteState::new(&m.palette_commands));
        return m;
    }
    let action = match m.palette.as_mut() {
        Some(state) => palette::apply(state, pm, &m.palette_commands),
        None => return m,
    };
    match action {
        PaletteAction::None => {}
        PaletteAction::Close => m.palette = None,
        PaletteAction::Invoke(id) => {
            m.palette = None;
            // Abrir un comando que necesita el contextual (terminal/editar)
            // requiere que `ctx_target` esté poblado: lo precomputamos aquí
            // igual que al abrir el menú contextual.
            if matches!(id.as_str(), "tools.terminalHere" | "tools.editInNada") {
                crate::helpers::compute_open_with(&mut m);
            }
            if let Some(msg) = palette_id_to_msg(&id) {
                handle.dispatch(msg);
            }
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Todo comando del catálogo debe mapear a un `Msg` — si no, sería una
    /// fila muerta que el usuario elige y no hace nada.
    #[test]
    fn cada_comando_tiene_msg() {
        for cmd in build_command_catalog() {
            assert!(
                palette_id_to_msg(&cmd.id).is_some(),
                "el comando «{}» ({}) no está mapeado en palette_id_to_msg",
                cmd.title,
                cmd.id,
            );
        }
    }

    /// Un id que no existe en el catálogo no debe mapear a nada.
    #[test]
    fn id_desconocido_no_mapea() {
        assert!(palette_id_to_msg("no.existe").is_none());
    }
}
