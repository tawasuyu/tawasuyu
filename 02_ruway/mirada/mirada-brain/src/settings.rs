//! La config del compositor como configuración editable.
//!
//! Refleja [`Config`] en un [`allichay::Schema`] para que el panel de control la
//! pinte con dientes y controles, y la haga editable sin tocar `config.ron` a
//! mano. Los cambios vuelven por [`Configurable::apply`], que muta el modelo; el
//! panel persiste con [`Config::save`] y el `FileWatch` del compositor recarga
//! en caliente.
//!
//! v1 cubrió los **escalares**: teselado, foco, decoración, fondo, terminal
//! dropdown y disposición de monitores. v2 suma la **tabla** del menú raíz
//! (etiqueta + comando, [`Control::Table`]). Los keymaps y los overrides por
//! salida (tablas/listas más ricas) siguen editándose por RON por ahora.

use allichay::{
    AllichayError, Column, Configurable, EnumOption, Field, FieldPath, FieldValue, Schema, Section,
};

use mirada_layout::WallpaperFit;

use crate::action::{layout_from_slug, layout_slug};
use crate::config::{Config, MenuEntry, OutputOverride, OverviewPlace};

/// Formatea un `f32` corto para las tablas de ajustes: hasta 3 decimales, sin
/// ceros ni punto colgantes (`1.0 → "1"`, `2.5 → "2.5"`).
fn fmt_f32(v: f32) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_string()
}

/// Las opciones de modo de teselado (slug + rótulo).
fn layout_options() -> Vec<EnumOption> {
    vec![
        EnumOption::new("master-stack", "Maestra + pila"),
        EnumOption::new("centered-master", "Maestra centrada"),
        EnumOption::new("spiral", "Espiral"),
        EnumOption::new("grid", "Grilla"),
        EnumOption::new("columns", "Columnas"),
        EnumOption::new("rows", "Filas"),
        EnumOption::new("monocle", "Monóculo"),
    ]
}

/// Opciones del modo de transición de Win+Tab (slug serde + rótulo).
fn switch_mode_options() -> Vec<EnumOption> {
    vec![
        EnumOption::new("direct", "Directo (salto seco)"),
        EnumOption::new("hyprland", "Deslizar (estilo Hyprland)"),
        EnumOption::new("prezi", "Prezi (zoom-out)"),
    ]
}

fn switch_mode_slug(m: crate::config::WorkspaceSwitchMode) -> &'static str {
    use crate::config::WorkspaceSwitchMode::*;
    match m {
        Direct => "direct",
        Hyprland => "hyprland",
        Prezi => "prezi",
    }
}

fn switch_mode_from_slug(s: &str) -> Option<crate::config::WorkspaceSwitchMode> {
    use crate::config::WorkspaceSwitchMode::*;
    match s {
        "direct" => Some(Direct),
        "hyprland" => Some(Hyprland),
        "prezi" => Some(Prezi),
        _ => None,
    }
}

/// Opciones de curva de animación (slug serde de [`crate::Easing`] + rótulo).
fn easing_options() -> Vec<EnumOption> {
    vec![
        EnumOption::new("linear", "Lineal"),
        EnumOption::new("ease_out_cubic", "Desacelerar (cúbica)"),
        EnumOption::new("ease_out_back", "Pop (sobre-impulso)"),
    ]
}

/// Fuentes del wallpaper (slug serde de `wallpaper_source` + rótulo). `auto`
/// es la imagen/marca de siempre; `video` enciende el wallpaper animado.
fn wallpaper_source_options() -> Vec<EnumOption> {
    vec![
        EnumOption::new("auto", "Imagen / marca"),
        EnumOption::new("color", "Color sólido"),
        EnumOption::new("gradient", "Gradiente"),
        EnumOption::new("procedural", "Procedural"),
        EnumOption::new("video", "Video (animado)"),
    ]
}

/// Las opciones de ajuste del wallpaper (slug + rótulo).
fn wallpaper_fit_options() -> Vec<EnumOption> {
    vec![
        EnumOption::new("stretch", "Estirar"),
        EnumOption::new("fit", "Encajar"),
        EnumOption::new("fill", "Cubrir"),
        EnumOption::new("center", "Centrar"),
        EnumOption::new("tile", "Mosaico"),
    ]
}

impl Configurable for Config {
    fn schema(&self) -> Schema {
        Schema::new()
            .section(
                Section::new("teselado", "Teselado")
                    .icon("🎛")
                    .help("Cómo se acomodan las ventanas")
                    .field(Field::dropdown(
                        "layout",
                        "Modo",
                        layout_slug(self.layout),
                        layout_options(),
                    ))
                    .field(Field::slider_int("gap", "Margen", self.gap as i64, 0, 48))
                    .field(Field::slider(
                        "master_ratio",
                        "Ancho de la maestra",
                        self.master_ratio as f64,
                        0.05,
                        0.95,
                        0.05,
                    ))
                    .field(Field::slider_int(
                        "master_count",
                        "Ventanas maestras",
                        self.master_count as i64,
                        1,
                        6,
                    ))
                    .field(Field::slider(
                        "master_step",
                        "Paso de la maestra",
                        self.master_step as f64,
                        0.01,
                        0.5,
                        0.01,
                    ))
                    .field(Field::slider_int(
                        "float_step",
                        "Paso de flotantes (px)",
                        self.float_step as i64,
                        1,
                        200,
                    ))
                    .field(Field::toggle(
                        "focus_follows_mouse",
                        "El foco sigue al puntero",
                        self.focus_follows_mouse,
                    ))
                    .field(Field::slider(
                        "tiledad",
                        "Tiledad (flotante ↔ teselado)",
                        self.tiledad as f64,
                        0.0,
                        1.0,
                        0.05,
                    )),
            )
            .section(
                Section::new("decoracion", "Decoración")
                    .icon("🎨")
                    .help("Marco y barra de título de las ventanas")
                    .field(Field::slider_int(
                        "border_width",
                        "Grosor del marco",
                        self.border_width as i64,
                        0,
                        12,
                    ))
                    .field(Field::slider_int(
                        "titlebar_height",
                        "Alto de la barra de título",
                        self.titlebar_height as i64,
                        0,
                        48,
                    ))
                    .field(Field::color("border_focus", "Color con foco", self.border_focus))
                    .field(Field::color(
                        "border_normal",
                        "Color sin foco",
                        self.border_normal,
                    ))
                    .field(Field::toggle(
                        "titlebar_gradient",
                        "Barra de título con degradé",
                        self.titlebar_gradient,
                    ))
                    .field(Field::toggle(
                        "titlebar_floating_only",
                        "Barra sólo en ventanas flotantes",
                        self.titlebar_floating_only,
                    )),
            )
            .section(
                Section::new("fondo", "Fondo")
                    .icon("")
                    .help("Wallpaper y fuente del escritorio")
                    .field(Field::dropdown(
                        "wallpaper_source",
                        "Fuente",
                        self.wallpaper_source.as_str(),
                        wallpaper_source_options(),
                    ))
                    .field(Field::text(
                        "wallpaper_path",
                        "Imagen / video de fondo",
                        self.wallpaper_path.clone(),
                    ))
                    .field(Field::dropdown(
                        "wallpaper_fit",
                        "Ajuste",
                        self.wallpaper_fit.slug(),
                        wallpaper_fit_options(),
                    ))
                    .field(Field::slider_int(
                        "wallpaper_video_fps",
                        "Video — FPS (0 = nativo; bajalo para abaratar)",
                        self.wallpaper_video_fps as i64,
                        0,
                        60,
                    ))
                    .field(Field::text(
                        "wallpaper_dir",
                        "Carpeta (fondo automático)",
                        self.wallpaper_dir.clone(),
                    ))
                    .field(Field::slider_int(
                        "wallpaper_interval_secs",
                        "Cambiar cada (s, 0 = fijo)",
                        self.wallpaper_interval_secs as i64,
                        0,
                        3600,
                    ))
                    .field(Field::text("font_path", "Fuente", self.font_path.clone())),
            )
            .section(
                Section::new("terminal", "Terminal")
                    .icon("⌨")
                    .help("La terminal desplegable (quake)")
                    .field(Field::text(
                        "dropterm_cmd",
                        "Comando",
                        self.dropterm_cmd.clone(),
                    ))
                    .field(Field::slider_int(
                        "dropterm_height_pct",
                        "Alto (% de pantalla)",
                        self.dropterm_height_pct as i64,
                        1,
                        100,
                    )),
            )
            .section(
                Section::new("monitores", "Monitores")
                    .icon("🖥")
                    .help("Disposición de varias salidas")
                    .field(Field::dropdown(
                        "output_direction",
                        "Disposición",
                        self.output_direction.clone(),
                        vec![
                            EnumOption::new("horizontal", "Horizontal"),
                            EnumOption::new("vertical", "Vertical"),
                        ],
                    ))
                    .field(
                        Field::table(
                            "outputs",
                            "Overrides por monitor",
                            vec![
                                Column::new("name", "Monitor"),
                                Column::new("wallpaper", "Fondo"),
                                Column::new("fit", "Ajuste"),
                                Column::new("order", "Orden"),
                            ],
                            self.outputs
                                .iter()
                                .map(|o| {
                                    vec![
                                        o.name.clone(),
                                        o.wallpaper_path.clone(),
                                        o.wallpaper_fit.clone(),
                                        o.order.to_string(),
                                    ]
                                })
                                .collect(),
                        )
                        .help("Conector DRM (HDMI-A-1, DP-1…). Escala y rotación siguen en RON."),
                    ),
            )
            .section(
                Section::new("menu", "Menú raíz")
                    .icon("☰")
                    .help("Acciones del click derecho sobre el fondo")
                    .field(Field::table(
                        "entradas",
                        "Entradas",
                        vec![
                            Column::new("label", "Etiqueta"),
                            Column::new("command", "Comando"),
                        ],
                        self.menu
                            .iter()
                            .map(|e| vec![e.label.clone(), e.command.clone()])
                            .collect(),
                    )),
            )
            .section(
                Section::new("vista_espacial", "Vista espacial")
                    .icon("🗺")
                    .help("El zoom-out tipo Prezi para saltar entre escritorios")
                    .field(Field::toggle(
                        "overview_enabled",
                        "Habilitar vista espacial",
                        self.overview_enabled,
                    ))
                    .field(Field::slider_int(
                        "overview_columns",
                        "Columnas (0 = automático)",
                        self.overview_columns as i64,
                        0,
                        8,
                    ))
                    .field(Field::slider_int(
                        "overview_anim_ms",
                        "Vuelo de cámara (ms)",
                        self.overview_anim_ms as i64,
                        0,
                        800,
                    ))
                    .field(Field::toggle(
                        "overview_show_titles",
                        "Mostrar títulos en las miniaturas",
                        self.overview_show_titles,
                    ))
                    .field(Field::dropdown(
                        "workspace_switch_mode",
                        "Transición Win+Tab",
                        switch_mode_slug(self.workspace_switch_mode),
                        switch_mode_options(),
                    ))
                    .field(Field::table(
                        "overview_geometry",
                        "Geometría 2D (col, fila por escritorio)",
                        vec![Column::new("col", "Columna"), Column::new("row", "Fila")],
                        self.overview_geometry_for(crate::action::WORKSPACE_COUNT)
                            .into_iter()
                            .map(|(c, r)| vec![c.to_string(), r.to_string()])
                            .collect(),
                    ))
                    .field(Field::table(
                        "overview_places",
                        "Plano Prezi (x, y, ancho, alto, giro° por escritorio)",
                        vec![
                            Column::new("x", "X"),
                            Column::new("y", "Y"),
                            Column::new("w", "Ancho"),
                            Column::new("h", "Alto"),
                            Column::new("rot", "Giro°"),
                        ],
                        self.overview_places_for(crate::action::WORKSPACE_COUNT)
                            .into_iter()
                            .map(|p| {
                                vec![
                                    fmt_f32(p.x),
                                    fmt_f32(p.y),
                                    fmt_f32(p.w),
                                    fmt_f32(p.h),
                                    fmt_f32(p.rot.to_degrees()),
                                ]
                            })
                            .collect(),
                    )),
            )
            .section(
                Section::new("inactividad", "Inactividad")
                    .icon("🌙")
                    .help("Apagar la pantalla y bloquear tras un rato sin uso (0 = nunca)")
                    .field(Field::slider_int(
                        "idle_screen_off_secs",
                        "Apagar pantalla tras (segundos)",
                        self.idle_screen_off_secs as i64,
                        0,
                        3600,
                    ))
                    .field(Field::slider_int(
                        "idle_lock_secs",
                        "Bloquear sesión tras (segundos)",
                        self.idle_lock_secs as i64,
                        0,
                        3600,
                    ))
                    .field(Field::toggle(
                        "idle_respect_inhibitors",
                        "Respetar reproductores (no apagar viendo vídeo)",
                        self.idle_respect_inhibitors,
                    )),
            )
            .section(
                Section::new("movimiento", "Movimiento")
                    .icon("✨")
                    .help("Animaciones del escritorio (0 ms = sin animación)")
                    .field(Field::toggle(
                        "reduce_motion",
                        "Reducir movimiento (apaga todas las animaciones)",
                        self.reduce_motion,
                    ))
                    .field(Field::slider_int(
                        "window_open_ms",
                        "Apertura de ventana — fundido (ms)",
                        self.window_open_ms as i64,
                        0,
                        600,
                    ))
                    .field(Field::dropdown(
                        "window_open_easing",
                        "Apertura de ventana — curva",
                        self.window_open_easing.slug(),
                        easing_options(),
                    ))
                    .field(Field::slider_int(
                        "window_open_scale_pct",
                        "Apertura de ventana — pop (% inicial, 100 = sin pop)",
                        self.window_open_scale_pct as i64,
                        50,
                        100,
                    ))
                    .field(Field::slider_int(
                        "focus_glow_ms",
                        "Glow de foco — fundido del marco (ms)",
                        self.focus_glow_ms as i64,
                        0,
                        600,
                    ))
                    .field(Field::slider_int(
                        "window_close_ms",
                        "Cierre de ventana — fade (ms, 0 = seco; opt-in, cuesta GPU)",
                        self.window_close_ms as i64,
                        0,
                        600,
                    ))
                    .field(Field::slider_int(
                        "unfocused_dim_pct",
                        "Atenuar ventanas sin foco (%, 0 = no)",
                        self.unfocused_dim_pct as i64,
                        0,
                        80,
                    ))
                    .field(Field::slider_int(
                        "slide_ms",
                        "Deslizar entre escritorios (ms)",
                        self.slide_ms as i64,
                        0,
                        600,
                    )),
            )
    }

    fn apply(&mut self, path: &FieldPath, value: FieldValue) -> Result<(), AllichayError> {
        let unknown = || AllichayError::UnknownPath(path.to_string());
        match path.leaf().ok_or_else(unknown)? {
            "layout" => {
                if let Some(m) = value.as_str().and_then(layout_from_slug) {
                    self.layout = m;
                }
            }
            "gap" => {
                if let Some(v) = value.as_int() {
                    self.gap = v as i32;
                }
            }
            "master_ratio" => {
                if let Some(v) = value.as_float() {
                    self.master_ratio = v as f32;
                }
            }
            "master_count" => {
                if let Some(v) = value.as_int() {
                    self.master_count = v.max(1) as usize;
                }
            }
            "master_step" => {
                if let Some(v) = value.as_float() {
                    self.master_step = v as f32;
                }
            }
            "float_step" => {
                if let Some(v) = value.as_int() {
                    self.float_step = v as i32;
                }
            }
            "focus_follows_mouse" => {
                if let Some(b) = value.as_bool() {
                    self.focus_follows_mouse = b;
                }
            }
            "tiledad" => {
                if let Some(v) = value.as_float() {
                    self.tiledad = (v as f32).clamp(0.0, 1.0);
                }
            }
            "border_width" => {
                if let Some(v) = value.as_int() {
                    self.border_width = v as i32;
                }
            }
            "titlebar_height" => {
                if let Some(v) = value.as_int() {
                    self.titlebar_height = v as i32;
                }
            }
            "titlebar_gradient" => {
                if let Some(b) = value.as_bool() {
                    self.titlebar_gradient = b;
                }
            }
            "titlebar_floating_only" => {
                if let Some(b) = value.as_bool() {
                    self.titlebar_floating_only = b;
                }
            }
            "border_focus" => {
                if let Some(c) = value.as_color() {
                    self.border_focus = c;
                }
            }
            "border_normal" => {
                if let Some(c) = value.as_color() {
                    self.border_normal = c;
                }
            }
            "wallpaper_source" => {
                if let Some(s) = value.as_str() {
                    self.wallpaper_source = s.to_string();
                }
            }
            "wallpaper_path" => {
                if let Some(s) = value.as_str() {
                    self.wallpaper_path = s.to_string();
                }
            }
            "wallpaper_fit" => {
                if let Some(f) = value.as_str().and_then(WallpaperFit::from_slug) {
                    self.wallpaper_fit = f;
                }
            }
            "wallpaper_video_fps" => {
                if let Some(v) = value.as_int() {
                    self.wallpaper_video_fps = v.clamp(0, 60) as u32;
                }
            }
            "wallpaper_dir" => {
                if let Some(s) = value.as_str() {
                    self.wallpaper_dir = s.to_string();
                }
            }
            "wallpaper_interval_secs" => {
                if let Some(v) = value.as_int() {
                    self.wallpaper_interval_secs = v.max(0) as u32;
                }
            }
            "font_path" => {
                if let Some(s) = value.as_str() {
                    self.font_path = s.to_string();
                }
            }
            "xkb_layout" => {
                if let Some(s) = value.as_str() {
                    self.xkb_layout = s.to_string();
                }
            }
            "xkb_variant" => {
                if let Some(s) = value.as_str() {
                    self.xkb_variant = s.to_string();
                }
            }
            "natural_scroll" => {
                if let Some(b) = value.as_bool() {
                    self.natural_scroll = b;
                }
            }
            "tap_to_click" => {
                if let Some(b) = value.as_bool() {
                    self.tap_to_click = b;
                }
            }
            "pointer_speed" => {
                if let Some(v) = value.as_float() {
                    self.pointer_speed = v.clamp(-1.0, 1.0);
                }
            }
            "dropterm_cmd" => {
                if let Some(s) = value.as_str() {
                    self.dropterm_cmd = s.to_string();
                }
            }
            "dropterm_height_pct" => {
                if let Some(v) = value.as_int() {
                    self.dropterm_height_pct = v.clamp(1, 100) as u32;
                }
            }
            "output_direction" => {
                if let Some(s) = value.as_str() {
                    // Sólo aceptamos los slugs válidos; otro valor se ignora.
                    if s == "horizontal" || s == "vertical" {
                        self.output_direction = s.to_string();
                    }
                }
            }
            "entradas" => {
                if let Some(rows) = value.as_table() {
                    // Reconstruimos el menú desde la tabla (etiqueta, comando),
                    // preservando el `submenu` de la entrada que estaba en esa
                    // posición — la tabla plana no lo edita, pero no debe
                    // perderlo. Filas nuevas son hojas; filas de más se truncan.
                    let prev = core::mem::take(&mut self.menu);
                    self.menu = rows
                        .iter()
                        .enumerate()
                        .map(|(i, r)| {
                            let label = r.first().cloned().unwrap_or_default();
                            let command = r.get(1).cloned().unwrap_or_default();
                            match prev.get(i) {
                                Some(p) if !p.submenu.is_empty() => MenuEntry {
                                    label,
                                    command: p.command.clone(),
                                    submenu: p.submenu.clone(),
                                },
                                _ => MenuEntry {
                                    label,
                                    command,
                                    submenu: Vec::new(),
                                },
                            }
                        })
                        .collect();
                }
            }
            "outputs" => {
                if let Some(rows) = value.as_table() {
                    // Reconstruimos los overrides desde la tabla, preservando por
                    // índice los campos que la tabla no edita (escala, rotación).
                    // El ajuste se valida: un slug inválido cae al previo (o vacío).
                    let prev = core::mem::take(&mut self.outputs);
                    self.outputs = rows
                        .iter()
                        .enumerate()
                        .map(|(i, r)| {
                            let p = prev.get(i);
                            let name = r.first().cloned().unwrap_or_default();
                            let wallpaper_path = r.get(1).cloned().unwrap_or_default();
                            let fit_in = r.get(2).map(String::as_str).unwrap_or("");
                            let wallpaper_fit =
                                if fit_in.is_empty() || WallpaperFit::from_slug(fit_in).is_some() {
                                    fit_in.to_string()
                                } else {
                                    p.map(|p| p.wallpaper_fit.clone()).unwrap_or_default()
                                };
                            let order = r
                                .get(3)
                                .and_then(|s| s.trim().parse::<i32>().ok())
                                .unwrap_or_else(|| p.map(|p| p.order).unwrap_or(0));
                            OutputOverride {
                                name,
                                wallpaper_path,
                                wallpaper_fit,
                                order,
                                scale_120: p.map(|p| p.scale_120).unwrap_or(0),
                                transform: p.map(|p| p.transform.clone()).unwrap_or_default(),
                            }
                        })
                        .collect();
                }
            }
            "overview_enabled" => {
                if let Some(b) = value.as_bool() {
                    self.overview_enabled = b;
                }
            }
            "overview_columns" => {
                if let Some(v) = value.as_int() {
                    self.overview_columns = v.clamp(0, 8) as u32;
                }
            }
            "overview_anim_ms" => {
                if let Some(v) = value.as_int() {
                    self.overview_anim_ms = v.clamp(0, 800) as u32;
                }
            }
            "overview_show_titles" => {
                if let Some(b) = value.as_bool() {
                    self.overview_show_titles = b;
                }
            }
            "workspace_switch_mode" => {
                if let Some(m) = value.as_str().and_then(switch_mode_from_slug) {
                    self.workspace_switch_mode = m;
                }
            }
            "overview_geometry" => {
                // Tabla (col, fila) por escritorio → geometría 2D del Prezi.
                if let Some(rows) = value.as_table() {
                    self.overview_geometry = rows
                        .iter()
                        .map(|r| {
                            let c = r.first().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
                            let row = r.get(1).and_then(|s| s.trim().parse().ok()).unwrap_or(0);
                            (c, row)
                        })
                        .collect();
                }
            }
            "overview_places" => {
                // Tabla (x, y, ancho, alto, giro°) por escritorio → plano rico.
                if let Some(rows) = value.as_table() {
                    let cell = |r: &[String], i: usize, dflt: f32| {
                        r.get(i).and_then(|s| s.trim().parse::<f32>().ok()).unwrap_or(dflt)
                    };
                    self.overview_places = rows
                        .iter()
                        .map(|r| {
                            OverviewPlace::new(
                                cell(r, 0, 0.0),
                                cell(r, 1, 0.0),
                                cell(r, 2, 1.0),
                                cell(r, 3, 1.0),
                                cell(r, 4, 0.0).to_radians(),
                            )
                        })
                        .collect();
                }
            }
            "idle_screen_off_secs" => {
                if let Some(v) = value.as_int() {
                    self.idle_screen_off_secs = v.clamp(0, 3600) as u32;
                }
            }
            "idle_lock_secs" => {
                if let Some(v) = value.as_int() {
                    self.idle_lock_secs = v.clamp(0, 3600) as u32;
                }
            }
            "idle_respect_inhibitors" => {
                if let Some(b) = value.as_bool() {
                    self.idle_respect_inhibitors = b;
                }
            }
            "reduce_motion" => {
                if let Some(b) = value.as_bool() {
                    self.reduce_motion = b;
                }
            }
            "window_open_ms" => {
                if let Some(v) = value.as_int() {
                    self.window_open_ms = v.clamp(0, 600) as u32;
                }
            }
            "window_open_easing" => {
                if let Some(e) = value.as_str().and_then(crate::Easing::from_slug) {
                    self.window_open_easing = e;
                }
            }
            "window_open_scale_pct" => {
                if let Some(v) = value.as_int() {
                    self.window_open_scale_pct = v.clamp(50, 100) as u8;
                }
            }
            "focus_glow_ms" => {
                if let Some(v) = value.as_int() {
                    self.focus_glow_ms = v.clamp(0, 600) as u32;
                }
            }
            "window_close_ms" => {
                if let Some(v) = value.as_int() {
                    self.window_close_ms = v.clamp(0, 600) as u32;
                }
            }
            "unfocused_dim_pct" => {
                if let Some(v) = value.as_int() {
                    self.unfocused_dim_pct = v.clamp(0, 80) as u8;
                }
            }
            "slide_ms" => {
                if let Some(v) = value.as_int() {
                    self.slide_ms = v.clamp(0, 600) as u32;
                }
            }
            _ => return Err(unknown()),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirada_layout::LayoutMode;

    #[test]
    fn schema_tiene_las_secciones() {
        let schema = Config::default().schema();
        let ids: Vec<&str> = schema.sections.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "teselado",
                "decoracion",
                "fondo",
                "terminal",
                "monitores",
                "menu",
                "vista_espacial",
                "inactividad",
                "movimiento"
            ]
        );
    }

    #[test]
    fn movimiento_aplica_animaciones() {
        let mut c = Config::default();
        c.apply(&"movimiento.window_open_ms".into(), FieldValue::Int(240))
            .unwrap();
        assert_eq!(c.window_open_ms, 240);
        // Se acota al rango.
        c.apply(&"movimiento.window_open_ms".into(), FieldValue::Int(99999))
            .unwrap();
        assert_eq!(c.window_open_ms, 600);
        // La curva entra por slug.
        c.apply(
            &"movimiento.window_open_easing".into(),
            FieldValue::Text("ease_out_back".into()),
        )
        .unwrap();
        assert_eq!(c.window_open_easing, crate::Easing::EaseOutBack);
        // El pop se acota a [50, 100].
        c.apply(&"movimiento.window_open_scale_pct".into(), FieldValue::Int(80))
            .unwrap();
        assert_eq!(c.window_open_scale_pct, 80);
        c.apply(&"movimiento.window_open_scale_pct".into(), FieldValue::Int(10))
            .unwrap();
        assert_eq!(c.window_open_scale_pct, 50);
        // El maestro de accesibilidad y el slide.
        c.apply(&"movimiento.reduce_motion".into(), FieldValue::Bool(true))
            .unwrap();
        assert!(c.reduce_motion);
        c.apply(&"movimiento.slide_ms".into(), FieldValue::Int(120))
            .unwrap();
        assert_eq!(c.slide_ms, 120);
        // El glow de foco.
        c.apply(&"movimiento.focus_glow_ms".into(), FieldValue::Int(200))
            .unwrap();
        assert_eq!(c.focus_glow_ms, 200);
        // El fade al cerrar (opt-in).
        c.apply(&"movimiento.window_close_ms".into(), FieldValue::Int(180))
            .unwrap();
        assert_eq!(c.window_close_ms, 180);
        // Atenuar sin foco, acotado a [0, 80].
        c.apply(&"movimiento.unfocused_dim_pct".into(), FieldValue::Int(40))
            .unwrap();
        assert_eq!(c.unfocused_dim_pct, 40);
        c.apply(&"movimiento.unfocused_dim_pct".into(), FieldValue::Int(99))
            .unwrap();
        assert_eq!(c.unfocused_dim_pct, 80);
    }

    #[test]
    fn inactividad_aplica_y_proyecta_idle_config() {
        let mut c = Config::default();
        c.apply(&"inactividad.idle_screen_off_secs".into(), FieldValue::Int(300))
            .unwrap();
        c.apply(&"inactividad.idle_lock_secs".into(), FieldValue::Int(600))
            .unwrap();
        c.apply(&"inactividad.idle_respect_inhibitors".into(), FieldValue::Bool(false))
            .unwrap();
        let ic = c.idle_config();
        assert_eq!(ic.screen_off_secs, 300);
        assert_eq!(ic.lock_secs, 600);
        assert!(!ic.respect_inhibitors);
        // Se acota al rango.
        c.apply(&"inactividad.idle_lock_secs".into(), FieldValue::Int(99999))
            .unwrap();
        assert_eq!(c.idle_lock_secs, 3600);
    }

    #[test]
    fn vista_espacial_expone_el_prezi_y_aplica() {
        let schema = Config::default().schema();
        let vista = schema
            .sections
            .iter()
            .find(|s| s.id == "vista_espacial")
            .expect("debe existir la sección vista_espacial");
        let field_ids: Vec<&str> = vista.fields.iter().map(|f| f.id.as_str()).collect();
        assert!(field_ids.contains(&"workspace_switch_mode"), "{field_ids:?}");
        assert!(field_ids.contains(&"overview_geometry"), "{field_ids:?}");

        // El modo se aplica desde el panel.
        let mut c = Config::default();
        c.apply(
            &FieldPath::empty().push("vista_espacial").push("workspace_switch_mode"),
            FieldValue::Text("hyprland".into()),
        )
        .unwrap();
        assert_eq!(c.workspace_switch_mode, crate::config::WorkspaceSwitchMode::Hyprland);
        // La geometría se aplica desde la tabla.
        c.apply(
            &FieldPath::empty().push("vista_espacial").push("overview_geometry"),
            FieldValue::Table(vec![vec!["2".into(), "1".into()], vec!["0".into(), "0".into()]]),
        )
        .unwrap();
        assert_eq!(c.overview_geometry, vec![(2, 1), (0, 0)]);
    }

    #[test]
    fn apply_menu_reconstruye_y_preserva_submenu() {
        let mut c = Config::default();
        c.menu = vec![
            MenuEntry {
                label: "Apps".into(),
                command: String::new(),
                submenu: vec![MenuEntry {
                    label: "Editor".into(),
                    command: "nada".into(),
                    submenu: Vec::new(),
                }],
            },
            MenuEntry {
                label: "Terminal".into(),
                command: "xterm".into(),
                submenu: Vec::new(),
            },
        ];
        // Renombramos la hoja y agregamos una fila nueva; la fila 0 (submenú)
        // cambia de etiqueta pero conserva sus hijos.
        let nuevo = FieldValue::Table(vec![
            vec!["Programas".into(), "ignorado".into()],
            vec!["Consola".into(), "alacritty".into()],
            vec!["Navegador".into(), "puriy".into()],
        ]);
        c.apply(&"menu.entradas".into(), nuevo).unwrap();
        assert_eq!(c.menu.len(), 3);
        assert_eq!(c.menu[0].label, "Programas");
        assert_eq!(c.menu[0].submenu.len(), 1); // submenú preservado
        assert!(c.menu[0].command.is_empty()); // el comando de un submenú no se toca
        assert_eq!(c.menu[1].command, "alacritty");
        assert_eq!(c.menu[2].label, "Navegador");
        assert!(c.menu[2].submenu.is_empty());
    }

    #[test]
    fn apply_layout_por_slug() {
        let mut c = Config::default();
        c.apply(&"teselado.layout".into(), FieldValue::Enum("grid".into()))
            .unwrap();
        assert_eq!(c.layout, LayoutMode::Grid);
    }

    #[test]
    fn apply_escalares_y_color() {
        let mut c = Config::default();
        c.apply(&"teselado.gap".into(), FieldValue::Int(16)).unwrap();
        c.apply(&"teselado.master_ratio".into(), FieldValue::Float(0.7))
            .unwrap();
        c.apply(&"decoracion.border_focus".into(), FieldValue::Color([1, 2, 3, 255]))
            .unwrap();
        assert_eq!(c.gap, 16);
        assert!((c.master_ratio - 0.7).abs() < 1e-6);
        assert_eq!(c.border_focus, [1, 2, 3, 255]);
    }

    #[test]
    fn apply_tiledad_se_acota_a_la_unidad() {
        let mut c = Config::default();
        c.apply(&"teselado.tiledad".into(), FieldValue::Float(0.85))
            .unwrap();
        assert!((c.tiledad - 0.85).abs() < 1e-6);
        // Fuera de [0,1] se recorta, no rompe.
        c.apply(&"teselado.tiledad".into(), FieldValue::Float(3.0))
            .unwrap();
        assert_eq!(c.tiledad, 1.0);
    }

    #[test]
    fn apply_wallpaper_fit_y_texto() {
        let mut c = Config::default();
        c.apply(&"fondo.wallpaper_fit".into(), FieldValue::Enum("fill".into()))
            .unwrap();
        c.apply(&"fondo.wallpaper_path".into(), FieldValue::Text("/w.png".into()))
            .unwrap();
        assert_eq!(c.wallpaper_fit, WallpaperFit::Fill);
        assert_eq!(c.wallpaper_path, "/w.png");
    }

    #[test]
    fn apply_wallpaper_video_fuente_y_fps() {
        let mut c = Config::default();
        c.apply(&"fondo.wallpaper_source".into(), FieldValue::Enum("video".into()))
            .unwrap();
        c.apply(&"fondo.wallpaper_video_fps".into(), FieldValue::Int(24))
            .unwrap();
        assert_eq!(c.wallpaper_source, "video");
        assert_eq!(c.wallpaper_video_fps, 24);
        // FPS se acota a [0, 60].
        c.apply(&"fondo.wallpaper_video_fps".into(), FieldValue::Int(999))
            .unwrap();
        assert_eq!(c.wallpaper_video_fps, 60);
    }

    #[test]
    fn apply_dropterm_height_se_acota() {
        let mut c = Config::default();
        c.apply(&"terminal.dropterm_height_pct".into(), FieldValue::Int(250))
            .unwrap();
        assert_eq!(c.dropterm_height_pct, 100);
    }

    #[test]
    fn apply_outputs_preserva_escala_y_rotacion() {
        let mut c = Config::default();
        c.outputs = vec![OutputOverride {
            name: "HDMI-A-1".into(),
            wallpaper_path: "/viejo.png".into(),
            wallpaper_fit: "fill".into(),
            order: 0,
            scale_120: 180,
            transform: "90".into(),
        }];
        // La tabla edita nombre/fondo/ajuste/orden; escala y rotación no se tocan.
        let nuevo = FieldValue::Table(vec![vec![
            "HDMI-A-1".into(),
            "/nuevo.png".into(),
            "center".into(),
            "2".into(),
        ]]);
        c.apply(&"monitores.outputs".into(), nuevo).unwrap();
        assert_eq!(c.outputs.len(), 1);
        let o = &c.outputs[0];
        assert_eq!(o.wallpaper_path, "/nuevo.png");
        assert_eq!(o.wallpaper_fit, "center");
        assert_eq!(o.order, 2);
        assert_eq!(o.scale_120, 180); // preservado
        assert_eq!(o.transform, "90"); // preservado
    }

    #[test]
    fn apply_outputs_fit_invalido_cae_al_previo() {
        let mut c = Config::default();
        c.outputs = vec![OutputOverride {
            name: "DP-1".into(),
            wallpaper_path: String::new(),
            wallpaper_fit: "fit".into(),
            order: 0,
            scale_120: 0,
            transform: String::new(),
        }];
        let nuevo = FieldValue::Table(vec![vec![
            "DP-1".into(),
            String::new(),
            "garabato".into(), // slug inválido
            "0".into(),
        ]]);
        c.apply(&"monitores.outputs".into(), nuevo).unwrap();
        assert_eq!(c.outputs[0].wallpaper_fit, "fit"); // conserva el válido previo
    }

    #[test]
    fn apply_ruta_desconocida_es_error() {
        let mut c = Config::default();
        assert!(c.apply(&"teselado.nope".into(), FieldValue::Int(1)).is_err());
    }
}
