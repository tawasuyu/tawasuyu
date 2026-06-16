//! Tests del módulo `desktop` — cubre el estado del escritorio y el bucle
//! `evento → comandos`.

use mirada_layout::{LayoutMode, LayoutParams, Rect, WindowId};
use mirada_protocol::{BodyEvent, BrainCommand};

use crate::action::{DesktopAction, WORKSPACE_COUNT};
use crate::permisos::Permisos;
use crate::rules::Rules;
use crate::session::DesktopState;

use super::estado::Desktop;
use super::geometria::nearest_in_direction;
use super::tipos::Output;

use crate::action::Direction;
use crate::desktop::DROPTERM_APP_ID;

/// Un escritorio con una salida 1920×1080 ya conectada.
fn desktop_with_screen() -> Desktop {
    let mut d = Desktop::new();
    d.on_event(BodyEvent::OutputAdded { id: 0, width: 1920, height: 1080 });
    d
}

fn open(d: &mut Desktop, id: WindowId) -> Vec<BrainCommand> {
    d.on_event(BodyEvent::WindowOpened {
        id,
        app_id: format!("app{id}"),
        title: format!("win {id}"),
    })
}

/// Extrae las colocaciones de un único `Place`.
fn places(cmds: &[BrainCommand]) -> &[mirada_protocol::WindowPlacement] {
    match cmds {
        [BrainCommand::Place(p)] => p,
        other => panic!("se esperaba un solo Place, no {other:?}"),
    }
}

#[test]
fn grab_keys_lists_the_whole_keymap() {
    let d = Desktop::new();
    match d.grab_keys() {
        BrainCommand::GrabKeys(keys) => {
            assert!(keys.contains(&"Super+j".to_string()));
            assert!(keys.contains(&"Super+Shift+e".to_string()));
        }
        other => panic!("se esperaba GrabKeys, no {other:?}"),
    }
}

#[test]
fn dragging_a_floating_window_over_a_tile_returns_it_to_tiling() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    open(&mut d, 2);
    // Flota la 1 (ToggleFloat actúa sobre la enfocada).
    d.apply(DesktopAction::FocusWindow(1));
    d.apply(DesktopAction::ToggleFloat);
    let active = d.active_index();
    assert!(d.workspaces[active].is_floating(1), "la 1 debería flotar");

    // Soltar la flotante sobre el centro de la pantalla: ahí vive la 2
    // (única teselada → ocupa toda el área). Debe volver al mosaico.
    d.on_event(BodyEvent::WindowDragged { id: 1, x: 960, y: 540 });
    assert!(
        !d.workspaces[active].is_floating(1),
        "soltada sobre una tesela, la 1 debe volver al mosaico"
    );

    // Soltarla sobre vacío (fuera de toda tesela) no la re-tila.
    d.apply(DesktopAction::FocusWindow(1));
    d.apply(DesktopAction::ToggleFloat);
    assert!(d.workspaces[active].is_floating(1));
    d.on_event(BodyEvent::WindowDragged { id: 1, x: -100, y: -100 });
    assert!(
        d.workspaces[active].is_floating(1),
        "soltada en vacío, sigue flotando"
    );
}

#[test]
fn set_keymap_swaps_the_bindings_and_regrabs() {
    let mut d = desktop_with_screen();
    for id in [1, 2, 3] {
        open(&mut d, id);
    }
    // El keymap por defecto no usa Alt.
    assert!(d.on_event(BodyEvent::Keybind("Alt+x".into())).is_empty());
    // Cargamos un keymap a medida; el comando devuelto re-registra grabs.
    let custom = crate::Keymap::from_ron(r#"( bindings: { "Alt+x": "focus-prev" } )"#).unwrap();
    match d.set_keymap(custom) {
        BrainCommand::GrabKeys(keys) => assert_eq!(keys, vec!["Alt+x".to_string()]),
        other => panic!("se esperaba GrabKeys, no {other:?}"),
    }
    // Ahora «Alt+x» sí mueve el foco, y «Super+j» ya no.
    assert_eq!(d.focused_window(), Some(3));
    d.on_event(BodyEvent::Keybind("Alt+x".into()));
    assert_eq!(d.focused_window(), Some(2));
    assert!(d.on_event(BodyEvent::Keybind("Super+j".into())).is_empty());
}

#[test]
fn focus_window_addresses_a_specific_window() {
    let mut d = desktop_with_screen();
    for id in [1, 2, 3] {
        open(&mut d, id);
    }
    assert_eq!(d.focused_window(), Some(3));
    d.apply(DesktopAction::FocusWindow(1));
    assert_eq!(d.focused_window(), Some(1));
}

#[test]
fn focus_window_jumps_to_the_workspace_that_holds_it() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    open(&mut d, 2); // enfocada
    // Manda la 2 al escritorio 3; seguimos en el 1.
    d.on_event(BodyEvent::Keybind("Super+Shift+3".into()));
    assert_eq!(d.active_index(), 0);
    // Enfocar la 2 nos lleva a su escritorio.
    d.apply(DesktopAction::FocusWindow(2));
    assert_eq!(d.active_index(), 2);
    assert_eq!(d.focused_window(), Some(2));
}

#[test]
fn window_lines_cover_every_window_with_its_workspace() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    open(&mut d, 2);
    d.on_event(BodyEvent::Keybind("Super+Shift+3".into())); // la 2 al esc. 3
    let lines = d.window_lines();
    assert_eq!(lines.len(), 2);
    let w1 = lines.iter().find(|l| l.id == 1).unwrap();
    let w2 = lines.iter().find(|l| l.id == 2).unwrap();
    assert_eq!(w1.workspace, 1);
    assert_eq!(w2.workspace, 3);
    // La 1 quedó enfocada en el escritorio activo (el 1).
    assert!(w1.focused);
    assert!(!w2.focused);
}

#[test]
fn toggle_float_marks_the_focused_window_and_floats_it_last() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    open(&mut d, 2); // enfocada
    let cmds = d.apply(DesktopAction::ToggleFloat);
    let p = places(&cmds);
    assert!(p.iter().find(|x| x.id == 2).unwrap().floating);
    // La flotante va al final de la lista — orden de pintado.
    assert_eq!(p.last().unwrap().id, 2);
    // Alternar de nuevo la devuelve al teselado.
    let cmds = d.apply(DesktopAction::ToggleFloat);
    assert!(!places(&cmds).iter().find(|x| x.id == 2).unwrap().floating);
}

#[test]
fn toggle_fullscreen_covers_the_screen_and_hides_the_rest() {
    let mut d = desktop_with_screen();
    for id in [1, 2, 3] {
        open(&mut d, id);
    }
    let cmds = d.apply(DesktopAction::ToggleFullscreen); // sobre la 3
    let p = places(&cmds);
    let fs = p.iter().find(|x| x.id == 3).unwrap();
    assert!(fs.fullscreen && fs.visible);
    assert_eq!(fs.rect, d.screen().unwrap());
    assert!(p.iter().filter(|x| x.id != 3).all(|x| !x.visible));
    // Alternar de nuevo restaura el teselado: las tres visibles.
    let cmds = d.apply(DesktopAction::ToggleFullscreen);
    assert_eq!(places(&cmds).iter().filter(|x| x.visible).count(), 3);
}

#[test]
fn a_rule_sends_a_new_window_to_its_workspace() {
    let mut d = desktop_with_screen();
    d.set_rules(Rules::from_ron(r#"( rules: [ (app_id: "app2", workspace: 3) ] )"#).unwrap());
    open(&mut d, 1); // app1 → sin regla → escritorio activo (1)
    open(&mut d, 2); // app2 → regla → escritorio 3
    assert_eq!(d.workspace_loads()[0], 1);
    assert_eq!(d.workspace_loads()[2], 1);
}

#[test]
fn a_rule_can_open_a_window_floating() {
    let mut d = desktop_with_screen();
    d.set_rules(Rules::from_ron(r#"( rules: [ (app_id: "app1", floating: true) ] )"#).unwrap());
    let cmds = open(&mut d, 1);
    assert!(places(&cmds).iter().find(|p| p.id == 1).unwrap().floating);
}

#[test]
fn without_a_screen_nothing_is_placed() {
    let mut d = Desktop::new();
    assert!(open(&mut d, 1).is_empty());
}

#[test]
fn opening_a_window_places_it() {
    let mut d = desktop_with_screen();
    let cmds = open(&mut d, 1);
    assert_eq!(places(&cmds).len(), 1);
    assert_eq!(d.focused_window(), Some(1));
}

#[test]
fn closing_a_window_removes_it_everywhere() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    open(&mut d, 2);
    let cmds = d.on_event(BodyEvent::WindowClosed { id: 1 });
    assert_eq!(places(&cmds).len(), 1);
    assert!(d.window_info(1).is_none());
    assert_eq!(d.focused_window(), Some(2));
}

#[test]
fn focus_keybind_cycles_within_the_active_workspace() {
    let mut d = desktop_with_screen();
    for id in [1, 2, 3] {
        open(&mut d, id);
    }
    assert_eq!(d.focused_window(), Some(3));
    d.on_event(BodyEvent::Keybind("Super+j".into())); // next, da la vuelta
    assert_eq!(d.focused_window(), Some(1));
    d.on_event(BodyEvent::Keybind("Super+k".into())); // prev
    assert_eq!(d.focused_window(), Some(3));
}

#[test]
fn close_focused_keybind_asks_to_close_the_focused_window() {
    let mut d = desktop_with_screen();
    open(&mut d, 7);
    let cmds = d.on_event(BodyEvent::Keybind("Super+q".into()));
    assert_eq!(cmds, vec![BrainCommand::Close(7)]);
    // No se elimina hasta que el Cuerpo confirme con WindowClosed.
    assert!(d.window_info(7).is_some());
}

#[test]
fn close_window_by_id_closes_only_existing_windows() {
    let mut d = desktop_with_screen();
    for id in [1, 2, 3] {
        open(&mut d, id);
    }
    // El foco está en 3, pero cerramos la 1 por id (no la enfocada).
    let cmds = d.apply(DesktopAction::CloseWindow(1));
    assert_eq!(cmds, vec![BrainCommand::Close(1)]);
    // Id inexistente: no emite nada (no rompe).
    assert!(d.apply(DesktopAction::CloseWindow(999)).is_empty());
}

#[test]
fn cycle_layout_walks_every_mode_and_returns() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    let start = d.active_workspace().params().mode;
    for _ in 0..LayoutMode::ALL.len() {
        let before = d.active_workspace().params().mode;
        d.on_event(BodyEvent::Keybind("Super+space".into()));
        assert_eq!(d.active_workspace().params().mode, before.next());
    }
    // Una vuelta completa devuelve al modo inicial.
    assert_eq!(d.active_workspace().params().mode, start);
}

#[test]
fn grow_and_shrink_master_adjust_the_ratio() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    let r0 = d.active_workspace().params().master_ratio;
    d.apply(DesktopAction::GrowMaster);
    assert!(d.active_workspace().params().master_ratio > r0);
    d.apply(DesktopAction::ShrinkMaster);
    assert!((d.active_workspace().params().master_ratio - r0).abs() < 1e-6);
}

#[test]
fn inc_and_dec_master_adjust_nmaster() {
    let mut d = desktop_with_screen();
    for id in [1, 2, 3] {
        open(&mut d, id);
    }
    assert_eq!(d.active_workspace().params().master_count, 1);
    d.apply(DesktopAction::IncMaster);
    assert_eq!(d.active_workspace().params().master_count, 2);
    d.apply(DesktopAction::DecMaster);
    d.apply(DesktopAction::DecMaster); // no baja de 1
    assert_eq!(d.active_workspace().params().master_count, 1);
}

#[test]
fn swap_master_exchanges_only_the_focused_and_the_master() {
    let mut d = desktop_with_screen();
    for id in [1, 2, 3, 4] {
        open(&mut d, id);
    }
    d.apply(DesktopAction::FocusWindow(3));
    d.apply(DesktopAction::SwapMaster);
    // 3 pasa al puesto maestro, 1 a donde estaba 3; el resto intacto.
    assert_eq!(d.active_workspace().windows(), &[3, 2, 1, 4]);
    assert_eq!(d.focused_window(), Some(3));
    // A diferencia de promote-to-master, que rota: promover la 4…
    d.apply(DesktopAction::FocusWindow(4));
    d.apply(DesktopAction::PromoteToMaster);
    assert_eq!(d.active_workspace().windows(), &[4, 3, 2, 1]);
}

#[test]
fn move_to_workspace_sends_the_window_and_follows_it() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    open(&mut d, 2); // enfocada
    d.apply(DesktopAction::MoveToWorkspace(2)); // índice 2 = escritorio 3
    // La 2 viajó y el foco saltó con ella.
    assert_eq!(d.active_index(), 2);
    assert_eq!(d.focused_window(), Some(2));
    assert_eq!(d.workspace_loads()[2], 1);
    // El escritorio original conserva sólo la 1.
    assert_eq!(d.workspace_loads()[0], 1);
}

#[test]
fn promote_to_master_brings_the_focused_window_to_the_front() {
    let mut d = desktop_with_screen();
    for id in [1, 2, 3] {
        open(&mut d, id);
    }
    d.apply(DesktopAction::FocusWindow(3));
    d.apply(DesktopAction::PromoteToMaster);
    assert_eq!(d.active_workspace().windows()[0], 3);
    assert_eq!(d.focused_window(), Some(3));
}

#[test]
fn master_ratio_stays_within_bounds() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    for _ in 0..50 {
        d.apply(DesktopAction::GrowMaster);
    }
    assert!(d.active_workspace().params().master_ratio <= 0.95);
    for _ in 0..50 {
        d.apply(DesktopAction::ShrinkMaster);
    }
    assert!(d.active_workspace().params().master_ratio >= 0.05);
}

#[test]
fn monocle_keybind_hides_all_but_the_focused_window() {
    let mut d = desktop_with_screen();
    for id in [1, 2, 3] {
        open(&mut d, id);
    }
    let cmds = d.on_event(BodyEvent::Keybind("Super+m".into()));
    let visible = places(&cmds).iter().filter(|p| p.visible).count();
    assert_eq!(visible, 1);
}

#[test]
fn switching_workspace_changes_what_is_placed() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    open(&mut d, 2);
    // Escritorio 2 (índice 1) está vacío.
    let cmds = d.on_event(BodyEvent::Keybind("Super+2".into()));
    assert!(places(&cmds).is_empty());
    assert_eq!(d.active_index(), 1);
    // Volver al 1 reaparece las dos ventanas.
    let cmds = d.on_event(BodyEvent::Keybind("Super+1".into()));
    assert_eq!(places(&cmds).len(), 2);
}

#[test]
fn send_to_workspace_moves_the_focused_window() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    open(&mut d, 2); // enfocada
    d.on_event(BodyEvent::Keybind("Super+Shift+3".into()));
    assert_eq!(d.workspace_loads()[0], 1); // sólo queda la 1
    assert_eq!(d.workspace_loads()[2], 1); // la 2 viajó al escritorio 3
    // La ventana 2 sigue registrada — sólo cambió de escritorio.
    assert!(d.window_info(2).is_some());
}

#[test]
fn pointer_focuses_a_window_in_the_active_workspace() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    open(&mut d, 2); // enfocada
    d.on_event(BodyEvent::PointerEntered { id: 1 });
    assert_eq!(d.focused_window(), Some(1));
}

#[test]
fn dragging_floats_a_window_at_the_given_rect() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    open(&mut d, 2);
    assert!(!d.active_workspace().is_floating(2));
    let target = Rect::new(300, 200, 640, 480);
    let cmds = d.on_event(BodyEvent::WindowFloatTo { id: 2, rect: target });
    // La 2 ahora flota exactamente en el rectángulo pedido.
    assert!(d.active_workspace().is_floating(2));
    let p = places(&cmds).iter().find(|p| p.id == 2).unwrap();
    assert!(p.floating);
    assert_eq!(p.rect, target);
}

#[test]
fn resizing_an_output_retiles_without_losing_the_workspace() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    d.on_event(BodyEvent::Keybind("Super+2".into())); // escritorio activo → 2
    assert_eq!(d.active_index(), 1);
    let cmds = d.on_event(BodyEvent::OutputResized {
        id: 0,
        width: 1920,
        height: 1040,
    });
    // A diferencia de quitar y volver a añadir la salida, el
    // escritorio activo se conserva.
    assert_eq!(d.active_index(), 1);
    assert!(matches!(cmds.as_slice(), [BrainCommand::Place(_)]));
}

#[test]
fn reservar_franja_desplaza_y_encoge_el_teselado() {
    let mut d = desktop_with_screen(); // 1920×1080
    open(&mut d, 1);
    // Una sola ventana ocupa toda el área útil (smart gaps).
    let cmds = open(&mut d, 1); // re-relayout
    let p0 = places(&cmds)[0].rect;
    assert_eq!(p0, Rect::new(0, 0, 1920, 1080));

    // Reserva 40px arriba: la ventana arranca en y=40 y pierde 40 de alto.
    let cmds = d.on_event(BodyEvent::OutputReserved {
        id: 0,
        top: 40,
        bottom: 0,
        left: 0,
        right: 0,
    });
    let p = places(&cmds)[0].rect;
    assert_eq!(p, Rect::new(0, 40, 1920, 1040));

    // Reserva izquierda en vez de arriba: desplaza en x y encoge el ancho.
    let cmds = d.on_event(BodyEvent::OutputReserved {
        id: 0,
        top: 0,
        bottom: 0,
        left: 48,
        right: 0,
    });
    let p = places(&cmds)[0].rect;
    assert_eq!(p, Rect::new(48, 0, 1872, 1080));

    // Liberar (cero en los cuatro) restaura el monitor entero.
    let cmds = d.on_event(BodyEvent::OutputReserved {
        id: 0,
        top: 0,
        bottom: 0,
        left: 0,
        right: 0,
    });
    assert_eq!(places(&cmds)[0].rect, Rect::new(0, 0, 1920, 1080));
}

#[test]
fn a_spawn_keybind_becomes_a_spawn_command() {
    let mut d = desktop_with_screen();
    let cmds = d.on_event(BodyEvent::Keybind("Super+Shift+Return".into()));
    assert_eq!(cmds, vec![BrainCommand::Spawn("foot".into())]);
}

#[test]
fn dragging_an_unknown_window_does_nothing() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    let cmds = d.on_event(BodyEvent::WindowFloatTo {
        id: 99,
        rect: Rect::new(0, 0, 100, 100),
    });
    assert!(cmds.is_empty());
}

#[test]
fn retitling_updates_the_registry_without_relayout() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    let cmds = d.on_event(BodyEvent::WindowRetitled {
        id: 1,
        title: "nuevo".into(),
    });
    assert!(cmds.is_empty());
    assert_eq!(d.window_info(1).unwrap().title, "nuevo");
}

#[test]
fn an_unknown_keybind_does_nothing() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    assert!(d.on_event(BodyEvent::Keybind("Super+F12".into())).is_empty());
}

#[test]
fn quit_emits_a_shutdown() {
    let mut d = desktop_with_screen();
    assert_eq!(
        d.on_event(BodyEvent::Keybind("Super+Shift+e".into())),
        vec![BrainCommand::Shutdown]
    );
}

// --- Multi-monitor -------------------------------------------------

/// Un escritorio con dos salidas 1920×1080.
fn desktop_with_two_outputs() -> Desktop {
    let mut d = Desktop::new();
    d.on_event(BodyEvent::OutputAdded { id: 0, width: 1920, height: 1080 });
    d.on_event(BodyEvent::OutputAdded { id: 1, width: 1920, height: 1080 });
    d
}

#[test]
fn outputs_lay_side_by_side() {
    let mut d = Desktop::new();
    d.on_event(BodyEvent::OutputAdded { id: 0, width: 1920, height: 1080 });
    d.on_event(BodyEvent::OutputAdded { id: 1, width: 2560, height: 1440 });
    assert_eq!(d.outputs().len(), 2);
    // La segunda salida arranca donde acaba la primera.
    assert_eq!(d.outputs()[1].rect.x, 1920);
}

#[test]
fn each_output_shows_a_distinct_workspace() {
    let d = desktop_with_two_outputs();
    assert_eq!(d.outputs()[0].workspace, 0);
    assert_eq!(d.outputs()[1].workspace, 1);
}

#[test]
fn switching_to_a_workspace_shown_on_another_output_focuses_it() {
    let mut d = desktop_with_two_outputs();
    // La salida enfocada (0, ws 0) pide el ws 1, que YA muestra la salida 1.
    // No se lo robamos (eso arrastraba ventanas entre monitores y confundía):
    // movemos el FOCO a esa salida. Ningún escritorio cambia de monitor.
    assert_eq!(d.focused_output(), 0);
    d.apply(DesktopAction::SwitchWorkspace(1));
    assert_eq!(d.focused_output(), 1);
    assert_eq!(d.outputs()[0].workspace, 0);
    assert_eq!(d.outputs()[1].workspace, 1);
}

#[test]
fn el_output_enfocado_sigue_al_puntero() {
    let mut d = desktop_with_two_outputs();
    assert_eq!(d.focused_output(), 0);
    // Puntero sobre la salida 1 (arranca en x=1920).
    assert!(d.focus_output_at(2000, 100));
    assert_eq!(d.focused_output(), 1);
    // Moverse dentro de la misma salida no cambia nada.
    assert!(!d.focus_output_at(2100, 200));
    // De vuelta a la salida 0.
    assert!(d.focus_output_at(500, 100));
    assert_eq!(d.focused_output(), 0);
}

#[test]
fn focus_output_next_moves_the_focus_between_outputs() {
    let mut d = desktop_with_two_outputs();
    assert_eq!(d.active_index(), 0); // salida 0 → ws 0
    d.apply(DesktopAction::FocusOutputNext);
    assert_eq!(d.active_index(), 1); // salida 1 → ws 1
    d.apply(DesktopAction::FocusOutputNext); // envuelve
    assert_eq!(d.active_index(), 0);
}

#[test]
fn relayout_places_windows_on_every_output() {
    let mut d = Desktop::new();
    d.on_event(BodyEvent::OutputAdded { id: 0, width: 1920, height: 1080 });
    d.on_event(BodyEvent::OutputAdded { id: 1, width: 1280, height: 720 });
    open(&mut d, 1); // en la salida 0 (ws 0)
    d.apply(DesktopAction::FocusOutputNext);
    let cmds = open(&mut d, 2); // en la salida 1 (ws 1)
    let p = places(&cmds);
    assert_eq!(p.len(), 2);
    // Cada ventana cae en el rectángulo de su salida.
    assert_eq!(p.iter().find(|x| x.id == 1).unwrap().rect.x, 0);
    assert_eq!(p.iter().find(|x| x.id == 2).unwrap().rect.x, 1920);
}

#[test]
fn keyboard_focus_is_unique_across_outputs() {
    let mut d = desktop_with_two_outputs();
    open(&mut d, 1);
    d.apply(DesktopAction::FocusOutputNext);
    let cmds = open(&mut d, 2);
    // Sólo una ventana con foco de teclado en todo el Place.
    assert_eq!(places(&cmds).iter().filter(|p| p.focused).count(), 1);
}

#[test]
fn removing_an_output_keeps_its_windows_in_their_workspace() {
    let mut d = desktop_with_two_outputs();
    d.apply(DesktopAction::FocusOutputNext); // foco en la salida 1 (ws 1)
    open(&mut d, 1); // en ws 1
    d.on_event(BodyEvent::OutputRemoved { id: 1 });
    // La ventana sigue registrada, en el ws 1.
    assert!(d.window_info(1).is_some());
    assert_eq!(d.workspace_loads()[1], 1);
    assert_eq!(d.outputs().len(), 1);
}

// --- Scratchpad ----------------------------------------------------

#[test]
fn send_to_scratchpad_hides_the_focused_window() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    open(&mut d, 2); // enfocada
    d.apply(DesktopAction::SendToScratchpad);
    assert_eq!(d.workspace_loads()[0], 1); // sólo queda la 1
    assert!(d.window_info(2).is_some()); // sigue registrada
}

#[test]
fn toggle_scratchpad_shows_then_hides_the_stashed_window() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    open(&mut d, 2);
    d.apply(DesktopAction::SendToScratchpad); // guarda la 2
    assert_eq!(d.workspace_loads()[0], 1);
    // Toggle la invoca, flotando.
    let cmds = d.apply(DesktopAction::ToggleScratchpad);
    assert!(places(&cmds).iter().find(|x| x.id == 2).unwrap().floating);
    assert_eq!(d.workspace_loads()[0], 2);
    // Toggle de nuevo la oculta.
    d.apply(DesktopAction::ToggleScratchpad);
    assert_eq!(d.workspace_loads()[0], 1);
}

#[test]
fn a_scratchpad_window_follows_you_across_workspaces() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    d.apply(DesktopAction::SendToScratchpad);
    d.apply(DesktopAction::ToggleScratchpad); // mostrada en el escritorio 1
    assert_eq!(d.workspace_loads()[0], 1);
    d.apply(DesktopAction::SwitchWorkspace(1)); // al escritorio 2
    d.apply(DesktopAction::ToggleScratchpad); // estaba en el 1 → la trae al 2
    assert_eq!(d.workspace_loads()[1], 1);
    assert_eq!(d.workspace_loads()[0], 0);
}

#[test]
fn closing_a_stashed_window_drops_it_from_the_scratchpad() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    d.apply(DesktopAction::SendToScratchpad);
    d.on_event(BodyEvent::WindowClosed { id: 1 });
    // Ya no hay nada que invocar.
    assert!(d.apply(DesktopAction::ToggleScratchpad).is_empty());
}

// --- Escritorios especiales con nombre (estilo Hyprland) ----------

#[test]
fn named_special_workspace_stashes_and_summons() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    open(&mut d, 2); // enfocada
    d.apply(DesktopAction::MoveToSpecialWorkspace("musica".into()));
    assert_eq!(d.workspace_loads()[0], 1); // la 2 se apartó al especial
    // Toggle del especial la trae flotando.
    let cmds = d.apply(DesktopAction::ToggleSpecialWorkspace("musica".into()));
    assert!(places(&cmds).iter().find(|x| x.id == 2).unwrap().floating);
    assert_eq!(d.workspace_loads()[0], 2);
    // Toggle de nuevo la oculta.
    d.apply(DesktopAction::ToggleSpecialWorkspace("musica".into()));
    assert_eq!(d.workspace_loads()[0], 1);
}

#[test]
fn special_workspaces_are_independent_by_name() {
    let mut d = desktop_with_screen();
    open(&mut d, 1); // enfocada → al especial "a"
    d.apply(DesktopAction::MoveToSpecialWorkspace("a".into()));
    open(&mut d, 2); // enfocada → al especial "b"
    d.apply(DesktopAction::MoveToSpecialWorkspace("b".into()));
    // Invocar "a" trae sólo la 1, no la 2.
    d.apply(DesktopAction::ToggleSpecialWorkspace("a".into()));
    assert!(d.workspaces[0].windows().contains(&1));
    assert!(!d.workspaces[0].windows().contains(&2));
}

#[test]
fn toggling_an_empty_special_does_nothing() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    assert!(d
        .apply(DesktopAction::ToggleSpecialWorkspace("vacio".into()))
        .is_empty());
}

// --- Terminal dropdown (quake) ------------------------------------

#[test]
fn dropterm_lazy_spawns_when_absent() {
    let mut d = desktop_with_screen();
    // Sin terminal dropdown todavía: el toggle la crea con el comando
    // de la config (por defecto, kitty con el app_id de la dropterm).
    let cmds = d.apply(DesktopAction::ToggleDropterm);
    let cmd = crate::config::Config::default().dropterm_cmd;
    assert_eq!(cmds, vec![BrainCommand::Spawn(cmd)]);
}

#[test]
fn dropterm_opens_floating_top_anchored_and_focused() {
    let mut d = desktop_with_screen(); // 1920×1080
    let cmds = d.on_event(BodyEvent::WindowOpened {
        id: 5,
        app_id: DROPTERM_APP_ID.into(),
        title: "dropterm".into(),
    });
    let p = places(&cmds).iter().find(|x| x.id == 5).unwrap();
    assert!(p.floating);
    assert_eq!(p.rect.x, 0);
    assert_eq!(p.rect.w, 1920); // a todo el ancho
    assert!(p.rect.h < 1080); // anclada arriba, no a pantalla completa
    assert_eq!(d.focused_window(), Some(5));
}

#[test]
fn dropterm_toggles_hide_then_show_keeping_focus() {
    let mut d = desktop_with_screen();
    // Ya abierta (spawn + WindowOpened).
    d.on_event(BodyEvent::WindowOpened {
        id: 5,
        app_id: DROPTERM_APP_ID.into(),
        title: "t".into(),
    });
    assert_eq!(d.workspace_loads()[0], 1);
    // Toggle la guarda.
    d.apply(DesktopAction::ToggleDropterm);
    assert_eq!(d.workspace_loads()[0], 0);
    assert!(d.window_info(5).is_some()); // sigue registrada
    // Toggle la baja de nuevo, flotando y enfocada.
    let cmds = d.apply(DesktopAction::ToggleDropterm);
    assert_eq!(d.workspace_loads()[0], 1);
    assert!(places(&cmds).iter().find(|x| x.id == 5).unwrap().floating);
    assert_eq!(d.focused_window(), Some(5));
}

#[test]
fn nearest_in_direction_picks_the_window_in_front() {
    let from = Rect::new(0, 0, 100, 100); // centro (50,50)
    let cands = vec![
        (1, Rect::new(0, 0, 100, 100)),     // la propia
        (2, Rect::new(200, 0, 100, 100)),   // a la derecha, enfrente
        (3, Rect::new(200, 400, 100, 100)), // a la derecha pero muy abajo
        (4, Rect::new(-200, 0, 100, 100)),  // a la izquierda
    ];
    assert_eq!(nearest_in_direction(from, &cands, 1, Direction::Right), Some(2));
    assert_eq!(nearest_in_direction(from, &cands, 1, Direction::Left), Some(4));
    // Hacia arriba no hay nada (todas a la misma altura o abajo).
    assert_eq!(nearest_in_direction(from, &cands, 1, Direction::Up), None);
}

#[test]
fn focus_dir_moves_focus_spatially_in_columns() {
    let mut d = desktop_with_screen();
    // Tres columnas: la 1 a la izquierda, la 3 a la derecha.
    d.apply(DesktopAction::SetLayout(LayoutMode::Columns));
    for id in [1, 2, 3] {
        open(&mut d, id);
    }
    // La última abierta (3) queda enfocada, en la columna derecha.
    assert_eq!(d.focused_window(), Some(3));
    // Foco a la izquierda → la del medio, luego la primera.
    d.apply(DesktopAction::FocusDir(Direction::Left));
    assert_eq!(d.focused_window(), Some(2));
    d.apply(DesktopAction::FocusDir(Direction::Left));
    assert_eq!(d.focused_window(), Some(1));
    // Más a la izquierda no hay nada: el foco no se mueve.
    d.apply(DesktopAction::FocusDir(Direction::Left));
    assert_eq!(d.focused_window(), Some(1));
    // Y de vuelta a la derecha.
    d.apply(DesktopAction::FocusDir(Direction::Right));
    assert_eq!(d.focused_window(), Some(2));
}

#[test]
fn move_dir_swaps_the_focused_tile_with_its_neighbor() {
    let mut d = desktop_with_screen();
    d.apply(DesktopAction::SetLayout(LayoutMode::Columns));
    for id in [1, 2, 3] {
        open(&mut d, id);
    }
    assert_eq!(d.active_workspace().windows(), &[1, 2, 3]);
    assert_eq!(d.focused_window(), Some(3)); // columna derecha
    // Mover a la izquierda intercambia la 3 con su vecina (la 2).
    d.apply(DesktopAction::MoveDir(Direction::Left));
    assert_eq!(d.active_workspace().windows(), &[1, 3, 2]);
    assert_eq!(d.focused_window(), Some(3)); // el foco acompaña a la movida
    // Y a la derecha la devuelve.
    d.apply(DesktopAction::MoveDir(Direction::Right));
    assert_eq!(d.active_workspace().windows(), &[1, 2, 3]);
    assert_eq!(d.focused_window(), Some(3));
}

#[test]
fn move_dir_nudges_a_floating_window() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    open(&mut d, 2); // enfocada → flotar
    d.apply(DesktopAction::ToggleFloat);
    let r0 = d.active_workspace().floating_rect(2).unwrap();
    d.apply(DesktopAction::MoveDir(Direction::Right));
    let r1 = d.active_workspace().floating_rect(2).unwrap();
    // Se desplazó float_step px a la derecha, sin cambiar tamaño.
    assert_eq!(r1.x, r0.x + d.config().float_step());
    assert_eq!((r1.w, r1.h), (r0.w, r0.h));
}

#[test]
fn resize_float_grows_and_shrinks_the_focused_floating_window() {
    let mut d = desktop_with_screen();
    open(&mut d, 1); // enfocada → flotar
    d.apply(DesktopAction::ToggleFloat);
    let r0 = d.active_workspace().floating_rect(1).unwrap();
    let step = d.config().float_step();
    d.apply(DesktopAction::ResizeFloatDir(Direction::Right));
    assert_eq!(d.active_workspace().floating_rect(1).unwrap().w, r0.w + step);
    d.apply(DesktopAction::ResizeFloatDir(Direction::Down));
    assert_eq!(d.active_workspace().floating_rect(1).unwrap().h, r0.h + step);
    // Sobre una teselada no hace nada.
    open(&mut d, 2); // teselada, enfocada
    assert!(d.apply(DesktopAction::ResizeFloatDir(Direction::Right)).is_empty());
}

#[test]
fn focus_and_send_to_output_dir_cross_monitors() {
    let mut d = desktop_with_two_outputs(); // salida 0 a la izq, 1 a la der
    open(&mut d, 1); // en la salida 0 (ws 0)
    assert_eq!(d.active_index(), 0);
    // Foco a la salida de la derecha → su escritorio (ws 1).
    d.apply(DesktopAction::FocusOutputDir(Direction::Right));
    assert_eq!(d.active_index(), 1);
    // Volver a la izquierda.
    d.apply(DesktopAction::FocusOutputDir(Direction::Left));
    assert_eq!(d.active_index(), 0);
    // Mandar la ventana 1 a la salida derecha → viaja al ws 1.
    d.apply(DesktopAction::SendToOutputDir(Direction::Right));
    assert_eq!(d.workspace_loads()[0], 0);
    assert_eq!(d.workspace_loads()[1], 1);
}

#[test]
fn master_step_from_config_drives_grow_master() {
    use crate::config::Config;
    let mut d = desktop_with_screen();
    d.set_config(Config::from_ron("( master_step: 0.1 )").unwrap());
    open(&mut d, 1);
    let r0 = d.active_workspace().params().master_ratio;
    d.apply(DesktopAction::GrowMaster);
    assert!((d.active_workspace().params().master_ratio - (r0 + 0.1)).abs() < 1e-6);
}

#[test]
fn clicked_focuses_even_with_focus_follows_mouse_off() {
    use crate::config::Config;
    let mut d = desktop_with_screen();
    d.set_config(Config::from_ron("( focus_follows_mouse: false )").unwrap());
    open(&mut d, 1);
    open(&mut d, 2); // enfocada
    // El hover ya no enfoca…
    d.on_event(BodyEvent::PointerEntered { id: 1 });
    assert_eq!(d.focused_window(), Some(2));
    // …pero el click sí.
    d.on_event(BodyEvent::Clicked { id: 1 });
    assert_eq!(d.focused_window(), Some(1));
}

#[test]
fn clicked_jumps_to_the_workspace_holding_the_window() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    open(&mut d, 2);
    d.on_event(BodyEvent::Keybind("Super+Shift+3".into())); // la 2 al esc. 3
    assert_eq!(d.active_index(), 0);
    d.on_event(BodyEvent::Clicked { id: 2 });
    assert_eq!(d.active_index(), 2);
    assert_eq!(d.focused_window(), Some(2));
}

#[test]
fn dragging_a_tiled_window_swaps_with_the_window_under_the_pointer() {
    let mut d = desktop_with_screen();
    for id in [1, 2, 3] {
        open(&mut d, id);
    }
    assert_eq!(d.active_workspace().windows(), &[1, 2, 3]);
    // El centro de la tesela de la 3.
    let o = d.outputs()[d.focused_output()];
    let layout = d.active_workspace().layout(o.work_rect());
    let r3 = layout.iter().find(|(id, _)| *id == 3).unwrap().1;
    let (cx, cy) = (r3.x + r3.w / 2, r3.y + r3.h / 2);
    // Arrastrar la 1 sobre la 3 las intercambia, el foco sigue a la 1.
    d.on_event(BodyEvent::WindowDragged { id: 1, x: cx, y: cy });
    assert_eq!(d.active_workspace().windows(), &[3, 2, 1]);
    assert_eq!(d.focused_window(), Some(1));
}

// (El antiguo `window_dragged_ignores_a_floating_window` se eliminó: ahora una
// flotante soltada sobre una tesela VUELVE al mosaico — cubierto por
// `dragging_a_floating_window_over_a_tile_returns_it_to_tiling`.)

#[test]
fn win_tab_salta_solo_a_escritorios_ocupados() {
    let mut d = desktop_with_screen();
    // Sin nada abierto en otros escritorios, Win+Tab no va a ninguna parte
    // (no vaga por vacíos).
    open(&mut d, 1);
    d.apply(DesktopAction::WorkspaceNext);
    assert_eq!(d.active_index(), 0, "sin otro ocupado, se queda");

    // Ocupa el escritorio 2 (manda la 1 allá) → quedan 0 y 2 ocupados.
    open(&mut d, 2);
    d.apply(DesktopAction::SendToWorkspace(2)); // la enfocada (2) va a ws 2
    assert_eq!(d.active_index(), 0);

    // Win+Tab salta a ws 2 SALTEANDO el 1 (vacío).
    d.apply(DesktopAction::WorkspaceNext);
    assert_eq!(d.active_index(), 2);
    // Y de vuelta, con wrap, al 0 (los del medio vacíos se saltan).
    d.apply(DesktopAction::WorkspaceNext);
    assert_eq!(d.active_index(), 0);
    // Prev también respeta ocupados.
    d.apply(DesktopAction::WorkspacePrev);
    assert_eq!(d.active_index(), 2);
}

#[test]
fn toggle_tiling_floats_all_then_restores() {
    let mut d = desktop_with_screen();
    for id in [1, 2, 3] {
        open(&mut d, id);
    }
    assert!([1, 2, 3]
        .iter()
        .all(|&id| !d.active_workspace().is_floating(id)));
    // Todo flota.
    d.apply(DesktopAction::ToggleTiling);
    assert!([1, 2, 3]
        .iter()
        .all(|&id| d.active_workspace().is_floating(id)));
    // Y vuelve al teselado.
    d.apply(DesktopAction::ToggleTiling);
    assert!([1, 2, 3]
        .iter()
        .all(|&id| !d.active_workspace().is_floating(id)));
}

#[test]
fn reload_config_reseeds_params_and_re_sends_decorations() {
    use crate::config::Config;
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    let cfg = Config::from_ron("( gap: 30, border_width: 5 )").unwrap();
    let cmds = d.reload_config(cfg);
    // El gap nuevo se sembró (el archivo manda).
    assert_eq!(d.active_workspace().params().gap, 30);
    // Se devuelve un SetDecorations con el marco nuevo…
    let dec = cmds.iter().find_map(|c| match c {
        BrainCommand::SetDecorations(dec) => Some(dec),
        _ => None,
    });
    assert_eq!(dec.expect("se esperaba un SetDecorations").border_width, 5);
    // …y un re-teselado, para que el gap/borde nuevos se vean al instante
    // (hay una ventana abierta, así que hay algo que colocar).
    assert!(
        cmds.iter().any(|c| matches!(c, BrainCommand::Place(_))),
        "reload_config debe re-teselar las ventanas abiertas: {cmds:?}"
    );
}

#[test]
fn capabilities_emits_the_loaded_policy() {
    let mut d = Desktop::new();
    // Por defecto, permisos vacíos (todo permitido).
    match d.capabilities() {
        BrainCommand::SetCapabilities(p) => assert!(p.clipboard_denylist.is_empty()),
        other => panic!("se esperaba SetCapabilities, no {other:?}"),
    }
    // set_caps reemplaza la política y devuelve el comando a enviar.
    let nueva = Permisos {
        clipboard_denylist: vec!["wl-paste".into()],
        virtual_input_denylist: vec!["wtype".into()],
        window_list_denylist: vec!["lswt".into()],
        screencopy_denylist: vec!["grim".into()],
        dmabuf_denylist: vec!["leak".into()],
    };
    match d.set_caps(nueva.clone()) {
        BrainCommand::SetCapabilities(p) => assert_eq!(p, nueva),
        other => panic!("se esperaba SetCapabilities, no {other:?}"),
    }
    // Y queda fijada: capabilities() la sigue emitiendo.
    match d.capabilities() {
        BrainCommand::SetCapabilities(p) => assert!(!p.clipboard_permitido("/usr/bin/wl-paste")),
        other => panic!("se esperaba SetCapabilities, no {other:?}"),
    }
}

#[test]
fn background_throttle_spaces_unfocused_visible_windows() {
    use crate::config::Config;
    let mut d = desktop_with_screen();
    d.set_config(Config::from_ron("( background_frame_divisor: 3 )").unwrap());
    open(&mut d, 1);
    open(&mut d, 2); // MasterStack: ambas visibles (maestra + pila)
    let cmds = d.apply(DesktopAction::FocusWindow(1));
    let p = places(&cmds);
    // La enfocada va a pleno ritmo.
    let f = p.iter().find(|p| p.id == 1).unwrap();
    assert!(f.focused);
    assert_eq!(f.frame_divisor, 1, "la enfocada no se throttlea");
    // La de fondo (visible, sin foco) se espacia al divisor configurado.
    let bg = p.iter().find(|p| p.id == 2).unwrap();
    assert!(bg.visible && !bg.focused);
    assert_eq!(bg.frame_divisor, 3, "el fondo visible se throttlea");
}

#[test]
fn background_throttle_off_by_default_keeps_full_rate() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    open(&mut d, 2);
    let cmds = d.apply(DesktopAction::FocusWindow(1));
    // Sin configurar (divisor 1), nadie se throttlea.
    assert!(places(&cmds).iter().all(|p| p.frame_divisor == 1));
}

#[test]
fn reload_config_preserves_open_windows_and_focus() {
    use crate::config::Config;
    let mut d = desktop_with_screen();
    for id in [1, 2, 3] {
        open(&mut d, id);
    }
    d.apply(DesktopAction::FocusWindow(2));
    d.reload_config(Config::from_ron("( gap: 14 )").unwrap());
    // Las ventanas siguen ahí, el foco intacto — recargar no las pierde.
    assert_eq!(d.active_workspace().windows(), &[1, 2, 3]);
    assert_eq!(d.focused_window(), Some(2));
    assert_eq!(d.active_workspace().params().gap, 14);
}

#[test]
fn reload_config_applies_focus_follows_mouse_live() {
    use crate::config::Config;
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    open(&mut d, 2); // enfocada
    // Por defecto el hover enfoca.
    d.on_event(BodyEvent::PointerEntered { id: 1 });
    assert_eq!(d.focused_window(), Some(1));
    // Recargar con foco-sigue-ratón apagado lo desactiva en vivo.
    d.reload_config(Config::from_ron("( focus_follows_mouse: false )").unwrap());
    d.on_event(BodyEvent::PointerEntered { id: 2 });
    assert_eq!(d.focused_window(), Some(1)); // el hover ya no mueve el foco
}

#[test]
fn reload_config_applies_the_dropterm_command_live() {
    use crate::config::Config;
    let mut d = desktop_with_screen();
    d.reload_config(
        Config::from_ron("( dropterm_cmd: \"foot --app-id mirada.dropterm\" )").unwrap(),
    );
    let cmds = d.apply(DesktopAction::ToggleDropterm);
    assert_eq!(
        cmds,
        vec![BrainCommand::Spawn("foot --app-id mirada.dropterm".into())]
    );
}

#[test]
fn set_config_seeds_the_layout_params_of_every_workspace() {
    use crate::config::Config;
    let mut d = Desktop::new();
    let cfg = Config::from_ron(r#"( gap: 20, master_ratio: 0.4, layout: "grid" )"#).unwrap();
    d.set_config(cfg);
    d.on_event(BodyEvent::OutputAdded { id: 0, width: 1920, height: 1080 });
    let p = d.active_workspace().params();
    assert_eq!(p.gap, 20);
    assert_eq!(p.mode, LayoutMode::Grid);
    assert!((p.master_ratio - 0.4).abs() < 1e-6);
}

#[test]
fn focus_follows_mouse_can_be_disabled_by_config() {
    use crate::config::Config;
    let mut d = desktop_with_screen();
    d.set_config(Config::from_ron("( focus_follows_mouse: false )").unwrap());
    open(&mut d, 1);
    open(&mut d, 2); // enfocada
    // Con el foco-sigue-ratón apagado, pasar el puntero no cambia el foco.
    d.on_event(BodyEvent::PointerEntered { id: 1 });
    assert_eq!(d.focused_window(), Some(2));
}

#[test]
fn config_sets_the_dropterm_command_and_height() {
    use crate::config::Config;
    let mut d = desktop_with_screen(); // 1920×1080
    d.set_config(
        Config::from_ron("( dropterm_cmd: \"foot --app-id mirada.dropterm\", dropterm_height_pct: 30 )")
            .unwrap(),
    );
    // El spawn perezoso usa el comando de la config.
    let cmds = d.apply(DesktopAction::ToggleDropterm);
    assert_eq!(
        cmds,
        vec![BrainCommand::Spawn("foot --app-id mirada.dropterm".into())]
    );
    // Y al abrirse, baja al 30 % del alto.
    let cmds = d.on_event(BodyEvent::WindowOpened {
        id: 9,
        app_id: DROPTERM_APP_ID.into(),
        title: "t".into(),
    });
    let p = places(&cmds).iter().find(|x| x.id == 9).unwrap();
    assert_eq!(p.rect.h, 1080 * 30 / 100);
}

#[test]
fn a_client_fullscreen_request_is_honoured() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    open(&mut d, 2);
    let cmds = d.on_event(BodyEvent::FullscreenRequest { id: 1, fullscreen: true });
    assert!(places(&cmds).iter().find(|x| x.id == 1).unwrap().fullscreen);
    // El cliente la suelta.
    let cmds = d.on_event(BodyEvent::FullscreenRequest { id: 1, fullscreen: false });
    assert!(!places(&cmds).iter().find(|x| x.id == 1).unwrap().fullscreen);
}

#[test]
fn a_fullscreen_request_for_an_unknown_window_does_nothing() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    assert!(d
        .on_event(BodyEvent::FullscreenRequest { id: 99, fullscreen: true })
        .is_empty());
}

#[test]
fn window_lines_show_a_stashed_window_as_workspace_zero() {
    let mut d = desktop_with_screen();
    open(&mut d, 1);
    d.apply(DesktopAction::SendToScratchpad);
    let line = d.window_lines().into_iter().find(|l| l.id == 1).unwrap();
    assert_eq!(line.workspace, 0);
}

// --- Persistencia de sesión (snapshot/restore) ----------------------

#[test]
fn snapshot_captures_per_workspace_modes_and_the_output_map() {
    let mut d = desktop_with_screen();
    // Cambia el modo del escritorio activo y manda otra salida a otro.
    d.apply(DesktopAction::SetLayout(LayoutMode::Grid));
    let snap = d.snapshot();
    assert_eq!(snap.version, crate::session::SESSION_VERSION);
    assert_eq!(snap.workspaces.len(), WORKSPACE_COUNT);
    assert_eq!(snap.workspaces[0].mode, LayoutMode::Grid);
    // Una salida conectada, mostrando el escritorio 0.
    assert_eq!(snap.output_workspaces, vec![0]);
}

#[test]
fn restore_reapplies_layout_params_to_each_workspace() {
    let snap = {
        let mut d = desktop_with_screen();
        d.apply(DesktopAction::SetLayout(LayoutMode::Spiral));
        d.apply(DesktopAction::IncMaster); // master_count 1 → 2
        d.snapshot()
    };
    // Un escritorio nuevo, sin salidas todavía.
    let mut d = Desktop::new();
    d.restore(&snap);
    // Los params del escritorio 0 se recuperaron.
    assert_eq!(d.workspaces[0].params().mode, LayoutMode::Spiral);
    assert_eq!(d.workspaces[0].params().master_count, 2);
}

#[test]
fn restore_places_each_output_on_its_remembered_workspace() {
    // Sesión: dos salidas, la primera mostraba el escritorio 4, la segunda
    // el 2.
    let snap = DesktopState {
        version: crate::session::SESSION_VERSION,
        workspaces: vec![LayoutParams::default(); WORKSPACE_COUNT],
        output_workspaces: vec![4, 2],
        focused_output: 1,
        window_homes: Vec::new(),
        groupings: Vec::new(),
    };
    let mut d = Desktop::new();
    d.restore(&snap);
    // Las salidas aparecen en orden y recuperan su escritorio.
    d.on_event(BodyEvent::OutputAdded { id: 0, width: 1920, height: 1080 });
    d.on_event(BodyEvent::OutputAdded { id: 1, width: 1920, height: 1080 });
    assert_eq!(d.outputs()[0].workspace, 4);
    assert_eq!(d.outputs()[1].workspace, 2);
    assert_eq!(d.focused_output(), 1);
    assert_eq!(d.active_index(), 2); // la salida enfocada (1) muestra el 2
}

#[test]
fn restore_with_a_conflicting_map_falls_back_to_a_free_workspace() {
    // Ambas salidas pretenden el mismo escritorio: la segunda no puede.
    let snap = DesktopState {
        version: crate::session::SESSION_VERSION,
        workspaces: vec![LayoutParams::default(); WORKSPACE_COUNT],
        output_workspaces: vec![3, 3],
        focused_output: 0,
        window_homes: Vec::new(),
        groupings: Vec::new(),
    };
    let mut d = Desktop::new();
    d.restore(&snap);
    d.on_event(BodyEvent::OutputAdded { id: 0, width: 1920, height: 1080 });
    d.on_event(BodyEvent::OutputAdded { id: 1, width: 1920, height: 1080 });
    assert_eq!(d.outputs()[0].workspace, 3);
    // La segunda cayó al primer escritorio libre (no el 3, ya tomado).
    assert_ne!(d.outputs()[1].workspace, 3);
}

#[test]
fn snapshot_remembers_which_workspace_each_app_lived_on() {
    let mut d = desktop_with_screen();
    open(&mut d, 1); // app1 nace en el escritorio 0…
    d.apply(DesktopAction::SendToWorkspace(2)); // …y se va al índice 2
    assert!(d.snapshot().window_homes.contains(&("app1".to_string(), 2)));
}

#[test]
fn a_reopened_window_returns_to_its_remembered_workspace() {
    let snap = {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        d.apply(DesktopAction::SendToWorkspace(2));
        d.snapshot()
    };
    let mut d = desktop_with_screen();
    d.restore(&snap);
    // app1 reaparece: vuelve al escritorio índice 2, no al activo (0).
    d.on_event(BodyEvent::WindowOpened { id: 1, app_id: "app1".into(), title: "x".into() });
    assert_eq!(d.workspace_loads()[2], 1);
    assert_eq!(d.workspace_loads()[0], 0);
}

#[test]
fn a_rule_beats_a_session_home() {
    let snap = {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        d.apply(DesktopAction::SendToWorkspace(2)); // hogar = índice 2
        d.snapshot()
    };
    let mut d = desktop_with_screen();
    // La regla manda app1 al escritorio 5 (índice 4) — pisa el hogar.
    d.set_rules(Rules::from_ron(r#"( rules: [ (app_id: "app1", workspace: 5) ] )"#).unwrap());
    d.restore(&snap);
    d.on_event(BodyEvent::WindowOpened { id: 1, app_id: "app1".into(), title: "x".into() });
    assert_eq!(d.workspace_loads()[4], 1); // donde dice la regla
    assert_eq!(d.workspace_loads()[2], 0); // no en el hogar de la sesión
}

#[test]
fn a_session_home_is_consumed_after_the_first_window() {
    let snap = {
        let mut d = desktop_with_screen();
        open(&mut d, 1);
        d.apply(DesktopAction::SendToWorkspace(2));
        d.snapshot()
    };
    let mut d = desktop_with_screen();
    d.restore(&snap);
    // Primera ventana de app1 → vuelve al hogar (índice 2).
    d.on_event(BodyEvent::WindowOpened { id: 1, app_id: "app1".into(), title: "x".into() });
    d.on_event(BodyEvent::WindowClosed { id: 1 });
    // Segunda ventana de app1 → el hogar ya se consumió: va al activo (0).
    d.on_event(BodyEvent::WindowOpened { id: 2, app_id: "app1".into(), title: "y".into() });
    assert_eq!(d.workspace_loads()[0], 1);
    assert_eq!(d.workspace_loads()[2], 0);
}

#[test]
fn group_stack_then_zoom_makes_the_stack_absorb_the_screen() {
    let mut d = desktop_with_screen();
    for id in [1, 2, 3, 4] {
        open(&mut d, id); // MasterStack, nmaster=1 → maestra 1, pila 2/3/4
    }
    // Pliega la pila (2,3,4) en un sub-espacio.
    d.apply(DesktopAction::GroupStack);
    assert!(d.active_workspace().is_grouped());
    // Con el foco en la pila, entrar: sólo se ven 2,3,4.
    d.apply(DesktopAction::FocusWindow(3));
    let cmds = d.apply(DesktopAction::ZoomIn);
    let p = places(&cmds);
    // Las visibles son la pila; la maestra 1 queda fuera del zoom.
    let visibles: Vec<_> = p.iter().filter(|p| p.visible).map(|p| p.id).collect();
    assert_eq!(visibles.len(), 3);
    assert!(visibles.contains(&2) && visibles.contains(&3) && visibles.contains(&4));
    // Pero la 1 no se omite: se lista dormida (suspended) para cortarle los
    // frames, no oculta a ciegas por ausencia.
    let one = p.iter().find(|p| p.id == 1).unwrap();
    assert!(one.suspended && !one.visible);
    // Salir y deshacer: vuelven las cuatro.
    d.apply(DesktopAction::ZoomOut);
    let cmds = d.apply(DesktopAction::Ungroup);
    assert_eq!(places(&cmds).len(), 4);
    assert!(!d.active_workspace().is_grouped());
}

#[test]
fn group_stack_nests_inside_the_current_zoom_level() {
    let mut d = desktop_with_screen();
    for id in [1, 2, 3, 4] {
        open(&mut d, id); // MasterStack nmaster=1 → maestra 1, pila 2/3/4
    }
    // Nivel 1: plegar la pila (2,3,4) y entrar.
    d.apply(DesktopAction::GroupStack);
    d.apply(DesktopAction::FocusWindow(3));
    d.apply(DesktopAction::ZoomIn);
    assert_eq!(d.active_workspace().zoom_depth(), 1);
    // Nivel 2: dentro del grupo, plegar SU pila (3,4) — la maestra del nivel
    // es la 2 — y entrar otra vez.
    d.apply(DesktopAction::GroupStack);
    d.apply(DesktopAction::FocusWindow(4));
    let cmds = d.apply(DesktopAction::ZoomIn);
    assert_eq!(d.active_workspace().zoom_depth(), 2);
    // En el nivel más profundo sólo se ven 3 y 4; 1 y 2 duermen.
    let p = places(&cmds);
    let visibles: Vec<_> = p.iter().filter(|p| p.visible).map(|p| p.id).collect();
    assert_eq!(visibles.len(), 2);
    assert!(visibles.contains(&3) && visibles.contains(&4));
    for id in [1, 2] {
        assert!(p.iter().find(|p| p.id == id).unwrap().suspended);
    }
}

#[test]
fn a_snapshot_round_trips_through_restore() {
    let mut d = desktop_with_screen();
    d.apply(DesktopAction::SetLayout(LayoutMode::CenteredMaster));
    let snap = d.snapshot();
    let mut d2 = Desktop::new();
    d2.restore(&snap);
    assert_eq!(d2.snapshot().workspaces, snap.workspaces);
}

/// Reabre una ventana con un `app_id` explícito (los ids nuevos difieren de
/// los de la sesión previa, como pasa con clientes Wayland que reconectan).
fn open_app(d: &mut Desktop, id: WindowId, app_id: &str) -> Vec<BrainCommand> {
    d.on_event(BodyEvent::WindowOpened {
        id,
        app_id: app_id.into(),
        title: format!("win {id}"),
    })
}

/// Reporta el linaje de una ventana (como haría el Cuerpo tras abrirla).
fn lineage(d: &mut Desktop, id: WindowId, pid: u32, ancestors: Vec<u32>) {
    d.on_event(BodyEvent::WindowLineage { id, pid, ancestors });
}

#[test]
fn group_constellation_folds_the_focused_windows_family() {
    let mut d = desktop_with_screen();
    for id in [1, 2, 3, 4] {
        open(&mut d, id);
    }
    // 2 es una terminal (pid 100); 3 es un editor que lanzó (ancestro 100).
    // 1 y 4 no tienen parentesco.
    lineage(&mut d, 1, 10, vec![1]);
    lineage(&mut d, 2, 100, vec![1]);
    lineage(&mut d, 3, 102, vec![100, 1]);
    lineage(&mut d, 4, 40, vec![1]);
    d.apply(DesktopAction::FocusWindow(2));
    d.apply(DesktopAction::GroupConstellation);
    assert!(d.active_workspace().is_grouped());
    // Entrar al grupo muestra la constelación {2,3}, no 1 ni 4.
    d.apply(DesktopAction::FocusWindow(3));
    let cmds = d.apply(DesktopAction::ZoomIn);
    let visibles: Vec<_> = places(&cmds)
        .iter()
        .filter(|p| p.visible)
        .map(|p| p.id)
        .collect();
    assert_eq!(visibles.len(), 2);
    assert!(visibles.contains(&2) && visibles.contains(&3));
}

#[test]
fn focus_constellation_jumps_between_activity_families() {
    let mut d = desktop_with_screen();
    for id in [1, 2, 3, 4] {
        open(&mut d, id);
    }
    // Dos familias: {1,2} (1 lanzó 2) y {3,4} (3 lanzó 4).
    lineage(&mut d, 1, 100, vec![1]);
    lineage(&mut d, 2, 110, vec![100, 1]);
    lineage(&mut d, 3, 300, vec![1]);
    lineage(&mut d, 4, 310, vec![300, 1]);
    d.apply(DesktopAction::FocusWindow(2)); // foco en la familia {1,2}
    d.apply(DesktopAction::FocusConstellationNext);
    // Salta a la otra constelación, a su primer miembro (la 3).
    assert_eq!(d.focused_window(), Some(3));
    // Otra vez vuelve cíclicamente a {1,2} (primer miembro: la 1).
    d.apply(DesktopAction::FocusConstellationNext);
    assert_eq!(d.focused_window(), Some(1));
    // Y hacia atrás otra vez a {3,4}.
    d.apply(DesktopAction::FocusConstellationPrev);
    assert_eq!(d.focused_window(), Some(3));
}

#[test]
fn focus_constellation_is_a_noop_with_a_single_family() {
    let mut d = desktop_with_screen();
    for id in [1, 2] {
        open(&mut d, id);
    }
    lineage(&mut d, 1, 100, vec![1]);
    lineage(&mut d, 2, 110, vec![100, 1]); // ambas en la misma familia
    d.apply(DesktopAction::FocusWindow(1));
    d.apply(DesktopAction::FocusConstellationNext);
    assert_eq!(d.focused_window(), Some(1)); // no hay otra a la que saltar
}

#[test]
fn group_constellation_does_nothing_for_a_lone_window() {
    let mut d = desktop_with_screen();
    for id in [1, 2] {
        open(&mut d, id);
    }
    // Sin parentesco entre ellas.
    lineage(&mut d, 1, 10, vec![1]);
    lineage(&mut d, 2, 20, vec![1]);
    d.apply(DesktopAction::FocusWindow(1));
    d.apply(DesktopAction::GroupConstellation);
    // La constelación de la 1 es ella sola → no se agrupa.
    assert!(!d.active_workspace().is_grouped());
}

#[test]
fn closing_a_window_forgets_its_lineage() {
    let mut d = desktop_with_screen();
    for id in [1, 2, 3] {
        open(&mut d, id);
    }
    lineage(&mut d, 1, 10, vec![1]);
    lineage(&mut d, 2, 100, vec![1]);
    lineage(&mut d, 3, 102, vec![100, 1]); // 3 desciende de 2
    // Cierro la 2: 3 pierde el puente, ya no forma constelación con nadie.
    d.on_event(BodyEvent::WindowClosed { id: 2 });
    d.apply(DesktopAction::FocusWindow(3));
    d.apply(DesktopAction::GroupConstellation);
    assert!(!d.active_workspace().is_grouped());
}

#[test]
fn a_grouping_is_captured_in_the_snapshot_by_app_id() {
    use crate::session::NodeShape;
    let mut d = desktop_with_screen();
    for id in [1, 2, 3, 4] {
        open(&mut d, id); // app_id = app1..app4
    }
    d.apply(DesktopAction::GroupStack); // root: [app1] + grupo{app2,app3,app4}
    let snap = d.snapshot();
    assert_eq!(snap.groupings.len(), 1);
    let (n, shape) = &snap.groupings[0];
    assert_eq!(*n, 0);
    // Tope: la maestra suelta + un sub-espacio con la pila.
    assert_eq!(shape.children.len(), 2);
    assert!(matches!(&shape.children[0], NodeShape::Leaf(a) if a == "app1"));
    match &shape.children[1] {
        NodeShape::Space(s) => {
            let leaves: Vec<_> = s
                .children
                .iter()
                .map(|c| match c {
                    NodeShape::Leaf(a) => a.as_str(),
                    _ => "?",
                })
                .collect();
            assert_eq!(leaves, vec!["app2", "app3", "app4"]);
        }
        _ => panic!("se esperaba un sub-espacio"),
    }
}

#[test]
fn a_grouping_rematerializes_when_all_member_apps_reopen() {
    // Sesión previa: cuatro apps, la pila plegada.
    let mut d = desktop_with_screen();
    for id in [1, 2, 3, 4] {
        open(&mut d, id);
    }
    d.apply(DesktopAction::GroupStack);
    let snap = d.snapshot();

    // Arranque nuevo: restauro y reabro las apps con ids DISTINTOS.
    let mut d2 = desktop_with_screen();
    d2.restore(&snap);
    assert!(!d2.active_workspace().is_grouped(), "sin ventanas, nada que agrupar");
    open_app(&mut d2, 50, "app1");
    open_app(&mut d2, 60, "app3");
    open_app(&mut d2, 70, "app2");
    // Falta app4: la agrupación sigue pendiente.
    assert!(!d2.active_workspace().is_grouped());
    open_app(&mut d2, 80, "app4");
    // Completo el cuadro → se rematerializa.
    assert!(d2.active_workspace().is_grouped());
    // Y entrar al grupo muestra la pila (las tres no-maestras), no la 50.
    d2.apply(DesktopAction::FocusWindow(60));
    let cmds = d2.apply(DesktopAction::ZoomIn);
    let visibles: Vec<_> = places(&cmds)
        .iter()
        .filter(|p| p.visible)
        .map(|p| p.id)
        .collect();
    assert_eq!(visibles.len(), 3);
    assert!(visibles.contains(&60) && visibles.contains(&70) && visibles.contains(&80));
    assert!(!visibles.contains(&50)); // app1 (maestra) queda fuera del zoom
}

#[test]
fn a_grouping_with_an_anonymous_window_is_not_persisted() {
    let mut d = desktop_with_screen();
    open(&mut d, 1); // app1
    open_app(&mut d, 2, ""); // sin app_id: no se puede anclar
    open(&mut d, 3); // app3
    d.apply(DesktopAction::GroupStack);
    assert!(d.active_workspace().is_grouped());
    // No se persiste: una hoja no tiene `app_id` que sobreviva al reinicio.
    assert!(d.snapshot().groupings.is_empty());
}

// Suprimir el warning de Output importado pero no usado directamente en las aserciones.
fn _use_output_type(_: &Output) {}
