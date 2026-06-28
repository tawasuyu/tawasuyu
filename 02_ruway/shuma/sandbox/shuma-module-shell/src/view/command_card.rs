use super::*;

/// Alto fijo de una línea de output. Lo comparten la fila de etapas
/// (`stage_capture_rows`) y la superficie de terminal (`surface_view`). Vivía
/// en el `output_pane` viejo (borrado en la Fase 5 del SDD-TERMINAL); ahora
/// reside acá, el módulo que aún lo necesita.
pub(crate) const ROW_H: f32 = 16.0; // una línea de output
/// Alto de la fila de chips de etapas de un pipe.
pub(crate) const STAGES_H: f32 = 20.0;
/// Duración del fade de colapso/despliegue de los bloques del output.
pub(crate) const COLLAPSE_ANIM: std::time::Duration = std::time::Duration::from_millis(160);

/// Paleta de etapa — hues desaturados, en la misma familia que la de
/// tokens. Cicla a las 6; un pipe con más etapas reusa colores, sigue
/// siendo legible.
const STAGE_PALETTE: [(u8, u8, u8); 6] = [
    (130, 195, 205), // teal
    (220, 190, 120), // ámbar
    (160, 205, 150), // verde
    (195, 160, 215), // violeta
    (220, 160, 150), // coral
    (150, 180, 225), // azul
];

/// Extrae el comando crudo del texto del header (`$ ls | wc`, o el de un
/// job de fondo `[0] $ sleep 5 &`) — para parsear las etapas del pipe.
pub(crate) fn extract_command(header: &str) -> String {
    let after = header.splitn(2, "$ ").nth(1).unwrap_or(header);
    after.trim().trim_end_matches('&').trim_end().to_string()
}

/// Fila de etapas con **captura en vivo** (tee): cada chip despliega las
/// líneas intermedias ya capturadas de su etapa, sin re-ejecutar. Devuelve
/// `(views, alto)` — la fila de chips más, por cada etapa desplegada, sus
/// líneas. `stage_lines` son las `OutputLine` con `stage = Some(_)` del
/// bloque. La última etapa no se captura (su salida es el cuerpo).
pub(crate) fn stage_capture_rows<HostMsg: Clone + 'static>(
    header_text: &str,
    stage_lines: &[&OutputLine],
    block: u64,
    state: &State,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> (Vec<View<HostMsg>>, f32) {
    let cmd = extract_command(header_text);
    let toks = shuma_line::tokenize(&cmd, state_dialect_default());
    let pipe = shuma_line::split_pipeline(&toks);
    if pipe.stages.len() < 2 {
        return (Vec::new(), 0.0);
    }

    // Chips de etapa.
    let mut row_children: Vec<View<HostMsg>> = vec![View::new(Style {
        size: Size {
            width: length(16.0_f32),
            height: length(16.0_f32),
        },
        ..Default::default()
    })
    .children(vec![llimphi_icons::icon_view(
        llimphi_icons::Icon::ChevronRight,
        theme.fg_muted,
        1.6,
    )])];

    for (i, st) in pipe.stages.iter().enumerate() {
        let captured = stage_lines.iter().filter(|l| l.stage == Some(i)).count();
        let bytes: usize = stage_lines
            .iter()
            .filter(|l| l.stage == Some(i))
            .map(|l| l.text.len())
            .sum();
        let expanded = state.expanded_stages.contains(&(block, i));
        let base = st
            .command
            .clone()
            .unwrap_or_else(|| format!("etapa {}", i + 1));
        // El índice `K` al frente hace obvia la ref `%cN.K` (direccionar la
        // etapa con :filtra/:write/:yank/:explica). Conteo doble (líneas +
        // bytes) sólo cuando hay captura.
        let label = if captured > 0 {
            format!("{i}· {base}  {captured}L {}", humanize_bytes(bytes))
        } else {
            format!("{i}· {base}")
        };
        // La última etapa no tiene captura (su salida es el cuerpo): chip
        // inerte, en color tenue, para que se vea la estructura del pipe.
        let is_last = i + 1 == pipe.stages.len();
        let fill = if expanded {
            theme.bg_row_hover
        } else {
            theme.bg_input
        };
        // Color estable por etapa para las que capturan; la última, tenue.
        let txt_color = if is_last {
            theme.fg_muted
        } else {
            stage_color(i)
        };
        let mut chip = View::new(Style {
            size: Size {
                width: Dimension::auto(),
                height: length(16.0_f32),
            },
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
        .text_aligned(label, 11.0, txt_color, Alignment::Start);
        if !is_last {
            chip = chip
                .hover_fill(theme.bg_row_hover)
                .on_click(lift(Msg::ToggleStage { block, stage: i }));
        }
        row_children.push(chip);
    }

    let chips_row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(STAGES_H),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(5.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(row_children);

    let mut out: Vec<View<HostMsg>> = vec![chips_row];
    let mut height = STAGES_H;

    // Líneas capturadas de cada etapa desplegada, en orden de etapa. Cada
    // etapa va como un bloque `Row[barra-guía coloreada | columna de
    // líneas]`: la barra ata visualmente las líneas a su chip por color.
    for (i, _st) in pipe.stages.iter().enumerate() {
        if !state.expanded_stages.contains(&(block, i)) {
            continue;
        }
        let lines: Vec<&&OutputLine> =
            stage_lines.iter().filter(|l| l.stage == Some(i)).collect();
        let color = stage_color(i);
        let dim = stage_color_dim(i);

        // Columna de líneas (o el placeholder si la etapa aún no emitió).
        let mut col_children: Vec<View<HostMsg>> = Vec::new();
        let block_h = if lines.is_empty() {
            col_children.push(
                row_text(ROW_H)
                    .text_aligned(
                        "(sin líneas capturadas)".to_string(),
                        11.0,
                        theme.fg_muted,
                        Alignment::Start,
                    ),
            );
            ROW_H
        } else {
            for l in &lines {
                col_children.push(
                    row_text(ROW_H)
                        .text_aligned(l.text.clone(), 12.0, dim, Alignment::Start)
                        .mono()
                        // 1 fila: sin esto una línea de etapa larga wrappea y
                        // pisa la de abajo (la fila es de altura fija ROW_H).
                        .max_lines(1),
                );
            }
            lines.len() as f32 * ROW_H
        };

        let col = View::new(Style {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            size: Size {
                width: Dimension::auto(),
                height: length(block_h),
            },
            ..Default::default()
        })
        .children(col_children);

        // Barra-guía: 2px de ancho, estira al alto del bloque (align-items
        // stretch por defecto en el Row), con sangría a izquierda.
        let bar = View::new(Style {
            size: Size {
                width: length(2.0_f32),
                height: percent(1.0_f32),
            },
            margin: Rect {
                left: length(8.0_f32),
                right: length(6.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .fill(color)
        .radius(1.0);

        out.push(
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size {
                    width: percent(1.0_f32),
                    height: length(block_h),
                },
                ..Default::default()
            })
            .children(vec![bar, col])
            // Desplegar/plegar la captura de la etapa con transición. Key en
            // un namespace propio (etapa) para no chocar con cuerpo/resumen.
            .animated_inout(((block << 8) | (i as u64 & 0xff)) ^ (1 << 62), COLLAPSE_ANIM),
        );
        height += block_h;

        // Fila de acciones sobre la etapa: las capturas dejan de ser un
        // registro muerto — se filtran (IA), copian, guardan o explican
        // direccionando `%cN.K`. Filtrar/guardar prellenan el input (el
        // usuario completa instrucción/archivo); copiar/explicar corren ya.
        let mk_action = |txt: String, msg: Msg, color: llimphi_ui::llimphi_raster::peniko::Color| {
            View::new(Style {
                size: Size {
                    width: Dimension::auto(),
                    height: length(18.0_f32),
                },
                padding: Rect {
                    left: length(7.0_f32),
                    right: length(7.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                ..Default::default()
            })
            .fill(theme.bg_input)
            .radius(3.0)
            .hover_fill(theme.bg_row_hover)
            .text_aligned(txt, 11.0, color, Alignment::Start)
            .on_click(lift(msg))
        };
        let actions = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(STAGES_H),
            },
            align_items: Some(AlignItems::Center),
            padding: Rect {
                left: length(16.0_f32),
                right: length(0.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            gap: Size {
                width: length(5.0_f32),
                height: length(0.0_f32),
            },
            ..Default::default()
        })
        .children(vec![
            mk_action(
                "🜲 filtrar".to_string(),
                Msg::PrefillInput(format!(":filtra %c{block}.{i} ")),
                theme.accent,
            ),
            mk_action(
                "copiar".to_string(),
                Msg::RunLine(format!(":yank %c{block}.{i}")),
                theme.fg_muted,
            ),
            mk_action(
                "guardar".to_string(),
                Msg::PrefillInput(format!(":write %c{block}.{i} ")),
                theme.fg_muted,
            ),
            mk_action(
                "explicar".to_string(),
                Msg::RunLine(format!(":explica %c{block}.{i}")),
                theme.fg_muted,
            ),
        ]);
        out.push(actions);
        height += STAGES_H;
    }

    (out, height)
}

/// Una fila de texto de alto `h`, ancho completo, sin padding lateral —
/// la sangría la da la barra-guía del bloque de etapa.
fn row_text<HostMsg: Clone + 'static>(h: f32) -> View<HostMsg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(h),
        },
        ..Default::default()
    })
}

/// Bytes a etiqueta compacta: `840`, `1.2K`, `3.4M`. Sin espacio para que
/// quepa en el chip.
pub(crate) fn humanize_bytes(n: usize) -> String {
    if n < 1024 {
        format!("{n}B")
    } else if n < 1024 * 1024 {
        format!("{:.1}K", n as f32 / 1024.0)
    } else {
        format!("{:.1}M", n as f32 / (1024.0 * 1024.0))
    }
}

/// Color estable por índice de etapa — para que cada etapa del pipe lea
/// distinto de un vistazo (chip + sus líneas + su barra-guía).
pub(crate) fn stage_color(i: usize) -> llimphi_ui::llimphi_raster::peniko::Color {
    use llimphi_ui::llimphi_raster::peniko::Color;
    let (r, g, b) = STAGE_PALETTE[i % STAGE_PALETTE.len()];
    Color::from_rgba8(r, g, b, 255)
}

/// Misma tinta, atenuada (alfa 80%) — para el texto de las líneas
/// capturadas: menos peso visual que el chip que las titula.
pub(crate) fn stage_color_dim(i: usize) -> llimphi_ui::llimphi_raster::peniko::Color {
    use llimphi_ui::llimphi_raster::peniko::Color;
    let (r, g, b) = STAGE_PALETTE[i % STAGE_PALETTE.len()];
    Color::from_rgba8(r, g, b, 204)
}

