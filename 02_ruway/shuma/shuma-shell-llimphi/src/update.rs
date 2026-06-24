//! Helpers de `update`: ruteo de mensajes, resolución de instancias, atajos.

use super::*;

/// La instancia-módulo que direcciona un `Slot` (compartida por todos los
/// lookups). `Slot::Session(i, w)` resuelve a la vista `w` de la sesión `i`.
pub(crate) fn instance_for_slot<'a>(m: &'a Model, slot: &Slot) -> Option<&'a Instance> {
    match slot {
        Slot::TopBar => m.topbar.as_ref(),
        Slot::BottomBar => m.bottombar.as_ref(),
        Slot::Main => m.main.as_ref(),
        Slot::Session(i, w) => m.session_instance(*i, *w),
    }
}

pub(crate) fn instance_for_slot_mut<'a>(m: &'a mut Model, slot: &Slot) -> Option<&'a mut Instance> {
    match slot {
        Slot::TopBar => m.topbar.as_mut(),
        Slot::BottomBar => m.bottombar.as_mut(),
        Slot::Main => m.main.as_mut(),
        Slot::Session(i, w) => m.session_instance_mut(*i, *w),
    }
}

/// Mantiene el cache del Explorer (`m.explorer`) coherente con la sesión y
/// el cwd actuales **cuando el panel está abierto sobre una sesión remota**.
/// Para sesiones locales limpia el cache (el panel cae a `read_dir` directo).
/// Idempotente: sólo dispara un listado off-thread cuando la clave
/// `(sesión, cwd)` cambia — se puede llamar en cada tick sin re-spawnear.
pub(crate) fn reconcile_explorer(m: &mut Model, handle: &Handle<Msg>) {
    if m.active_tool != Some(Tool::Explorer) {
        return;
    }
    // ¿La sesión activa se lista por la vía remota (SSH)? Sólo si está
    // conectada — si no, evitamos intentos de ssh inútiles.
    let remoto = m.active().and_then(|s| {
        if s.conn != ConnState::Connected {
            return None;
        }
        match &s.shell().state {
            ModuleState::Shell(sh)
                if matches!(sh.source, Source::Remote { .. } | Source::RemoteContainer { .. }) =>
            {
                Some((sh.source.clone(), sh.cwd.display().to_string()))
            }
            _ => None,
        }
    });
    let Some((source, cwd)) = remoto else {
        // Local / no conectada: el panel usa `read_dir`; tiramos cache viejo.
        if m.explorer.key.is_some() {
            m.explorer = ExplorerCache::default();
        }
        return;
    };
    let key = (m.active_session, cwd.clone());
    if m.explorer.key.as_ref() == Some(&key) {
        return; // ya cargado/cargando para esta (sesión, cwd)
    }
    m.explorer.key = Some(key);
    m.explorer.state = ExplorerState::Loading;
    spawn_explorer_list(handle, m.active_session, source, cwd);
}

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
        // El shell de la sesión activa (el canvas) recibe la inserción.
        let insert_msg =
            ModuleMsg::Shell(shuma_module_shell::Msg::InsertAtCursor(text.clone()));
        let target = Slot::Session(m.active_session, Which::Shell);
        return apply_module_msg(m, target, insert_msg);
    }

    if let Some(inst) = instance_for_slot_mut(&mut m, &slot) {
        route_to_instance(inst, msg);
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
    // Monitores/shortcuts de la sesión activa (sus tres vistas).
    let i = model.active_session;
    if let Some(s) = model.sessions.get(i) {
        push(&mut out, Slot::Session(i, Which::Shell), s.shell());
        push(&mut out, Slot::Session(i, Which::Canvas), &s.canvas);
        push(&mut out, Slot::Session(i, Which::Matilda), &s.matilda);
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
    // Cada sesión drena TODOS sus paneles (tiling + flotantes, de toda tab —
    // los de fondo siguen produciendo output) y sincroniza su lienzo desde el
    // shell con foco.
    for s in m.sessions.iter_mut() {
        s.workspace.for_each_pane_mut(tick_one);
        let graph = match &s.shell().state {
            ModuleState::Shell(sh) => Some(sh.intent_graph().clone()),
            _ => None,
        };
        if let (Some(graph), ModuleState::Canvas(c)) = (graph, &mut s.canvas.state) {
            *c = shuma_module_canvas::update(
                c.clone(),
                shuma_module_canvas::Msg::SyncGraph(graph),
            );
        }
    }
}


/// M4 — polling de runtime de matilda. Si hay una instancia matilda
/// **Local** montada (slots fijos topbar/bottombar/main), re-observa el
/// runtime (`docker ps` + `systemctl`) en un thread y lo reenvía como
/// `Msg::SetRuntime`. Llamado desde el `Tick` a cadencia lenta (cada 5 s).
/// El remoto no se poll-ea todavía (necesita SSH por tick — futuro).
pub(crate) fn poll_matilda_runtime(m: &Model, handle: &Handle<Msg>) {
    for slot in [Slot::TopBar, Slot::BottomBar, Slot::Main] {
        let inst = match slot {
            Slot::TopBar => m.topbar.as_ref(),
            Slot::BottomBar => m.bottombar.as_ref(),
            Slot::Main => m.main.as_ref(),
            _ => None,
        };
        if let Some(inst) = inst {
            if let ModuleState::Matilda(st) = &inst.state {
                if !st.source.is_remote() {
                    let slot_back = slot.clone();
                    handle.spawn(move || {
                        let rt = shuma_module_matilda::poll_runtime();
                        Msg::Module(
                            slot_back,
                            ModuleMsg::Matilda(shuma_module_matilda::Msg::SetRuntimeQuiet(rt)),
                        )
                    });
                    // M2 — y las series CPU/mem, pero sólo si el operador está
                    // inspeccionando un contenedor (la sparkline sólo se pinta
                    // bajo el seleccionado, y `docker stats` es caro).
                    if st.selected_container.is_some() {
                        let slot_back = slot.clone();
                        let source = st.source.clone();
                        handle.spawn(move || {
                            let stats =
                                shuma_module_matilda::source_stats_remote_blocking(&source)
                                    .unwrap_or_default();
                            Msg::Module(
                                slot_back,
                                ModuleMsg::Matilda(shuma_module_matilda::Msg::SetStatsQuiet(stats)),
                            )
                        });
                    }
                }
            }
        }
    }
}

/// M5 — polling periódico de la flota. Si una instancia matilda ya tiene
/// flota poblada (el usuario pulsó «Fleet» al menos una vez), re-observa cada
/// host declarado por SSH en un thread y reenvía el resultado **silencioso**
/// (`SetHostRuntimeQuiet`/`SetHostErrorQuiet`) — refresca el semáforo sin
/// spamear el log ni parpadear a «consultando». Cadencia lenta (cada ~30 s,
/// llamado desde `Tick`). Un guard por host (`fleet_poll_inflight`) evita que
/// un host colgado acumule threads tick tras tick.
pub(crate) fn poll_matilda_fleet(m: &Model, handle: &Handle<Msg>) {
    for slot in [Slot::TopBar, Slot::BottomBar, Slot::Main] {
        let inst = match slot {
            Slot::TopBar => m.topbar.as_ref(),
            Slot::BottomBar => m.bottombar.as_ref(),
            Slot::Main => m.main.as_ref(),
            _ => None,
        };
        let Some(inst) = inst else { continue };
        let ModuleState::Matilda(st) = &inst.state else { continue };
        // Sólo se poll-ea una flota que el usuario ya activó (no abrir SSH
        // de rutina sin que lo haya pedido pulsando «Fleet»).
        if st.fleet.is_empty() {
            continue;
        }
        let inflight = st.fleet_poll_inflight.clone();
        for host in st.desired.hosts().cloned() {
            // Guard anti-apilamiento: si ya hay un fetch en vuelo para este
            // host, lo saltamos este tick.
            {
                let mut set = match inflight.lock() {
                    Ok(g) => g,
                    Err(_) => continue,
                };
                if !set.insert(host.name.clone()) {
                    continue;
                }
            }
            let slot_back = slot.clone();
            let inflight = inflight.clone();
            handle.spawn(move || {
                let msg = match shuma_module_matilda::host_runtime_remote_blocking(&host) {
                    Ok(runtime) => shuma_module_matilda::Msg::SetHostRuntimeQuiet {
                        host: host.name.clone(),
                        runtime,
                    },
                    Err(error) => shuma_module_matilda::Msg::SetHostErrorQuiet {
                        host: host.name.clone(),
                        error,
                    },
                };
                if let Ok(mut set) = inflight.lock() {
                    set.remove(&host.name);
                }
                Msg::Module(slot_back, ModuleMsg::Matilda(msg))
            });
        }
    }
}

/// M4 — polling del runtime del Source montado **remoto**. El `poll_matilda_
/// runtime` sólo cubre instancias locales (re-observar local es barato); las
/// remotas necesitan SSH, así que se poll-ean a la cadencia lenta (~30 s) y
/// silenciosas (`SetRuntimeQuiet`). Un guard atómico (`runtime_poll_inflight`)
/// evita apilar fetches si el host montado queda colgado.
pub(crate) fn poll_matilda_remote_runtime(m: &Model, handle: &Handle<Msg>) {
    use std::sync::atomic::Ordering;
    for slot in [Slot::TopBar, Slot::BottomBar, Slot::Main] {
        let inst = match slot {
            Slot::TopBar => m.topbar.as_ref(),
            Slot::BottomBar => m.bottombar.as_ref(),
            Slot::Main => m.main.as_ref(),
            _ => None,
        };
        let Some(inst) = inst else { continue };
        let ModuleState::Matilda(st) = &inst.state else { continue };
        if !st.source.is_remote() {
            continue;
        }
        let inflight = st.runtime_poll_inflight.clone();
        // compare_exchange false→true: si ya había un fetch en vuelo, saltamos.
        if inflight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            continue;
        }
        let source = st.source.clone();
        let want_stats = st.selected_container.is_some();
        let slot_back = slot.clone();
        handle.spawn(move || {
            let msg = match shuma_module_matilda::source_runtime_remote_blocking(&source) {
                Ok(rt) => shuma_module_matilda::Msg::SetRuntimeQuiet(rt),
                // Un fallo de SSH no debe spamear: lo dejamos pasar silencioso
                // (el próximo tick reintenta). LogLines vacío = no-op visible.
                Err(_) => shuma_module_matilda::Msg::LogLines(Vec::new()),
            };
            inflight.store(false, Ordering::Release);
            Msg::Module(slot_back, ModuleMsg::Matilda(msg))
        });
        // M2 — series CPU/mem del Source remoto, sólo si hay un contenedor
        // bajo inspección (la sparkline sólo se pinta bajo el seleccionado).
        if want_stats {
            let source = st.source.clone();
            let slot_back = slot.clone();
            handle.spawn(move || {
                let stats = shuma_module_matilda::source_stats_remote_blocking(&source)
                    .unwrap_or_default();
                Msg::Module(
                    slot_back,
                    ModuleMsg::Matilda(shuma_module_matilda::Msg::SetStatsQuiet(stats)),
                )
            });
        }
    }
}

/// E5 — cumple las peticiones LLM pendientes de los shells montados. El
/// módulo sólo expresa la intención (`State::llm_request`); acá la tomamos,
/// corremos `pluma-llm` en un thread (con su propio runtime tokio) y
/// devolvemos `Msg::LlmResult`. Sin credenciales, `from_env` cae a Mock — el
/// `:?` funciona igual (respuesta canned), nunca se cuelga.
pub(crate) fn fulfill_llm_requests(m: &mut Model, handle: &Handle<Msg>) {
    fn llm_one(slot: Slot, st: &mut shuma_module_shell::State, handle: &Handle<Msg>) {
        let Some(req) = st.take_llm_request() else {
            return;
        };
        let kind = req.kind;
        let slot_back = slot.clone();
        handle.spawn(move || {
            let (ok, text) = match run_llm_blocking(&req) {
                Ok(t) => (true, t),
                Err(e) => (false, e),
            };
            Msg::Module(
                slot_back,
                ModuleMsg::Shell(shuma_module_shell::Msg::LlmResult { kind, ok, text }),
            )
        });
    }
    if let Some(Instance { state: ModuleState::Shell(st), .. }) = m.topbar.as_mut() {
        llm_one(Slot::TopBar, st, handle);
    }
    if let Some(Instance { state: ModuleState::Shell(st), .. }) = m.bottombar.as_mut() {
        llm_one(Slot::BottomBar, st, handle);
    }
    if let Some(Instance { state: ModuleState::Shell(st), .. }) = m.main.as_mut() {
        llm_one(Slot::Main, st, handle);
    }
    for (i, sess) in m.sessions.iter_mut().enumerate() {
        if let ModuleState::Shell(st) = &mut sess.shell_mut().state {
            llm_one(Slot::Session(i, Which::Shell), st, handle);
        }
    }
}

/// Corre una petición LLM de forma **bloqueante** (en un thread del chasis):
/// arma el `ChatRequest`, resuelve el backend por env (Mock si no hay
/// credenciales) y devuelve el texto o un mensaje de error.
fn run_llm_blocking(req: &shuma_module_shell::LlmRequest) -> Result<String, String> {
    use pluma_llm::pluma_llm_core::ChatRequest;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime: {e}"))?;
    rt.block_on(async {
        // Backend según `[ai.llm]` del shumarc (configurable en wawa-panel); si
        // no se fijó uno, cae a la resolución por env (Mock sin credenciales).
        let client = build_llm_client(&req.llm).map_err(|e| format!("sin backend LLM: {e}"))?;
        let chat = ChatRequest::una_vuelta(req.prompt.clone(), req.max_tokens)
            .con_sistema(req.system.clone());
        client
            .complete(&chat)
            .await
            .map(|r| r.content)
            .map_err(|e| format!("{e}"))
    })
}

/// Construye el cliente LLM desde `[ai.llm]`. Si `backend` está vacío, resuelve
/// por env (`from_env`). Traduce los tipos planos de `shuma-config` al
/// `LlmConfig` de `pluma-llm` (mantiene a `shuma-config` liviano).
fn build_llm_client(
    s: &shuma_config::LlmSettings,
) -> Result<std::sync::Arc<dyn pluma_llm::pluma_llm_core::ChatClient>, String> {
    use pluma_llm::{build_client, BackendKind, LlmConfig};
    if !s.is_set() {
        return pluma_llm::from_env().map_err(|e| format!("{e}"));
    }
    let kind = match s.backend.trim().to_lowercase().as_str() {
        "anthropic" => BackendKind::Anthropic,
        "gemini" => BackendKind::Gemini,
        "deepseek" => BackendKind::DeepSeek,
        "cohere" => BackendKind::Cohere,
        "ollama" => BackendKind::Ollama,
        "mock" => BackendKind::Mock,
        other => return Err(format!("backend LLM desconocido: «{other}»")),
    };
    let none_if_empty = |v: &str| {
        let v = v.trim();
        (!v.is_empty()).then(|| v.to_string())
    };
    let cfg = LlmConfig {
        kind,
        model: none_if_empty(&s.model),
        api_key: none_if_empty(&s.api_key),
        endpoint: none_if_empty(&s.endpoint),
    };
    build_client(&cfg).map_err(|e| format!("{e}"))
}

/// Cumple las búsquedas semánticas pendientes (`:buscar`) de los shells
/// montados. Mismo patrón que [`fulfill_llm_requests`]: el módulo expresa la
/// intención (`State::semantic_request`), acá la corremos en un thread con
/// `rimay-verbo` (daemon o mock) y devolvemos `Msg::SemanticResult`.
pub(crate) fn fulfill_semantic_requests(m: &mut Model, handle: &Handle<Msg>) {
    fn one(slot: Slot, st: &mut shuma_module_shell::State, handle: &Handle<Msg>) {
        let Some(req) = st.take_semantic_request() else {
            return;
        };
        let slot_back = slot.clone();
        handle.spawn(move || {
            let (ok, hits) = match run_semantic_blocking(&req) {
                Ok(h) => (true, h),
                Err(e) => (false, vec![(e, 0.0)]),
            };
            // `files` va al panel del Explorer (chasis); `history` al output del
            // shell (módulo).
            if req.scope == "files" {
                Msg::FileSearchResult { slot: slot_back, query: req.query.clone(), ok, hits }
            } else {
                Msg::Module(
                    slot_back,
                    ModuleMsg::Shell(shuma_module_shell::Msg::SemanticResult { ok, hits }),
                )
            }
        });
    }
    if let Some(Instance { state: ModuleState::Shell(st), .. }) = m.topbar.as_mut() {
        one(Slot::TopBar, st, handle);
    }
    if let Some(Instance { state: ModuleState::Shell(st), .. }) = m.bottombar.as_mut() {
        one(Slot::BottomBar, st, handle);
    }
    if let Some(Instance { state: ModuleState::Shell(st), .. }) = m.main.as_mut() {
        one(Slot::Main, st, handle);
    }
    for (i, sess) in m.sessions.iter_mut().enumerate() {
        if let ModuleState::Shell(st) = &mut sess.shell_mut().state {
            one(Slot::Session(i, Which::Shell), st, handle);
        }
    }
}

/// Corre una búsqueda semántica bloqueante (en un thread del chasis): embebe la
/// consulta + candidatos con el daemon `rimay-verbo` (o un mock determinista si
/// no hay daemon), rankea por coseno y devuelve los mejores `(comando, score)`.
fn run_semantic_blocking(
    req: &shuma_module_shell::SemanticRequest,
) -> Result<Vec<(String, f32)>, String> {
    const TOP_K: usize = 10;
    const MIN_SCORE: f32 = 0.20;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime: {e}"))?;
    rt.block_on(async {
        use crate::semantic::SemanticIndex;
        use rimay_verbo::Provider;
        let provider = if req.socket.trim().is_empty() {
            rimay_verbo::conectar_o_mock(req.dim).await
        } else {
            rimay_verbo::conectar_o_mock_en(std::path::Path::new(req.socket.trim()), req.dim).await
        };
        // Índice persistido por scope (history/files): sólo embebe lo nuevo, poda
        // lo caído y se reconstruye solo si cambió el modelo de embeddings.
        let mut idx = SemanticIndex::load(SemanticIndex::path_for(&req.scope), provider.model_id().clone());
        idx.ensure(&*provider, &req.candidates).await?;
        let keys: Vec<String> = req.candidates.iter().map(|(k, _)| k.clone()).collect();
        idx.retain(&keys);
        if let Err(e) = idx.save() {
            // No es fatal: la búsqueda igual responde, sólo no cacheó.
            eprintln!("shuma · no pude persistir el índice semántico: {e}");
        }
        let q = provider
            .embed(&req.query)
            .await
            .map_err(|e| format!("embeddings: {e}"))?;
        let hits = idx.search(&q, TOP_K, MIN_SCORE);
        // En `files` la clave es `mtime\0ruta` (para re-embeber al cambiar el
        // archivo); mostramos sólo la ruta.
        let hits = if req.scope == "files" {
            hits
                .into_iter()
                .map(|(k, s)| (k.split_once('\0').map(|(_, p)| p.to_string()).unwrap_or(k), s))
                .collect()
        } else {
            hits
        };
        Ok(hits)
    })
}

pub(crate) fn monitor_key(slot: &Slot, spec: &MonitorSpec) -> String {
    let slot_label = match slot {
        Slot::TopBar => "topbar",
        Slot::BottomBar => "bottombar",
        Slot::Main => "main",
        Slot::Session(i, w) => return format!("session:{i}:{w:?}/{}", spec.id),
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
            // Lo agregamos al log del matilda de la sesión activa (feedback).
            if let Some(s) = m.sessions.get_mut(m.active_session) {
                if let ModuleState::Matilda(mat) = &mut s.matilda.state {
                    mat.log.push(format!("? command: {line}"));
                }
            }
        }
        ShortcutAction::FocusTab { target } => {
            // Un shortcut que pide enfocar un módulo abre su herramienta a la
            // derecha (matilda → panel Matilda). El shell es el canvas, no una
            // herramienta, así que no aplica.
            if target == shuma_module_matilda::ID {
                m.active_tool = Some(Tool::Matilda);
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
            // M5 — Fleet: consulta el runtime de cada host declarado por SSH,
            // un thread por host, y reenvía SetHostRuntime/SetHostError.
            if action_id == "matilda.fleet" {
                let hosts: Vec<matilda_core::Host> =
                    match instance_for_slot(&m, &slot).map(|i| &i.state) {
                        Some(ModuleState::Matilda(st)) => st.desired.hosts().cloned().collect(),
                        _ => Vec::new(),
                    };
                // Marca cada host como Pending en el módulo.
                m = apply_module_msg(
                    m,
                    slot.clone(),
                    ModuleMsg::Matilda(shuma_module_matilda::Msg::RefreshFleet),
                );
                for host in hosts {
                    let slot_back = slot.clone();
                    handle.spawn(move || {
                        let msg = match shuma_module_matilda::host_runtime_remote_blocking(&host) {
                            Ok(runtime) => shuma_module_matilda::Msg::SetHostRuntime {
                                host: host.name.clone(),
                                runtime,
                            },
                            Err(error) => shuma_module_matilda::Msg::SetHostError {
                                host: host.name.clone(),
                                error,
                            },
                        };
                        Msg::Module(slot_back, ModuleMsg::Matilda(msg))
                    });
                }
                return m;
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
    let inst = instance_for_slot(model, slot)?;
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
    let inst = instance_for_slot(model, slot)?;
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
    // El slot Main como shell gana (config wrapper de una sola app).
    if let Some(inst) = model.main.as_ref() {
        if matches!(inst.state, ModuleState::Shell(_)) {
            return Some(Msg::Module(
                Slot::Main,
                ModuleMsg::Shell(shuma_module_shell::Msg::Scroll(dpx)),
            ));
        }
    }
    // Si no, la rueda recorre el shell de la sesión activa (siempre tiene uno).
    Some(Msg::Module(
        Slot::Session(model.active_session, Which::Shell),
        ModuleMsg::Shell(shuma_module_shell::Msg::Scroll(dpx)),
    ))
}

pub(crate) fn forward_key_to_focused_shell(model: &Model, e: &KeyEvent) -> Option<Msg> {
    // Si una ventana secundaria tiene un draft con foco de campo, las
    // teclas van al draft (no al shell). El runtime de Llimphi dispatcha
    // on_key tanto para primary como para secondary, así que modelamos
    // el "foco" por estado de la app.
    if let Some(d) = model.host_draft.as_ref() {
        if d.focused.is_some() {
            return Some(Msg::HostDraftKey(e.clone()));
        }
    }
    if let Some(d) = model.container_draft.as_ref() {
        if d.focus.is_some() {
            return Some(Msg::ContainerDraftKey(e.clone()));
        }
    }
    // Con un modal bloqueante abierto (containers/hosts) y sin campo del draft
    // focado, Esc lo cierra. Es el único escape de teclado ahora que el clic
    // en el scrim ya no descarta el modal (`on_dismiss: Noop`).
    if matches!(&e.key, llimphi_ui::Key::Named(llimphi_ui::NamedKey::Escape)) {
        if model.containers_modal_open {
            return Some(Msg::CloseContainersModal);
        }
        if model.hosts_modal_open {
            return Some(Msg::CloseHostsModal);
        }
    }
    // Ctrl+= / Ctrl++ → zoom in · Ctrl+- → zoom out · Ctrl+0 → reset.
    // Atajos universales: aplican al shell de la sesión activa sin
    // importar si está focado en el input o un TUI; los chequeamos
    // antes de las ramas main/pending. Detectamos por tres caminos
    // (winit puede entregar la tecla por uno u otro según layout/IME):
    //   1. `Key::Character` con el char.
    //   2. `Key::Named(NamedKey::Equal/Minus/...)` (si winit las
    //      promueve a Named en algunos backends).
    //   3. `e.text` (string ya resuelto con modifiers).
    if e.modifiers.ctrl {
        let ch: Option<&str> = match &e.key {
            llimphi_ui::Key::Character(c) => Some(c.as_str()),
            _ => e.text.as_deref(),
        };
        let zoom_msg: Option<shuma_module_shell::Msg> = match ch {
            Some("+" | "=") => Some(shuma_module_shell::Msg::ZoomBy(1.1)),
            Some("-" | "_") => Some(shuma_module_shell::Msg::ZoomBy(1.0 / 1.1)),
            Some("0") => Some(shuma_module_shell::Msg::ZoomReset),
            _ => None,
        };
        if let Some(z) = zoom_msg {
            return Some(Msg::Module(
                Slot::Session(model.active_session, Which::Shell),
                ModuleMsg::Shell(z),
            ));
        }
    }
    // 1) Slot Main siempre gana — si está configurado como shell.
    if let Some(inst) = model.main.as_ref() {
        if matches!(inst.state, ModuleState::Shell(_)) {
            return Some(Msg::Module(
                Slot::Main,
                ModuleMsg::Shell(shuma_module_shell::Msg::Key(e.clone())),
            ));
        }
    }
    // 2) Si la sesión activa está en form de creación (pending), las teclas
    // van al form, no al shell oculto.
    if let Some(s) = model.sessions.get(model.active_session) {
        if s.pending {
            // Escape sin foco de campo cancela toda la creación.
            if matches!(
                &e.key,
                llimphi_ui::Key::Named(llimphi_ui::NamedKey::Escape)
            ) && s.pending_focus.is_none()
            {
                return Some(Msg::CancelNewSession);
            }
            return Some(Msg::PendingKey(e.clone()));
        }
    }
    // 3) Las teclas van al shell de la sesión activa (es el canvas principal).
    Some(Msg::Module(
        Slot::Session(model.active_session, Which::Shell),
        ModuleMsg::Shell(shuma_module_shell::Msg::Key(e.clone())),
    ))
}

/// Path del inventario JSON de un slot de matilda, si lo tiene cargado.
pub(crate) fn matilda_inventory_path(slot: &Slot, model: &Model) -> Option<std::path::PathBuf> {
    let inst = instance_for_slot(model, slot)?;
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
    let inst = instance_for_slot(model, slot)?;
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

/// Si `slot` aloja un matilda, busca un host de la flota por nombre en su
/// inventario deseado. Clonado para que el thread SSH lo consuma sin tomar
/// prestado del modelo. Usado por las acciones de flota (M5).
pub(crate) fn matilda_host_by_name(
    slot: &Slot,
    model: &Model,
    name: &str,
) -> Option<matilda_core::Host> {
    let inst = instance_for_slot(model, slot)?;
    let state = match &inst.state {
        ModuleState::Matilda(s) => s.as_ref(),
        _ => return None,
    };
    state.desired.hosts().find(|h| h.name == name).cloned()
}

/// M2 — inputs para el thread del live-tail: el `Source`, el contenedor y la
/// bandera `stop` que el módulo acaba de crear en su `log_stream`. `None` si el
/// slot no es matilda o no hay stream activo (no debería pasar tras un
/// `StartLogStream` aplicado). Clonados para que el thread no tome prestado.
pub(crate) fn matilda_log_stream_inputs(
    slot: &Slot,
    model: &Model,
) -> Option<(Source, String, std::sync::Arc<std::sync::atomic::AtomicBool>)> {
    let inst = instance_for_slot(model, slot)?;
    let state = match &inst.state {
        ModuleState::Matilda(s) => s.as_ref(),
        _ => return None,
    };
    let ls = state.log_stream.as_ref()?;
    Some((state.source.clone(), ls.container.clone(), ls.stop.clone()))
}

pub(crate) fn dispatch_to_module(slot: &Slot, model: &Model, action_id: &str) -> Option<ModuleMsg> {
    let inst = instance_for_slot(model, slot)?;
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
