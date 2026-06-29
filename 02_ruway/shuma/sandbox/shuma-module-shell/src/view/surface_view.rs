use super::*;

// ── Superficie de terminal virtualizada (la ÚNICA vía de output desde la Fase 5) ──
//
// El `output_pane` viejo + las cards per-comando IDE fueron borrados; esta es la
// única superficie de output (salvo PTY/TUI fullscreen). Mapea el modelo del shell
// (`OutputLine` + bloques + `collapsed` + `block_command`) al modelo de bloques
// de `llimphi-widget-terminal`: cada comando = un header (chrome) + su cuerpo
// (rango de líneas en un `Scrollback`); colapsar = no emitir el cuerpo. El
// scroll del widget vive en la superficie (no en un `transform`), evitando de
// raíz el bug clip+transform; convertimos `scroll_px` (px desde el fondo, el
// modelo del shell) ↔ `scroll_y` (px desde arriba, el del widget).

/// Alto fijo del header de comando en la superficie (px).
const SURFACE_HEADER_H: f32 = 22.0;

/// Alto del header de una sub-sección (sub-collapsable dentro de un block).
/// Un pelo más bajo que `SURFACE_HEADER_H` para destacar la jerarquía.
const SECTION_HEADER_H: f32 = 20.0;

/// Alto del header de columnas de una tabla de sección (px).
const SECTION_TABLE_HEADER_H: f32 = 22.0;
/// Alto de una fila de tabla de sección (px).
const SECTION_TABLE_ROW_H: f32 = 20.0;

/// Cap de filas renderizadas por sección-tabla. Más allá, agregamos una
/// fila final "+N filas …" en lugar de pintar 5000 Views. Cuando el usuario
/// ordena por una columna, la limitación sigue aplicando (ve los top-N
/// según ese orden) — útil para tablas muy gordas tipo `ls -lR /usr`.
pub(crate) const SECTION_TABLE_MAX_ROWS: usize = 200;

/// Padding inferior (px) bajo una imagen horneada en el scrollback.
const IMAGE_PAD: f32 = 6.0;

/// Tamaño en px (ancho, alto-con-padding) de una imagen horneada (kitty/sixel)
/// dadas las métricas de la superficie. Si el protocolo pidió celdas
/// (`cols`/`rows`) se respetan; si no, se encaja el tamaño en píxeles a un
/// ancho máximo razonable preservando el aspecto.
fn baked_image_size(
    img: &crate::types::TermImage,
    m: llimphi_widget_terminal::TermMetrics,
) -> (f32, f32) {
    let cw = m.char_width.max(1.0);
    let ch = m.line_height.max(1.0);
    let target_w = if img.cols > 0 {
        img.cols as f32 * cw
    } else {
        (img.px_w as f32).min(72.0 * cw)
    };
    let target_h = if img.rows > 0 {
        img.rows as f32 * ch
    } else {
        let aspect = img.px_h as f32 / img.px_w.max(1) as f32;
        target_w * aspect
    };
    (target_w, target_h + IMAGE_PAD)
}

/// Chrome de una imagen horneada: un nodo del ancho del card con la imagen
/// alineada a la izquierda, encajada (`Contain`) en su caja `w`×`h`.
fn baked_image_view<Msg: Clone + 'static>(
    img: &crate::types::TermImage,
    w: f32,
    h: f32,
) -> View<Msg> {
    let inner = View::new(Style {
        size: Size {
            width: length(w),
            height: length((h - IMAGE_PAD).max(1.0)),
        },
        ..Default::default()
    })
    .image(img.image.clone())
    .image_fit(llimphi_ui::ImageFit::Contain);
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(h),
        },
        ..Default::default()
    })
    .children(vec![inner])
}

/// Alto total de una tabla con `n_rows` filas (capeado por SECTION_TABLE_MAX_ROWS,
/// +1 fila para el mensaje "+N filas …" cuando aplica).
pub(crate) fn section_table_height(n_rows: usize) -> f32 {
    let visible = n_rows.min(SECTION_TABLE_MAX_ROWS);
    let truncado = if n_rows > SECTION_TABLE_MAX_ROWS { 1.0 } else { 0.0 };
    SECTION_TABLE_HEADER_H + (visible as f32 + truncado) * SECTION_TABLE_ROW_H
}

/// Pinta una sub-sección como tabla con headers clickeables (ordenar
/// asc/desc/sin orden) + filas mono striped. Las filas se ordenan según
/// `sort = (col, ascending)`; si `None`, orden natural del output.
pub(crate) fn section_table_view<HostMsg: Clone + 'static>(
    block: u64,
    section: usize,
    columns: &[String],
    rows: &[Vec<String>],
    sort: Option<(usize, bool)>,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> View<HostMsg> {
    use llimphi_ui::llimphi_raster::peniko::Color;
    // Anchos heurísticos por columna — mejor sería medir, pero
    // `ls -l` tiene anchos típicos predecibles.
    fn col_width(idx: usize, n: usize, name: &str) -> f32 {
        match name {
            "permisos" => 100.0,
            "links" => 50.0,
            "owner" | "group" => 80.0,
            "size" => 80.0,
            "fecha" => 100.0,
            "comando" => 200.0, // :stats — nombres de binario + flags cortas
            "variable" => 180.0, // env — nombres de variable
            "hash" => 90.0, // git log --oneline
            _ if idx == n - 1 => 0.0, // última = flex
            _ => 90.0,
        }
    }
    let n = columns.len();
    // Header row.
    let mut header_children: Vec<View<HostMsg>> = Vec::with_capacity(n);
    for (col, name) in columns.iter().enumerate() {
        let arrow = match sort {
            Some((c, true)) if c == col => " ▲",
            Some((c, false)) if c == col => " ▼",
            _ => "",
        };
        let w = col_width(col, n, name);
        let mut style = Style {
            size: Size { width: length(w.max(40.0)), height: percent(1.0_f32) },
            align_items: Some(AlignItems::Center),
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            flex_shrink: 0.0,
            ..Default::default()
        };
        if w == 0.0 {
            style.size.width = Dimension::auto();
            style.flex_grow = 1.0;
        }
        header_children.push(
            View::new(style)
                .hover_fill(theme.bg_row_hover)
                .on_click(lift(Msg::SortSectionColumn { block, section, col }))
                .text_aligned(
                    format!("{name}{arrow}"),
                    11.0,
                    theme.fg_placeholder,
                    Alignment::Start,
                )
                .mono(),
        );
    }
    let header = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(SECTION_TABLE_HEADER_H) },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(header_children);

    // Rows. Aplica orden si lo hay (clon para no mutar el state).
    let mut order: Vec<usize> = (0..rows.len()).collect();
    if let Some((col, asc)) = sort {
        order.sort_by(|&a, &b| {
            let ax = rows[a].get(col).map(|s| s.as_str()).unwrap_or("");
            let bx = rows[b].get(col).map(|s| s.as_str()).unwrap_or("");
            // Si parecen números, orden numérico; si no, lexicográfico.
            let cmp = match (ax.parse::<u64>().ok(), bx.parse::<u64>().ok()) {
                (Some(an), Some(bn)) => an.cmp(&bn),
                _ => ax.cmp(bx),
            };
            if asc { cmp } else { cmp.reverse() }
        });
    }
    let total_rows = order.len();
    let visible_rows = total_rows.min(SECTION_TABLE_MAX_ROWS);
    let truncated = total_rows > SECTION_TABLE_MAX_ROWS;
    let mut row_views: Vec<View<HostMsg>> = Vec::with_capacity(visible_rows + 1);
    // Stripe sutil: en vez de alternar bg_panel/transparente (saltaba a la
    // vista), las filas pares llevan un velo apenas perceptible del fg —
    // guía el ojo sin armar un tablero de ajedrez.
    let stripe_tint = {
        let c = theme.fg_text.to_rgba8();
        Color::from_rgba8(c.r, c.g, c.b, 10)
    };
    for (vis_idx, &ri) in order.iter().take(visible_rows).enumerate() {
        let row = &rows[ri];
        let stripe = if vis_idx % 2 == 0 {
            stripe_tint
        } else {
            Color::from_rgba8(0, 0, 0, 0) // transparente
        };
        // Tipo de entrada según la máscara de permisos (col "permisos"):
        // colorea el nombre (dir = accent, ejecutable = verde, symlink =
        // cian) — el `ls -l` se lee como un explorador.
        let perms = columns
            .iter()
            .position(|c| c == "permisos")
            .and_then(|ci| row.get(ci))
            .map(|s| s.as_str())
            .unwrap_or("");
        let name_color = if perms.starts_with('d') {
            theme.accent
        } else if perms.starts_with('l') {
            Color::from_rgba8(100, 200, 200, 255)
        } else if perms.contains('x') {
            Color::from_rgba8(130, 205, 140, 255)
        } else {
            theme.fg_text
        };
        let mut cells: Vec<View<HostMsg>> = Vec::with_capacity(n);
        for col in 0..n {
            let name = columns.get(col).map(|s| s.as_str()).unwrap_or("");
            let w = col_width(col, n, name);
            // Color por columna: metadata en tonos propios, nombre según
            // tipo — espeja el coloreo semántico del cuerpo de output.
            let cell_color = match name {
                "permisos" => Color::from_rgba8(140, 152, 175, 255),
                "links" | "owner" | "group" => theme.fg_muted,
                "size" => Color::from_rgba8(209, 154, 102, 255),
                "fecha" => Color::from_rgba8(126, 166, 180, 255),
                _ if col == n - 1 => name_color,
                _ => theme.fg_text,
            };
            let mut style = Style {
                size: Size { width: length(w.max(40.0)), height: percent(1.0_f32) },
                align_items: Some(AlignItems::Center),
                padding: Rect {
                    left: length(8.0_f32),
                    right: length(8.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                flex_shrink: 0.0,
                ..Default::default()
            };
            if w == 0.0 {
                style.size.width = Dimension::auto();
                style.flex_grow = 1.0;
            }
            cells.push(
                View::new(style)
                    .text_aligned(
                        row.get(col).cloned().unwrap_or_default(),
                        11.0,
                        cell_color,
                        Alignment::Start,
                    )
                    .mono()
                    .max_lines(1),
            );
        }
        row_views.push(
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size { width: percent(1.0_f32), height: length(SECTION_TABLE_ROW_H) },
                ..Default::default()
            })
            .fill(stripe)
            .hover_fill(theme.bg_row_hover)
            .children(cells),
        );
    }
    // Mensaje de truncado: si la tabla tiene más filas que SECTION_TABLE_MAX_ROWS,
    // mostramos una última fila informativa.
    if truncated {
        let extra = total_rows - visible_rows;
        row_views.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(SECTION_TABLE_ROW_H) },
                padding: Rect {
                    left: length(8.0_f32),
                    right: length(8.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text_aligned(
                format!("… +{extra} filas (sort por una columna para acotar)"),
                10.0,
                theme.fg_muted,
                Alignment::Start,
            )
            .mono(),
        );
    }

    let mut all = vec![header];
    all.extend(row_views);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: length(section_table_height(rows.len())),
        },
        ..Default::default()
    })
    .children(all)
}

/// `true` si la sub-sección `idx` del bloque `block` debe arrancar colapsada
/// por default. Heurística: dirs con profundidad ≥ 2 (al menos un `/`
/// después del primero) — para que `ls -R` en un árbol grande no rinda
/// miles de filas al toque. El usuario togglea con click; el set
/// `section_collapsed` guarda el OVERRIDE del default (no el estado).
pub(crate) fn section_default_collapsed(title: &str) -> bool {
    // `./` y `.` siempre expandidos. `./algo` también (depth 1). `./a/b`
    // ya cierra (depth 2).
    let stripped = title.trim_start_matches("./");
    stripped.matches('/').count() >= 1
}

/// Estado efectivo de plegado de una sub-sección: el default (heurística
/// por profundidad) flippeado por el override del usuario.
pub(crate) fn is_section_collapsed(state: &State, block: u64, idx: usize, title: &str) -> bool {
    let default_col = section_default_collapsed(title);
    let user_toggled = state.section_collapsed.contains(&(block, idx));
    default_col ^ user_toggled
}

/// Header clickeable de una sub-sección. Pinta chevron + título + el conteo
/// de líneas; click emite `Msg::ToggleSection`. `idx` se usa como número
/// visible ("1.", "2.", …) para navegar listas largas.
fn section_header<HostMsg: Clone + 'static>(
    block: u64,
    idx: usize,
    title: &str,
    line_count: usize,
    collapsed: bool,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> View<HostMsg> {
    let chevron = if collapsed {
        llimphi_icons::Icon::ChevronRight
    } else {
        llimphi_icons::Icon::ChevronDown
    };
    let marker = View::new(Style {
        size: Size { width: length(12.0_f32), height: length(12.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![llimphi_icons::icon_view(chevron, theme.fg_muted, 1.6)]);
    let title_v = View::new(Style {
        size: Size { width: Dimension::auto(), height: length(14.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(
        format!("{}. {}", idx + 1, title),
        11.0,
        theme.fg_text,
        Alignment::Start,
    )
    .mono()
    .max_lines(1);
    let count = View::new(Style {
        size: Size { width: length(60.0_f32), height: length(14.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(
        format!("{line_count} líneas"),
        10.0,
        theme.fg_muted,
        Alignment::End,
    )
    .mono();
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(SECTION_HEADER_H) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        padding: Rect {
            left: length(18.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .hover_fill(theme.bg_row_hover)
    .on_click(lift(Msg::ToggleSection { block, idx }))
    .children(vec![marker, title_v, count])
}

/// Header de un comando como **chrome** de la superficie: chevron + `$ comando`
/// + badge de estado (icono + "hace N"). Click → pliega/despliega el bloque.
/// Chrome header del bloque de líneas spilleadas: rotula "Archivado de
/// spill (N visibles · M total)" y avisa al usuario que el resto se ve
/// con `:scrollback open`. Sin click handler (informativo).
fn spilled_archive_header<HostMsg: Clone + 'static>(
    loaded: usize,
    above: u64,
    theme: &Theme,
) -> View<HostMsg> {
    let label = if above > 0 {
        format!(
            "≡ Archivado ({loaded} cargadas · ▲ {above} más arriba — scrolleá al tope · `:scrollback open` para todo)"
        )
    } else {
        format!("≡ Archivado · inicio del historial ({loaded} líneas)")
    };
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(SURFACE_HEADER_H) },
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(label, 11.0, theme.fg_muted, Alignment::Start)
    .mono()
}

/// `has_stdout` (param 6) gatea el chip de reprocess (sin stdout, no hay
/// nada que reprocesar).
/// Alto del notice «¿quisiste decir…?» (A4).
const DID_YOU_MEAN_H: f32 = 18.0;

/// A4 — fila clickeable bajo un bloque fallido: *«¿`cargo build` en vez de
/// `cagro build`? → click lo lleva al input»*. No ejecuta nada solo; deja la
/// línea corregida lista para revisar y Enter.
fn did_you_mean_notice<HostMsg: Clone + 'static>(
    block: u64,
    corregida: &str,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> View<HostMsg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(DID_YOU_MEAN_H) },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_input)
    .radius(3.0)
    .hover_fill(theme.bg_row_hover)
    .on_click(lift(Msg::AcceptDidYouMean(block)))
    .text_aligned(
        format!("¿quisiste decir «{corregida}»?  ·  click lo lleva al input"),
        10.0,
        theme.accent,
        Alignment::Start,
    )
    .mono()
    .max_lines(1)
}

#[allow(clippy::too_many_arguments)]
fn surface_header<HostMsg: Clone + 'static>(
    block: u64,
    header_text: &str,
    status: Option<CmdStatus>,
    expandable: bool,
    collapsed: bool,
    has_stdout: bool,
    titular: Option<&str>,
    state: &State,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> View<HostMsg> {
    let chevron = if collapsed {
        llimphi_icons::Icon::ChevronRight
    } else {
        llimphi_icons::Icon::ChevronDown
    };
    let marker = View::new(Style {
        size: Size { width: length(14.0_f32), height: length(14.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(if expandable {
        vec![llimphi_icons::icon_view(chevron, theme.fg_muted, 1.6)]
    } else {
        Vec::new()
    });

    let cmd_color = if expandable || status == Some(CmdStatus::Running) {
        theme.accent
    } else {
        theme.fg_muted
    };
    let cmd = View::new(Style {
        size: Size { width: Dimension::auto(), height: length(16.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(header_text.to_string(), 12.0, cmd_color, Alignment::Start)
    .mono()
    .max_lines(1);

    let running = status == Some(CmdStatus::Running);
    let is_input_focus = state.input_focus == Some(block);
    // E2 — tag `%cN` clickeable: hace visible el número del bloque (para
    // referenciarlo en `%cN | grep …`) y, al click, inserta la ref en el
    // input. Sólo en bloques con stdout (los que son fuente de datos útil).
    let mut children = if has_stdout {
        let ref_tag = View::new(Style {
            size: Size { width: Dimension::auto(), height: length(14.0_f32) },
            flex_shrink: 0.0,
            padding: Rect {
                left: length(3.0_f32),
                right: length(3.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .radius(3.0)
        .hover_fill(theme.bg_row_hover)
        .on_click(lift(Msg::InsertBlockRef(block)))
        .text_aligned(format!("%c{block}"), 9.0, theme.fg_muted, Alignment::Start)
        .mono();
        vec![marker, ref_tag, cmd]
    } else {
        vec![marker, cmd]
    };
    // Titular semáforo (A5): cuando el bloque está colapsado, el header gana
    // el resumen contado del cuerpo (errores/avisos/líneas/duración). El nerdo
    // habitual escanea la columna de headers como un log semáforo sin
    // desplegar nada. Color = dosis de alarma: rojo si hubo errores, ámbar si
    // sólo avisos, tenue si limpio.
    if let Some(t) = titular {
        let color = if titular_tiene_error(t) {
            theme.fg_destructive
        } else if titular_tiene_aviso(t) {
            llimphi_ui::llimphi_raster::peniko::Color::from_rgba8(220, 190, 120, 255)
        } else {
            theme.fg_muted
        };
        children.push(
            View::new(Style {
                size: Size { width: Dimension::auto(), height: length(16.0_f32) },
                // Crece con base 0 (como el comando): se lleva el espacio
                // sobrante y el texto, alineado a la derecha, no se mide
                // contra un ancho apretado (que lo recortaba/envolvía).
                flex_grow: 1.0,
                flex_basis: length(0.0_f32),
                ..Default::default()
            })
            .text_aligned(t.to_string(), 10.0, color, Alignment::End)
            .mono()
            .max_lines(1),
        );
    }
    // Chip de foco de input: sólo en comandos vivos. Marca/dirige a quién le
    // va el Enter de la línea (stdin). Click lo fija; el header entero también
    // foca al pasar el mouse (`on_pointer_enter`, abajo). Cuando ESTE es el
    // destino, se pinta encendido (acento) para que se vea de un vistazo a
    // cuál de los comandos en paralelo está escuchando la línea.
    if running {
        let (fill, fg, label) = if is_input_focus {
            (theme.accent, theme.bg_panel, "⌨ recibe input")
        } else {
            (theme.bg_input, theme.fg_muted, "⌨ dar input")
        };
        children.push(
            View::new(Style {
                size: Size { width: Dimension::auto(), height: length(16.0_f32) },
                flex_shrink: 0.0,
                padding: Rect {
                    left: length(5.0_f32),
                    right: length(5.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                ..Default::default()
            })
            .fill(fill)
            .radius(3.0)
            .hover_fill(theme.bg_row_hover)
            .on_click(lift(Msg::FocusJob(block)))
            .text_aligned(label.to_string(), 10.0, fg, Alignment::Start)
            .mono(),
        );
    }
    // Chip de reprocess: alimenta el stdout de este bloque al stdin del
    // próximo comando (paridad con el `command_card` del path viejo). Clic
    // arma/desarma; el hit-test innermost-wins le da prioridad sobre el
    // header (que pliega el bloque). Colapsado = modo escaneo: el titular
    // semáforo reemplaza los chips de acción para no saturar la fila.
    if has_stdout && !collapsed {
        let armed = state.reprocess_source == Some(block);
        let (fill, fg) = if armed {
            (theme.accent, theme.bg_panel)
        } else {
            (theme.bg_input, theme.fg_muted)
        };
        children.push(
            View::new(Style {
                size: Size { width: Dimension::auto(), height: length(16.0_f32) },
                flex_shrink: 0.0,
                padding: Rect {
                    left: length(5.0_f32),
                    right: length(5.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                ..Default::default()
            })
            .fill(fill)
            .radius(3.0)
            .hover_fill(theme.bg_row_hover)
            .on_click(lift(Msg::SetReprocess(block)))
            .text_aligned("» stdin".to_string(), 10.0, fg, Alignment::Start)
            .mono(),
        );
    }
    // Chip "🜲 filtrar": filtro IA sobre la salida del bloque. Prellena el input
    // `:filtra %cN ` y deja el cursor para la instrucción (no auto-ejecuta).
    // Sólo en bloques con cuerpo; oculto al colapsar.
    if expandable && !collapsed {
        children.push(
            View::new(Style {
                size: Size { width: Dimension::auto(), height: length(16.0_f32) },
                flex_shrink: 0.0,
                padding: Rect {
                    left: length(5.0_f32),
                    right: length(5.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                ..Default::default()
            })
            .fill(theme.bg_input)
            .radius(3.0)
            .hover_fill(theme.bg_row_hover)
            .on_click(lift(Msg::PrefillInput(format!(":filtra %c{block} "))))
            .text_aligned("🜲 filtrar".to_string(), 10.0, theme.accent, Alignment::Start)
            .mono(),
        );
    }
    // Chip "copiar": copia el bloque entero (comando + stdout + stderr) al
    // clipboard, sin depender de una selección — paridad con el "copy command
    // + output" de las terminales modernas. Sólo en bloques con cuerpo. Click
    // propio (innermost-wins) para no plegar el bloque. Oculto al colapsar
    // (modo escaneo: manda el titular semáforo).
    if expandable && !collapsed {
        children.push(
            View::new(Style {
                size: Size { width: Dimension::auto(), height: length(16.0_f32) },
                flex_shrink: 0.0,
                padding: Rect {
                    left: length(5.0_f32),
                    right: length(5.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                ..Default::default()
            })
            .fill(theme.bg_input)
            .radius(3.0)
            .hover_fill(theme.bg_row_hover)
            .on_click(lift(Msg::CopyCommandBlock(block)))
            .text_aligned("copiar".to_string(), 10.0, theme.fg_muted, Alignment::Start)
            .mono(),
        );
    }
    // Chip "⇄ comparar": cotejo de un clic. Marca este bloque como ancla; con
    // otro ya marcado, dispara `:compara %cA %cB` entre ambos. Resalta (acento)
    // cuando ESTE es el bloque marcado; muestra contra cuál cotejará si el
    // ancla es otro. Sólo en bloques con cuerpo; oculto al colapsar.
    if expandable && !collapsed {
        let (label, color, fill) = match state.compare_anchor {
            Some(a) if a == block => ("⇄ elegido".to_string(), theme.bg_panel, theme.accent),
            Some(a) => (format!("⇄ vs %c{a}"), theme.accent, theme.bg_input),
            None => ("⇄ comparar".to_string(), theme.fg_muted, theme.bg_input),
        };
        children.push(
            View::new(Style {
                size: Size { width: Dimension::auto(), height: length(16.0_f32) },
                flex_shrink: 0.0,
                padding: Rect {
                    left: length(5.0_f32),
                    right: length(5.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                ..Default::default()
            })
            .fill(fill)
            .radius(3.0)
            .hover_fill(theme.bg_row_hover)
            .on_click(lift(Msg::CompareWith(block)))
            .text_aligned(label, 10.0, color, Alignment::Start)
            .mono(),
        );
    }
    if let Some(st) = status {
        let (icon, color) = st.icon_color(theme);
        children.push(
            View::new(Style {
                size: Size { width: length(12.0_f32), height: length(12.0_f32) },
                flex_shrink: 0.0,
                ..Default::default()
            })
            .children(vec![llimphi_icons::icon_view(icon, color, 1.8)]),
        );
        // Mientras corre, mostrar bytes recibidos en vivo en el slot del
        // timestamp — feedback inmediato de que el stream está moviendo
        // datos (más útil que "hace 0 s"). Al terminar, vuelve al "hace…".
        let right_text = if st == CmdStatus::Running && state.current_block == block {
            format_bytes_short(state.current_run_bytes)
        } else {
            relative_time(
                state.block_started.get(&block).copied().unwrap_or(0),
                now_unix_secs(),
            )
        };
        children.push(
            View::new(Style {
                size: Size { width: length(96.0_f32), height: length(16.0_f32) },
                flex_shrink: 0.0,
                ..Default::default()
            })
            .text_aligned(right_text, 10.0, theme.fg_muted, Alignment::End)
            .mono(),
        );
    }

    // El header del comando vivo que recibe el input se tiñe (bg_input_focus)
    // para distinguirlo de los otros en paralelo.
    let header_fill = if running && is_input_focus {
        theme.bg_input_focus
    } else {
        theme.bg_panel
    };
    let mut v = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(SURFACE_HEADER_H) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        padding: Rect {
            left: length(6.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(header_fill);
    if expandable {
        v = v.on_click(lift(Msg::ToggleBlock(block)));
    }
    // Mientras corre, pasar el mouse por encima dirige el input a este comando
    // (el "mousemove" del pedido). Es no destructivo: re-focar la línea (mouse
    // sobre el input) o hover en otro job vivo cambia el destino al instante.
    // El `hover_fill` no es sólo cosmético: el hit-test de hover de Llimphi sólo
    // elige nodos con `hover_fill`, así que es lo que hace que el
    // `on_pointer_enter` dispare al pasar el mouse por el header.
    if running {
        v = v
            .hover_fill(theme.bg_input_focus)
            .on_pointer_enter(lift(Msg::FocusJob(block)));
    }
    v.children(children)
}

/// `true` salvo opt-out explícito (`SHUMA_FONDO_QUIETO=1`): el fondo del
/// output respira con una deriva lenta del accent. Leído una vez por proceso.
fn fondo_vivo_enabled() -> bool {
    use std::sync::OnceLock;
    static EN: OnceLock<bool> = OnceLock::new();
    *EN.get_or_init(|| std::env::var_os("SHUMA_FONDO_QUIETO").is_none())
}

/// Pinta el **fondo vivo** sobre el panel hundido: dos lóbulos radiales del
/// accent con alpha bajísimo (≤ 4%) cuyo centro deriva en una curva de
/// Lissajous con períodos primos entre sí (~37 s y ~53 s) — nunca repite
/// exactamente, nunca distrae. El texto va por encima con contraste intacto.
fn paint_fondo_vivo(
    scene: &mut vello::Scene,
    rect: llimphi_ui::PaintRect,
    accent: llimphi_ui::llimphi_raster::peniko::Color,
) {
    use llimphi_ui::llimphi_raster::kurbo::{Affine, Rect as KurboRect};
    use llimphi_ui::llimphi_raster::peniko::{Color, Fill, Gradient};
    use vello::kurbo::Point;

    let t = now_unix_millis() as f64 / 1000.0;
    let a = accent.to_rgba8();
    let bounds = KurboRect::new(
        rect.x as f64,
        rect.y as f64,
        (rect.x + rect.w) as f64,
        (rect.y + rect.h) as f64,
    );
    // Cada lóbulo: (período x, período y, fase, alpha pico, radio relativo).
    let lobulos: [(f64, f64, f64, u8, f64); 2] = [
        (37.0, 53.0, 0.0, 10, 0.85),
        (53.0, 41.0, 2.4, 7, 0.65),
    ];
    for (px, py, fase, alpha, rr) in lobulos {
        let cx = rect.x as f64
            + rect.w as f64 * (0.5 + 0.38 * (t * std::f64::consts::TAU / px + fase).sin());
        let cy = rect.y as f64
            + rect.h as f64 * (0.5 + 0.38 * (t * std::f64::consts::TAU / py + fase * 0.7).cos());
        let radio = (rect.w.max(rect.h) as f64) * rr;
        let grad = Gradient::new_radial(Point::new(cx, cy), radio as f32).with_stops(
            [
                Color::from_rgba8(a.r, a.g, a.b, alpha),
                Color::from_rgba8(a.r, a.g, a.b, 0),
            ]
            .as_slice(),
        );
        scene.fill(Fill::NonZero, Affine::IDENTITY, &grad, None, &bounds);
    }
}

/// `output_pane` reimplementado sobre `llimphi-widget-terminal::block_surface`.
/// Mismo modelo de datos, virtualización real (sólo se materializa lo visible).
pub(crate) fn output_pane_surface<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> View<HostMsg> {
    use llimphi_widget_terminal::{
        blocks_height, Item, LineStyle, Scrollback, TermMetrics, TermPalette,
    };

    // Agrupar por bloque preservando el orden de primera aparición (igual que
    // el camino viejo). Con superficie no hace falta capar a 400: el widget
    // virtualiza, así que pasamos todo el buffer vigente.
    let mut order: Vec<u64> = Vec::new();
    let mut groups: std::collections::HashMap<u64, Vec<&OutputLine>> =
        std::collections::HashMap::new();
    for line in &state.output {
        if !groups.contains_key(&line.block) {
            order.push(line.block);
        }
        groups.entry(line.block).or_default().push(line);
    }

    // Zoom multiplica font_size, row_h y char_width. Lo controla
    // Ctrl+rueda y Ctrl+- / Ctrl+= / Ctrl+0. Clampeo [0.5, 3.0].
    let zoom = state.font_zoom.clamp(0.5, 3.0);
    let row_h = ROW_H * zoom;
    let metrics = TermMetrics {
        font_size: 12.0 * zoom,
        line_height: row_h,
        char_width: 12.0 * 0.6 * zoom,
    };
    use llimphi_ui::llimphi_raster::peniko::Color;
    let mut palette = TermPalette::from_theme(theme);
    // La superficie entera es el panel hundido; los cuerpos se leen sobre él.
    // El bg del widget va TRANSPARENTE: el nodo exterior pinta el hundido +
    // el fondo vivo (deriva lenta del accent) y se ve a través del widget.
    palette.bg = Color::from_rgba8(0, 0, 0, 0);

    // Refresh del cache de spilled visibles (Fase 5.11): lee desde el spill
    // file sólo si `spilled_count` cambió desde el último frame. El cache
    // es up-to-`MAX_SPILLED_VISIBLE` líneas; el view las prepende al store.
    crate::refresh_surf_spilled_visible(&state.surf_history, &state.surf_spilled_visible);
    let (spilled_cache_lines, spilled_first_id): (Vec<String>, u64) = state
        .surf_spilled_visible
        .lock()
        .map(|c| (c.lines.clone(), c.first_id))
        .unwrap_or_default();

    // Store de scrollback + items + estilo por línea (alineado al índice del
    // store, que crece en lockstep con `push_line`).
    let mut store = Scrollback::new(0);
    let mut items: Vec<Item<HostMsg>> = Vec::new();
    let mut styles: Vec<(bool, Vec<(usize, usize, llimphi_ui::llimphi_raster::peniko::Color)>)> =
        Vec::new();

    // Prepend de las líneas spilleadas: arrancan en `store[0..]`. Tinte
    // discreto (fg_muted) para marcarlas visualmente como archive y un chrome
    // header antes. `first_id` = cuántas líneas quedan AÚN más arriba de la
    // ventana cargada (Fase 5.12): scrollear al tope las pagina hacia atrás;
    // más allá del tope de carga, `:scrollback open`.
    if !spilled_cache_lines.is_empty() {
        items.push(Item::chrome(
            SURFACE_HEADER_H,
            spilled_archive_header::<HostMsg>(
                spilled_cache_lines.len(),
                spilled_first_id,
                theme,
            ),
        ));
        let start = store.len();
        for text in &spilled_cache_lines {
            // Las spilled van en `fg_muted` para diferenciarlas del live.
            let muted = theme.fg_muted;
            styles.push((false, vec![(0usize, text.len(), muted)]));
            store.push_line(text);
        }
        items.push(Item::lines(start, store.len()));
    }

    for id in &order {
        let g = &groups[id];
        if *id != 0 {
            // Bloque-comando: header (chrome) + cuerpo (si no está colapsado).
            let collapsed = state.collapsed.contains(id);
            let has_prompt = g
                .first()
                .map(|l| l.kind == OutputKind::Prompt)
                .unwrap_or(false);
            let header_text = if has_prompt {
                g[0].text.clone()
            } else {
                state
                    .block_command
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| "$ … (salida recortada)".to_string())
            };
            // Estado: última notice de cierre del bloque, o "corriendo".
            let mut status = g
                .iter()
                .filter(|l| l.stage.is_none())
                .filter_map(|l| CmdStatus::from_notice(&l.text))
                .last();
            let still_running = status.is_none()
                && ((state.current_block == *id && state.is_running())
                    || state.bg_jobs.iter().any(|j| {
                        j.lock()
                            .map(|gg| gg.block == *id && !gg.handle.is_finished())
                            .unwrap_or(false)
                    }));
            if still_running {
                status = Some(CmdStatus::Running);
            }

            let lines = body_lines_for_block(state, *id);
            let kinds = body_kinds_for_block(state, *id);
            let runs = body_color_runs(state, *id, theme);
            // Bloques que NO se desplanizan en secciones/tablas: la respuesta de
            // IA (prosa/transformación; el desplanizado comería el tinte de
            // acento) y el cotejo de `:compara` (ya viene en columnas alineadas;
            // el detector de tablas lo rompería). Se pintan planos.
            let is_ai_block = kinds.iter().any(|k| *k == OutputKind::Ai);
            let is_compare_block = state
                .block_command
                .get(id)
                .map(|c| c.starts_with("≡ :compara"))
                .unwrap_or(false);
            let skip_sections = is_ai_block || is_compare_block;
            let has_stages = g.iter().any(|l| l.stage.is_some());
            let has_stdout = g
                .iter()
                .any(|l| l.kind == OutputKind::Stdout && l.stage.is_none());
            let expandable = !lines.is_empty() || has_stages;

            // Titular semáforo sólo cuando está colapsado y hay cuerpo: el
            // header resume lo que el usuario no está viendo.
            let titular = if collapsed && !lines.is_empty() {
                let dur = state
                    .block_ended
                    .get(id)
                    .zip(state.block_started.get(id))
                    .map(|(end, s)| end.saturating_sub(*s));
                Some(semaforo_titular(&lines, &state.cwd, dur))
            } else {
                None
            };

            items.push(Item::chrome(
                SURFACE_HEADER_H,
                surface_header(
                    *id,
                    &header_text,
                    status,
                    expandable,
                    collapsed,
                    has_stdout,
                    titular.as_deref(),
                    state,
                    theme,
                    lift,
                ),
            ));

            // Chrome de etapas (tee): chips clickeables + capturas desplegadas
            // por etapa. Paridad con el `command_card` viejo. Vacío si el
            // bloque no tiene etapas o si está colapsado. Reusa el helper del
            // path viejo (`stage_capture_rows`) y lo envuelve como un chrome
            // de alto medido por el helper, opaco para la virtualización.
            if !collapsed && has_stages {
                let stage_lines: Vec<&OutputLine> =
                    g.iter().filter(|l| l.stage.is_some()).copied().collect();
                let (views, h) =
                    stage_capture_rows(&header_text, &stage_lines, *id, state, theme, lift);
                if !views.is_empty() && h > 0.0 {
                    let chrome_view = View::new(Style {
                        flex_direction: FlexDirection::Column,
                        size: Size { width: percent(1.0_f32), height: length(h) },
                        ..Default::default()
                    })
                    .children(views);
                    items.push(Item::chrome(h, chrome_view));
                }
            }

            if !collapsed && !lines.is_empty() {
                // Detector de sub-secciones por comando: si reconoce el
                // patrón (p. ej. `ls -R`), parte el output en grupos con
                // su propio header colapsable.
                let cmd_for_sections = state
                    .block_command
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| header_text.clone());
                if let Some(sections) = (!skip_sections)
                    .then(|| crate::sections::detect_sections(&cmd_for_sections, &lines))
                    .flatten()
                {
                    for (sidx, sec) in sections.iter().enumerate() {
                        let sec_col = is_section_collapsed(state, *id, sidx, &sec.title);
                        let item_count = sec.kind.count();
                        // Header (oculto si la sección no tiene título — caso
                        // `ls -l` simple sin árbol).
                        if !sec.title.is_empty() {
                            items.push(Item::chrome(
                                SECTION_HEADER_H,
                                section_header(
                                    *id, sidx, &sec.title, item_count, sec_col, theme, lift,
                                ),
                            ));
                        }
                        if !sec_col {
                            match &sec.kind {
                                crate::sections::SectionKind::Lines(secl) => {
                                    let start = store.len();
                                    for line in secl {
                                        styles.push((false, Vec::new()));
                                        store.push_line(line);
                                    }
                                    items.push(Item::lines(start, store.len()));
                                }
                                crate::sections::SectionKind::Table { columns, rows } => {
                                    let sort = state.section_sort.get(&(*id, sidx)).copied();
                                    let h = section_table_height(rows.len());
                                    items.push(Item::chrome(
                                        h,
                                        section_table_view(
                                            *id, sidx, columns, rows, sort, theme, lift,
                                        ),
                                    ));
                                }
                            }
                        }
                    }
                } else {
                    let start = store.len();
                    for (i, line) in lines.iter().enumerate() {
                        let is_err = matches!(kinds.get(i), Some(OutputKind::Stderr));
                        styles.push((is_err, runs.get(i).cloned().unwrap_or_default()));
                        store.push_line(line);
                    }
                    items.push(Item::lines(start, store.len()));
                }
            }

            // Imágenes (kitty/sixel) horneadas del PTY (chafa/icat/img2sixel…):
            // una por chrome bajo el cuerpo. Persisten en el scrollback tras
            // cerrar el comando, como cualquier otra salida.
            if !collapsed {
                if let Some(imgs) = state.block_images.get(id) {
                    for img in imgs {
                        let (w, h) = baked_image_size(img, metrics);
                        items.push(Item::chrome(h, baked_image_view::<HostMsg>(img, w, h)));
                    }
                }
            }

            // A4 — notice «¿quisiste decir…?»: si el bloque falló por
            // `command not found` y hay una corrección, una fila clickeable que
            // lleva la línea corregida al input. Aparece esté o no colapsado.
            if let Some(corregida) = state.did_you_mean.get(id) {
                items.push(Item::chrome(
                    DID_YOU_MEAN_H,
                    did_you_mean_notice(*id, corregida, theme, lift),
                ));
            }
        } else {
            // Líneas sueltas (notices iniciales sin bloque dueño) — cuerpo sin
            // header, coloreadas por su decoración semántica.
            let start = store.len();
            for &line in g.iter() {
                let is_err = line.kind == OutputKind::Stderr;
                let line_runs: Vec<_> = if is_err {
                    vec![(0usize, line.text.len(), theme.fg_destructive)]
                } else {
                    shuma_line::decorate_line(&line.text, &state.cwd)
                        .into_iter()
                        .filter(|d| d.start < d.end && d.end <= line.text.len())
                        .map(|d| (d.start, d.end, decoration_color(&d.kind, theme)))
                        .collect()
                };
                styles.push((is_err, line_runs));
                store.push_line(&line.text);
            }
            if store.len() > start {
                items.push(Item::lines(start, store.len()));
            }
        }
    }

    // Scroll: convertir el modelo del shell (`scroll_px` desde el fondo) al del
    // widget (`scroll_y` desde arriba). El viewport lo midió el painter el frame
    // anterior; publicamos el overflow para que `Msg::Scroll` clampe.
    let measured = state.out_viewport_h.lock().map(|g| *g).unwrap_or(0.0);
    let content_h = blocks_height(&items, row_h);
    let viewport_h = if measured >= 1.0 { measured } else { 600.0 };
    let overflow = (content_h - viewport_h).max(0.0);
    if let Ok(mut g) = state.out_overflow.lock() {
        *g = overflow;
    }
    // Anclaje estable bajo append (Fase 5 del SDD-TERMINAL): si el usuario
    // está scrolled-up (`scroll_px > 0`), su `scroll_y` se interpreta
    // contra el `surf_scroll_anchor` (el overflow al momento de su última
    // entrada de scroll), NO contra el `overflow` vigente. Append → el
    // overflow crece, pero la fila que el usuario tenía a la vista
    // permanece en la misma `y` del viewport.
    let scroll_y = if state.scroll_px <= 0.5 {
        overflow // pinned al fondo
    } else {
        (state.surf_scroll_anchor - state.scroll_px).clamp(0.0, overflow)
    };

    // Estilo por línea: stderr → tinte rojo tenue; runs ya traen el coloreo
    // semántico (paths/urls/stderr-rojo) calculado arriba.
    let err_bg = {
        let c = theme.fg_destructive.to_rgba8();
        llimphi_ui::llimphi_raster::peniko::Color::from_rgba8(c.r, c.g, c.b, 36)
    };
    let line_style = move |idx: usize, _text: &str| match styles.get(idx) {
        Some((is_err, runs)) => LineStyle {
            fg: None,
            runs: runs.clone(),
            bg: if *is_err { Some(err_bg) } else { None },
        },
        None => LineStyle::default(),
    };

    // La rueda y el arrastre de la barra del widget llegan acá como delta a
    // sumar a `scroll_y` (desde arriba); el shell lo guarda como `scroll_px`
    // (desde el fondo), así que invertimos el signo.
    let lift_scroll = (*lift).clone();
    let on_scroll = move |delta: f32| lift_scroll(Msg::Scroll(-delta));

    use llimphi_widget_terminal::{
        block_surface_with_scroll, gutter_width, SelectionConfig,
    };

    // Snapshot del layout para que el `update` resuelva clicks contra la
    // geometría real del frame anterior, sin re-armar los items.
    let items_geo: Vec<llimphi_widget_terminal::ItemGeo> =
        items.iter().map(|it| it.geo()).collect();
    let gw = gutter_width(&store, metrics);
    if let Ok(mut g) = state.surf_layout.lock() {
        *g = Some(crate::SurfLayout {
            items_geo,
            scroll_y,
            viewport_h,
            metrics,
            gutter_w: gw,
            store: std::sync::Arc::new(store.clone()),
        });
    }

    // Handler de drag de selección: forwardea cada `(phase, lx0, ly0, dx, dy)`
    // del viewport al `update` como `Msg::SurfSelectDrag`. El `update` mantiene
    // el acumulador y resuelve la posición a `Point` con `point_at_geo`.
    let lift_drag = (*lift).clone();
    let on_drag = std::sync::Arc::new(
        move |phase, lx0, ly0, dx, dy| -> Option<HostMsg> {
            Some(lift_drag(Msg::SurfSelectDrag {
                phase,
                dx,
                dy,
                ax: lx0,
                ay: ly0,
            }))
        },
    );
    // Doble-click → select-word, paridad con terminales clásicas.
    let lift_dbl = (*lift).clone();
    let on_double_click = std::sync::Arc::new(
        move |lx, ly, rect_w, rect_h| -> Option<HostMsg> {
            Some(lift_dbl(Msg::SurfDoubleClick {
                lx,
                ly,
                rect_w,
                rect_h,
            }))
        },
    );
    let sel_cfg = SelectionConfig {
        range: state.surf_selection.as_ref(),
        on_drag: Some(on_drag),
        on_double_click: Some(on_double_click),
    };

    let surface = block_surface_with_scroll::<HostMsg, _, _>(
        &store,
        items,
        scroll_y,
        state.surf_scroll_x.max(0.0),
        viewport_h,
        metrics,
        &palette,
        line_style,
        on_scroll,
        None,
        sel_cfg,
    );

    // Nodo flex que toma el espacio sobrante (entre header e input) y mide su
    // alto real para el próximo frame (el widget recibe un alto fijo = el
    // medido; el painter de medición vive acá, en el nodo flex-rellenado).
    // El mismo painter pinta el **fondo vivo**: dos lóbulos radiales del
    // accent a alpha bajísimo que derivan en Lissajous lento (~40 s de
    // período). El chasis ya redibuja cada ~100 ms por el caret, así que el
    // movimiento sale gratis. Opt-out: `SHUMA_FONDO_QUIETO=1`.
    let slot = Arc::clone(&state.out_viewport_h);
    let glow_accent = theme.accent;
    let painter = move |scene: &mut vello::Scene,
                        _ts: &mut llimphi_ui::llimphi_text::Typesetter,
                        rect: llimphi_ui::PaintRect| {
        if let Ok(mut g) = slot.lock() {
            *g = rect.h;
        }
        if fondo_vivo_enabled() && rect.w > 1.0 && rect.h > 1.0 {
            paint_fondo_vivo(scene, rect, glow_accent);
        }
    };
    let lift_menu = (*lift).clone();
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_basis: length(0.0_f32),
        flex_grow: 1.0,
        min_size: Size {
            width: Dimension::auto(),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.sunken())
    .radius(3.0)
    .clip(true)
    .paint_with(painter)
    // Right-click sobre el contenedor de la surface abre el menú contextual.
    // El hit-test innermost-wins le da prioridad a hijos con sus propios
    // handlers (p. ej. la barra de find).
    .on_right_click_at(move |x, y, _w, _h| Some(lift_menu(Msg::SurfOpenMenu { x, y })))
    .children({
        // Barra de find encima de la superficie, sólo si está abierta. Es
        // focus-grabbing (la dispatch ya rutea las teclas a `handle_find_key`).
        let mut kids: Vec<View<HostMsg>> = Vec::new();
        if let Some(f) = &state.find {
            kids.push(find_bar_view::<HostMsg>(f, theme, lift));
        }
        // Status del spill: chip que muestra "N líneas archivadas" cuando
        // el history persistente ya recortó al disco. Sólo visible si
        // spill está activo y hay contenido archivado.
        if let Some(status) = spill_status_view::<HostMsg>(state, theme) {
            kids.push(status);
        }
        // Cursor I-beam sobre el cuerpo: señala que el texto es seleccionable
        // (drag selecciona, doble-click la palabra, click derecho el menú). Las
        // decoraciones clickeables (paths/URLs) que traen su propio cursor ganan
        // por hit-test innermost-wins.
        kids.push(surface.cursor(llimphi_ui::Cursor::Text));
        // El menú contextual va como overlay arriba de todo.
        if let Some(menu) = surf_context_menu(state, theme, lift) {
            kids.push(menu);
        }
        kids
    })
}

/// Chip de status del spill del scrollback: "≡ N líneas archivadas en
/// <path>". Sólo aparece si `state.surf_history.spilled_count() > 0`
/// (es decir, el archivo de spill tiene contenido — la sesión llenó el
/// cap en memoria y siguió volcando a disco). `None` mientras esté vacío.
fn spill_status_view<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
) -> Option<View<HostMsg>> {
    let count = state.surf_history.lock().ok().map(|h| h.spilled_count()).unwrap_or(0);
    if count == 0 {
        return None;
    }
    Some(
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(2.0_f32),
                bottom: length(2.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_panel)
        .text_aligned(
            format!("≡ {count} líneas archivadas en spill"),
            10.0,
            theme.fg_muted,
            Alignment::Start,
        )
        .mono(),
    )
}

/// Menú contextual del surface (click derecho): Copiar selección · Copiar
/// todo · Seleccionar todo. `None` si no está abierto.
pub(crate) fn surf_context_menu<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> Option<View<HostMsg>> {
    use llimphi_widget_context_menu::{
        context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
    };
    let (x, y) = state.surf_menu?;
    let mut copiar = ContextMenuItem::action("Copiar").with_shortcut("Ctrl+C");
    if state.surf_selection.as_ref().map_or(true, |s| s.is_empty()) {
        copiar = copiar.disabled();
    }
    let items = vec![
        copiar,
        ContextMenuItem::action("Copiar todo"),
        ContextMenuItem::action("Seleccionar todo"),
    ];
    let lift_pick = lift.clone();
    let menu = context_menu_view(ContextMenuSpec {
        anchor: (x, y),
        viewport: (1280.0, 800.0),
        header: None,
        items,
        active: usize::MAX,
        on_pick: std::sync::Arc::new(move |i| lift_pick(Msg::SurfMenuPick(i))),
        on_dismiss: lift(Msg::SurfMenuDismiss),
        palette: ContextMenuPalette::from_theme(theme),
    });
    Some(
        View::new(Style {
            position: Position::Absolute,
            inset: Rect {
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
        .children(vec![menu]),
    )
}

/// Barra de búsqueda Ctrl+F: lupa + query (cursor) + contador `M/N` + chip
/// `Aa` (toggle case) + flechas + ✕. Compacta, encima de la superficie de
/// output. Los clics emiten los `Msg::Find*` ya cableados.
pub(crate) fn find_bar_view<HostMsg: Clone + 'static>(
    f: &crate::FindState,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> View<HostMsg> {
    let lup = View::new(Style {
        size: Size { width: length(14.0_f32), height: length(14.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![llimphi_icons::icon_view(
        llimphi_icons::Icon::Search,
        theme.fg_muted,
        1.6,
    )]);

    // Query con cursor titilante simulado por sufijo "▏" — paridad simple
    // con el cabezal del shell sin meter blink (innecesario en una barra).
    let mut shown = f.query.clone();
    shown.push('▏');
    let query_view = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: Dimension::auto(), height: length(20.0_f32) },
        ..Default::default()
    })
    .text_aligned(shown, 13.0, theme.fg_text, Alignment::Start)
    .mono();

    // Contador `M/N`. Sin matches: "0/0" muted, sin destacar.
    let total = f.matches.len();
    let cur = f.current.map(|i| i + 1).unwrap_or(0);
    let counter_color = if total == 0 && !f.query.is_empty() {
        theme.fg_destructive
    } else {
        theme.fg_muted
    };
    let counter = View::new(Style {
        size: Size { width: length(54.0_f32), height: length(20.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(format!("{cur}/{total}"), 11.0, counter_color, Alignment::End)
    .mono();

    let case_chip = {
        let (fill, fg) = if f.case_insensitive {
            (theme.accent, theme.bg_panel)
        } else {
            (theme.bg_input, theme.fg_muted)
        };
        View::new(Style {
            size: Size { width: length(24.0_f32), height: length(20.0_f32) },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .fill(fill)
        .radius(3.0)
        .hover_fill(theme.bg_row_hover)
        .on_click(lift(Msg::FindToggleCase))
        .text_aligned("Aa".to_string(), 11.0, fg, Alignment::Center)
        .mono()
    };

    let arrow = |icon, msg: Msg| {
        View::new(Style {
            size: Size { width: length(20.0_f32), height: length(20.0_f32) },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .fill(theme.bg_input)
        .radius(3.0)
        .hover_fill(theme.bg_row_hover)
        .on_click(lift(msg))
        .children(vec![llimphi_icons::icon_view(icon, theme.fg_muted, 1.6)])
    };
    let prev_btn = arrow(llimphi_icons::Icon::ChevronUp, Msg::FindPrev);
    let next_btn = arrow(llimphi_icons::Icon::ChevronDown, Msg::FindNext);
    let close_btn = View::new(Style {
        size: Size { width: length(20.0_f32), height: length(20.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_input)
    .radius(3.0)
    .hover_fill(theme.bg_row_hover)
    .on_click(lift(Msg::FindClose))
    .children(vec![llimphi_icons::icon_view(
        llimphi_icons::Icon::X,
        theme.fg_muted,
        1.6,
    )]);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![lup, query_view, counter, case_chip, prev_btn, next_btn, close_btn])
}

#[cfg(test)]
mod baked_image_tests {
    use super::*;

    fn metrics() -> llimphi_widget_terminal::TermMetrics {
        llimphi_widget_terminal::TermMetrics {
            font_size: 12.0,
            line_height: 16.0,
            char_width: 7.2,
        }
    }

    fn img(cols: u16, rows: u16, px_w: u32, px_h: u32) -> crate::types::TermImage {
        crate::types::TermImage {
            image: llimphi_image::from_rgba8(vec![0u8; 4], 1, 1),
            col: 0,
            row: 0,
            cols,
            rows,
            px_w,
            px_h,
        }
    }

    /// Con celdas pedidas (kitty c=/r=), el tamaño es exactamente celdas ×
    /// métrica de celda; el alto incluye el padding inferior.
    #[test]
    fn tamano_por_celdas() {
        let m = metrics();
        let (w, h) = baked_image_size(&img(10, 4, 100, 40), m);
        assert!((w - 10.0 * m.char_width).abs() < 0.01, "ancho {w}");
        assert!((h - (4.0 * m.line_height + IMAGE_PAD)).abs() < 0.01, "alto {h}");
    }

    /// Sin celdas, se encaja por píxeles a un ancho máximo preservando el
    /// aspecto (alto = ancho × aspecto + padding).
    #[test]
    fn tamano_por_pixeles_preserva_aspecto() {
        let m = metrics();
        let (w, h) = baked_image_size(&img(0, 0, 200, 100), m);
        let aspect = 100.0 / 200.0_f32;
        assert!(((h - IMAGE_PAD) - w * aspect).abs() < 0.5, "w={w} h={h}");
    }

    /// Imágenes muy anchas se capan al ancho máximo (72 celdas), no crecen sin
    /// límite.
    #[test]
    fn ancho_capado() {
        let m = metrics();
        let (w, _) = baked_image_size(&img(0, 0, 100_000, 100), m);
        assert!(w <= 72.0 * m.char_width + 0.01, "ancho capado: {w}");
    }
}
