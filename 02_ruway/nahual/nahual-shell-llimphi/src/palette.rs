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
    vec![
        // ---- Navegación ----
        PaletteCommand::new("nav.open", "Abrir selección", "Navegar").with_shortcut("Enter"),
        PaletteCommand::new("nav.parent", "Subir al padre", "Navegar").with_shortcut("⌫"),
        PaletteCommand::new("nav.back", "Atrás", "Navegar"),
        PaletteCommand::new("nav.forward", "Adelante", "Navegar"),
        PaletteCommand::new("nav.filter", "Filtrar carpeta…", "Navegar").with_shortcut("/"),
        // ---- Vista ----
        PaletteCommand::new("view.list", "Vista lista", "Vista"),
        PaletteCommand::new("view.details", "Vista detalle", "Vista"),
        PaletteCommand::new("view.icons", "Vista iconos", "Vista"),
        PaletteCommand::new("view.gallery", "Vista galería", "Vista"),
        PaletteCommand::new("view.toggleDual", "Panel doble", "Vista").with_shortcut("d"),
        PaletteCommand::new("view.togglePreview", "Panel de previsualización", "Vista"),
        PaletteCommand::new("view.wheelZoom", "Rueda: zoom", "Vista"),
        PaletteCommand::new("view.wheelList", "Rueda: lista", "Vista"),
        PaletteCommand::new("view.cycleTheme", "Cambiar tema", "Vista"),
        // ---- Archivo ----
        PaletteCommand::new("file.newDir", "Nueva carpeta", "Archivo").with_shortcut("F7"),
        PaletteCommand::new("file.newFile", "Nuevo archivo", "Archivo"),
        PaletteCommand::new("file.rename", "Renombrar", "Archivo").with_shortcut("F2"),
        PaletteCommand::new("file.delete", "Borrar", "Archivo").with_shortcut("Supr"),
        PaletteCommand::new("file.batchRename", "Renombrar por lote…", "Archivo"),
        PaletteCommand::new("file.mark", "Marcar / desmarcar", "Archivo").with_shortcut("Ins"),
        PaletteCommand::new("file.copyToOther", "Copiar al otro panel", "Archivo").with_shortcut("F5"),
        PaletteCommand::new("file.moveToOther", "Mover al otro panel", "Archivo").with_shortcut("F6"),
        PaletteCommand::new("file.addFavorite", "Añadir a favoritos", "Archivo"),
        // ---- Etiquetas ----
        PaletteCommand::new("label.red", "● Etiqueta roja", "Etiqueta"),
        PaletteCommand::new("label.orange", "● Etiqueta naranja", "Etiqueta"),
        PaletteCommand::new("label.yellow", "● Etiqueta amarilla", "Etiqueta"),
        PaletteCommand::new("label.green", "● Etiqueta verde", "Etiqueta"),
        PaletteCommand::new("label.blue", "● Etiqueta azul", "Etiqueta"),
        PaletteCommand::new("label.purple", "● Etiqueta violeta", "Etiqueta"),
        PaletteCommand::new("label.gray", "● Etiqueta gris", "Etiqueta"),
        PaletteCommand::new("label.none", "Quitar etiqueta", "Etiqueta"),
        // ---- Fuentes (montaje) ----
        PaletteCommand::new("source.mountNouser", "Montar Mónadas (nouser)", "Fuente").with_shortcut("m"),
        PaletteCommand::new("source.mountMinga", "Montar grafo minga", "Fuente").with_shortcut("g"),
        PaletteCommand::new("source.unmount", "Desmontar fuente", "Fuente"),
        // ---- Sesiones ----
        PaletteCommand::new("session.new", "Nueva sesión", "Sesión"),
        // ---- Herramientas ----
        PaletteCommand::new("tools.terminalHere", "Abrir terminal aquí", "Herramientas"),
        PaletteCommand::new("tools.editInNada", "Editar en Nada", "Herramientas"),
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
        // Herramientas.
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
