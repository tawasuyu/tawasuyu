//! Atajos de teclado y picker de la app `tullpu`: traducción de
//! `KeyEvent` a `Msg` (atajos globales + por capa seleccionada), routeo
//! del fuzzy file picker y la extensión del formato de export.
//!
//! Behavior-preserving split de `main.rs` — sin cambios funcionales.

use llimphi_module_file_picker::{
    self as picker, PickerAction, PickerMsg, PickerState,
};
use llimphi_ui::{Key, KeyEvent, NamedKey};

use tullpu_render::FormatoExport;

use crate::historial::pushear_snapshot;
use crate::ops::agregar_capa_desde_archivo;
use crate::model::*;

pub(crate) fn extension_export(f: FormatoExport) -> &'static str {
    match f {
        FormatoExport::Png => "png",
        FormatoExport::Jpeg { .. } => "jpg",
        FormatoExport::Webp => "webp",
    }
}

/// Traduce un `KeyEvent` a un `Msg` según el catálogo de atajos. Se asume
/// que el llamante ya descartó el caso "picker abierto" — acá routeamos
/// libremente sobre el modelo principal. Función pura para que el test
/// pueda cubrir el dispatch sin levantar la app.
///
/// Catálogo:
/// - `Delete` / `Backspace` → eliminar capa seleccionada
/// - `Ctrl+D` → duplicar
/// - `V` → toggle visibilidad
/// - `B` → ciclar blend forward, `Shift+B` ciclar reverse
/// - `[` / `]` → bump opacidad ∓0.1
/// - `Ctrl+S` → export PNG, `Ctrl+Shift+S` → WebP
/// - `Ctrl+Z` → undo, `Ctrl+Shift+Z` o `Ctrl+Y` → redo (globales)
/// Paso del nudge de la selección: 10 px con Shift, 1 px sin él.
pub(crate) fn paso_nudge(shift: bool) -> i32 {
    if shift {
        10
    } else {
        1
    }
}

pub(crate) fn hotkey_a_msg(model: &Model, event: &KeyEvent) -> Option<Msg> {
    use llimphi_ui::KeyState;
    if event.state != KeyState::Pressed {
        return None;
    }
    let m = event.modifiers;
    // Atajos globales (no requieren selección).
    match &event.key {
        Key::Character(s) if m.ctrl && !m.shift && s.eq_ignore_ascii_case("s") => {
            return Some(Msg::Exportar(FormatoExport::Png));
        }
        Key::Character(s) if m.ctrl && m.shift && s.eq_ignore_ascii_case("s") => {
            return Some(Msg::Exportar(FormatoExport::Webp));
        }
        Key::Character(s) if m.ctrl && !m.shift && s.eq_ignore_ascii_case("z") => {
            return Some(Msg::Undo);
        }
        Key::Character(s) if m.ctrl && m.shift && s.eq_ignore_ascii_case("z") => {
            return Some(Msg::Redo);
        }
        Key::Character(s) if m.ctrl && !m.shift && s.eq_ignore_ascii_case("y") => {
            return Some(Msg::Redo);
        }
        // Ctrl+A = seleccionar todo el lienzo. Global: no depende de la
        // capa, arma un marquee que cubre el canvas entero.
        Key::Character(s) if m.ctrl && !m.shift && s.eq_ignore_ascii_case("a") => {
            return Some(Msg::SeleccionarTodo);
        }
        // Ctrl+Shift+E = aplanar visibles (Photoshop "Merge Visible").
        // Global: no requiere selección — opera sobre todo el lienzo.
        Key::Character(s) if m.ctrl && m.shift && s.eq_ignore_ascii_case("e") => {
            return Some(Msg::AplanarVisibles);
        }
        // Reset de vista: zoom 100% del fit + pan a cero. Global porque
        // no depende de capa seleccionada — es navegación del viewport.
        Key::Character(s) if !m.ctrl && !m.alt && s == "0" => {
            return Some(Msg::ResetVista);
        }
        // Herramientas: `m` mover (pan), `i` cuentagotas (eyedropper —
        // Photoshop standard). Globales porque cambian el modo del
        // lienzo, no operan sobre la capa.
        Key::Character(s) if !m.ctrl && !m.alt && s.eq_ignore_ascii_case("m") => {
            return Some(Msg::CambiarHerramienta(Herramienta::Mover));
        }
        Key::Character(s) if !m.ctrl && !m.alt && s.eq_ignore_ascii_case("i") => {
            return Some(Msg::CambiarHerramienta(Herramienta::Cuentagotas));
        }
        Key::Character(s) if !m.ctrl && !m.alt && s.eq_ignore_ascii_case("r") => {
            return Some(Msg::CambiarHerramienta(Herramienta::Marco));
        }
        // `g` = balde (flood fill). Global como las demás herramientas.
        Key::Character(s) if !m.ctrl && !m.alt && s.eq_ignore_ascii_case("g") => {
            return Some(Msg::CambiarHerramienta(Herramienta::Balde));
        }
        // Esc limpia la selección (si hay) — global porque no compite
        // con otros modales: cuando picker está abierto o se está
        // renombrando, este `hotkey_a_msg` no se invoca (los modales
        // capturan Esc antes).
        Key::Named(NamedKey::Escape) if !m.ctrl && !m.alt => {
            if model.seleccion.is_some() || model.seleccion_drag.is_some() {
                return Some(Msg::LimpiarSeleccion);
            }
        }
        _ => {}
    }
    // El resto opera sobre la capa seleccionada.
    let id = model.seleccionada?;
    match &event.key {
        Key::Named(NamedKey::F2) => Some(Msg::IniciarRenombrar(id)),
        // Shift+Del/Backspace con selección activa: rellena el rect con
        // el color activo (Photoshop usa Alt+Backspace para fill de
        // foreground; acá Shift queda libre y es la convención del
        // resto de la app para "variante" de una acción). Sin selección
        // no aplica y cae a las arms de abajo.
        Key::Named(NamedKey::Delete) | Key::Named(NamedKey::Backspace)
            if m.shift && !m.ctrl && model.seleccion.is_some() =>
        {
            Some(Msg::RellenarSeleccionEnCapa)
        }
        // Con selección activa, Del/Backspace limpian los píxeles
        // del rect (Photoshop standard). Sin selección, eliminan la
        // capa entera (comportamiento previo). El conflicto se
        // resuelve por el contexto del marquee, no por un modifier.
        Key::Named(NamedKey::Delete) | Key::Named(NamedKey::Backspace) if !m.ctrl => {
            if model.seleccion.is_some() {
                Some(Msg::LimpiarSeleccionEnCapa)
            } else {
                Some(Msg::Eliminar(id))
            }
        }
        Key::Character(s) if m.ctrl && !m.shift && s.eq_ignore_ascii_case("d") => {
            Some(Msg::Duplicar(id))
        }
        // Ctrl+J = layer via copy (Photoshop): sólo con selección
        // activa. Sin selección no aplica (la capa entera ya se
        // duplica con Ctrl+D).
        Key::Character(s)
            if m.ctrl
                && !m.shift
                && s.eq_ignore_ascii_case("j")
                && model.seleccion.is_some() =>
        {
            Some(Msg::DuplicarSeleccionACapa)
        }
        // Portapapeles interno. Copiar/cortar exigen selección;
        // pegar exige clip (no selección — crea una capa nueva).
        Key::Character(s)
            if m.ctrl
                && !m.shift
                && s.eq_ignore_ascii_case("c")
                && model.seleccion.is_some() =>
        {
            Some(Msg::CopiarSeleccion)
        }
        Key::Character(s)
            if m.ctrl
                && !m.shift
                && s.eq_ignore_ascii_case("x")
                && model.seleccion.is_some() =>
        {
            Some(Msg::CortarSeleccion)
        }
        Key::Character(s)
            if m.ctrl
                && !m.shift
                && s.eq_ignore_ascii_case("v")
                && model.portapapeles.is_some() =>
        {
            Some(Msg::PegarPortapapeles)
        }
        // Ctrl+E = merge down (combinar con la capa de abajo). Sin
        // selección no aplica. Photoshop standard.
        Key::Character(s) if m.ctrl && !m.shift && s.eq_ignore_ascii_case("e") => {
            Some(Msg::Combinar(id))
        }
        Key::Character(s) if !m.ctrl && !m.alt && s.eq_ignore_ascii_case("v") => {
            Some(Msg::ToggleVisible(id))
        }
        Key::Character(s) if !m.ctrl && !m.alt && s.eq_ignore_ascii_case("b") => {
            // El cycle inverso se distingue por shift; sin shift es forward.
            // Reutilizamos `CiclarBlend` para forward; para reverse emitimos
            // un mensaje propio que el update conoce.
            if m.shift {
                Some(Msg::CiclarBlendInverso(id))
            } else {
                Some(Msg::CiclarBlend(id))
            }
        }
        Key::Character(s) if !m.ctrl && !m.alt && s == "[" => {
            Some(Msg::BumpOpacidad(id, -0.1))
        }
        Key::Character(s) if !m.ctrl && !m.alt && s == "]" => {
            Some(Msg::BumpOpacidad(id, 0.1))
        }
        // Flechas: nudge del contenido de la selección 1 px (10 px con
        // Shift). Sólo con selección activa; sin ella las flechas no
        // hacen nada acá (no hay scroll de capas que mover).
        Key::Named(NamedKey::ArrowLeft) if model.seleccion.is_some() => {
            Some(Msg::MoverSeleccion { dx: -paso_nudge(m.shift), dy: 0 })
        }
        Key::Named(NamedKey::ArrowRight) if model.seleccion.is_some() => {
            Some(Msg::MoverSeleccion { dx: paso_nudge(m.shift), dy: 0 })
        }
        Key::Named(NamedKey::ArrowUp) if model.seleccion.is_some() => {
            Some(Msg::MoverSeleccion { dx: 0, dy: -paso_nudge(m.shift) })
        }
        Key::Named(NamedKey::ArrowDown) if model.seleccion.is_some() => {
            Some(Msg::MoverSeleccion { dx: 0, dy: paso_nudge(m.shift) })
        }
        _ => None,
    }
}

/// Routea un `PickerMsg` al módulo y traduce el `PickerAction` resultante:
/// `Open(path)` decodea el PNG/JPEG, lo ajusta al tamaño del lienzo y lo
/// apila como capa raster nueva encima de la seleccionada (o al tope).
pub(crate) fn aplicar_picker(mut model: Model, pm: PickerMsg) -> Model {
    if matches!(pm, PickerMsg::Open) && model.picker.is_none() {
        model.picker = Some(PickerState::new(
            &model.imagenes_disponibles,
            &model.raiz,
        ));
        model.estado = format!(
            "picker · {} imágenes · ↓↑ navega · Enter agrega · Esc cierra",
            model.imagenes_disponibles.len(),
        );
        return model;
    }
    let action = match model.picker.as_mut() {
        Some(state) => picker::apply(state, pm, &model.imagenes_disponibles, &model.raiz),
        None => return model,
    };
    match action {
        PickerAction::Close => {
            model.picker = None;
            model.estado = "listo".into();
        }
        PickerAction::Open(path) => {
            model.picker = None;
            if agregar_capa_desde_archivo(&mut model, &path) {
                pushear_snapshot(&mut model, None);
            }
        }
        PickerAction::None => {}
    }
    model
}
