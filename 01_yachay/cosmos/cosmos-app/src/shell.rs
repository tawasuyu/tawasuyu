//! Shell — coordinador de los tres widgets.
//!
//! Es el "director de orquesta": dueño del tree, del canvas y del panel,
//! reenvía eventos entre ellos y aplica las mutaciones en la store.
//!
//! Flujo:
//!
//! ```text
//!   Tree.Selected(Chart)        →  Shell  →  load chart + compose + set_mode(Wheel)
//!   Tree.Selected(Group/Contact)→  Shell  →  charts_under_*  + set_mode(Thumbnails)
//!   Canvas.TimeOffsetChanged    →  Shell  →  compose(current_chart, off, requests)
//!   Canvas.LayerVisibility[T]   →  Shell  →  flip module_configs[transit][enabled]
//!   Panel.ControlChanged        →  Shell  →  update module_configs OR canvas visibility
//! ```
//!
//! ## module_configs
//!
//! Mapa `module_id → JSON` con la configuración persistente de cada
//! módulo (transit, progression, …). De ahí derivamos los
//! `PipelineRequest` que la engine consume. Los toggles "visuales"
//! del NatalModule (`show_sign_dial`, `show_houses`, …) NO viven acá
//! — afectan solo el render del canvas, no la composición.

use std::collections::HashMap;

use gpui::{
    ClickEvent, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled,
    Window, div, prelude::*, px,
};

use cosmobiologia_canvas::{
    AstrologyCanvas, CanvasEvent, CanvasMode, ThumbnailItem, ThumbnailScope,
};
use cosmobiologia_engine::{
    EventoConocido, LayerKind, NatalOptions, OUTER_RING_MODULES, PipelineRequest,
    compose_with_options, svg_export,
};
use cosmobiologia_model::{
    Chart, ChartId, ChartKind, ContactId, FreeChartId, ModuleState, StoredBirthData,
    StoredChartConfig, TreeSelection,
};
use cosmobiologia_panel::{ChartOption, ControlPanel, PanelEvent};
use cosmobiologia_store::Store;
use cosmobiologia_tree::{
    parse_city_atlas_tsv, FreeChartEntry, TahuantinsuyuTree, TreeEvent,
};
use nahual_core::{LayoutDirection, NodeId};
use nahual_theme::Theme;
use nahual_widget_container_core::ChildSlot;
use nahual_widget_splitter::{SplitContainer, SplitEvent};
use nahual_widget_theme_switcher::theme_switcher;

/// Posición del panel de control dentro del shell. `Bottom` mantiene
/// el layout histórico (tree+canvas arriba, panel abajo); las variantes
/// laterales colapsan los splitters anidados en uno solo de 3 columnas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelDock {
    Bottom,
    Right,
    Left,
}

impl PanelDock {
    fn as_setting(&self) -> &'static str {
        match self {
            PanelDock::Bottom => "bottom",
            PanelDock::Right => "right",
            PanelDock::Left => "left",
        }
    }

    fn from_setting(s: &str) -> Option<Self> {
        match s {
            "bottom" => Some(PanelDock::Bottom),
            "right" => Some(PanelDock::Right),
            "left" => Some(PanelDock::Left),
            _ => None,
        }
    }
}

/// Status del broker brahman tal como lo vimos en el último ping.
/// Se refresca cada 30 segundos desde un background task.
#[derive(Clone, Debug)]
pub enum BrahmanStatus {
    /// Aún no probamos (boot, primer ciclo).
    Pending,
    /// Connect OK al broker, devolvió la lista de sessions activas.
    Connected { session_count: usize },
    /// Connect falló — broker no escucha en el socket o tomó timeout.
    /// `reason` se incluye para diagnóstico en logs aunque la UI hoy
    /// muestra solo "offline".
    Offline {
        #[allow(dead_code)]
        reason: String,
    },
}

pub struct Shell {
    store: Store,
    /// Los tres widgets viven como children de los splitters vía
    /// AnyView clone; retenemos los Entity acá para que las
    /// subscripciones sigan vivas y para poder rearmar el layout al
    /// cambiar `dock` sin recrear los widgets.
    tree: Entity<TahuantinsuyuTree>,
    canvas: Entity<AstrologyCanvas>,
    panel: Entity<ControlPanel>,
    /// Splitter "exterior". En dock=Bottom es vertical con (main_split,
    /// panel) como hijos; en dock=Right/Left es horizontal y agrupa
    /// tree+canvas+panel en una sola tira.
    outer_split: Entity<SplitContainer>,
    /// Splitter horizontal interno con (tree, canvas). Solo se usa
    /// cuando dock=Bottom; en docks laterales queda vivo pero sin ser
    /// hijo del árbol activo.
    main_split: Entity<SplitContainer>,
    /// Dock activo del panel — determina cómo se arman los splitters
    /// y cuáles flex se persisten.
    dock: PanelDock,
    /// Último estado conocido del broker brahman — refrescado cada
    /// 30s desde el background task.
    brahman_status: BrahmanStatus,
    current_chart: Option<Chart>,
    current_offset_minutes: i64,
    /// Estado de los módulos overlay (transit, progression, …) por
    /// `module_id`. Las claves dentro del JSON dependen del módulo (la
    /// convención es `"enabled": bool` para el toggle principal).
    module_configs: HashMap<String, serde_json::Value>,
    /// Sequence counter para descartar resultados de cómputos
    /// background que llegan después de uno más reciente. Cada
    /// `render_current` lo incrementa y la closure async compara antes
    /// de aplicar el render al canvas.
    render_seq: u64,
    /// Cartas "libres" — no persistidas en la store. Incluye la
    /// especial `sky_now()` (Cielo ahora) + cualquier creada por el
    /// usuario desde la sección "Cartas libres" del tree. Cada vez
    /// que muta este mapa, llamamos `tree.set_free_charts`.
    free_charts: HashMap<FreeChartId, Chart>,
    /// Counter para id de cartas libres nuevas — el id se concatena
    /// con el prefijo `free-` y el counter, así son únicos dentro de
    /// la sesión sin pelearse con UUIDs reales.
    next_free_id: u32,
}

impl Shell {
    pub fn new(store: Store, cx: &mut Context<Self>) -> Self {
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();

        let tree = cx.new(|cx| {
            let mut t = TahuantinsuyuTree::new(store.clone(), cx);
            // Si hay un atlas custom en $XDG_DATA_HOME/cosmobiologia/
            // atlas.tsv, lo cargamos y reemplazamos el atlas hardcoded
            // de 90 ciudades. Formato TSV: name<TAB>lat<TAB>lon<TAB>tz_min.
            if let Some(atlas) = load_city_atlas_from_xdg() {
                t.set_city_atlas(atlas, cx);
            }
            t
        });
        let canvas = cx.new(AstrologyCanvas::new);
        let panel = cx.new(ControlPanel::new);

        cx.subscribe(&tree, |this: &mut Self, _, ev: &TreeEvent, cx| {
            this.on_tree_event(ev, cx);
        })
        .detach();

        cx.subscribe(&panel, |this: &mut Self, _, ev: &PanelEvent, cx| {
            this.on_panel_event(ev, cx);
        })
        .detach();

        cx.subscribe(&canvas, |this: &mut Self, _, ev: &CanvasEvent, cx| {
            this.on_canvas_event(ev, cx);
        })
        .detach();

        // Splitters vacíos — `apply_dock` los puebla según el layout
        // activo. Horizontal/Vertical son defaults; cada apply ajusta la
        // dirección antes de setear children.
        let main_split = cx.new(|cx| SplitContainer::new(LayoutDirection::Horizontal, cx));
        let outer_split = cx.new(|cx| SplitContainer::new(LayoutDirection::Vertical, cx));

        // Persistir flex en `DragEnd`. La key del setting depende del
        // dock activo, así no se pisan los flexes de un layout con los
        // de otro al mudarse. Se lee dentro del closure para tomar el
        // dock actualizado, no el capturado en `new`.
        let store_main = store.clone();
        cx.subscribe(&main_split, move |this: &mut Self, sc, ev: &SplitEvent, cx| {
            if matches!(ev, SplitEvent::DragEnd) {
                let key = split_key_main(this.dock);
                save_split_flex(&store_main, key, sc.read(cx));
            }
        })
        .detach();
        let store_outer = store.clone();
        cx.subscribe(&outer_split, move |this: &mut Self, sc, ev: &SplitEvent, cx| {
            if matches!(ev, SplitEvent::DragEnd) {
                let key = split_key_outer(this.dock);
                save_split_flex(&store_outer, key, sc.read(cx));
            }
        })
        .detach();

        let dock = load_dock(&store).unwrap_or(PanelDock::Bottom);

        let mut shell = Self {
            store,
            tree,
            canvas,
            panel,
            outer_split,
            main_split,
            dock,
            brahman_status: BrahmanStatus::Pending,
            current_chart: None,
            current_offset_minutes: 0,
            module_configs: HashMap::new(),
            render_seq: 0,
            free_charts: HashMap::new(),
            next_free_id: 0,
        };
        shell.apply_dock(dock, cx);
        shell.refresh_chart_options(cx);
        shell.spawn_brahman_status_loop(cx);
        // Inicializar "Cielo ahora" como carta libre fija y empujarla
        // al tree. Queda seleccionada por default — el usuario abre
        // la app y ya ve el firmamento actual.
        shell.ensure_sky_now(cx);
        shell.apply_selection(
            TreeSelection::FreeChart(FreeChartId::sky_now()),
            cx,
        );
        shell
    }

    /// Garantiza que `sky-now` exista en `free_charts` y publica la
    /// lista actualizada al tree. Recomputa la carta del cielo si ya
    /// estaba (refresca al reloj actual).
    fn ensure_sky_now(&mut self, cx: &mut Context<Self>) {
        self.free_charts
            .insert(FreeChartId::sky_now(), build_present_sky_chart());
        self.push_free_charts_to_tree(cx);
    }

    fn push_free_charts_to_tree(&self, cx: &mut Context<Self>) {
        // Orden de display: "Cielo ahora" primero, después el resto
        // por id (los ids `free-N` quedan ordenados por creación).
        let mut entries: Vec<FreeChartEntry> = Vec::new();
        if let Some(c) = self.free_charts.get(&FreeChartId::sky_now()) {
            entries.push(FreeChartEntry {
                id: FreeChartId::sky_now(),
                label: c.label.clone(),
                birth_data: c.birth_data.clone(),
            });
        }
        let mut others: Vec<(&FreeChartId, &Chart)> = self
            .free_charts
            .iter()
            .filter(|(k, _)| !k.is_sky_now())
            .collect();
        others.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        for (id, c) in others {
            entries.push(FreeChartEntry {
                id: id.clone(),
                label: c.label.clone(),
                birth_data: c.birth_data.clone(),
            });
        }
        self.tree
            .update(cx, |t, cx| t.set_free_charts(entries, cx));
    }

    /// Arma el árbol de splitters según el dock pedido y persiste la
    /// elección. Idempotente: llamar con el dock actual reconstruye los
    /// children con flexes leídos del setting (útil tras `new`).
    pub fn apply_dock(&mut self, dock: PanelDock, cx: &mut Context<Self>) {
        self.dock = dock;

        let tree_view = gpui::AnyView::from(self.tree.clone());
        let canvas_view = gpui::AnyView::from(self.canvas.clone());
        let panel_view = gpui::AnyView::from(self.panel.clone());
        let main_view = gpui::AnyView::from(self.main_split.clone());

        match dock {
            PanelDock::Bottom => {
                let flex_main = load_split_flex_n(
                    &self.store,
                    split_key_main(dock),
                    &[1.0, 4.0],
                );
                let flex_outer = load_split_flex_n(
                    &self.store,
                    split_key_outer(dock),
                    &[4.0, 1.0],
                );
                self.main_split.update(cx, |sc, cx| {
                    sc.set_direction(LayoutDirection::Horizontal, cx);
                    sc.set_children(
                        vec![
                            ChildSlot {
                                id: NodeId::new("tts-tree"),
                                flex: flex_main[0],
                                label: None,
                                view: tree_view.clone(),
                            },
                            ChildSlot {
                                id: NodeId::new("tts-canvas"),
                                flex: flex_main[1],
                                label: None,
                                view: canvas_view.clone(),
                            },
                        ],
                        cx,
                    );
                });
                self.outer_split.update(cx, |sc, cx| {
                    sc.set_direction(LayoutDirection::Vertical, cx);
                    sc.set_children(
                        vec![
                            ChildSlot {
                                id: NodeId::new("tts-main"),
                                flex: flex_outer[0],
                                label: None,
                                view: main_view,
                            },
                            ChildSlot {
                                id: NodeId::new("tts-panel"),
                                flex: flex_outer[1],
                                label: None,
                                view: panel_view,
                            },
                        ],
                        cx,
                    );
                });
            }
            PanelDock::Right => {
                let flex = load_split_flex_n(
                    &self.store,
                    split_key_outer(dock),
                    &[1.0, 4.0, 1.5],
                );
                self.outer_split.update(cx, |sc, cx| {
                    sc.set_direction(LayoutDirection::Horizontal, cx);
                    sc.set_children(
                        vec![
                            ChildSlot {
                                id: NodeId::new("tts-tree"),
                                flex: flex[0],
                                label: None,
                                view: tree_view,
                            },
                            ChildSlot {
                                id: NodeId::new("tts-canvas"),
                                flex: flex[1],
                                label: None,
                                view: canvas_view,
                            },
                            ChildSlot {
                                id: NodeId::new("tts-panel"),
                                flex: flex[2],
                                label: None,
                                view: panel_view,
                            },
                        ],
                        cx,
                    );
                });
            }
            PanelDock::Left => {
                let flex = load_split_flex_n(
                    &self.store,
                    split_key_outer(dock),
                    &[1.5, 1.0, 4.0],
                );
                self.outer_split.update(cx, |sc, cx| {
                    sc.set_direction(LayoutDirection::Horizontal, cx);
                    sc.set_children(
                        vec![
                            ChildSlot {
                                id: NodeId::new("tts-panel"),
                                flex: flex[0],
                                label: None,
                                view: panel_view,
                            },
                            ChildSlot {
                                id: NodeId::new("tts-tree"),
                                flex: flex[1],
                                label: None,
                                view: tree_view,
                            },
                            ChildSlot {
                                id: NodeId::new("tts-canvas"),
                                flex: flex[2],
                                label: None,
                                view: canvas_view,
                            },
                        ],
                        cx,
                    );
                });
            }
        }

        if let Err(e) = self.store.set_setting("layout.panel_dock", dock.as_setting()) {
            eprintln!("[shell] persist panel_dock: {}", e);
        }
        cx.notify();
    }

    /// Loop que cada 30s pregunta al broker la lista de sessions
    /// activas y actualiza `brahman_status`. El cómputo bloqueante
    /// (list_sessions_blocking abre su propio tokio runtime) corre en
    /// el background_executor — no bloquea el UI thread. Cuando llega
    /// el resultado, el `this.update` dispara cx.notify para repintar
    /// el badge del header.
    fn spawn_brahman_status_loop(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                let result = cx
                    .background_executor()
                    .spawn(async {
                        brahman_sidecar::list_sessions_blocking("cosmobiologia-observer")
                    })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    this.brahman_status = match result {
                        Ok(list) => BrahmanStatus::Connected {
                            session_count: list.entries.len(),
                        },
                        Err(e) => BrahmanStatus::Offline {
                            reason: format!("{:?}", e),
                        },
                    };
                    cx.notify();
                });
                let timer = cx
                    .background_executor()
                    .timer(std::time::Duration::from_secs(30));
                timer.await;
            }
        })
        .detach();
    }

    /// Recarga la lista de opciones para los `Control::ChartPicker` y
    /// la pushea al panel. Llamado al boot + tras cada
    /// `TreeEvent::HierarchyChanged`.
    fn refresh_chart_options(&self, cx: &mut Context<Self>) {
        let charts = self.store.list_all_charts().unwrap_or_default();
        let options: Vec<ChartOption> = charts
            .into_iter()
            .map(|c| ChartOption {
                id: c.id.to_string(),
                label: format!("{} — {}", c.label, format_birth_brief(&c.birth_data)),
            })
            .collect();
        self.panel
            .update(cx, |p, cx| p.set_chart_options(options, cx));
    }

    fn on_tree_event(&mut self, ev: &TreeEvent, cx: &mut Context<Self>) {
        let selection = match ev {
            TreeEvent::Selected(s) => s,
            TreeEvent::Opened(s) => s,
            TreeEvent::HierarchyChanged => {
                // La jerarquía cambió (alta/baja de cartas) — refrescar
                // las opciones del picker para que aparezcan / desaparezcan
                // en el dropdown.
                self.refresh_chart_options(cx);
                cx.notify();
                return;
            }
            TreeEvent::NewFreeChartRequested => {
                let id = FreeChartId(format!("free-{}", self.next_free_id));
                self.next_free_id += 1;
                // Default: misma data que "Cielo ahora" pero con label
                // distinto. El usuario edita después con el editor
                // inline (fase B).
                let mut chart = build_present_sky_chart();
                chart.label = format!("Carta libre #{}", self.next_free_id);
                self.free_charts.insert(id.clone(), chart);
                self.push_free_charts_to_tree(cx);
                self.apply_selection(TreeSelection::FreeChart(id), cx);
                return;
            }
            TreeEvent::SaveFreeChartRequested(_id) => {
                // El menú del tree abre el modal directamente; este
                // evento queda como hook por si una tecla u otra
                // UI quiere disparar el flujo sin pasar por el menú.
                return;
            }
            TreeEvent::FreeChartSaveConfirmed {
                source_id,
                chart_name,
                contact,
                new_contact_name,
            } => {
                self.persist_free_chart(
                    source_id.clone(),
                    chart_name.clone(),
                    contact.clone(),
                    new_contact_name.clone(),
                    cx,
                );
                return;
            }
            TreeEvent::FreeChartEditConfirmed {
                source_id,
                birth_data,
                label,
            } => {
                if let Some(chart) = self.free_charts.get_mut(source_id) {
                    chart.birth_data = birth_data.clone();
                    chart.label = label.clone();
                }
                self.push_free_charts_to_tree(cx);
                // Si la carta editada era la activa, re-render.
                if let Some(current) = self.current_chart.as_mut() {
                    // Heurística: comparamos por label (ya cambiado al
                    // que pidió el usuario). Si el label de la activa
                    // coincide, era esta carta.
                    if current.label == label.clone()
                        || current.birth_data.subject_name.as_deref() == Some("Cielo")
                    {
                        if let Some(updated) = self.free_charts.get(source_id) {
                            *current = updated.clone();
                            self.render_current(cx);
                        }
                    }
                }
                return;
            }
            TreeEvent::DeleteFreeChartRequested(id) => {
                if id.is_sky_now() {
                    return; // no se borra el Cielo
                }
                self.free_charts.remove(id);
                self.push_free_charts_to_tree(cx);
                // Si la carta borrada era la activa, vuelve al Cielo.
                if let Some(current) = self.current_chart.as_ref() {
                    if current.label.starts_with(&format!("Carta libre")) {
                        self.apply_selection(
                            TreeSelection::FreeChart(FreeChartId::sky_now()),
                            cx,
                        );
                    }
                }
                return;
            }
        };
        self.apply_selection(selection.clone(), cx);
    }

    /// Persiste una carta libre como `Chart` en la store. El usuario
    /// eligió en el modal: nombre + contacto destino (existente o
    /// uno nuevo creado al vuelo). La carta libre se REMUEVE del
    /// mapa tras el persist exitoso — si quedaba seleccionada,
    /// volvemos a "Cielo ahora". Si falla la persistencia, la carta
    /// libre se conserva y logueamos.
    fn persist_free_chart(
        &mut self,
        source_id: FreeChartId,
        chart_name: String,
        contact: Option<ContactId>,
        new_contact_name: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(chart) = self.free_charts.get(&source_id).cloned() else {
            return;
        };
        // 1) Resolver el contact destino (existente o crear nuevo).
        let contact_id = match (contact, new_contact_name) {
            (Some(cid), _) => cid,
            (None, Some(name)) => match self.store.create_contact(None, &name, None) {
                Ok(c) => c.id,
                Err(e) => {
                    eprintln!("[shell] create_contact al guardar libre: {}", e);
                    return;
                }
            },
            (None, None) => {
                eprintln!("[shell] persist_free_chart sin contacto ni nombre nuevo");
                return;
            }
        };
        // 2) Crear la carta.
        match self.store.create_chart(
            contact_id,
            chart.kind,
            &chart_name,
            &chart.birth_data,
            &chart.config,
            chart.related_chart_id,
        ) {
            Ok(_) => {
                eprintln!(
                    "[shell] carta libre {:?} guardada como '{}' bajo contacto {}",
                    source_id, chart_name, contact_id
                );
            }
            Err(e) => {
                eprintln!("[shell] create_chart al guardar libre: {}", e);
                return;
            }
        }
        // 3) Sky-now se conserva (siempre es); las demás se quitan
        // del mapa libre. Si era la activa, volver al Cielo.
        if !source_id.is_sky_now() {
            self.free_charts.remove(&source_id);
            self.push_free_charts_to_tree(cx);
            // Si la activa era esta libre, regresar al Cielo.
            self.apply_selection(
                TreeSelection::FreeChart(FreeChartId::sky_now()),
                cx,
            );
        }
        self.refresh_chart_options(cx);
    }

    fn apply_selection(&mut self, sel: TreeSelection, cx: &mut Context<Self>) {
        match sel {
            TreeSelection::Chart(id) => {
                let chart = match self.store.get_chart(id) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("[shell] get_chart {}: {}", id, e);
                        return;
                    }
                };
                let age = current_age_years(&chart.birth_data);
                self.current_chart = Some(chart.clone());
                self.current_offset_minutes = 0;
                // 1) Defaults frescos para esta carta: edad objetivo =
                //    edad actual. Estos quedan en module_configs como
                //    valor base si el usuario nunca tocó el slider.
                self.module_configs.clear();
                for module_id in ["progression", "solar_arc", "planetary_return"] {
                    let entry = self
                        .module_configs
                        .entry(module_id.into())
                        .or_insert_with(|| serde_json::json!({}));
                    if let serde_json::Value::Object(map) = entry {
                        map.insert("target_age_years".into(), serde_json::json!(age));
                    }
                }
                // El módulo planetary_return además necesita un body
                // por default — el shell elige "sun" si el usuario no
                // tocó el Select. La persistencia luego puede pisar
                // este valor.
                if let Some(serde_json::Value::Object(map)) =
                    self.module_configs.get_mut("planetary_return")
                {
                    map.entry(String::from("body"))
                        .or_insert(serde_json::json!("sun"));
                }
                // 2) Sobreescribir con lo que el usuario persistió la
                //    última vez para esta carta (SQLite `module_state`).
                self.load_persisted_module_states(chart.id);
                // 3) Sincronizar panel: active_kind + toggles/sliders.
                self.panel.update(cx, |p, cx| {
                    p.set_active_kind(Some(chart.kind), cx);
                });
                self.sync_panel_from_configs(cx);
                self.render_current(cx);
            }
            TreeSelection::Contact(id) => {
                self.current_chart = None;
                self.current_offset_minutes = 0;
                let charts = self.store.list_charts(id).unwrap_or_default();
                let items: Vec<ThumbnailItem> = charts
                    .into_iter()
                    .map(|c| ThumbnailItem {
                        chart_id: c.id,
                        label: SharedString::from(c.label),
                        subtitle: Some(SharedString::from(format!("{:?}", c.kind))),
                        preview: None,
                    })
                    .collect();
                self.canvas.update(cx, |c, cx| {
                    c.set_mode(
                        CanvasMode::Thumbnails {
                            scope: ThumbnailScope::Contact(id),
                            items,
                        },
                        cx,
                    );
                });
                self.panel.update(cx, |p, cx| p.set_active_kind(None, cx));
            }
            TreeSelection::Group(id) => {
                self.current_chart = None;
                self.current_offset_minutes = 0;
                let charts = self.store.charts_under_group(id).unwrap_or_default();
                let items: Vec<ThumbnailItem> = charts
                    .into_iter()
                    .map(|c| ThumbnailItem {
                        chart_id: c.id,
                        label: SharedString::from(c.label),
                        subtitle: Some(SharedString::from(format!("{:?}", c.kind))),
                        preview: None,
                    })
                    .collect();
                self.canvas.update(cx, |c, cx| {
                    c.set_mode(
                        CanvasMode::Thumbnails {
                            scope: ThumbnailScope::Group(id),
                            items,
                        },
                        cx,
                    );
                });
                self.panel.update(cx, |p, cx| p.set_active_kind(None, cx));
            }
            TreeSelection::FreeChart(id) => {
                // Si es "Cielo ahora", refrescamos el reloj antes de
                // renderizar — el usuario espera ver el momento actual,
                // no el momento al que se cargó la carta.
                if id.is_sky_now() {
                    self.free_charts
                        .insert(FreeChartId::sky_now(), build_present_sky_chart());
                    self.push_free_charts_to_tree(cx);
                }
                let Some(chart) = self.free_charts.get(&id).cloned() else {
                    eprintln!("[shell] free chart {:?} no encontrada", id);
                    return;
                };
                self.current_chart = Some(chart);
                self.current_offset_minutes = 0;
                self.module_configs.clear();
                self.panel
                    .update(cx, |p, cx| p.set_active_kind(Some(ChartKind::Natal), cx));
                self.sync_panel_from_configs(cx);
                self.render_current(cx);
            }
            TreeSelection::FreeChartsRoot => {
                // Grilla de thumbnails de las cartas libres. Como
                // ThumbnailItem requiere ChartId, usamos `default()`
                // para las libres — el canvas las muestra como
                // entradas no-clickeables (eso está OK; el usuario
                // hace click en el row del tree para seleccionar).
                self.current_chart = None;
                self.current_offset_minutes = 0;
                let items: Vec<ThumbnailItem> = self
                    .free_charts
                    .values()
                    .map(|c| ThumbnailItem {
                        chart_id: c.id,
                        label: SharedString::from(c.label.clone()),
                        subtitle: Some(SharedString::from("libre".to_string())),
                        preview: None,
                    })
                    .collect();
                self.canvas.update(cx, |c, cx| {
                    c.set_mode(
                        CanvasMode::Thumbnails {
                            scope: ThumbnailScope::Group(Default::default()),
                            items,
                        },
                        cx,
                    );
                });
                self.panel.update(cx, |p, cx| p.set_active_kind(None, cx));
            }
            TreeSelection::GeneralRoot => {
                // "General" agrupa los contactos sin grupo padre. El
                // canvas muestra thumbnails de TODAS las cartas de
                // esos contactos.
                self.current_chart = None;
                self.current_offset_minutes = 0;
                let mut items: Vec<ThumbnailItem> = Vec::new();
                if let Ok(contacts) = self.store.list_contacts(None) {
                    for ct in contacts {
                        if let Ok(charts) = self.store.list_charts(ct.id) {
                            for c in charts {
                                items.push(ThumbnailItem {
                                    chart_id: c.id,
                                    label: SharedString::from(c.label),
                                    subtitle: Some(SharedString::from(format!(
                                        "{} · {:?}",
                                        ct.name, c.kind
                                    ))),
                                    preview: None,
                                });
                            }
                        }
                    }
                }
                // Reusamos el scope Group con un id sentinela "vacío":
                // como GeneralRoot no es un Group real, dejamos que el
                // canvas pinte la grilla con el set de items y nada
                // más — el `scope` no se usa para nada que requiera
                // el id.
                self.canvas.update(cx, |c, cx| {
                    c.set_mode(
                        CanvasMode::Thumbnails {
                            scope: ThumbnailScope::Group(Default::default()),
                            items,
                        },
                        cx,
                    );
                });
                self.panel.update(cx, |p, cx| p.set_active_kind(None, cx));
            }
        }
    }

    /// Deriva los `PipelineRequest` activos a partir del `module_configs`.
    fn build_requests(&self) -> Vec<PipelineRequest> {
        let mut requests = Vec::new();
        if module_enabled(&self.module_configs, "transit") {
            requests.push(PipelineRequest::Transit);
        }
        if module_enabled(&self.module_configs, "progression") {
            let age = self.module_age_or_current("progression");
            requests.push(PipelineRequest::SecondaryProgression {
                target_age_years: age,
            });
        }
        if module_enabled(&self.module_configs, "solar_arc") {
            let age = self.module_age_or_current("solar_arc");
            requests.push(PipelineRequest::SolarArc {
                target_age_years: age,
            });
        }
        if module_enabled(&self.module_configs, "synastry") {
            if let Some(partner) = self.resolve_synastry_partner() {
                requests.push(PipelineRequest::Synastry {
                    partner_chart: Box::new(partner),
                });
            }
        }
        if module_enabled(&self.module_configs, "midpoints") {
            requests.push(PipelineRequest::Midpoints);
        }
        if module_enabled(&self.module_configs, "uranian") {
            requests.push(PipelineRequest::Uranian);
        }
        if module_enabled(&self.module_configs, "lots") {
            requests.push(PipelineRequest::Lots);
        }
        if module_enabled(&self.module_configs, "fixed_stars") {
            requests.push(PipelineRequest::FixedStars);
        }
        if module_enabled(&self.module_configs, "topocentric") {
            requests.push(PipelineRequest::Topocentric);
        }
        if module_enabled(&self.module_configs, "primary_directions") {
            let age = self.module_age_or_current("primary_directions");
            let key = self
                .module_configs
                .get("primary_directions")
                .and_then(|c| c.get("key"))
                .and_then(|v| v.as_str())
                .unwrap_or("naibod")
                .to_string();
            requests.push(PipelineRequest::PrimaryDirections {
                target_age_years: age,
                key,
            });
        }
        if module_enabled(&self.module_configs, "composite") {
            if let Some(partner) = self.resolve_composite_partner() {
                requests.push(PipelineRequest::Composite {
                    partner_chart: Box::new(partner),
                });
            }
        }
        if module_enabled(&self.module_configs, "planetary_return") {
            let age = self.module_age_or_current("planetary_return");
            let body = self
                .module_configs
                .get("planetary_return")
                .and_then(|c| c.get("body"))
                .and_then(|v| v.as_str())
                .unwrap_or("sun")
                .to_string();
            let shift_days = self
                .module_configs
                .get("planetary_return")
                .and_then(|c| c.get("shift_days"))
                .and_then(|v| v.as_f64())
                .map(|v| v as i64)
                .unwrap_or(0);
            requests.push(PipelineRequest::PlanetaryReturn {
                body,
                target_age_years: age,
                shift_days,
            });
        }
        requests
    }

    /// Resuelve la carta partner para sinastría: 1) si el picker tiene
    /// un `partner_chart_id` válido en `module_configs`, lo usa; 2)
    /// si no, cae al automático (primera carta hermana del contacto
    /// actual). `None` si nada matchea — el request se salta.
    fn resolve_synastry_partner(&self) -> Option<Chart> {
        let manual = self
            .module_configs
            .get("synastry")
            .and_then(|c| c.get("partner_chart_id"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<cosmobiologia_model::ChartId>().ok())
            .and_then(|id| self.store.get_chart(id).ok());
        manual.or_else(|| self.find_synastry_partner_auto())
    }

    fn find_synastry_partner_auto(&self) -> Option<Chart> {
        let current = self.current_chart.as_ref()?;
        let siblings = self.store.list_charts(current.contact_id).ok()?;
        siblings.into_iter().find(|c| c.id != current.id)
    }

    /// Resuelve el partner para Composite — mismo patrón que Synastry:
    /// 1) lee module_configs["composite"]["partner_chart_id"] y resuelve
    /// el chart; 2) fallback al primer hermano del contacto actual.
    fn resolve_composite_partner(&self) -> Option<Chart> {
        let manual = self
            .module_configs
            .get("composite")
            .and_then(|c| c.get("partner_chart_id"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<cosmobiologia_model::ChartId>().ok())
            .and_then(|id| self.store.get_chart(id).ok());
        manual.or_else(|| self.find_synastry_partner_auto())
    }

    /// Deriva las `NatalOptions` activas a partir del `module_configs["natal"]`.
    /// Si la entry no existe, devuelve defaults (majors=true, minors=false,
    /// multiplier=1.0).
    fn build_natal_options(&self) -> NatalOptions {
        let cfg = self.module_configs.get("natal");
        let read_bool = |key: &str, default: bool| -> bool {
            cfg.and_then(|c| c.get(key))
                .and_then(|v| v.as_bool())
                .unwrap_or(default)
        };
        let read_f64 = |key: &str, default: f64| -> f64 {
            cfg.and_then(|c| c.get(key))
                .and_then(|v| v.as_f64())
                .unwrap_or(default)
        };
        NatalOptions {
            show_majors: read_bool("aspect_majors", true),
            show_minors: read_bool("aspect_minors", false),
            orb_multiplier: read_f64("orb_multiplier", 1.0),
            show_dignities: read_bool("show_dignities", false),
            harmonic: read_f64("harmonic", 1.0).round().clamp(1.0, 64.0) as u32,
        }
    }

    /// Lee `module_state` desde SQLite para la carta dada y los mergea
    /// con los defaults ya cargados en `module_configs`. Los valores
    /// persistidos ganan sobre los defaults.
    fn load_persisted_module_states(&mut self, chart_id: ChartId) {
        let states = match self.store.list_module_states(chart_id) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[shell] list_module_states {}: {}", chart_id, e);
                return;
            }
        };
        for st in states {
            // Re-mergeamos `enabled` (columna separada en SQL) dentro
            // del JSON config, así el resto del shell sigue leyendo
            // todo desde una única estructura.
            let mut combined = match st.config {
                serde_json::Value::Object(m) => serde_json::Value::Object(m),
                _ => serde_json::json!({}),
            };
            if let serde_json::Value::Object(map) = &mut combined {
                map.insert("enabled".into(), serde_json::Value::Bool(st.enabled));
            }
            // Mergear sobre defaults previos (no sobreescribir si la
            // entrada nueva no trae un campo).
            match self.module_configs.entry(st.module_id) {
                std::collections::hash_map::Entry::Vacant(v) => {
                    v.insert(combined);
                }
                std::collections::hash_map::Entry::Occupied(mut o) => {
                    if let (serde_json::Value::Object(dst), serde_json::Value::Object(src)) =
                        (o.get_mut(), &combined)
                    {
                        for (k, v) in src {
                            dst.insert(k.clone(), v.clone());
                        }
                    } else {
                        o.insert(combined);
                    }
                }
            }
        }
    }

    /// Pushea cada toggle/slider/picker del `module_configs` al panel
    /// para que la UI refleje el estado persistido al cargar una carta.
    fn sync_panel_from_configs(&mut self, cx: &mut Context<Self>) {
        let snapshot: Vec<(String, serde_json::Value)> = self
            .module_configs
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        self.panel.update(cx, |p, cx| {
            for (module_id, config) in &snapshot {
                if let serde_json::Value::Object(map) = config {
                    for (key, value) in map {
                        if let Some(b) = value.as_bool() {
                            p.set_toggle(module_id, key, b, cx);
                        } else if let Some(f) = value.as_f64() {
                            p.set_slider(module_id, key, f, cx);
                        } else if let Some(s) = value.as_str() {
                            p.set_string(module_id, key, Some(s.to_string()), cx);
                        } else if value.is_null() {
                            p.set_string(module_id, key, None, cx);
                        }
                    }
                }
            }
        });
    }

    /// Persiste el estado actual de un módulo a SQLite. Extrae
    /// `enabled` del JSON y lo guarda en la columna dedicada; el resto
    /// va al `config_json`.
    fn persist_module(&self, module_id: &str) {
        let Some(chart) = self.current_chart.as_ref() else {
            return;
        };
        let Some(config) = self.module_configs.get(module_id) else {
            return;
        };
        let enabled = config
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let mut clean = config.clone();
        if let serde_json::Value::Object(map) = &mut clean {
            map.remove("enabled");
        }
        let state = ModuleState {
            chart_id: chart.id,
            module_id: module_id.to_string(),
            enabled,
            config: clean,
        };
        if let Err(e) = self.store.upsert_module_state(&state) {
            eprintln!("[shell] upsert_module_state {}: {}", module_id, e);
        }
    }

    /// Lee `target_age_years` del módulo o cae a la edad actual del
    /// sujeto (calculada desde la fecha de nacimiento y el reloj).
    fn module_age_or_current(&self, module_id: &str) -> f64 {
        self.module_configs
            .get(module_id)
            .and_then(|c| c.get("target_age_years"))
            .and_then(|v| v.as_f64())
            .unwrap_or_else(|| {
                self.current_chart
                    .as_ref()
                    .map(|c| current_age_years(&c.birth_data))
                    .unwrap_or(0.0)
            })
    }

    fn render_current(&mut self, cx: &mut Context<Self>) {
        let Some(chart) = self.current_chart.as_ref() else {
            return;
        };
        // Snapshot de inputs para mover al background. La sesión
        // VSOP2013 vive en un static `OnceLock` adentro del bridge, así
        // que es compartible read-only entre threads sin que ningún
        // dato cruce más allá del Chart clonado + requests/options.
        let chart = chart.clone();
        let offset = self.current_offset_minutes;
        let requests = self.build_requests();
        let natal_options = self.build_natal_options();
        self.render_seq = self.render_seq.wrapping_add(1);
        let my_seq = self.render_seq;

        cx.spawn(async move |this, cx| {
            // El compute corre en el background_executor — no bloquea
            // el UI thread. Para una rueda completa con varios overlays
            // puede tomar 100-200ms; sin esto, los drags del slider se
            // sentirían atorados.
            let chart_for_bg = chart.clone();
            let requests_for_bg = requests.clone();
            let opts_for_bg = natal_options.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    compose_with_options(&chart_for_bg, offset, &requests_for_bg, &opts_for_bg)
                })
                .await;

            let _ = this.update(cx, |this, cx| {
                // Descartar si llegó un render más nuevo en el medio.
                // Sin este check, durante un drag rápido un compute
                // viejo podría sobrescribir el más reciente.
                if this.render_seq != my_seq {
                    return;
                }
                match result {
                    Ok(render) => {
                        this.canvas.update(cx, |c, cx| {
                            c.set_mode(
                                CanvasMode::Wheel {
                                    render: Box::new(render),
                                },
                                cx,
                            );
                        });
                    }
                    Err(e) => {
                        eprintln!(
                            "[shell] compose {} (+{}min, {} reqs): {}",
                            chart.id,
                            offset,
                            requests.len(),
                            e
                        );
                    }
                }
            });
        })
        .detach();
    }

    fn on_canvas_event(&mut self, ev: &CanvasEvent, cx: &mut Context<Self>) {
        match ev {
            CanvasEvent::TimeOffsetChanged(off) => {
                self.current_offset_minutes = *off;
                if self.current_chart.is_some() {
                    self.render_current(cx);
                }
            }
            CanvasEvent::LayerVisibilityChanged { kind, visible } => {
                // El toggle de Outer ([T]) no es visibility puro: dispara
                // un pipeline distinto. Lo traducimos a un cambio en
                // module_configs["transit"]["enabled"] + re-render.
                if matches!(kind, LayerKind::Outer) {
                    set_module_enabled(&mut self.module_configs, "transit", *visible);
                    self.persist_module("transit");
                    self.panel.update(cx, |p, cx| {
                        p.set_toggle("transit", "enabled", *visible, cx)
                    });
                    self.render_current(cx);
                    return;
                }
                // El resto son visibility puros sobre el canvas. Sync el
                // panel para que el toggle visual coincida con la hotkey.
                let key = match kind {
                    LayerKind::SignDial => "show_sign_dial",
                    LayerKind::Houses => "show_houses",
                    LayerKind::Aspects => "show_aspects",
                    LayerKind::Bodies => "show_bodies",
                    _ => return,
                };
                self.panel
                    .update(cx, |p, cx| p.set_toggle("natal", key, *visible, cx));
            }
            CanvasEvent::ShowCoordsChanged(visible) => {
                // Sync el toggle del panel para que coincida con la
                // hotkey C. No persist — los coord labels son una
                // preferencia visual, no parte del module_state.
                self.panel.update(cx, |p, cx| {
                    p.set_toggle("natal", "show_coords", *visible, cx)
                });
            }
            CanvasEvent::ChartRequested(_) => {
                // Fase 7: doble click sobre un thumbnail abre la carta.
            }
            CanvasEvent::ExportSvgRequested => {
                self.export_current_to_svg();
            }
            CanvasEvent::GrAgeDelta(delta) => {
                self.scrub_gr_age(*delta, cx);
            }
            CanvasEvent::HarmonicSelected(n) => {
                self.select_harmonic(*n, cx);
            }
        }
    }

    /// Fija el armónico de la carta natal (clic en una barra del
    /// espectro): escribe `harmonic` en `module_configs["natal"]`,
    /// sincroniza el slider del panel y recompone.
    fn select_harmonic(&mut self, n: u32, cx: &mut Context<Self>) {
        let entry = self
            .module_configs
            .entry("natal".into())
            .or_insert_with(|| serde_json::json!({}));
        if let serde_json::Value::Object(map) = entry {
            map.insert("harmonic".into(), serde_json::json!(n));
        }
        self.panel.update(cx, |p, cx| {
            p.set_slider("natal", "harmonic", n as f64, cx)
        });
        self.persist_module("natal");
        self.render_current(cx);
    }

    /// Scrubbing en vivo de la edad GR vía jog-dial. Acumula `delta`
    /// sobre `target_age_years` del módulo `primary_directions`,
    /// clampa a [0,120], sincroniza el slider del panel y recompone.
    fn scrub_gr_age(&mut self, delta_years: f64, cx: &mut Context<Self>) {
        if !module_enabled(&self.module_configs, "primary_directions") {
            return;
        }
        let current = self.module_age_or_current("primary_directions");
        let next = (current + delta_years).clamp(0.0, 120.0);
        if (next - current).abs() < 1e-6 {
            return;
        }
        let entry = self
            .module_configs
            .entry("primary_directions".into())
            .or_insert_with(|| serde_json::json!({}));
        if let serde_json::Value::Object(map) = entry {
            map.insert("target_age_years".into(), serde_json::json!(next));
        }
        self.panel.update(cx, |p, cx| {
            p.set_slider("primary_directions", "target_age_years", next, cx)
        });
        self.persist_module("primary_directions");
        self.render_current(cx);
    }

    /// Recompone la carta actual + escribe el SVG a un archivo en
    /// `$XDG_DATA_HOME/cosmobiologia/exports/<label>_<short_id>.svg`.
    /// Logea la ruta a stderr — futuro: file save dialog GPUI.
    fn export_current_to_svg(&self) {
        let Some(chart) = self.current_chart.as_ref() else {
            eprintln!("[shell] export svg: sin carta activa");
            return;
        };
        let requests = self.build_requests();
        let natal_options = self.build_natal_options();
        let render = match compose_with_options(
            chart,
            self.current_offset_minutes,
            &requests,
            &natal_options,
        ) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[shell] export svg compose: {}", e);
                return;
            }
        };
        let svg = svg_export::render_to_svg(&render);
        let dir = directories::ProjectDirs::from("net", "gioser", "cosmobiologia")
            .map(|d| d.data_dir().join("exports"))
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("[shell] mkdir {:?}: {}", dir, e);
            return;
        }
        let safe_label: String = chart
            .label
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let short = format!("{}", chart.id).chars().take(8).collect::<String>();
        let path = dir.join(format!("{}_{}.svg", safe_label, short));
        if let Err(e) = std::fs::write(&path, svg) {
            eprintln!("[shell] write {:?}: {}", path, e);
        } else {
            eprintln!("[shell] SVG exportado → {}", path.display());
        }
    }

    fn on_panel_event(&mut self, ev: &PanelEvent, cx: &mut Context<Self>) {
        match ev {
            PanelEvent::ControlChanged {
                module_id, key, value,
            } => {
                let bool_val = value.as_bool().unwrap_or(true);
                if module_id == "natal" {
                    // Distinguimos: show_* = visibility (no recompose),
                    // aspect_*/orb_* = filtros de engine (recompose +
                    // persist).
                    let kind = match key.as_str() {
                        "show_sign_dial" => Some(LayerKind::SignDial),
                        "show_houses" => Some(LayerKind::Houses),
                        "show_aspects" => Some(LayerKind::Aspects),
                        "show_bodies" => Some(LayerKind::Bodies),
                        _ => None,
                    };
                    if let Some(k) = kind {
                        self.canvas
                            .update(cx, |c, cx| c.set_layer_visible(k, bool_val, cx));
                    } else if key == "show_coords" {
                        // Coord labels viven en el canvas (no son una
                        // capa pintada como otros show_*). Sync sin
                        // recompose ni persist en module_state.
                        self.canvas
                            .update(cx, |c, cx| c.set_show_coords(bool_val, cx));
                    } else {
                        // Filtros: actualizar module_configs + recompose.
                        let entry = self
                            .module_configs
                            .entry("natal".into())
                            .or_insert_with(|| serde_json::json!({}));
                        if let serde_json::Value::Object(map) = entry {
                            map.insert(key.clone(), value.clone());
                        }
                        self.persist_module("natal");
                        self.render_current(cx);
                    }
                } else {
                    // Cualquier otro módulo: actualizamos su config y
                    // recompomemos. La engine vuelve a llamarse con el
                    // PipelineRequest derivado del nuevo estado.
                    let entry = self
                        .module_configs
                        .entry(module_id.clone())
                        .or_insert_with(|| serde_json::json!({}));
                    if let serde_json::Value::Object(map) = entry {
                        map.insert(key.clone(), value.clone());
                    }
                    // Transit, Synastry y Solar Return comparten el
                    // outer ring del canvas — son mutuamente excluyentes.
                    // Al prender uno, apagamos los otros + sync panel +
                    // persist.
                    if key == "enabled" && bool_val && OUTER_RING_MODULES.contains(&module_id.as_str()) {
                        for &other in OUTER_RING_MODULES.iter() {
                            if other != module_id && module_enabled(&self.module_configs, other) {
                                set_module_enabled(&mut self.module_configs, other, false);
                                let other_str = other.to_string();
                                self.panel.update(cx, |p, cx| {
                                    p.set_toggle(&other_str, "enabled", false, cx)
                                });
                                self.persist_module(&other_str);
                            }
                        }
                    }
                    // Sincronizar visualmente el toggle [T] del canvas
                    // cuando el cambio afecta el outer ring (transit,
                    // synastry o solar_return).
                    if OUTER_RING_MODULES.contains(&module_id.as_str()) && key == "enabled" {
                        self.canvas.update(cx, |c, cx| {
                            c.set_layer_visible(LayerKind::Outer, bool_val, cx)
                        });
                    }
                    self.persist_module(module_id);
                    self.render_current(cx);
                }
            }
            PanelEvent::ModuleToggled { .. } => {
                // Fase 7: encender/apagar módulos enteros desde un
                // header con switch (vs. el toggle por-control de hoy).
            }
            PanelEvent::Action { module_id, key } => {
                self.on_panel_action(module_id.clone(), key.clone(), cx);
            }
        }
    }

    /// Click sobre un `Control::Action` del panel. Por ahora maneja:
    /// - planetary_return.save_as_free → captura la carta del
    ///   retorno actual (cuerpo + edad) como FreeChart con sufijo
    ///   `rs-{N}` / `lunar-{N}` / etc. según el cuerpo elegido.
    ///
    /// Otros módulos overlay (progression, solar_arc, primary_directions)
    /// son extensión natural — TODO.
    fn on_panel_action(&mut self, module_id: String, key: String, cx: &mut Context<Self>) {
        match key.as_str() {
            "save_as_free" => match module_id.as_str() {
                "planetary_return" => self.save_planetary_return_as_free(cx),
                "transit" => self.save_transit_as_free(cx),
                "progression" => self.save_progression_as_free(cx),
                // Solar arc y direcciones primarias son transformaciones
                // matemáticas puras (no tienen un birth_data real
                // equivalente). Guardarlas exigiría un `ChartKind`
                // `Derived { source, transform, params }`. TODO.
                _ => {}
            },
            "rectificar" => self.run_rectificacion(cx),
            _ => {}
        }
    }

    /// Lanza el rectificador automático (Sistema GR): lee las edades de
    /// los eventos conocidos de los sliders del módulo, barre las horas
    /// candidatas y escribe el resultado en el campo «Resultado» del
    /// panel. El barrido es síncrono — para ±15 min son ~31 cartas.
    fn run_rectificacion(&mut self, cx: &mut Context<Self>) {
        // Clonamos la carta: `rectificar` necesita `&Chart` y luego
        // `panel.update` toma `&mut self` — no pueden solaparse.
        let Some(chart) = self.current_chart.clone() else {
            return;
        };
        let cfg = self.module_configs.get("primary_directions");
        let read_age = |key: &str| -> f64 {
            cfg.and_then(|c| c.get(key))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
        };
        // Edades > 0 — una ranura en 0 es "sin usar".
        let eventos: Vec<EventoConocido> = ["evento_1", "evento_2", "evento_3"]
            .iter()
            .map(|k| read_age(k))
            .filter(|edad| *edad > 0.5)
            .map(|edad| EventoConocido { edad_years: edad })
            .collect();
        let key_gr = cfg
            .and_then(|c| c.get("key"))
            .and_then(|v| v.as_str())
            .unwrap_or("naibod")
            .to_string();

        // Ventana ±15 min — dos pasadas (minuto grueso, segundo fino).
        match cosmobiologia_engine::rectificar(&chart, &eventos, 15, &key_gr) {
            Ok(r) => {
                // Offset en segundos → texto «±Xm Ys».
                let seg = r.mejor_offset_segundos;
                let signo = if seg < 0 { "-" } else { "+" };
                let abs = seg.abs();
                let resumen = format!(
                    "{signo}{}m {:02}s · error {:.2}a",
                    abs / 60,
                    abs % 60,
                    r.mejor_puntaje
                );
                self.panel.update(cx, |p, cx| {
                    p.set_string("primary_directions", "resultado", Some(resumen), cx)
                });
                // Publicar el perfil al canvas: dibuja la curva del
                // barrido, cuyo valle marca la hora rectificada.
                self.canvas
                    .update(cx, |c, cx| c.set_rectificacion(Some(r), cx));
            }
            Err(_) => {
                self.panel.update(cx, |p, cx| {
                    p.set_string(
                        "primary_directions",
                        "resultado",
                        Some("define al menos un evento (edad > 0)".to_string()),
                        cx,
                    )
                });
            }
        }
    }

    /// Snapshot del cielo en este instante anclado al lugar del
    /// natal. Sufijo `transito-{fecha}`. Útil para guardar "qué
    /// estaba pasando ahora en la carta de Pedro".
    fn save_transit_as_free(&mut self, cx: &mut Context<Self>) {
        let Some(natal) = self.current_chart.as_ref() else {
            eprintln!("[shell] save_transit: sin carta activa");
            return;
        };
        if natal.id == ChartId::default() {
            eprintln!("[shell] save_transit: la carta activa es libre");
            return;
        }
        match cosmobiologia_engine::compute_transit_chart(natal) {
            Ok((birth, instant_label)) => {
                let label = format!("{} transito · {}", natal.label, instant_label);
                self.insert_derived_free_chart(natal.clone(), birth, label, cx);
            }
            Err(e) => eprintln!("[shell] compute_transit_chart: {}", e),
        }
    }

    /// Carta progresada secundaria a la edad del slider. La
    /// progresada es natal + N días simbólicos; el Chart resultante
    /// tiene un birth_data REAL (no simbólico) — cuando se computa
    /// como natal de nuevo, da las posiciones progresadas correctas.
    /// Sufijo `prog-{N}a`.
    fn save_progression_as_free(&mut self, cx: &mut Context<Self>) {
        let Some(natal) = self.current_chart.as_ref() else {
            eprintln!("[shell] save_progression: sin carta activa");
            return;
        };
        if natal.id == ChartId::default() {
            eprintln!("[shell] save_progression: la carta activa es libre");
            return;
        }
        let age = self.module_age_or_current("progression");
        match cosmobiologia_engine::compute_progression_chart(natal, age) {
            Ok((birth, instant_label)) => {
                let label = format!(
                    "{} prog-{:.0}a · {}",
                    natal.label, age, instant_label
                );
                self.insert_derived_free_chart(natal.clone(), birth, label, cx);
            }
            Err(e) => eprintln!("[shell] compute_progression_chart: {}", e),
        }
    }

    /// Inserta un Chart derivado (transit/progression/PR) como
    /// FreeChart conservando contact/kind/related/config del natal
    /// original. El id es sintético; el usuario puede después
    /// "Guardar como…" para persistirlo bajo un contacto.
    fn insert_derived_free_chart(
        &mut self,
        source_natal: Chart,
        new_birth: StoredBirthData,
        new_label: String,
        cx: &mut Context<Self>,
    ) {
        let id = FreeChartId(format!("free-{}", self.next_free_id));
        self.next_free_id += 1;
        let mut chart = source_natal;
        chart.id = ChartId::default();
        chart.label = new_label;
        chart.birth_data = new_birth;
        self.free_charts.insert(id.clone(), chart);
        self.push_free_charts_to_tree(cx);
        self.apply_selection(TreeSelection::FreeChart(id), cx);
    }

    /// Computa la carta del retorno planetario actual (con cuerpo +
    /// edad del módulo) y la inserta como FreeChart. El usuario
    /// puede después "Guardar como…" para persistirla bajo un
    /// contacto (típicamente el mismo del natal).
    fn save_planetary_return_as_free(&mut self, cx: &mut Context<Self>) {
        let Some(natal) = self.current_chart.as_ref() else {
            eprintln!("[shell] save_planetary_return: sin carta activa");
            return;
        };
        if natal.id == ChartId::default() {
            eprintln!("[shell] save_planetary_return: la carta activa es libre, no natal");
            return;
        }
        let cfg = self
            .module_configs
            .get("planetary_return")
            .cloned()
            .unwrap_or(serde_json::json!({}));
        let body = cfg
            .get("body")
            .and_then(|v| v.as_str())
            .unwrap_or("sun")
            .to_string();
        let age = self.module_age_or_current("planetary_return");
        let shift_days = cfg
            .get("shift_days")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0) as i64;

        // Pedimos al engine la fecha exacta del retorno. La engine
        // expone `compute_planetary_return_chart` que devuelve un
        // `StoredBirthData` listo para reusar como carta natal.
        match cosmobiologia_engine::compute_planetary_return_chart(
            natal, &body, age, shift_days,
        ) {
            Ok((birth, instant_label)) => {
                let suffix = match body.as_str() {
                    "sun" => "rs",
                    "moon" => "lunar",
                    other => other,
                };
                let label = format!(
                    "{} {}-{:.0}a · {}",
                    natal.label, suffix, age, instant_label
                );
                self.insert_derived_free_chart(natal.clone(), birth, label, cx);
            }
            Err(e) => {
                eprintln!("[shell] compute_planetary_return_chart: {}", e);
            }
        }
    }
}

// =====================================================================
// Helpers de module_configs
// =====================================================================

// OUTER_RING_MODULES viene de cosmobiologia_engine — single source of
// truth. Shell y canvas leen del mismo slice.


/// Lee `$XDG_DATA_HOME/cosmobiologia/atlas.tsv` si existe y lo parsea
/// como atlas de ciudades. Devuelve `None` cuando no hay archivo o
/// quedó vacío después del parse — el tree cae al atlas hardcoded.
fn load_city_atlas_from_xdg() -> Option<Vec<cosmobiologia_tree::CityPreset>> {
    let path = directories::ProjectDirs::from("net", "gioser", "cosmobiologia")
        .map(|d| d.data_dir().join("atlas.tsv"))?;
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&path).ok()?;
    let atlas = parse_city_atlas_tsv(&content);
    if atlas.is_empty() {
        eprintln!(
            "[shell] atlas.tsv encontrado en {:?} pero sin filas válidas — fallback a hardcoded",
            path
        );
        return None;
    }
    eprintln!("[shell] atlas custom cargado: {} ciudades", atlas.len());
    Some(atlas)
}

/// Carta efímera del "Cielo ahora": birth_data = momento actual en
/// Greenwich (UTC, lat 51.4769°, lon 0°). El `Chart` se construye al
/// vuelo, NO se persiste en la store, y los IDs son `Default` (todo
/// ceros) — la carta es un singleton conceptual de la vista, no un
/// registro. Los módulos que consultan `current_chart.id` deben
/// tolerar este ID sentinela.
fn build_present_sky_chart() -> Chart {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (year, month, day, hour, minute, second) = unix_to_civil_utc(secs);
    let birth = StoredBirthData {
        year,
        month,
        day,
        hour,
        minute,
        second: second as f64,
        tz_offset_minutes: 0,
        // Greenwich Royal Observatory — origen histórico del meridiano
        // primario. Lat = 51°28'38"N, Lon = 0°.
        latitude_deg: 51.4769,
        longitude_deg: 0.0,
        altitude_m: 47.0,
        time_certainty: Default::default(),
        subject_name: Some("Cielo".into()),
        birthplace_label: Some("Greenwich (UTC)".into()),
    };
    Chart {
        id: ChartId::default(),
        contact_id: ContactId::default(),
        kind: ChartKind::Natal,
        label: format!(
            "Cielo {:04}-{:02}-{:02} {:02}:{:02} UTC",
            year, month, day, hour, minute
        ),
        birth_data: birth,
        config: StoredChartConfig::default(),
        related_chart_id: None,
        created_at_ms: 0,
    }
}

/// Convierte un timestamp Unix (segundos UTC desde 1970-01-01) a
/// componentes calendario proleptic-Gregorianos `(year, month, day,
/// hour, minute, second)`. Algoritmo de Howard Hinnant
/// (`days_to_civil`), exacto en todo el rango representable por i64.
fn unix_to_civil_utc(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    let day_seconds: i64 = 86_400;
    let z = secs.div_euclid(day_seconds);
    let s = secs.rem_euclid(day_seconds);
    // Hinnant: shift z para que el "era" empiece en 0000-03-01.
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { (y + 1) as i32 } else { y as i32 };
    let hour = (s / 3600) as u32;
    let minute = ((s % 3600) / 60) as u32;
    let second = (s % 60) as u32;
    (year, month, day, hour, minute, second)
}

/// Etiqueta breve para mostrar al elegir una carta en el picker:
/// `"YYYY-MM-DD · Lugar"` cuando hay lugar, sino solo la fecha.
fn format_birth_brief(birth: &cosmobiologia_model::StoredBirthData) -> String {
    let date = format!("{:04}-{:02}-{:02}", birth.year, birth.month, birth.day);
    match &birth.birthplace_label {
        Some(p) if !p.is_empty() => format!("{} · {}", date, p),
        _ => date,
    }
}

/// Edad en años decimales desde el nacimiento hasta el reloj actual.
/// Aproximación: ignora la TZ de nacimiento (no afecta a resolución de
/// año) y usa una fracción de año tropical sobre los segundos Unix.
fn current_age_years(birth: &cosmobiologia_model::StoredBirthData) -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let birth_year_frac = birth.year as f64
        + (birth.month.saturating_sub(1) as f64) / 12.0
        + (birth.day.saturating_sub(1) as f64) / 365.25;
    let now_year_frac = 1970.0 + now_secs / (365.2422 * 86400.0);
    (now_year_frac - birth_year_frac).max(0.0)
}

fn module_enabled(cfgs: &HashMap<String, serde_json::Value>, id: &str) -> bool {
    cfgs.get(id)
        .and_then(|c| c.get("enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn set_module_enabled(
    cfgs: &mut HashMap<String, serde_json::Value>,
    id: &str,
    enabled: bool,
) {
    let entry = cfgs
        .entry(id.to_string())
        .or_insert_with(|| serde_json::json!({}));
    if let serde_json::Value::Object(map) = entry {
        map.insert("enabled".into(), serde_json::Value::Bool(enabled));
    }
}

/// Lee del `settings` el flex de un splitter en formato "f0,f1,..." y
/// lo devuelve como `Vec<f32>` con la misma longitud que `defaults`.
/// Si no hay nada persistido, faltan campos, o algún flex es ≤0, cae a
/// `defaults`. Validación estricta porque un flex 0 colapsa al panel.
fn load_split_flex_n(store: &Store, key: &str, defaults: &[f32]) -> Vec<f32> {
    let Ok(Some(raw)) = store.get_setting(key) else {
        return defaults.to_vec();
    };
    let parsed: Vec<f32> = raw
        .split(',')
        .filter_map(|s| s.trim().parse::<f32>().ok())
        .collect();
    if parsed.len() != defaults.len() || parsed.iter().any(|&f| f <= 0.0) {
        return defaults.to_vec();
    }
    parsed
}

/// Persiste los flex actuales de un splitter — soporta N children.
fn save_split_flex(store: &Store, key: &str, sc: &SplitContainer) {
    let children = sc.children();
    if children.is_empty() {
        return;
    }
    let payload: String = children
        .iter()
        .map(|c| format!("{:.4}", c.flex))
        .collect::<Vec<_>>()
        .join(",");
    if let Err(e) = store.set_setting(key, &payload) {
        eprintln!("[shell] save_split_flex {}: {}", key, e);
    }
}

/// Key del setting donde se persiste el splitter "outer" (el de mayor
/// nivel del árbol). En dock=Bottom guarda (main,panel); en docks
/// laterales guarda los flex de las 3 columnas — usamos keys distintas
/// para no pisar valores entre layouts.
fn split_key_outer(dock: PanelDock) -> &'static str {
    match dock {
        PanelDock::Bottom => "layout.outer_split",
        PanelDock::Right => "layout.dock_right",
        PanelDock::Left => "layout.dock_left",
    }
}

/// Key del setting del splitter horizontal interno. Solo se usa cuando
/// dock=Bottom (en docks laterales no hay main_split activo).
fn split_key_main(dock: PanelDock) -> &'static str {
    match dock {
        PanelDock::Bottom => "layout.main_split",
        // En docks laterales el main_split está dormido — escribir acá
        // no hace daño pero tampoco se usa al recargar.
        PanelDock::Right => "layout.main_split_right",
        PanelDock::Left => "layout.main_split_left",
    }
}

fn load_dock(store: &Store) -> Option<PanelDock> {
    let raw = store.get_setting("layout.panel_dock").ok().flatten()?;
    PanelDock::from_setting(raw.trim())
}

impl Shell {
    /// Tres botones compactos en el header — uno por dock disponible.
    /// El dock activo se marca con `bg=accent`; los demás van planos.
    /// Click llama a `apply_dock` que reorganiza splitters y persiste.
    fn render_dock_switcher(
        &self,
        theme: &nahual_theme::Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut row = div()
            .id("tts-dock-switcher")
            .flex()
            .flex_row()
            .gap(px(2.0))
            .px(px(2.0))
            .py(px(2.0))
            .rounded(px(6.0))
            .bg(theme.bg_panel_alt.clone())
            .border_1()
            .border_color(theme.border);

        for (dock, glyph) in [
            (PanelDock::Left, "◧"),
            (PanelDock::Bottom, "▭"),
            (PanelDock::Right, "◨"),
        ] {
            let active = self.dock == dock;
            let fg = if active { theme.fg_text } else { theme.fg_muted };
            let id: SharedString = SharedString::from(format!("tts-dock-{}", dock.as_setting()));
            let mut btn = div()
                .id(gpui::ElementId::from(id))
                .w(px(22.0))
                .h(px(20.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.0))
                .text_size(px(12.0))
                .text_color(fg)
                .hover(|s| s.bg(theme.bg_row_hover))
                .child(SharedString::from(glyph))
                .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                    if this.dock != dock {
                        this.apply_dock(dock, cx);
                    }
                }));
            if active {
                btn = btn.bg(theme.accent);
            }
            row = row.child(btn);
        }

        row
    }
}

impl Render for Shell {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();

        // Badge del estado del broker brahman — pequeña pill con
        // color según el estado actual del ping cada-30s.
        let (badge_text, badge_color) = match &self.brahman_status {
            BrahmanStatus::Pending => ("Brahman · …".to_string(), theme.fg_muted),
            BrahmanStatus::Connected { session_count } => (
                format!("Brahman ✓ {} sessions", session_count),
                theme.accent,
            ),
            BrahmanStatus::Offline { .. } => {
                ("Brahman · offline".to_string(), theme.fg_disabled)
            }
        };
        let brahman_badge = div()
            .px(px(8.0))
            .py(px(2.0))
            .rounded(px(8.0))
            .bg(theme.bg_panel_alt.clone())
            .border_1()
            .border_color(theme.border)
            .text_size(px(10.0))
            .text_color(badge_color)
            .child(SharedString::from(badge_text));

        let header = div()
            .h(px(34.0))
            .px(px(12.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(theme.fg_text)
                    .child("☉ Tahuantinsuyu"),
            )
            .child(
                div()
                    .text_size(px(10.0))
                    .text_color(theme.fg_muted)
                    .child("estudio de astrología profesional"),
            )
            .child(div().flex_grow())
            .child(self.render_dock_switcher(&theme, cx))
            .child(brahman_badge)
            .child(theme_switcher(cx));

        let body = div()
            .flex_grow()
            .w_full()
            .child(self.outer_split.clone());

        div()
            .size_full()
            .bg(theme.bg_app.clone())
            .flex()
            .flex_col()
            .child(header)
            .child(body)
    }
}

// =====================================================================
// Tests de integración del Shell
// =====================================================================
//
// Cubren los caminos que combinan lógica del shell con persistencia y
// el bridge real de eternal. Los tests puramente unitarios de cada
// crate (engine, store, modules) viven en sus respectivos `tests`
// modules; acá testeamos los wiring points del binario.

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[test]
    fn unix_to_civil_at_epoch() {
        assert_eq!(unix_to_civil_utc(0), (1970, 1, 1, 0, 0, 0));
    }

    #[test]
    fn unix_to_civil_known_dates() {
        // 2024-01-01T00:00:00 UTC = 1704067200
        assert_eq!(unix_to_civil_utc(1_704_067_200), (2024, 1, 1, 0, 0, 0));
        // 2024-02-29T12:34:56 UTC = año bisiesto
        let secs = 1_704_067_200 + (31 + 28) * 86_400 + 12 * 3600 + 34 * 60 + 56;
        assert_eq!(unix_to_civil_utc(secs), (2024, 2, 29, 12, 34, 56));
    }

    #[test]
    fn unix_to_civil_pre_epoch_wraps_correctly() {
        // -1 segundo = 1969-12-31T23:59:59 UTC
        assert_eq!(unix_to_civil_utc(-1), (1969, 12, 31, 23, 59, 59));
    }

    #[test]
    fn unix_to_civil_year_2000() {
        // 2000-01-01T00:00:00 UTC = 946684800
        assert_eq!(unix_to_civil_utc(946_684_800), (2000, 1, 1, 0, 0, 0));
    }

    fn sample_chart_for(_contact_id: ContactId) -> (StoredBirthData, StoredChartConfig) {
        (
            StoredBirthData {
                year: 1987,
                month: 3,
                day: 14,
                hour: 5,
                minute: 22,
                second: 0.0,
                tz_offset_minutes: -240,
                latitude_deg: 10.4806,
                longitude_deg: -66.9036,
                altitude_m: 900.0,
                time_certainty: Default::default(),
                subject_name: Some("Sergio".into()),
                birthplace_label: Some("Caracas".into()),
            },
            StoredChartConfig::default(),
        )
    }

    /// Smoke test: el Shell se construye sin panic con una store
    /// in-memory. Cubre que las suscripciones cross-widget (tree, panel,
    /// canvas, ambos splitters) se cablean sin colisiones y que el
    /// background loop del brahman status arranca limpio.
    #[gpui::test]
    fn shell_constructs_smoke(cx: &mut TestAppContext) {
        cx.update(|cx| {
            Theme::install_default(cx);
            let store = Store::in_memory().expect("in-memory store");
            let _shell = cx.new(|cx| Shell::new(store, cx));
            // Si llegamos acá sin panic, el cableado funciona.
        });
    }

    /// La selección de una carta vía `apply_selection` (mismo pathway
    /// que dispara el TreeEvent) puebla `current_chart` y arranca un
    /// compute. El render asíncrono se resuelve después; verificamos
    /// solo los efectos sincrónicos: chart cargada y `render_seq`
    /// avanzado.
    #[gpui::test]
    fn select_chart_updates_current(cx: &mut TestAppContext) {
        cx.update(|cx| {
            Theme::install_default(cx);
            let store = Store::in_memory().expect("store");
            let group = store.create_group(None, "Test", None).unwrap();
            let contact = store
                .create_contact(Some(group.id), "Subject", None)
                .unwrap();
            let (birth, config) = sample_chart_for(contact.id);
            let chart = store
                .create_chart(contact.id, ChartKind::Natal, "Natal", &birth, &config, None)
                .unwrap();

            let shell = cx.new(|cx| Shell::new(store, cx));
            shell.update(cx, |s, cx| {
                s.apply_selection(TreeSelection::Chart(chart.id), cx);
            });
            shell.read_with(cx, |s, _| {
                let cur = s.current_chart.as_ref().expect("current_chart set");
                assert_eq!(cur.id, chart.id);
                assert_eq!(cur.label, "Natal");
                assert!(s.render_seq >= 1, "render_seq debió avanzar al menos a 1");
            });
        });
    }

    /// Toggleando un módulo overlay vía `module_configs` directamente
    /// (simulando el efecto de un `PanelEvent::ControlChanged`), la
    /// función `build_requests` debe reflejar el cambio.
    #[gpui::test]
    fn module_toggles_produce_requests(cx: &mut TestAppContext) {
        cx.update(|cx| {
            Theme::install_default(cx);
            let store = Store::in_memory().expect("store");
            let shell = cx.new(|cx| Shell::new(store, cx));

            shell.update(cx, |s, _cx| {
                // Sin módulos activos → no hay requests.
                assert!(s.build_requests().is_empty());

                set_module_enabled(&mut s.module_configs, "transit", true);
                set_module_enabled(&mut s.module_configs, "midpoints", true);
                set_module_enabled(&mut s.module_configs, "uranian", true);

                let reqs = s.build_requests();
                assert_eq!(reqs.len(), 3);
                assert!(matches!(reqs[0], PipelineRequest::Transit));
                assert!(matches!(reqs[1], PipelineRequest::Midpoints));
                assert!(matches!(reqs[2], PipelineRequest::Uranian));

                set_module_enabled(&mut s.module_configs, "transit", false);
                let reqs = s.build_requests();
                assert_eq!(reqs.len(), 2);
                assert!(!reqs
                    .iter()
                    .any(|r| matches!(r, PipelineRequest::Transit)));
            });
        });
    }

    /// `NatalOptions` derivados de `module_configs["natal"]` deben
    /// respetar orb_multiplier, show_minors y show_dignities cuando los
    /// hay, y caer a defaults razonables cuando no.
    #[gpui::test]
    fn natal_options_read_from_configs(cx: &mut TestAppContext) {
        cx.update(|cx| {
            Theme::install_default(cx);
            let store = Store::in_memory().expect("store");
            let shell = cx.new(|cx| Shell::new(store, cx));

            shell.update(cx, |s, _cx| {
                let opts = s.build_natal_options();
                assert!(opts.show_majors);
                assert!(!opts.show_minors);
                assert_eq!(opts.orb_multiplier, 1.0);
                assert!(!opts.show_dignities);

                s.module_configs.insert(
                    "natal".into(),
                    serde_json::json!({
                        "aspect_majors": true,
                        "aspect_minors": true,
                        "orb_multiplier": 1.75,
                        "show_dignities": true,
                    }),
                );
                let opts = s.build_natal_options();
                assert!(opts.show_minors);
                assert_eq!(opts.orb_multiplier, 1.75);
                assert!(opts.show_dignities);
            });
        });
    }

    /// El flex de los splitters persiste entre instancias de Shell que
    /// comparten la misma store (in-memory): primera shell escribe via
    /// `save_split_flex`, segunda shell lee via `load_split_flex_n` al
    /// boot. Cubre 2 y 3 hijos (Bottom vs docks laterales).
    #[test]
    fn split_flex_round_trip_via_store() {
        let store = Store::in_memory().expect("store");
        let defaults_2 = vec![1.0_f32, 4.0];
        let defaults_3 = vec![1.0_f32, 4.0, 1.5];

        // Sin nada persistido → defaults.
        assert_eq!(load_split_flex_n(&store, "layout.x", &defaults_2), defaults_2);

        store.set_setting("layout.x", "2.5,3.5").unwrap();
        assert_eq!(
            load_split_flex_n(&store, "layout.x", &defaults_2),
            vec![2.5_f32, 3.5]
        );

        store.set_setting("layout.x", "1.0,4.0,2.0").unwrap();
        assert_eq!(
            load_split_flex_n(&store, "layout.x", &defaults_3),
            vec![1.0_f32, 4.0, 2.0]
        );

        // Valor corrupto → defaults.
        store.set_setting("layout.x", "garbage").unwrap();
        assert_eq!(load_split_flex_n(&store, "layout.x", &defaults_2), defaults_2);

        // Cantidad incorrecta → defaults.
        store.set_setting("layout.x", "2,3,4").unwrap();
        assert_eq!(load_split_flex_n(&store, "layout.x", &defaults_2), defaults_2);

        // Valor ≤0 → defaults.
        store.set_setting("layout.x", "0,5").unwrap();
        assert_eq!(load_split_flex_n(&store, "layout.x", &defaults_2), defaults_2);
    }

    /// PanelDock roundtrip via store.
    #[test]
    fn panel_dock_setting_roundtrip() {
        assert_eq!(PanelDock::from_setting("bottom"), Some(PanelDock::Bottom));
        assert_eq!(PanelDock::from_setting("right"), Some(PanelDock::Right));
        assert_eq!(PanelDock::from_setting("left"), Some(PanelDock::Left));
        assert_eq!(PanelDock::from_setting("nope"), None);

        let store = Store::in_memory().expect("store");
        assert_eq!(load_dock(&store), None);
        store
            .set_setting("layout.panel_dock", PanelDock::Right.as_setting())
            .unwrap();
        assert_eq!(load_dock(&store), Some(PanelDock::Right));
    }
}
