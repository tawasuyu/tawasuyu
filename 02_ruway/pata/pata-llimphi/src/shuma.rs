//! El cabezal `shuma_input` y su drawer **Quake** — hospeda el **shell real** de
//! shuma.
//!
//! La frontera del SDD §5: el marco (`pata`) provee el borde; `shuma` provee el
//! contenido. `shuma_input` es el cabezal que vive en una barra; al activarlo
//! (click o hotkey) el frontend **despliega un drawer** estilo Quake sobre el
//! escritorio que **monta el módulo [`shuma_module_shell`]** —exactamente el
//! mismo shell de `shuma-shell-llimphi`: cards por comando, etapas de pipe
//! clickeables, cuerpo IDE-text read-only, barra de scroll arrastrable y
//! detección PTY/TUI (vim/htop a pantalla completa)—.
//!
//! pata **no reimplementa** nada del shell (Regla 2: la lógica de dominio no sabe
//! quién la pinta): instancia el [`shuma_module_shell::State`], le rutea las
//! teclas (`Msg::Key`), el latido que drena la salida (`Msg::Tick`) y los clicks
//! —que el `view` ya emite envueltos por el `lift` [`Msg::ShumaShell`]— y pinta
//! su `view`. Esto reemplaza de un saque las dos viejas reimplementaciones: las
//! cards propias del path winit y el terminal PTY aparte del path layer-shell.

use llimphi_motion::Tween;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, AlignItems, FlexDirection, JustifyContent, Position, Size, Style},
    Rect as TaffyRect,
};
use llimphi_ui::View;

use pata_core::WidgetSpec;
use shuma_module::Source;

use crate::{shuma_app, Msg};

/// Alto máximo del drawer (path winit), como fracción de la pantalla.
const DRAWER_FRAC: f32 = 0.45;

/// El estado del cabezal del shell y su drawer. Vive en el `Model` del frontend
/// —es interacción, no modelo de dominio—, no en `pata-core`. El **contenido**
/// del drawer es el shell real, hospedado en [`ShumaState::inner`].
pub struct ShumaState {
    /// `true` cuando el drawer está desplegado.
    pub open: bool,
    /// El **shell real**, hospedado como módulo. Fuente de verdad del contenido
    /// (input, runs, historial, cwd, PTY/TUI). pata sólo le rutea eventos y lo
    /// pinta; nunca toca sus campos directamente.
    pub inner: shuma_module_shell::State,
    /// Hotkey que abre/cierra el drawer (de la prop `hotkey`), o `None`.
    pub hotkey: Option<String>,
    /// Prompt al frente del cabezal (`›`, `$`, …).
    pub prompt: String,
    /// Texto del cabezal cuando el drawer está plegado.
    pub placeholder: String,
    /// Animación de despliegue `0..1` (0 = replegado, 1 = desplegado).
    pub anim: Tween<f32>,
    /// `true` si el config declaró algún `shuma_input` (si no, no hay cabezal
    /// ni drawer).
    pub present: bool,
    /// `true` si el drawer está maximizado (ocupa casi toda la pantalla en vez
    /// del 45% por defecto). Lo conmuta el botón ▢ de la barra de título.
    pub maximized: bool,
}

impl Default for ShumaState {
    fn default() -> Self {
        Self {
            open: false,
            inner: shuma_module_shell::State::new(Source::Local),
            hotkey: None,
            prompt: "›".into(),
            placeholder: "shuma".into(),
            anim: Tween::idle(0.0),
            present: false,
            maximized: false,
        }
    }
}

impl ShumaState {
    /// Construye el estado desde la spec del `shuma_input` (prompt/placeholder/
    /// hotkey). Marca `present = true`.
    pub fn from_spec(spec: &WidgetSpec) -> Self {
        let hotkey = spec.str_prop("hotkey", "");
        Self {
            prompt: spec.str_prop("prompt", "›").to_string(),
            placeholder: spec.str_prop("placeholder", "shuma").to_string(),
            hotkey: if hotkey.is_empty() {
                None
            } else {
                Some(hotkey.to_string())
            },
            present: true,
            ..Self::default()
        }
    }

    /// `true` si el drawer debe pintarse (abierto o aún animando el cierre).
    pub fn visible(&self) -> bool {
        self.open || self.anim.value() > 0.01
    }
}

/// El cabezal de la barra: **el input vivo del shell**. No es un placeholder ni
/// un cabezal-rótulo — es el mismísimo `shell_input_view` del shell hospedado,
/// llevado a la barra. Tipeás acá, las teclas las recibe el shell, Enter ejecuta.
/// Click en el chip → despliega el drawer (para ver la salida); el shell además
/// recibe `FocusInput` por su propio `on_click` interno.
pub fn headline_view(
    state: &ShumaState,
    full: Option<&shuma_app::Model>,
    theme: &Theme,
) -> View<Msg> {
    // Live-wire: con la shuma completa montada, el cabezal ES el input vivo de
    // la **sesión activa** de la shuma (mismo `shell_input_view`, ruteado a esa
    // sesión vía `lift_shuma`), no un chip. Tipear acá ejecuta en esa sesión y
    // FocusInput despliega el drawer. Si la activa no es un shell (form de nueva
    // sesión), caemos al chip como fallback.
    if let Some(full) = full {
        if let Some(input) = shuma_app::active_input_view(full, theme, crate::lift_shuma) {
            return wrap_headline(vec![input], state.open);
        }
        return headline_chip(state, theme);
    }
    let input = shuma_module_shell::input_view(&state.inner, theme, Msg::ShumaShell);
    let mut children = vec![input];
    // A6 — aviso de comando largo: cuando el drawer está plegado (no estás
    // mirando la salida) y terminó algún comando largo, el cabezal gana un punto
    // ámbar. Es el equivalente en pata de la badge del diente del chasis; al
    // abrir el drawer se acusa y desaparece. Sin notificaciones del sistema.
    if !state.open && state.inner.long_alerts() > 0 {
        children.push(long_alert_badge());
    }
    wrap_headline(children, state.open)
}

/// Envuelve los hijos del cabezal (input vivo + badge) en el contenedor que
/// llena el espacio de la barra. Click sobre el borde (no sobre el input)
/// despliega/repliega el drawer; el click directo sobre el input lo focaliza
/// (handler más profundo gana) y, en live-wire, abre el drawer vía el `update`.
fn wrap_headline(children: Vec<View<Msg>>, open: bool) -> View<Msg> {
    let v = View::new(Style {
        flex_direction: FlexDirection::Row,
        // Llenar el espacio disponible de la barra en vez de un bloque fijo de
        // 380 px "botado en el medio": flex_basis 0 + grow toma el remanente; un
        // min razonable evita que se aplaste con muchos widgets. El alto lo fija
        // el propio input (auto).
        size: Size {
            width: auto(),
            height: auto(),
        },
        min_size: Size {
            width: length(220.0_f32),
            height: auto(),
        },
        flex_basis: length(0.0_f32),
        flex_grow: 1.0,
        flex_shrink: 1.0,
        padding: TaffyRect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::FlexStart),
        gap: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .on_click(Msg::ShumaToggle)
    .children(children);
    // Hover-drawer: con el drawer plegado, pasar el puntero por el cabezal lo
    // despliega ("abre con hover"); el `leave` de la superficie lo repliega.
    // Sólo cuando está plegado, para no re-togglear al recorrer el drawer ya
    // abierto.
    if open {
        v
    } else {
        v.on_pointer_enter(Msg::ShumaToggle)
    }
}

/// A6 — el punto ámbar con halo del cabezal: «terminó un comando largo». Mismo
/// lenguaje visual que la badge del diente del chasis (`session_tooth_icon`).
fn long_alert_badge() -> View<Msg> {
    View::new(Style {
        size: Size { width: length(16.0_f32), height: length(16.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .paint_with(|scene, _ts, rect| {
        use llimphi_ui::llimphi_raster::kurbo::{Affine, Circle};
        use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let cx = (rect.x + rect.w * 0.5) as f64;
        let cy = (rect.y + rect.h * 0.5) as f64;
        let ambar = Color::from_rgb8(0xf7, 0xc8, 0x7a);
        let rad = (rect.w.min(rect.h) as f64 * 0.22).max(2.5);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            ambar.with_alpha(0.30),
            None,
            &Circle::new((cx, cy), rad * 1.9),
        );
        scene.fill(Fill::NonZero, Affine::IDENTITY, ambar, None, &Circle::new((cx, cy), rad));
    })
}

/// El cabezal en modo live-wire (shuma completa): un chip `prompt placeholder`
/// que despliega el drawer al click. Sin input vivo —ese vive adentro del drawer.
fn headline_chip(state: &ShumaState, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    let etiqueta = format!("{} {}", state.prompt, state.placeholder);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: auto(),
            height: auto(),
        },
        min_size: Size {
            width: length(220.0_f32),
            height: auto(),
        },
        flex_basis: length(0.0_f32),
        flex_grow: 1.0,
        flex_shrink: 1.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::FlexStart),
        ..Default::default()
    })
    .on_click(Msg::ShumaToggle)
    .text_aligned(etiqueta, 13.0, theme.fg_muted, Alignment::Start)
}

/// La fracción de pantalla que ocupa el drawer según esté o no maximizado.
fn drawer_frac(maximized: bool) -> f32 {
    if maximized {
        0.97
    } else {
        DRAWER_FRAC
    }
}

/// Tipo de botón de la barra de título del drawer — cada uno se pinta a mano
/// (vectores) para no depender de glifos que no estén en la fuente fallback.
#[derive(Clone, Copy)]
enum TbKind {
    /// Desdockea: abre la sesión en una instancia standalone de shuma.
    Undock,
    /// Minimiza: repliega el drawer (el input sigue en la barra).
    Minimize,
    /// Maximiza / restaura el alto del drawer.
    Maximize,
    /// Cierra el drawer.
    Close,
}

/// Un botón cuadrado de la barra de título, con su ícono pintado y su `on_click`.
fn tb_button(kind: TbKind, msg: Msg, theme: &Theme) -> View<Msg> {
    let fg = theme.fg_muted;
    let danger = kind_is_close(kind);
    View::new(Style {
        size: Size { width: length(28.0_f32), height: length(24.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        use llimphi_ui::llimphi_raster::kurbo::{Affine, Line, Rect as KRect, Stroke};
        use llimphi_ui::llimphi_raster::peniko::Color;
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        // Caja de 12×12 centrada en la que se dibuja el glifo.
        let s = 11.0_f64;
        let cx = (rect.x + rect.w * 0.5) as f64;
        let cy = (rect.y + rect.h * 0.5) as f64;
        let (x0, y0, x1, y1) = (cx - s * 0.5, cy - s * 0.5, cx + s * 0.5, cy + s * 0.5);
        let col = if danger {
            Color::from_rgb8(0xe0, 0x6c, 0x6c)
        } else {
            Color::new([fg.components[0], fg.components[1], fg.components[2], 0.85])
        };
        let st = Stroke::new(1.4);
        match kind {
            TbKind::Minimize => {
                scene.stroke(&st, Affine::IDENTITY, col, None, &Line::new((x0, y1), (x1, y1)));
            }
            TbKind::Maximize => {
                scene.stroke(
                    &st,
                    Affine::IDENTITY,
                    col,
                    None,
                    &KRect::new(x0, y0, x1, y1),
                );
            }
            TbKind::Close => {
                scene.stroke(&st, Affine::IDENTITY, col, None, &Line::new((x0, y0), (x1, y1)));
                scene.stroke(&st, Affine::IDENTITY, col, None, &Line::new((x0, y1), (x1, y0)));
            }
            TbKind::Undock => {
                // Cajita con una flecha saliendo hacia arriba-derecha.
                scene.stroke(
                    &st,
                    Affine::IDENTITY,
                    col,
                    None,
                    &KRect::new(x0, y0 + 2.5, x1 - 2.5, y1),
                );
                let a = (x1 - 4.0, y0 + 4.0);
                let b = (x1 + 1.0, y0 - 1.0);
                scene.stroke(&st, Affine::IDENTITY, col, None, &Line::new(a, b));
                scene.stroke(&st, Affine::IDENTITY, col, None, &Line::new(b, (b.0 - 4.5, b.1)));
                scene.stroke(&st, Affine::IDENTITY, col, None, &Line::new(b, (b.0, b.1 + 4.5)));
            }
        }
    })
    .on_click(msg)
}

fn kind_is_close(kind: TbKind) -> bool {
    matches!(kind, TbKind::Close)
}

/// La barra de título del drawer: el título a la izquierda y, a la derecha, los
/// controles desdockear · minimizar · maximizar · cerrar. Click en su fondo es
/// un no-op (`ShumaAnim`) para no cerrar el drawer al arrastrar/pulsar el borde.
pub fn drawer_titlebar(state: &ShumaState, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    let titulo = View::new(Style {
        flex_grow: 1.0,
        flex_basis: length(0.0_f32),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(state.placeholder.clone(), 12.0, theme.fg_muted, Alignment::Start);

    let controles = vec![
        tb_button(TbKind::Undock, Msg::ShumaUndock, theme),
        tb_button(TbKind::Minimize, Msg::ShumaToggle, theme),
        tb_button(TbKind::Maximize, Msg::ShumaMaximize, theme),
        tb_button(TbKind::Close, Msg::ShumaToggle, theme),
    ];

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        padding: TaffyRect {
            left: length(10.0_f32),
            right: length(6.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        gap: Size { width: length(2.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .on_click(Msg::ShumaAnim)
    .children(vec![titulo].into_iter().chain(controles).collect())
}

/// El drawer desplegado **con la shuma COMPLETA** (live-wire): mismo scrim +
/// panel inferior que [`drawer_overlay`], pero el cuerpo es la shuma entera
/// (dientes/sesiones/menubar/canvas) elevada al `Msg` de pata vía `Msg::ShumaFull`.
/// El overlay de la propia shuma (menús/modales) se pinta encima del cuerpo.
pub fn drawer_overlay_full(
    state: &ShumaState,
    full: &shuma_app::Model,
    screen: (i32, i32),
    theme: &Theme,
) -> Option<View<Msg>> {
    if !state.visible() {
        return None;
    }
    let t = state.anim.value().clamp(0.0, 1.0);
    let (_sw, sh) = screen;
    let alto = (sh as f32 * drawer_frac(state.maximized) * t).max(1.0);

    // Cuerpo: la shuma completa. Su `view` trae su propio fondo, rails y chrome.
    // Va dentro de un contenedor que crece bajo la barra de título.
    let cuerpo = View::new(Style {
        flex_grow: 1.0,
        flex_basis: length(0.0_f32),
        min_size: Size { width: auto(), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![shuma_app::view(full, crate::lift_shuma)]);
    let mut hijos = vec![drawer_titlebar(state, theme), cuerpo];
    // Overlay interno de la shuma (dropdowns/menús/modales) por encima del cuerpo.
    if let Some(ov) = shuma_app::view_overlay(full, crate::lift_shuma) {
        hijos.push(ov);
    }

    let panel = View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: auto(),
            bottom: length(0.0_f32),
        },
        size: Size {
            width: percent(1.0_f32),
            height: length(alto),
        },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .on_click(Msg::ShumaAnim)
    .children(hijos);

    let scrim = View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(0.0_f32),
            top: length(0.0_f32),
            right: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .alpha(0.55 * t)
    .on_click(Msg::ShumaToggle)
    .children(vec![panel]);

    Some(scrim)
}

/// El drawer desplegado (path **winit**): scrim que cierra al click + panel
/// inferior con el shell real hospedado. `None` si no hay nada que mostrar.
pub fn drawer_overlay(state: &ShumaState, screen: (i32, i32), theme: &Theme) -> Option<View<Msg>> {
    if !state.visible() {
        return None;
    }
    let t = state.anim.value().clamp(0.0, 1.0);
    let (_sw, sh) = screen;
    let alto = (sh as f32 * drawer_frac(state.maximized) * t).max(1.0);

    // El cuerpo es el shell real: su `view` ya trae cards/input/scroll/PTY y
    // pinta su propio fondo (`bg_app`). Los clicks de sus widgets vuelven como
    // `Msg::ShumaShell(..)` gracias al `lift`. Va dentro de un contenedor que
    // crece bajo la barra de título.
    let cuerpo = View::new(Style {
        flex_grow: 1.0,
        flex_basis: length(0.0_f32),
        min_size: Size { width: auto(), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![shuma_module_shell::view(&state.inner, theme, Msg::ShumaShell)]);

    let panel = View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: auto(),
            bottom: length(0.0_f32),
        },
        size: Size {
            width: percent(1.0_f32),
            height: length(alto),
        },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    // Absorbe los clicks sobre el borde del panel (padding) para que no se
    // filtren al scrim y cierren el drawer; `ShumaAnim` es un no-op de re-render.
    .on_click(Msg::ShumaAnim)
    .children(vec![drawer_titlebar(state, theme), cuerpo]);

    // Scrim a pantalla completa: oscurece el fondo y cierra al click.
    let scrim = View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(0.0_f32),
            top: length(0.0_f32),
            right: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .alpha(0.55 * t)
    .on_click(Msg::ShumaToggle)
    .children(vec![panel]);

    Some(scrim)
}

/// El **cuerpo** del drawer (sin scrim ni posición absoluta), para el backend
/// `wlr-layer-shell`: ahí la propia layer surface ya *es* el panel del Quake (la
/// barra crece hacia arriba), así que no hace falta scrim ni animación. Es el
/// shell real hospedado, **sin el input** — el input ya vive en la barra (ver
/// [`headline_view`]). Llena el contenedor que le da el caller.
pub fn drawer_body_view(state: &ShumaState, theme: &Theme) -> View<Msg> {
    shuma_module_shell::body_view(&state.inner, theme, Msg::ShumaShell)
}

/// El **cuerpo** del drawer en modo live-wire (path layer-shell): la shuma
/// COMPLETA (dientes/sesiones/menubar/canvas) elevada al `Msg` de pata. La
/// sesión activa pinta su cuerpo SIN input (vive en la barra, `hosted_bar`), así
/// que no se duplica. Apila el overlay interno (dropdowns/menús/modales) encima.
pub fn drawer_body_view_full(full: &shuma_app::Model, _theme: &Theme) -> View<Msg> {
    let mut hijos = vec![shuma_app::view(full, crate::lift_shuma)];
    if let Some(ov) = shuma_app::view_overlay(full, crate::lift_shuma) {
        hijos.push(ov);
    }
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(hijos)
}

