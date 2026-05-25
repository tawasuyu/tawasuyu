//! Hot-reload del `layout.json` vía `notify` watcher.
//!
//! Anatomía:
//! 1. Un thread del SO corre el watcher (`notify::recommended_watcher`) que
//!    spawnea su propio thread de polling. Cuando detecta cambios en el
//!    archivo objetivo, manda `()` por un `std::sync::mpsc::channel`.
//! 2. Una task de gpui (`cx.spawn`) hace `try_recv` cada N ms (timer en el
//!    `background_executor`). Si llega algo, relee el JSON y actualiza el
//!    `LayoutModel` con `replace_tree`.
//!
//! Esquema separado intencional: notify trabaja en hilos del SO (no
//! integra con el executor de gpui), así que rebotamos vía mpsc para no
//! tocar entities desde threads ajenos. El tradeoff es una latencia de
//! poll N (250ms por default) — imperceptible para edición manual de un
//! JSON.
//!
//! Ignoramos cambios cuando el JSON quedó inválido (parse error) — el
//! `LayerConfig::load_or_default` cae al árbol default. Si querés que la
//! UI muestre el error, agregar un AppEvent::ConfigError y un toast en
//! Fase 8.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use gpui::{App, AsyncApp, Entity};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use nahual_core::LayerConfig;

use crate::layout_model::LayoutModel;

/// Frecuencia de polling del receiver. 250ms es el sweet spot:
/// suficientemente rápido para sentirse "instantáneo" pero sin gastar CPU.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Spawnea el watcher + el polling task. Devuelve el `RecommendedWatcher`
/// — el caller debe mantenerlo vivo (drop ⇒ stop watching). Por
/// conveniencia retorna también nada más; el caller suele guardar el
/// watcher en una global o filed-leakeada.
pub fn spawn_watch(
    path: PathBuf,
    model: Entity<LayoutModel>,
    cx: &mut App,
) -> notify::Result<RecommendedWatcher> {
    let (tx, rx) = channel::<()>();

    // Watcher: el cierre se ejecuta en el thread que `notify` provee. Solo
    // empujamos `()` al canal — el side mpsc maneja toda la lógica.
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            // Solo nos interesan modify/create — los Access se ignoran
            // para no triggerear en lecturas (ej. cat).
            if matches!(
                ev.kind,
                notify::EventKind::Modify(_)
                    | notify::EventKind::Create(_)
                    | notify::EventKind::Remove(_)
            ) {
                let _ = tx.send(());
            }
        }
    })?;

    // Watcheamos el directorio padre, no el archivo en sí. Muchos editores
    // hacen "rename + create" al guardar (atomic write), lo que rompe
    // watching del file directo. Ver el dir y filtrar por path es robusto.
    let parent = path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    watcher.watch(&parent, RecursiveMode::NonRecursive)?;

    // Spawnea el polling task en el ForegroundExecutor para poder llamar
    // model.update sin cross-thread issues.
    let path_for_task = path.clone();
    cx.foreground_executor()
        .spawn(poll_loop(rx, path_for_task, model, cx.to_async()))
        .detach();

    Ok(watcher)
}

async fn poll_loop(
    rx: Receiver<()>,
    path: PathBuf,
    model: Entity<LayoutModel>,
    mut cx: AsyncApp,
) {
    let timer = cx.background_executor().clone();
    loop {
        timer.timer(POLL_INTERVAL).await;
        // Drenamos todos los eventos acumulados en este ciclo —
        // múltiples writes seguidos colapsan a UN solo reload.
        let mut had_event = false;
        while rx.try_recv().is_ok() {
            had_event = true;
        }
        if !had_event {
            continue;
        }

        // Releemos el JSON desde disco. Si parsea bien, replace_tree;
        // si no, el `load_or_default` cae al default (no rompe la UI).
        let tree = LayerConfig::load_or_default(path.to_string_lossy().as_ref());
        let _ = model.update(&mut cx, |m, cx| m.replace_tree(tree, cx));
    }
}
