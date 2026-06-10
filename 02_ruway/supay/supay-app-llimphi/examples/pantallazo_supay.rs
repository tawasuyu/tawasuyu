//! Pantallazo headless de `supay-app-llimphi` — Fase 0 de supay
//! (modernizar Doom sin tocar su alma).
//!
//! Monta la **view real** de la app (menubar · escenario raycaster ·
//! HUD estilo Doom) con un `World` sembrado a mitad de partida: el
//! jugador en el corredor central del mapa 16×16, un imp persiguiendo,
//! otro muriendo bajo el flash del impacto, una bala en vuelo con su
//! luz dinámica, decals de disparos en las paredes de slime, pickups de
//! munición/vida a la vista, crosshair, minimap con luces y cono FOV, y
//! el menú contextual del escenario abierto. Tick fijo (942) — texturas
//! animadas y flicker deterministas, nada depende de la hora actual.
//!
//! El raycaster (paredes con textura procedural + sprites billboarded
//! con z-test + minimap) es el código REAL de la app vía `#[path]` a
//! `src/render.rs`; como la app es bin-only, la capa fina de `Model` /
//! helpers de color / HUD / menús se calca acá tal cual de
//! `src/main.rs` (mismo patrón que `khipu-app/examples/pantallazo_mapa.rs`).
//!
//! Pinta a una textura wgpu sin ventana y vuelca PNG (mismo patrón que
//! `agora-app/examples/pantallazo_agora.rs`).
//!
//! `cargo run -p supay-app-llimphi --example pantallazo_supay --release -- [out.png]`
#![allow(dead_code)]

use std::fs::File;
use std::io::BufWriter;
use std::sync::Arc;

use llimphi_motion::Tween;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy::{
    self,
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::kurbo::{BezPath, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::{Alignment, Typesetter};
use llimphi_ui::{measure_text_node, mount, paint, KeyEvent, PaintRect, View};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_menubar::{menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};

use app_bus::{AppMenu, Menu, MenuItem};

// El mundo simulado + su geometría: todo en el core agnóstico.
use supay_mini_core::*;

// El raycaster REAL de la app (paredes texturadas, sprites con z-test,
// crosshair, minimap): `render.rs` usa `super::*`, así que este example
// le provee exactamente los mismos nombres que `src/main.rs`.
#[path = "../src/render.rs"]
mod render;
use render::*;

const W: u32 = 1600;
const H: u32 = 1000;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

// =====================================================================
// Capa de render de la app, calcada tal cual de `src/main.rs` (la app
// es bin-only y estos ítems viven junto a `fn main`): apariencia de
// sprites, luces del nivel, materiales, fog y el Model/Msg.
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

/// Mismo `Model` que la app: mundo (core) + estado de UI (menús/tema).
struct Model {
    world: World,
    theme: Theme,
    menu_open: Option<usize>,
    menu_active: usize,
    menu_anim: Tween<f32>,
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

    let t = rimay_localize::t;
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
        cell(&t("supay-hud-health"), &health.to_string(), health_color),
        cell(&t("supay-hud-ammo"), &ammo.to_string(), ammo_color),
        cell(&t("supay-hud-target"), mat_name, (0.85, 0.80, 0.70)),
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

fn viewport_of(_model: &Model) -> (f32, f32) {
    (W as f32, H as f32)
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
    let t = rimay_localize::t;
    let can_fire = !model.world.game_over && !model.world.victory && model.world.ammo > 0;
    let fire_item = MenuItem::new(t("supay-action-fire"), "play.fire").shortcut("Space");
    let fire_item = if can_fire {
        fire_item
    } else {
        fire_item.disabled()
    };

    let cur = rimay_localize::current_locale();
    let lang_item = |label: &str, code: &str| {
        let mut it = MenuItem::new(label, format!("lang.{code}"));
        if cur == code {
            it = it.icon("\u{2714}");
        }
        it
    };

    AppMenu::new()
        .menu(
            Menu::new(t("file"))
                .item(MenuItem::new(t("supay-action-reset"), "file.reset"))
                .item(
                    MenuItem::new(t("exit"), "file.quit")
                        .shortcut("Esc")
                        .separated(),
                ),
        )
        .menu(Menu::new(t("supay-menu-play")).item(fire_item))
        .menu(Menu::new(t("view")).item(MenuItem::new(t("cycle-theme"), "view.theme")))
        .menu(Menu::new(t("help")).item(MenuItem::new(t("about"), "help.about")))
        .menu(
            Menu::new(t("language"))
                .item(lang_item("Español", "es-PE"))
                .item(lang_item("English", "en-US"))
                .item(lang_item("Runasimi", "qu-PE")),
        )
}

fn context_menu_for_scene(model: &Model, x: f32, y: f32) -> View<Msg> {
    let t = rimay_localize::t;
    let can_fire = !model.world.game_over && !model.world.victory && model.world.ammo > 0;
    let header = if model.world.game_over {
        t("supay-status-game-over")
    } else if model.world.victory {
        t("supay-status-victory")
    } else {
        format!("{} {}", t("supay-hud-ammo"), model.world.ammo)
    };

    let fire = ContextMenuItem::action(t("supay-action-fire")).with_shortcut("Space");
    let items = vec![
        if can_fire { fire } else { fire.disabled() },
        ContextMenuItem::separator(),
        ContextMenuItem::action(t("supay-action-reset")),
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
// Estado sembrado — mitad de partida, todo determinístico (tick fijo)
// =====================================================================

/// `World` a mitad de partida: el jugador avanzó por el corredor
/// central (fila 8 del mapa), gastó munición y recibió daño; un imp lo
/// persigue, otro acaba de morir bajo el flash del impacto, una bala
/// sigue en vuelo, y quedan decals de tiros errados en el slime.
fn modelo_demo() -> Model {
    let mut world = World::new();
    // Tick fijo → flicker de antorchas, slime animado y bob del imp
    // deterministas (la app lo incrementa a 35 Hz; acá queda congelado).
    world.tick = 942;
    // Jugador en el corredor central mirando al este.
    world.px = 2.4;
    world.py = 8.5;
    world.pa = 0.04;
    // Mitad de partida: recibió daño y gastó balas.
    world.health = 38;
    world.ammo = 23;
    // Enemigos: uno persiguiendo a la vista, uno muriendo (recién
    // baleado), uno lejos en su patrulla y un cadáver de hace rato.
    world.enemies = vec![
        Enemy {
            x: 5.8,
            y: 7.9,
            hp: 50,
            state: EnemyState::Walking,
            attack_cd: 12,
        },
        Enemy {
            x: 7.2,
            y: 9.3,
            hp: 0,
            state: EnemyState::Dying(7),
            attack_cd: 0,
        },
        Enemy {
            x: 12.5,
            y: 11.5,
            hp: ENEMY_HP,
            state: EnemyState::Idle,
            attack_cd: 0,
        },
        Enemy {
            x: 4.1,
            y: 9.4,
            hp: 0,
            state: EnemyState::Dead,
            attack_cd: 0,
        },
    ];
    // Una bala en vuelo hacia el imp — aporta su luz dinámica cálida.
    world.bullets = vec![Bullet {
        x: 4.6,
        y: 8.2,
        vx: BULLET_SPEED,
        vy: 0.02,
        ttl: 48,
    }];
    // Decals de tiros errados contra los bloques de slime de la fila 7.
    world.decals = vec![
        Decal {
            x: 3.9,
            y: 7.45,
            ttl: 180,
        },
        Decal {
            x: 6.92,
            y: 7.5,
            ttl: 120,
        },
    ];
    // Flash del impacto que está matando al imp (TTL a media vida).
    world.temp_lights = vec![TempLight {
        x: 7.0,
        y: 9.1,
        color: FLASH_COLOR_IMPACT,
        strength: FLASH_STRENGTH_IMPACT,
        ttl: 3,
        ttl_max: FLASH_TTL,
    }];
    // Material apuntado: el rayo central REAL del mundo (no inventado).
    world.last_hit_material = cast_ray(world.px, world.py, world.pa).material;

    Model {
        world,
        theme: Theme::dark(),
        menu_open: None,
        menu_active: usize::MAX,
        menu_anim: Tween::idle(1.0),
        // Menú contextual del escenario abierto (right-click), a la
        // derecha del crosshair — el overlay real de la app.
        context_menu: Some((W as f32 * 0.66, H as f32 * 0.52)),
    }
}

/// Misma composición que el `view()` de `Supay`: menubar arriba, el
/// escenario raycaster al centro (flex_grow) y el HUD al pie.
fn view_demo(model: &Model) -> View<Msg> {
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

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/supay.png".to_string());
    if let Some(dir) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }

    // Locale fijo (es-PE) para un pantallazo estable, sin depender del
    // wawa-config del host.
    rimay_localize::init();
    let _ = rimay_localize::set_locale("es-PE");

    let model = modelo_demo();
    let root = view_demo(&model);

    // view → layout → scene (misma secuencia que el eventloop real).
    let mut layout = LayoutTree::new();
    let mounted = mount(&mut layout, root);
    let mut ts = Typesetter::new();
    let computed = {
        let tmap = &mounted.text_measures;
        layout
            .compute_with_measure(mounted.root, (W as f32, H as f32), |nid, known, avail| {
                match tmap.get(&nid) {
                    Some(tm) => measure_text_node(&mut ts, tm, known, avail),
                    None => taffy::Size::ZERO,
                }
            })
            .expect("layout")
    };
    let mut scene = vello::Scene::new();
    paint(&mut scene, &mounted, &computed, &mut ts, None, None);

    // El overlay (menú contextual) se monta en un árbol aparte y se
    // pinta encima — calco de cómo lo compone el eventloop.
    if let Some((x, y)) = model.context_menu {
        let overlay = context_menu_for_scene(&model, x, y);
        let mut olayout = LayoutTree::new();
        let omounted = mount(&mut olayout, overlay);
        let ocomputed = {
            let tmap = &omounted.text_measures;
            olayout
                .compute_with_measure(omounted.root, (W as f32, H as f32), |nid, known, avail| {
                    match tmap.get(&nid) {
                        Some(tm) => measure_text_node(&mut ts, tm, known, avail),
                        None => taffy::Size::ZERO,
                    }
                })
                .expect("layout overlay")
        };
        paint(&mut scene, &omounted, &ocomputed, &mut ts, None, None);
    }

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pantallazo-supay"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FMT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    // Fondo: el mismo casi-negro del `view()` de la app.
    let bg = Color::from_rgba8(5, 5, 8, 255);
    renderer
        .render_to_view(&hal, &scene, &view, W, H, bg)
        .expect("render_to_view");

    write_png(&hal, &target, &out);
    eprintln!("pantallazo_supay: escrito {out} ({W}x{H})");
}

/// Lee la textura a CPU y la vuelca como PNG RGBA8.
fn write_png(hal: &Hal, target: &wgpu::Texture, path: &str) {
    let unpadded = (W * 4) as usize;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded.div_ceil(align) * align;
    let buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * H as usize) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
                rows_per_image: Some(H),
            },
        },
        wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
    );
    hal.queue.submit(std::iter::once(enc.finish()));
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((W * H * 4) as usize);
    for row in 0..H as usize {
        let s = row * padded;
        pixels.extend_from_slice(&data[s..s + unpadded]);
    }
    drop(data);
    buf.unmap();
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().unwrap();
    w.write_image_data(&pixels).unwrap();
}
