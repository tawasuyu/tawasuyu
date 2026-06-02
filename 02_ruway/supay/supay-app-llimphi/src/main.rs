//! `supay-app-llimphi` — Fase 0.5 del proyecto supay (frontend Llimphi).
//!
//! Raycaster estilo Wolfenstein/Doom-early. **El mundo y la simulación
//! viven en `supay-mini-core`** (agnóstico de GUI); este crate sólo lo
//! pinta: raycast por columna con texturas procedurales + sprites
//! billboarded con z-test + sector lights + niebla volumétrica, más la
//! barra de menú, el HUD y el contextual. Cambiar de GUI (TUI/web) no
//! pierde nada de gameplay — esa es la regla #2 del repo.
//!
//! Controles: W/S adelante/atrás, A/D strafe, ←/→ giro, Space dispara
//! (o reinicia en game over/victory), Esc cierra.

use std::sync::Arc;
use std::time::Duration;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::kurbo::{BezPath, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, PaintRect, View};

use llimphi_motion::{animate, motion, Tween};
use llimphi_theme::Theme;
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view, MenuBarSpec,
    DEFAULT_HEIGHT as MENU_H,
};

use app_bus::{AppMenu, Menu, MenuItem};

// El mundo simulado + su geometría: todo en el core agnóstico.
use supay_mini_core::*;

// =====================================================================
// Apariencia de sprites — mapeo kind → (color base, altura fraccional).
// Es decisión de RENDER (cómo se ve cada entidad), no del mundo.
// =====================================================================

fn sprite_appearance(kind: SpriteKind) -> ((f32, f32, f32), f32) {
    match kind {
        SpriteKind::Barrel => ((0.32, 0.78, 0.30), 0.5),
        SpriteKind::Pillar => ((0.55, 0.50, 0.42), 1.0),
        SpriteKind::Imp => ((0.78, 0.20, 0.18), 0.85),
        SpriteKind::Torch => ((0.95, 0.78, 0.30), 0.7),
        SpriteKind::Bullet => ((1.00, 0.90, 0.30), 0.15),
        SpriteKind::Decal => ((0.18, 0.08, 0.06), 0.20),
        SpriteKind::DyingImp => ((0.45, 0.12, 0.10), 0.65),
        SpriteKind::Corpse => ((0.22, 0.07, 0.06), 0.30),
        SpriteKind::AmmoBox => ((0.45, 0.85, 0.95), 0.35),
        SpriteKind::HealthKit => ((0.30, 0.95, 0.40), 0.35),
    }
}

// =====================================================================
// Sector lights (render) — luces puntuales del nivel con falloff 1/d².
// Es data de iluminación del renderer; el mundo no la simula.
// =====================================================================

#[derive(Clone, Copy)]
struct Light {
    x: f32,
    y: f32,
    color: (f32, f32, f32),
    strength: f32,
}

const LIGHTS: &[Light] = &[
    Light {
        x: 8.5,
        y: 12.5,
        color: (1.00, 0.70, 0.35),
        strength: 2.2,
    },
    Light {
        x: 3.5,
        y: 13.5,
        color: (1.00, 0.70, 0.35),
        strength: 1.8,
    },
    Light {
        x: 7.5,
        y: 5.5,
        color: (0.85, 0.20, 0.15),
        strength: 2.5,
    },
    Light {
        x: 13.5,
        y: 6.5,
        color: (0.35, 0.55, 1.00),
        strength: 1.4,
    },
];

const BULLET_LIGHT_STRENGTH: f32 = 1.4;
const BULLET_LIGHT_COLOR: (f32, f32, f32) = (1.0, 0.85, 0.40);

/// Contribución sumada de todas las luces (sector + bullets + flashes)
/// al `(hit_x, hit_y)`. `tick` modula el flicker de las cálidas.
fn lighting_contribution(
    hit_x: f32,
    hit_y: f32,
    tick: u64,
    bullets: &[Bullet],
    temp_lights: &[TempLight],
) -> (f32, f32, f32) {
    let mut acc = (0.0_f32, 0.0_f32, 0.0_f32);
    for (idx, l) in LIGHTS.iter().enumerate() {
        let dx = l.x - hit_x;
        let dy = l.y - hit_y;
        let d2 = dx * dx + dy * dy;
        let flicker = if l.color.0 > l.color.2 {
            let phase = idx as f32 * 1.37;
            1.0 + 0.12 * (tick as f32 * 0.31 + phase).sin()
                + 0.05 * (tick as f32 * 0.71 + phase * 1.7).sin()
        } else {
            1.0
        };
        let atten = l.strength * flicker / (1.0 + 0.6 * d2);
        acc.0 += l.color.0 * atten;
        acc.1 += l.color.1 * atten;
        acc.2 += l.color.2 * atten;
    }
    for b in bullets {
        let dx = b.x - hit_x;
        let dy = b.y - hit_y;
        let d2 = dx * dx + dy * dy;
        let atten = BULLET_LIGHT_STRENGTH / (1.0 + 1.2 * d2);
        acc.0 += BULLET_LIGHT_COLOR.0 * atten;
        acc.1 += BULLET_LIGHT_COLOR.1 * atten;
        acc.2 += BULLET_LIGHT_COLOR.2 * atten;
    }
    for tl in temp_lights {
        let dx = tl.x - hit_x;
        let dy = tl.y - hit_y;
        let d2 = dx * dx + dy * dy;
        let life = tl.ttl as f32 / tl.ttl_max.max(1) as f32;
        let atten = tl.strength * life / (1.0 + 1.0 * d2);
        acc.0 += tl.color.0 * atten;
        acc.1 += tl.color.1 * atten;
        acc.2 += tl.color.2 * atten;
    }
    acc
}

// =====================================================================
// Materiales y color (render) — id de pared → color base; fog y shading.
// =====================================================================

fn material_color(id: u8) -> (f32, f32, f32) {
    match id {
        1 => (0.62, 0.55, 0.46), // techbase beige
        2 => (0.48, 0.18, 0.16), // ladrillo rojo infernal
        3 => (0.28, 0.40, 0.52), // metal azul
        4 => (0.18, 0.55, 0.30), // slime verde
        _ => (0.5, 0.5, 0.5),
    }
}

fn floor_color(y_frac: f32) -> Color {
    let g = 0.08 + 0.16 * y_frac;
    let r = 0.08 + 0.10 * y_frac;
    let b = 0.10 + 0.05 * y_frac;
    rgb(r, g, b)
}

fn ceiling_color(y_frac: f32) -> Color {
    let g = 0.02 + 0.04 * (1.0 - y_frac);
    let r = 0.04 + 0.10 * (1.0 - y_frac);
    let b = 0.04 + 0.05 * (1.0 - y_frac);
    rgb(r, g, b)
}

const FOG_COLOR: (f32, f32, f32) = (0.05, 0.04, 0.06);
const FOG_END: f32 = 14.0;

fn apply_fog(color: (f32, f32, f32), dist: f32) -> (f32, f32, f32) {
    let t = (dist / FOG_END).clamp(0.0, 1.0);
    lerp_rgb(color, FOG_COLOR, t)
}

fn shade_by_dist(color: (f32, f32, f32), dist: f32) -> (f32, f32, f32) {
    let k = 0.18;
    let atten = 1.0 / (1.0 + k * dist);
    (color.0 * atten, color.1 * atten, color.2 * atten)
}

fn lerp_rgb(a: (f32, f32, f32), b: (f32, f32, f32), t: f32) -> (f32, f32, f32) {
    (
        a.0 + (b.0 - a.0) * t,
        a.1 + (b.1 - a.1) * t,
        a.2 + (b.2 - a.2) * t,
    )
}

fn rgb(r: f32, g: f32, b: f32) -> Color {
    let to = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color::from_rgba8(to(r), to(g), to(b), 255)
}

// =====================================================================
// Modelo y bucle — Model = mundo (core) + estado de UI (menús/tema)
// =====================================================================

const TICK_HZ: u64 = 35; // ticks/seg — la frecuencia canónica de Doom
const TICK_MS: u64 = 1_000 / TICK_HZ;

struct Model {
    /// El mundo simulado — toda la lógica de juego vive en el core.
    world: World,
    /// Tema activo — sólo viste la barra de menú y los overlays.
    theme: Theme,
    /// Barra de menú: índice del menú raíz abierto (`None` cerrado).
    menu_open: Option<usize>,
    /// Fila resaltada dentro del dropdown (`usize::MAX` = ninguna).
    menu_active: usize,
    /// Animación de aparición/swap del dropdown.
    menu_anim: Tween<f32>,
    /// Menú contextual del escenario: ancla `(x, y)` en coords ventana.
    context_menu: Option<(f32, f32)>,
}

#[derive(Clone)]
enum Msg {
    Tick,
    Key(KeyEvent),
    Fire,
    Reset,
    Quit,
    MenuOpen(Option<usize>),
    MenuCommand(String),
    MenuNav(i32),
    MenuActivate,
    MenuTick,
    CloseMenus,
    ContextMenuOpen(f32, f32),
    CycleTheme,
}

struct Supay;

impl App for Supay {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "supay · fase 0 · raycaster"
    }

    fn initial_size() -> (u32, u32) {
        (960, 600)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        handle.spawn_periodic(Duration::from_millis(TICK_MS), || Msg::Tick);
        Model {
            world: World::new(),
            theme: Theme::dark(),
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            context_menu: None,
        }
    }

    fn on_key(model: &Model, e: &KeyEvent) -> Option<Msg> {
        // Con el menú principal abierto las flechas navegan.
        if e.state == KeyState::Pressed {
            if let Some(mi) = model.menu_open {
                let n = app_menu(model).menus.len().max(1);
                match &e.key {
                    Key::Named(NamedKey::Escape) => return Some(Msg::CloseMenus),
                    Key::Named(NamedKey::ArrowLeft) => {
                        return Some(Msg::MenuOpen(Some((mi + n - 1) % n)));
                    }
                    Key::Named(NamedKey::ArrowRight) => {
                        return Some(Msg::MenuOpen(Some((mi + 1) % n)));
                    }
                    Key::Named(NamedKey::ArrowDown) => return Some(Msg::MenuNav(1)),
                    Key::Named(NamedKey::ArrowUp) => return Some(Msg::MenuNav(-1)),
                    Key::Named(NamedKey::Enter) => return Some(Msg::MenuActivate),
                    _ => return None,
                }
            }
        }
        if matches!(&e.key, Key::Named(NamedKey::Escape)) && e.state == KeyState::Pressed {
            if model.menu_open.is_some() || model.context_menu.is_some() {
                return Some(Msg::CloseMenus);
            }
            return Some(Msg::Quit);
        }
        // Space: dispara en juego; reinicia en game over/victory.
        if e.state == KeyState::Pressed && matches!(&e.key, Key::Named(NamedKey::Space)) {
            return Some(if model.world.game_over || model.world.victory {
                Msg::Reset
            } else {
                Msg::Fire
            });
        }
        Some(Msg::Key(e.clone()))
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Quit => {
                handle.quit();
            }
            Msg::Fire => {
                m.world.fire();
            }
            Msg::Reset => {
                m.world.reset();
            }
            Msg::Key(e) => {
                let pressed = e.state == KeyState::Pressed;
                let text_lower = e
                    .text
                    .as_ref()
                    .map(|s| s.to_lowercase())
                    .unwrap_or_default();
                let ch = text_lower.chars().next();
                match (&e.key, ch) {
                    (_, Some('w')) => m.world.input.forward = pressed,
                    (_, Some('s')) => m.world.input.backward = pressed,
                    (_, Some('a')) => m.world.input.strafe_left = pressed,
                    (_, Some('d')) => m.world.input.strafe_right = pressed,
                    (Key::Named(NamedKey::ArrowLeft), _) => m.world.input.turn_left = pressed,
                    (Key::Named(NamedKey::ArrowRight), _) => m.world.input.turn_right = pressed,
                    (Key::Named(NamedKey::ArrowUp), _) => m.world.input.forward = pressed,
                    (Key::Named(NamedKey::ArrowDown), _) => m.world.input.backward = pressed,
                    _ => {}
                }
            }
            Msg::Tick => {
                m.world.tick = m.world.tick.wrapping_add(1);
                m.world.advance();
            }
            Msg::MenuOpen(which) => {
                m.menu_open = which;
                m.context_menu = None;
                m.menu_active = usize::MAX;
                if which.is_some() {
                    m.menu_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(handle, motion::FAST, || Msg::MenuTick);
                }
            }
            Msg::MenuNav(dir) => {
                if let Some(mi) = m.menu_open {
                    let menu = app_menu(&m);
                    m.menu_active = menubar_nav(&menu, mi, m.menu_active, dir);
                }
            }
            Msg::MenuActivate => {
                if let Some(mi) = m.menu_open {
                    let menu = app_menu(&m);
                    if let Some(cmd) = menubar_command_at(&menu, mi, m.menu_active) {
                        m.menu_open = None;
                        m.context_menu = None;
                        handle_menu_command(&cmd, handle);
                    }
                }
            }
            Msg::MenuTick => {}
            Msg::CloseMenus => {
                m.menu_open = None;
                m.context_menu = None;
                m.menu_active = usize::MAX;
            }
            Msg::ContextMenuOpen(x, y) => {
                m.menu_open = None;
                m.context_menu = Some((x, y));
            }
            Msg::CycleTheme => {
                m.theme = Theme::next_after(m.theme.name);
            }
            Msg::MenuCommand(cmd) => {
                m.menu_open = None;
                m.context_menu = None;
                handle_menu_command(&cmd, handle);
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let menu = app_menu(model);
        let menubar = menubar_view(&menubar_spec(&menu, model));
        let scene = scene_pane(model);
        let hud = hud_panel(model);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(rgb(0.02, 0.02, 0.03))
        .children(vec![menubar, scene, hud])
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        if let Some((x, y)) = model.context_menu {
            return Some(context_menu_for_scene(model, x, y));
        }
        let menu = app_menu(model);
        menubar_overlay_animated(
            &menubar_spec(&menu, model),
            model.menu_active,
            model.menu_anim.value(),
        )
    }
}

/// HUD inferior estilo Doom clásico: vida, munición, material apuntado.
fn hud_panel(model: &Model) -> View<Msg> {
    let mat_name = match model.world.last_hit_material {
        1 => "techbase",
        2 => "ladrillo",
        3 => "metal",
        4 => "slime",
        _ => "—",
    };
    let hud_bg = rgb(0.08, 0.06, 0.06);
    let border = rgb(0.42, 0.10, 0.06);

    let cell = |label: &str, value: &str, value_color: (f32, f32, f32)| -> View<Msg> {
        let label_v = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(12.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(
            label.to_string(),
            10.0,
            rgb(0.65, 0.55, 0.45),
            Alignment::Center,
        );
        let value_v = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(24.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(
            value.to_string(),
            20.0,
            rgb(value_color.0, value_color.1, value_color.2),
            Alignment::Center,
        );
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            flex_grow: 1.0,
            align_items: Some(AlignItems::Center),
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(4.0_f32),
                bottom: length(4.0_f32),
            },
            ..Default::default()
        })
        .children(vec![label_v, value_v])
    };

    let health = model.world.health;
    let ammo = model.world.ammo;
    let health_color = if health > 50 {
        (0.80, 0.95, 0.55)
    } else if health > 25 {
        (0.95, 0.85, 0.30)
    } else {
        (0.95, 0.30, 0.25)
    };
    let ammo_color = if ammo > 0 {
        (0.95, 0.85, 0.30)
    } else {
        (0.95, 0.30, 0.25)
    };

    let row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(vec![
        cell("VIDA", &health.to_string(), health_color),
        cell("MUNICION", &ammo.to_string(), ammo_color),
        cell("OBJETIVO", mat_name, (0.85, 0.80, 0.70)),
    ]);

    let border_strip = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(2.0_f32),
        },
        ..Default::default()
    })
    .fill(border);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: length(52.0_f32),
        },
        ..Default::default()
    })
    .fill(hud_bg)
    .children(vec![border_strip, row])
}

// =====================================================================
// Menú principal + contextual del escenario
// =====================================================================

fn viewport_of(_model: &Model) -> (f32, f32) {
    let (w, h) = Supay::initial_size();
    (w as f32, h as f32)
}

fn menubar_spec<'a>(menu: &'a AppMenu, model: &'a Model) -> MenuBarSpec<'a, Msg> {
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme: &model.theme,
        viewport: viewport_of(model),
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

fn app_menu(model: &Model) -> AppMenu {
    let can_fire = !model.world.game_over && !model.world.victory && model.world.ammo > 0;
    let fire_item = MenuItem::new("Disparar", "play.fire").shortcut("Space");
    let fire_item = if can_fire {
        fire_item
    } else {
        fire_item.disabled()
    };

    AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(MenuItem::new("Reiniciar partida", "file.reset"))
                .item(
                    MenuItem::new("Salir", "file.quit")
                        .shortcut("Esc")
                        .separated(),
                ),
        )
        .menu(Menu::new("Jugar").item(fire_item))
        .menu(Menu::new("Ver").item(MenuItem::new("Cambiar tema", "view.theme")))
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

fn handle_menu_command(cmd: &str, handle: &Handle<Msg>) {
    let msg = match cmd {
        "file.reset" => Some(Msg::Reset),
        "file.quit" => Some(Msg::Quit),
        "play.fire" => Some(Msg::Fire),
        "view.theme" => Some(Msg::CycleTheme),
        _ => None,
    };
    if let Some(msg) = msg {
        handle.dispatch(msg);
    }
}

fn context_menu_for_scene(model: &Model, x: f32, y: f32) -> View<Msg> {
    let can_fire = !model.world.game_over && !model.world.victory && model.world.ammo > 0;
    let header = if model.world.game_over {
        "fin de partida".to_string()
    } else if model.world.victory {
        "victoria".to_string()
    } else {
        format!("munición {}", model.world.ammo)
    };

    let fire = ContextMenuItem::action("Disparar").with_shortcut("Space");
    let items = vec![
        if can_fire { fire } else { fire.disabled() },
        ContextMenuItem::separator(),
        ContextMenuItem::action("Reiniciar partida"),
    ];

    let cmds: Vec<&'static str> = vec!["play.fire", "", "file.reset"];
    let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> =
        Arc::new(move |i: usize| Msg::MenuCommand(cmds.get(i).copied().unwrap_or("").to_string()));

    context_menu_view(ContextMenuSpec {
        anchor: (x, y),
        viewport: viewport_of(model),
        header: Some(header),
        items,
        active: usize::MAX,
        on_pick,
        on_dismiss: Msg::CloseMenus,
        palette: ContextMenuPalette::from_theme(&model.theme),
    })
}

// =====================================================================
// Submódulo de render (raycaster). El mundo + geometría vienen del core.
// =====================================================================
mod render;
use render::*;

fn main() {
    llimphi_ui::run::<Supay>();
}
