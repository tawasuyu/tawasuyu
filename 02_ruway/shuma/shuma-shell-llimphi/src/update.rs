//! Helpers de `update`: ruteo de mensajes, resolución de instancias, atajos.

use super::*;

/// Enruta un `ModuleMsg` al `update` del módulo correspondiente, y se
/// encarga de interceptar mensajes que el chasis quiera promocionar
/// (p. ej. el click en la command bar abre el drawer).
pub(crate) fn apply_module_msg(mut m: Model, slot: Slot, msg: ModuleMsg) -> Model {
    // Hook: click en la command bar (que llega como `ToggleMode`) abre
    // el drawer si está cerrado. Si ya está abierto, deja que el módulo
    // togglee su modo libremente.
    // Hook: el `shuma-module-canvas` pide insertar una referencia
    // `%cN`/`%pN` en el input del shell. Buscamos la primera instancia
    // `Shell` (en el mismo orden que `sync_canvas_from_primary_shell`)
    // y le mandamos `InsertAtCursor`. Si la shell vive en una tab,
    // la enfocamos. La variante NO se propaga al canvas — el canvas
    // solo emite la intención.
    if let ModuleMsg::Canvas(shuma_module_canvas::Msg::InsertRef(text)) = &msg {
        if let Some(target) = first_shell_slot(&m) {
            let insert_msg =
                ModuleMsg::Shell(shuma_module_shell::Msg::InsertAtCursor(text.clone()));
            if let Slot::Tab(i) = &target {
                m.active_tab = *i;
            }
            return apply_module_msg(m, target, insert_msg);
        }
        // Sin shell activo: el pedido se descarta silencioso.
        return m;
    }

    match slot {
        Slot::TopBar => {
            if let Some(inst) = m.topbar.as_mut() {
                route_to_instance(inst, msg);
            }
        }
        Slot::BottomBar => {
            if let Some(inst) = m.bottombar.as_mut() {
                route_to_instance(inst, msg);
            }
        }
        Slot::Main => {
            if let Some(inst) = m.main.as_mut() {
                route_to_instance(inst, msg);
            }
        }
        Slot::Tab(idx) => {
            if let Some(inst) = m.tabs.get_mut(idx) {
                route_to_instance(inst, msg);
            }
        }
    }
    m
}

/// Mapea una entrada genérica `SlotEntry` del shumarc a una `Instance`.
/// `None` si el `module` no matchea ningún `Kind` compilado — se
/// imprime warning en lugar de fallar para no romper el arranque.
pub(crate) fn resolve_slot(entry: Option<&config::SlotEntry>) -> Option<Instance> {
    let entry = entry?;
    resolve_instance(
        &entry.module,
        entry.source.clone(),
        entry.label.clone(),
        entry.inventory.as_deref(),
    )
}

pub(crate) fn resolve_tab(entry: &config::TabEntry) -> Option<Instance> {
    resolve_instance(
        &entry.id,
        entry.source.clone(),
        entry.label.clone(),
        entry.inventory.as_deref(),
    )
}

pub(crate) fn resolve_instance(
    id: &str,
    source: Source,
    label: Option<String>,
    inventory: Option<&std::path::Path>,
) -> Option<Instance> {
    let label = label.unwrap_or_else(|| source.label());
    match id {
        shuma_module_launcher::ID => Some(Instance::launcher(
            shuma_module_launcher::State::from_apps_dir(),
        )),
        shuma_module_commandbar::ID => Some(Instance::command_bar(
            shuma_module_commandbar::State::default(),
        )),
        shuma_module_shell::ID => Some(Instance::shell(label, source)),
        shuma_module_matilda::ID => {
            Some(Instance::matilda_with_inventory(label, source, inventory))
        }
        shuma_module_minga::ID => Some(Instance::minga(label, source)),
        shuma_module_canvas::ID => Some(Instance::canvas(label)),
        unknown => {
            eprintln!("shuma: módulo desconocido «{unknown}» — se ignora");
            None
        }
    }
}

/// Fallback al inventario de ejemplo cuando el path declarado falla
/// — replica el default de `State::new` sin perder el path para reloads.
pub(crate) fn example_inventory_fallback() -> matilda_core::Inventory {
    shuma_module_matilda::example_inventory()
}

/// Lee un inventario JSON desde un path. Errores van a stderr y la
/// función retorna `None` — el chasis cae al ejemplo en lugar de
/// fallar el arranque (mismo criterio que el config TOML malformado).
pub(crate) fn load_matilda_inventory(path: &std::path::Path) -> Option<matilda_core::Inventory> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "shuma: no se pudo leer inventario {} ({e}) — uso ejemplo",
                path.display()
            );
            return None;
        }
    };
    match serde_json::from_str::<matilda_core::Inventory>(&text) {
        Ok(inv) => Some(inv),
        Err(e) => {
            eprintln!(
                "shuma: inventario {} mal formado ({e}) — uso ejemplo",
                path.display()
            );
            None
        }
    }
}

/// Recolecta las `ModuleContributions` de todas las instancias vivas.
/// Devuelve un `Vec<(Slot, ModuleContributions)>` para que el caller
/// sepa de qué módulo viene cada monitor/shortcut.
pub(crate) fn collect_contributions(model: &Model) -> Vec<(Slot, ModuleContributions)> {
    let mut out: Vec<(Slot, ModuleContributions)> = Vec::new();

    let push = |out: &mut Vec<(Slot, ModuleContributions)>, slot: Slot, inst: &Instance| {
        let c = match &inst.state {
            ModuleState::Launcher(s) => shuma_module_launcher::contributions(s),
            ModuleState::CommandBar(s) => shuma_module_commandbar::contributions(s),
            ModuleState::Shell(s) => shuma_module_shell::contributions(s),
            ModuleState::Matilda(s) => shuma_module_matilda::contributions(s),
            ModuleState::Minga(s) => shuma_module_minga::contributions(s),
            ModuleState::Canvas(s) => shuma_module_canvas::contributions(s),
        };
        out.push((slot, c));
    };

    if let Some(inst) = &model.topbar {
        push(&mut out, Slot::TopBar, inst);
    }
    if let Some(inst) = &model.bottombar {
        push(&mut out, Slot::BottomBar, inst);
    }
    if let Some(inst) = &model.main {
        push(&mut out, Slot::Main, inst);
    }
    for (i, inst) in model.tabs.iter().enumerate() {
        push(&mut out, Slot::Tab(i), inst);
    }
    out
}

/// Muestrea **todos** los monitores extra (los aporta cada módulo
/// activo) e inserta el último valor en su buffer del modelo.
/// Recorta cada buffer a `HISTORY` muestras.
pub(crate) fn sample_extra_monitors(m: &mut Model) {
    let contribs = collect_contributions(m);
    for (slot, c) in contribs {
        for spec in &c.monitors {
            let key = monitor_key(&slot, spec);
            let sample = (spec.sampler)();
            let entry = m.extra_history.entry(key.clone()).or_default();
            entry.push(sample.value);
            if entry.len() > HISTORY {
                let excess = entry.len() - HISTORY;
                entry.drain(0..excess);
            }
            m.extra_display.insert(key, sample.display);
        }
    }
}

/// Aplica `Msg::Tick` a cada `Instance` de tipo `Shell` activa para que
/// drene la salida streamed de `shuma-exec`. Llamado a cadencia rápida
/// (`SHELL_TICK`) sin tocar el muestreo de sysmon (`TICK`).
///
/// Después de drenar, sincroniza el `intent_graph` de la primera shell
/// encontrada hacia todas las instancias `Canvas` activas — el lienzo
/// de contexto refleja en tiempo real los `%cN`/`%pN` del shell.
pub(crate) fn drain_shell_instances(m: &mut Model) {
    fn tick_one(inst: &mut Instance) {
        if let ModuleState::Shell(s) = &mut inst.state {
            *s = shuma_module_shell::update(s.clone(), shuma_module_shell::Msg::Tick);
        }
    }
    if let Some(inst) = m.topbar.as_mut() {
        tick_one(inst);
    }
    if let Some(inst) = m.bottombar.as_mut() {
        tick_one(inst);
    }
    if let Some(inst) = m.main.as_mut() {
        tick_one(inst);
    }
    for inst in m.tabs.iter_mut() {
        tick_one(inst);
    }
    sync_canvas_from_primary_shell(m);
}

/// Toma el `intent_graph` de la primera instancia `Shell` encontrada
/// (en orden: topbar, bottombar, main, drawer tabs) y lo empuja a cada
/// instancia `Canvas` activa vía `Msg::SyncGraph`. Si no hay shells, el
/// canvas mantiene lo último que tenía (incluyendo su grafo de demo).
pub(crate) fn sync_canvas_from_primary_shell(m: &mut Model) {
    let snapshot = find_primary_shell_graph(m);
    let Some(graph) = snapshot else { return };
    let sync_one = |inst: &mut Instance| {
        if let ModuleState::Canvas(s) = &mut inst.state {
            *s = shuma_module_canvas::update(
                s.clone(),
                shuma_module_canvas::Msg::SyncGraph(graph.clone()),
            );
        }
    };
    if let Some(inst) = m.topbar.as_mut() {
        sync_one(inst);
    }
    if let Some(inst) = m.bottombar.as_mut() {
        sync_one(inst);
    }
    if let Some(inst) = m.main.as_mut() {
        sync_one(inst);
    }
    for inst in m.tabs.iter_mut() {
        sync_one(inst);
    }
}

/// Slot del primer `Shell` activo siguiendo el mismo orden que
/// `find_primary_shell_graph`. Lo usa el hook de `Msg::Canvas(InsertRef)`
/// para encontrar a quién enrutarle el `InsertAtCursor`.
pub(crate) fn first_shell_slot(m: &Model) -> Option<Slot> {
    if matches!(
        m.topbar.as_ref().map(|i| &i.state),
        Some(ModuleState::Shell(_))
    ) {
        return Some(Slot::TopBar);
    }
    if matches!(
        m.bottombar.as_ref().map(|i| &i.state),
        Some(ModuleState::Shell(_))
    ) {
        return Some(Slot::BottomBar);
    }
    if matches!(
        m.main.as_ref().map(|i| &i.state),
        Some(ModuleState::Shell(_))
    ) {
        return Some(Slot::Main);
    }
    m.tabs.iter().enumerate().find_map(|(i, inst)| {
        if matches!(inst.state, ModuleState::Shell(_)) {
            Some(Slot::Tab(i))
        } else {
            None
        }
    })
}

pub(crate) fn find_primary_shell_graph(m: &Model) -> Option<shuma_intent::SessionGraph> {
    let pick = |inst: &Instance| match &inst.state {
        ModuleState::Shell(s) => Some(s.intent_graph().clone()),
        _ => None,
    };
    if let Some(inst) = m.topbar.as_ref() {
        if let Some(g) = pick(inst) {
            return Some(g);
        }
    }
    if let Some(inst) = m.bottombar.as_ref() {
        if let Some(g) = pick(inst) {
            return Some(g);
        }
    }
    if let Some(inst) = m.main.as_ref() {
        if let Some(g) = pick(inst) {
            return Some(g);
        }
    }
    for inst in &m.tabs {
        if let Some(g) = pick(inst) {
            return Some(g);
        }
    }
    None
}

pub(crate) fn monitor_key(slot: &Slot, spec: &MonitorSpec) -> String {
    let slot_label = match slot {
        Slot::TopBar => "topbar",
        Slot::BottomBar => "bottombar",
        Slot::Main => "main",
        Slot::Tab(i) => return format!("tab:{i}/{}", spec.id),
    };
    format!("{slot_label}/{}", spec.id)
}

/// Resuelve un `ShortcutClicked` en una transición concreta del
/// modelo. Las tres variantes:
///
/// - `Command(line)` — por ahora, sólo se loguea en el log de Matilda
///   si está disponible; la ejecución real va con la integración del
///   REPL.
/// - `FocusTab(target)` — busca una tab con `Kind::id() == target` y la
///   activa.
/// - `ModuleAction(action_id)` — dispatcha al módulo emisor vía su
///   `dispatch(action_id) -> Option<Msg>`.
pub(crate) fn handle_shortcut(
    mut m: Model,
    slot: Slot,
    action: ShortcutAction,
    handle: &Handle<Msg>,
) -> Model {
    match action {
        ShortcutAction::Command { line } => {
            // Hack temporario: lo agregamos al log del primer matilda
            // que encontremos para que el usuario vea feedback.
            if let Some(inst) = m
                .tabs
                .iter_mut()
                .find(|i| matches!(i.state, ModuleState::Matilda(_)))
            {
                if let ModuleState::Matilda(s) = &mut inst.state {
                    s.log.push(format!("? command: {line}"));
                }
            }
        }
        ShortcutAction::FocusTab { target } => {
            if let Some(i) = m.tabs.iter().position(|inst| inst.kind.id() == target) {
                m.active_tab = i;
            }
        }
        ShortcutAction::ModuleAction { action_id } => {
            // Reload del inventario: el path lo lleva el State del
            // módulo (cargado por el chasis al construir la instancia
            // desde el shumarc). Sirve para Local y Remote por igual.
            if action_id == "matilda.reload" {
                if let Some(path) = matilda_inventory_path(&slot, &m) {
                    let mmsg = match load_matilda_inventory(&path) {
                        Some(inv) => shuma_module_matilda::Msg::SetDesired(inv),
                        None => shuma_module_matilda::Msg::LogLine(format!(
                            "✘ reload: ver stderr ({})",
                            path.display()
                        )),
                    };
                    return apply_module_msg(m, slot, ModuleMsg::Matilda(mmsg));
                } else {
                    return apply_module_msg(
                        m,
                        slot,
                        ModuleMsg::Matilda(shuma_module_matilda::Msg::LogLine(
                            "✘ sin inventory_path: agregá `inventory = …` al shumarc".into(),
                        )),
                    );
                }
            }
            // Hooks remotos: ciertas acciones de matilda necesitan
            // SSH + tokio. Las delegamos a un thread (`Handle::spawn`)
            // que al volver dispatcha un Msg con el resultado.
            if let Some((source, desired)) = remote_matilda_inputs(&slot, &m) {
                if action_id == "matilda.discover" {
                    m = apply_module_msg(
                        m,
                        slot.clone(),
                        ModuleMsg::Matilda(shuma_module_matilda::Msg::LogLine(format!(
                            "→ conectando a {} para discover…",
                            source.label()
                        ))),
                    );
                    let slot_back = slot.clone();
                    handle.spawn(move || {
                        let msg =
                            match shuma_module_matilda::discover_remote_blocking(&source, &desired)
                            {
                                Ok(inv) => shuma_module_matilda::Msg::SetCurrent(inv),
                                Err(e) => shuma_module_matilda::Msg::LogLine(format!(
                                    "✘ discover remoto: {e}"
                                )),
                            };
                        Msg::Module(slot_back, ModuleMsg::Matilda(msg))
                    });
                    return m;
                }
                if action_id == "matilda.dry_run" {
                    m = apply_module_msg(
                        m,
                        slot.clone(),
                        ModuleMsg::Matilda(shuma_module_matilda::Msg::LogLine(format!(
                            "→ dry-run remoto en {} (sin tocar nada)…",
                            source.label()
                        ))),
                    );
                    let slot_back = slot.clone();
                    handle.spawn(move || {
                        let msg = match shuma_module_matilda::dry_run_remote_blocking(
                            &source, &desired,
                        ) {
                            Ok(lines) => shuma_module_matilda::Msg::DryRunReport(lines),
                            Err(e) => {
                                shuma_module_matilda::Msg::LogLine(format!("✘ dry-run remoto: {e}"))
                            }
                        };
                        Msg::Module(slot_back, ModuleMsg::Matilda(msg))
                    });
                    return m;
                }
                if action_id == "matilda.apply" {
                    m = apply_module_msg(
                        m,
                        slot.clone(),
                        ModuleMsg::Matilda(shuma_module_matilda::Msg::LogLine(format!(
                            "→ apply remoto en {} por SSH…",
                            source.label()
                        ))),
                    );
                    let slot_back = slot.clone();
                    handle.spawn(move || {
                        let msg =
                            match shuma_module_matilda::apply_remote_blocking(&source, &desired) {
                                Ok((lines, new_current)) => {
                                    shuma_module_matilda::Msg::ApplyReport { lines, new_current }
                                }
                                Err(e) => shuma_module_matilda::Msg::LogLine(format!(
                                    "✘ apply remoto: {e}"
                                )),
                            };
                        Msg::Module(slot_back, ModuleMsg::Matilda(msg))
                    });
                    return m;
                }
            }
            // Minga refresh: el módulo es "declarativo" en update (no
            // toca sled) — el load real lo hacemos acá en un thread y
            // reenviamos el snapshot como SnapshotReady.
            if action_id == "minga.refresh" {
                if let Some(repo_path) = minga_repo_path(&slot, &m) {
                    let slot_back = slot.clone();
                    handle.spawn(move || {
                        let result = shuma_module_minga::load_snapshot(&repo_path);
                        Msg::Module(
                            slot_back,
                            ModuleMsg::Minga(shuma_module_minga::Msg::SnapshotReady(result)),
                        )
                    });
                    // Y también marcar el state como "refreshing".
                    return apply_module_msg(
                        m,
                        slot,
                        ModuleMsg::Minga(shuma_module_minga::Msg::Refresh),
                    );
                }
            }
            // Minga verify_all: recorre las raíces del snapshot y las
            // verifica una por una en un thread.
            if action_id == "minga.verify_all" {
                if let (Some(repo_path), Some(alphas)) =
                    (minga_repo_path(&slot, &m), minga_visible_alphas(&slot, &m))
                {
                    let slot_back = slot.clone();
                    handle.spawn(move || {
                        let results = shuma_module_minga::verify_all_blocking(&repo_path, &alphas);
                        Msg::Module(
                            slot_back,
                            ModuleMsg::Minga(shuma_module_minga::Msg::VerifyAllReady(results)),
                        )
                    });
                    return apply_module_msg(
                        m,
                        slot,
                        ModuleMsg::Minga(shuma_module_minga::Msg::VerifyAll),
                    );
                }
            }
            let msg = dispatch_to_module(&slot, &m, action_id);
            if let Some(mmsg) = msg {
                m = apply_module_msg(m, slot, mmsg);
            }
        }
    }
    m
}

/// Path del repo Minga de un slot que aloje el módulo minga.
pub(crate) fn minga_repo_path(slot: &Slot, model: &Model) -> Option<std::path::PathBuf> {
    let inst = match slot {
        Slot::TopBar => model.topbar.as_ref()?,
        Slot::BottomBar => model.bottombar.as_ref()?,
        Slot::Main => model.main.as_ref()?,
        Slot::Tab(i) => model.tabs.get(*i)?,
    };
    match &inst.state {
        ModuleState::Minga(s) => Some(s.repo_path.clone()),
        _ => None,
    }
}

/// Lista de α-hashes de las raíces actualmente visibles en el snapshot
/// del módulo minga. `None` si el slot no es minga o no tiene snapshot
/// cargado todavía.
pub(crate) fn minga_visible_alphas(
    slot: &Slot,
    model: &Model,
) -> Option<Vec<minga_core::ContentHash>> {
    let inst = match slot {
        Slot::TopBar => model.topbar.as_ref()?,
        Slot::BottomBar => model.bottombar.as_ref()?,
        Slot::Main => model.main.as_ref()?,
        Slot::Tab(i) => model.tabs.get(*i)?,
    };
    match &inst.state {
        ModuleState::Minga(s) => s
            .snapshot
            .as_ref()
            .map(|snap| snap.recent.iter().map(|r| r.alpha).collect()),
        _ => None,
    }
}

/// Si la tab activa (o el slot Main, si lo hay) es un shell, genera el
/// `Msg::Module` que reenvía la tecla. El módulo shell distingue Enter
/// (submit) de inserción de texto internamente.
/// Rutea la rueda del mouse al shell focado (mismo orden de prioridad
/// que las teclas). `dpx` ya viene en px (positivo = ver historial).
pub(crate) fn forward_wheel_to_focused_shell(model: &Model, dpx: f32) -> Option<Msg> {
    if let Some(inst) = model.main.as_ref() {
        if matches!(inst.state, ModuleState::Shell(_)) {
            return Some(Msg::Module(
                Slot::Main,
                ModuleMsg::Shell(shuma_module_shell::Msg::Scroll(dpx)),
            ));
        }
    }
    if let Some(inst) = model.tabs.get(model.active_tab) {
        if matches!(inst.state, ModuleState::Shell(_)) {
            return Some(Msg::Module(
                Slot::Tab(model.active_tab),
                ModuleMsg::Shell(shuma_module_shell::Msg::Scroll(dpx)),
            ));
        }
    }
    None
}

pub(crate) fn forward_key_to_focused_shell(model: &Model, e: &KeyEvent) -> Option<Msg> {
    // 1) Slot Main siempre gana — si está configurado como shell, las
    //    teclas van ahí. Permite al usuario poner el shell como módulo
    //    principal de la ventana.
    if let Some(inst) = model.main.as_ref() {
        if matches!(inst.state, ModuleState::Shell(_)) {
            return Some(Msg::Module(
                Slot::Main,
                ModuleMsg::Shell(shuma_module_shell::Msg::Key(e.clone())),
            ));
        }
    }
    // 2) Tab activo, si es un shell.
    if let Some(inst) = model.tabs.get(model.active_tab) {
        if matches!(inst.state, ModuleState::Shell(_)) {
            return Some(Msg::Module(
                Slot::Tab(model.active_tab),
                ModuleMsg::Shell(shuma_module_shell::Msg::Key(e.clone())),
            ));
        }
    }
    None
}

/// Path del inventario JSON de un slot de matilda, si lo tiene cargado.
pub(crate) fn matilda_inventory_path(slot: &Slot, model: &Model) -> Option<std::path::PathBuf> {
    let inst = match slot {
        Slot::TopBar => model.topbar.as_ref()?,
        Slot::BottomBar => model.bottombar.as_ref()?,
        Slot::Main => model.main.as_ref()?,
        Slot::Tab(i) => model.tabs.get(*i)?,
    };
    let state = match &inst.state {
        ModuleState::Matilda(s) => s.as_ref(),
        _ => return None,
    };
    state.inventory_path.clone()
}

/// Si `slot` contiene una instancia de `matilda` y su `source` es
/// `Remote`, retorna `(source, desired)` clonados para que el thread
/// SSH los consuma sin tomar prestado del modelo.
pub(crate) fn remote_matilda_inputs(
    slot: &Slot,
    model: &Model,
) -> Option<(Source, matilda_core::Inventory)> {
    let inst = match slot {
        Slot::TopBar => model.topbar.as_ref()?,
        Slot::BottomBar => model.bottombar.as_ref()?,
        Slot::Main => model.main.as_ref()?,
        Slot::Tab(i) => model.tabs.get(*i)?,
    };
    let state = match &inst.state {
        ModuleState::Matilda(s) => s.as_ref(),
        _ => return None,
    };
    if state.source.is_remote() {
        Some((state.source.clone(), state.desired.clone()))
    } else {
        None
    }
}

pub(crate) fn dispatch_to_module(slot: &Slot, model: &Model, action_id: &str) -> Option<ModuleMsg> {
    let inst = match slot {
        Slot::TopBar => model.topbar.as_ref()?,
        Slot::BottomBar => model.bottombar.as_ref()?,
        Slot::Main => model.main.as_ref()?,
        Slot::Tab(i) => model.tabs.get(*i)?,
    };
    match inst.kind {
        Kind::Launcher => shuma_module_launcher::dispatch(action_id).map(ModuleMsg::Launcher),
        Kind::CommandBar => shuma_module_commandbar::dispatch(action_id).map(ModuleMsg::CommandBar),
        Kind::Shell => shuma_module_shell::dispatch(action_id).map(ModuleMsg::Shell),
        Kind::Matilda => shuma_module_matilda::dispatch(action_id).map(ModuleMsg::Matilda),
        Kind::Minga => shuma_module_minga::dispatch(action_id).map(ModuleMsg::Minga),
        Kind::Canvas => shuma_module_canvas::dispatch(action_id).map(ModuleMsg::Canvas),
    }
}

pub(crate) fn route_to_instance(inst: &mut Instance, msg: ModuleMsg) {
    match (&mut inst.state, msg) {
        (ModuleState::Launcher(s), ModuleMsg::Launcher(m)) => {
            *s = shuma_module_launcher::update(s.clone(), m);
        }
        (ModuleState::CommandBar(s), ModuleMsg::CommandBar(m)) => {
            *s = shuma_module_commandbar::update(s.clone(), m);
        }
        (ModuleState::Shell(s), ModuleMsg::Shell(m)) => {
            *s = shuma_module_shell::update(s.clone(), m);
        }
        (ModuleState::Matilda(s), ModuleMsg::Matilda(m)) => {
            **s = shuma_module_matilda::update((**s).clone(), m);
        }
        (ModuleState::Minga(s), ModuleMsg::Minga(m)) => {
            *s = shuma_module_minga::update(s.clone(), m);
        }
        (ModuleState::Canvas(s), ModuleMsg::Canvas(m)) => {
            *s = shuma_module_canvas::update(s.clone(), m);
        }
        // Combinación inconsistente (state ≠ msg kind): no hace nada.
        // El registry no debería emitirlos; si pasa es un bug del chasis.
        _ => {}
    }
}
