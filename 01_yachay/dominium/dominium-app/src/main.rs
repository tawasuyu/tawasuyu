//! `dominium` — la ventana viva del simulador de campo medio.
//!
//! Compone toda la cadena de dominium en un app GPUI:
//!
//! ```text
//!   dominium-core ─► dominium-physics ─► dominium-iso ─►
//!   dominium-render-plan ─► dominium-canvas-gpui ─► [esta ventana]
//! ```
//!
//! Un bucle de fondo avanza la simulación ~11 veces por segundo; cada
//! tick reconstruye la maqueta isométrica y la repinta. El panel
//! derecho muestra las estadísticas agregadas y dos controles
//! (play/pausa, re-sembrar). Cuando la población colapsa, el mundo se
//! re-siembra solo: la demo nunca se queda en negro.

use std::time::Duration;

use dominium_canvas_gpui::DominiumCanvas;
use dominium_core::{SimParams, World};
use dominium_iso::{IsoProjector, ZWeights};
use dominium_physics::tick;
use dominium_render_plan::{build_plan, PlanConfig};
use gpui::{
    div, hsla, prelude::*, px, Context, IntoElement, Render, SharedString, Window,
};
use nahual_launcher::launch_app;
use nahual_theme::Theme;

/// Lado de la grilla cuadrada del mundo.
const GRID: usize = 40;
/// Población inicial de Lemmings.
const LEMMINGS: usize = 50;
/// Periodo del bucle de simulación.
const TICK_MS: u64 = 90;

/// PRNG mínimo (LCG de 64 bits) — siembra reproducible sin dependencias.
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next_u32(&mut self) -> u32 {
        // Constantes de Knuth (MMIX).
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
    /// Flotante uniforme en `[0, 1)`.
    fn next_f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// Siembra un mundo: continentes de `materia`, vetas de `oro`, niebla de
/// `psique` y una población de Lemmings con sesgos y acciones variadas.
fn seed(seed: u64) -> World {
    let mut w = World::new(GRID, GRID);
    let mut rng = Lcg::new(seed);
    for cy in 0..GRID {
        for cx in 0..GRID {
            let idx = w.grid.idx(cx, cy);
            // m² concentra la materia en parches → aspecto de continentes.
            let m = rng.next_f32();
            w.grid.materia[idx] = m * m * 60.0;
            if rng.next_f32() > 0.92 {
                w.grid.oro[idx] = rng.next_f32() * 40.0;
            }
            w.grid.psique[idx] = rng.next_f32() * 12.0;
        }
    }
    for _ in 0..LEMMINGS {
        let x = rng.next_f32() * (GRID as f32 - 1.0);
        let y = rng.next_f32() * (GRID as f32 - 1.0);
        let psi = [
            rng.next_f32(),
            rng.next_f32(),
            rng.next_f32(),
            rng.next_f32(),
        ];
        let i = w.lemmings.spawn(x, y, 30.0 + rng.next_f32() * 40.0, psi);
        w.lemmings.accion[i] = (rng.next_u32() % 6) as u8;
    }
    w
}

/// Estadísticas agregadas de un instante de la simulación.
struct Stats {
    poblacion: usize,
    materia: f32,
    oro: f32,
    energia: f32,
}

/// El estado del simulador y su presentación.
struct Sim {
    world: World,
    params: SimParams,
    iso: IsoProjector,
    weights: ZWeights,
    cfg: PlanConfig,
    running: bool,
    /// Ticks transcurridos en la época actual.
    tick: u64,
    /// Cuántas veces se re-sembró el mundo (colapso poblacional).
    epoch: u64,
    /// Semilla rodante para cada re-siembra.
    rng_seed: u64,
}

impl Sim {
    fn new(cx: &mut Context<Self>) -> Self {
        let rng_seed = 0xD0_31_31_07;
        let sim = Self {
            world: seed(rng_seed),
            params: SimParams::default(),
            iso: IsoProjector::new(12.0, 0.05),
            weights: ZWeights::default(),
            cfg: PlanConfig {
                tile: 15.0,
                lemming_size: 8.0,
                lemming_lift: 0.7,
                palette: Default::default(),
            },
            running: true,
            tick: 0,
            epoch: 0,
            rng_seed,
        };
        sim.start_loop(cx);
        sim
    }

    /// Lanza el bucle de fondo que avanza la simulación.
    fn start_loop(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(TICK_MS))
                .await;
            let alive = this.update(cx, |sim, cx| {
                if sim.running {
                    sim.advance();
                    cx.notify();
                }
            });
            if alive.is_err() {
                break; // la entidad murió → ventana cerrada.
            }
        })
        .detach();
    }

    /// Un paso de simulación; re-siembra si la población colapsa.
    fn advance(&mut self) {
        tick(&mut self.world, &self.params);
        self.tick += 1;
        if self.world.lemmings.is_empty() {
            self.epoch += 1;
            self.rng_seed = self.rng_seed.wrapping_mul(2862933555777941757).wrapping_add(1);
            self.world = seed(self.rng_seed);
            self.tick = 0;
        }
    }

    /// Re-siembra el mundo a mano (botón ↺).
    fn reseed(&mut self) {
        self.rng_seed = self.rng_seed.wrapping_add(0x9E3779B9);
        self.world = seed(self.rng_seed);
        self.tick = 0;
        self.epoch += 1;
    }

    /// Calcula las estadísticas del instante actual.
    fn stats(&self) -> Stats {
        let g = &self.world.grid;
        Stats {
            poblacion: self.world.lemmings.len(),
            materia: g.materia.iter().sum(),
            oro: g.oro.iter().sum(),
            energia: self.world.lemmings.energia.iter().sum(),
        }
    }
}

/// Fila etiqueta/valor del panel de estadísticas.
fn stat_row(label: &str, value: String, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .justify_between()
        .child(div().text_color(theme.fg_muted).child(SharedString::from(label.to_string())))
        .child(div().text_color(theme.fg_text).child(SharedString::from(value)))
}

impl Render for Sim {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();
        let panel = hsla(220.0 / 360.0, 0.18, 0.10, 1.0);
        let chip = hsla(220.0 / 360.0, 0.16, 0.16, 1.0);
        let canvas_bg = hsla(220.0 / 360.0, 0.22, 0.06, 1.0);
        let accent = theme.accent;
        let stats = self.stats();

        // --- Barra de estado ---
        let estado = if self.running { "● corriendo" } else { "‖ en pausa" };
        let status = div()
            .h(px(34.))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px(px(14.))
            .bg(panel)
            .text_color(theme.fg_text)
            .child(SharedString::from(format!(
                "dominium · campo medio   ·   época {}   ·   tick {}",
                self.epoch, self.tick
            )))
            .child(div().text_color(accent).child(SharedString::from(estado.to_string())));

        // --- Maqueta isométrica ---
        let plan = build_plan(&self.world, &self.iso, &self.weights, &self.cfg);
        let canvas = div()
            .flex_1()
            .overflow_hidden()
            .child(DominiumCanvas::new(plan).background(canvas_bg));

        // --- Botones de control ---
        let play_label = if self.running { "‖  Pausar" } else { "▶  Reanudar" };
        let play = div()
            .id("play")
            .px(px(10.))
            .py(px(7.))
            .bg(chip)
            .rounded(px(5.))
            .text_color(theme.fg_text)
            .cursor_pointer()
            .hover(|s| s.bg(theme.bg_row_hover))
            .child(SharedString::from(play_label.to_string()))
            .on_click(cx.listener(|sim, _ev, _w, cx| {
                sim.running = !sim.running;
                cx.notify();
            }));
        let reset = div()
            .id("reset")
            .px(px(10.))
            .py(px(7.))
            .bg(chip)
            .rounded(px(5.))
            .text_color(theme.fg_text)
            .cursor_pointer()
            .hover(|s| s.bg(theme.bg_row_hover))
            .child("↺  Re-sembrar")
            .on_click(cx.listener(|sim, _ev, _w, cx| {
                sim.reseed();
                cx.notify();
            }));

        // --- Panel de estadísticas ---
        let side = div()
            .w(px(216.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(12.))
            .bg(panel)
            .text_color(theme.fg_text)
            .child(div().text_color(theme.fg_muted).child("[SIM]"))
            .child(play)
            .child(reset)
            .child(div().h(px(1.)).bg(theme.border))
            .child(stat_row("Población", format!("{}", stats.poblacion), &theme))
            .child(stat_row("Materia", format!("{:.0}", stats.materia), &theme))
            .child(stat_row("Oro", format!("{:.0}", stats.oro), &theme))
            .child(stat_row("Energía", format!("{:.0}", stats.energia), &theme))
            .child(div().h(px(1.)).bg(theme.border))
            .child(
                div()
                    .text_color(theme.fg_muted)
                    .child(SharedString::from(format!("grilla {GRID}×{GRID}"))),
            )
            .child(
                div()
                    .text_color(theme.fg_muted)
                    .child("relieve = materia (Z)"),
            );

        // --- Composición ---
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg_app)
            .child(status)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .child(canvas)
                    .child(side),
            )
    }
}

fn main() {
    launch_app("brahman · dominium", (1120., 720.), Sim::new);
}
