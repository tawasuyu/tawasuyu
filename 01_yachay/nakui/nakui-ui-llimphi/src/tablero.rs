//! El núcleo de reportería: cómputo de los agregados de una card
//! (`compute_card_full` + resolución de `group_ref` y labels de campo),
//! drill-down, filtros de toggles, y el render de las vistas `Dashboard`
//! y `Report` (incluido el volcado a Markdown). Se apoya en `charts`
//! para el dibujo y en `nahual-meta-runtime` para el agregado puro.

use super::*;

use llimphi_widget_panel::{panel_signature_painter, PanelStyle};

/// Resuelve las claves de un desglose (UUIDs) al label legible del
/// record referido en `ref_entity`. Las claves que no son UUID se
/// dejan tal cual; los records borrados se marcan como tales. Mismo
/// criterio que [`cell_display`] para columnas `ref_entity`.
pub(crate) fn resolve_breakdown_keys(
    result: &mut MetricResult,
    backend: &NakuiBackend,
    ref_entity: &str,
) {
    let resolve = |key: &str| -> String {
        match Uuid::parse_str(key) {
            Ok(uuid) => backend
                .load_record(ref_entity, uuid)
                .map(|rec| human_label_for_record(&rec, &uuid))
                .unwrap_or_else(|| format!("(borrado · {})", short_uuid(&uuid))),
            Err(_) => key.to_string(),
        }
    };
    match result {
        MetricResult::Breakdown(rows) => {
            for (k, _) in rows.iter_mut() {
                *k = resolve(k);
            }
        }
        MetricResult::ValueBreakdown(rows) => {
            for (k, _) in rows.iter_mut() {
                *k = resolve(k);
            }
        }
        // Resuelve las claves del eje principal (`groups`) si son refs.
        MetricResult::MultiBreakdown { groups, .. } => {
            for g in groups.iter_mut() {
                *g = resolve(g);
            }
        }
        MetricResult::Scalar(_) => {}
    }
}

/// Mapa `valor_crudo → label legible` para un campo de una entity,
/// derivado de su `FieldSpec` en el Form del módulo: opciones de un
/// `Select` (value → label) o booleano (`true`/`false` → Sí/No). `None`
/// si el campo no tiene un mapeo legible (texto/número/fecha/ref/etc.).
pub(crate) fn field_label_map(module: &Module, entity: &str, field: &str) -> Option<BTreeMap<String, String>> {
    let fv = find_form_view(module, entity)?;
    let spec = fv.fields.iter().find(|f| f.name == field)?;
    match spec.kind {
        FieldKind::Select => {
            let map: BTreeMap<String, String> = spec
                .options
                .iter()
                .map(|o| (o.value.clone(), o.display().to_string()))
                .collect();
            (!map.is_empty()).then_some(map)
        }
        FieldKind::Boolean => Some(
            [
                ("true".to_string(), "Sí".to_string()),
                ("false".to_string(), "No".to_string()),
            ]
            .into_iter()
            .collect(),
        ),
        _ => None,
    }
}

/// Reemplaza una clave por su label si el mapa la cubre (no-op si no).
pub(crate) fn relabel(k: &mut String, map: &BTreeMap<String, String>) {
    if let Some(label) = map.get(k.as_str()) {
        *k = label.clone();
    }
}

/// Reemplaza las claves crudas de un desglose por labels legibles según
/// el `FieldSpec` del campo de grupo (y de serie, en multi-serie). No
/// toca la dimensión de grupo si la card usa `group_ref` (ya resuelta a
/// labels de record) o `bucket` (claves de fecha). Las series de un
/// `SumBySeries` siempre se humanizan. Sólo afecta lo mostrado/exportado
/// — el drill-down sigue usando las `raw_keys` crudas.
pub(crate) fn humanize_breakdown_labels(result: &mut MetricResult, module: &Module, card: &DashboardCard) {
    let entity = &card.entity;
    if card.group_ref.is_none() && card.bucket.is_none() {
        if let Some(field) = metric_group_field(&card.metric) {
            if let Some(map) = field_label_map(module, entity, field) {
                match result {
                    MetricResult::Breakdown(rows) => {
                        rows.iter_mut().for_each(|(k, _)| relabel(k, &map))
                    }
                    MetricResult::ValueBreakdown(rows) => {
                        rows.iter_mut().for_each(|(k, _)| relabel(k, &map))
                    }
                    MetricResult::MultiBreakdown { groups, .. } => {
                        groups.iter_mut().for_each(|k| relabel(k, &map))
                    }
                    MetricResult::Scalar(_) => {}
                }
            }
        }
    }
    if let nahual_meta_schema::Metric::SumBySeries { series, .. } = &card.metric {
        if let Some(map) = field_label_map(module, entity, series) {
            if let MetricResult::MultiBreakdown { series: rows, .. } = result {
                rows.iter_mut().for_each(|(name, _)| relabel(name, &map));
            }
        }
    }
}

/// Computa el agregado de una card resolviendo `group_ref` y labels de
/// campo si los hay. Toma el lock del backend por card — el tablero no
/// es ruta caliente. `extra` son filtros adicionales (toggles de reporte
/// activos) que se aplican (AND) sobre los records antes de agregar.
pub(crate) fn compute_card_result(
    model: &Model,
    module: &Module,
    card: &DashboardCard,
    extra: &[&CardFilter],
) -> MetricResult {
    compute_card_full(model, module, card, extra).0
}

/// Como [`compute_card_result`] pero devuelve también las claves de
/// grupo *crudas* (sin resolver por `group_ref`), alineadas 1:1 con las
/// filas del resultado. El drill-down las usa para filtrar la lista por
/// el valor real (UUID), aunque la card muestre el label resuelto.
pub(crate) fn compute_card_full(
    model: &Model,
    module: &Module,
    card: &DashboardCard,
    extra: &[&CardFilter],
) -> (MetricResult, Vec<String>) {
    let guard = model.backend.lock().ok();
    let mut records = guard
        .as_ref()
        .map(|b| b.list_records(&card.entity))
        .unwrap_or_default();
    if !extra.is_empty() {
        records.retain(|(_, v)| extra.iter().all(|f| record_matches(v, f)));
    }
    // Serie temporal: si la card define `bucket` sobre el campo de grupo
    // (una fecha ISO), reescribimos ese campo a su bucket (año/mes/día)
    // *antes* de agregar, así records de distintos días caen en el mismo
    // grupo. La agregación queda agnóstica al truncado.
    let group_field = metric_group_field(&card.metric);
    let bucketed = match (card.bucket, group_field) {
        (Some(bucket), Some(field)) => {
            for (_, v) in records.iter_mut() {
                if let Some(s) = v.get(field).and_then(Value::as_str) {
                    let key = bucket_date(s, bucket);
                    if let Some(obj) = v.as_object_mut() {
                        obj.insert(field.to_string(), Value::String(key));
                    }
                }
            }
            true
        }
        _ => false,
    };
    let mut result = compute_metric(&card.metric, card.filter.as_ref(), &records);
    // Series temporales: orden cronológico (por clave) y sin recorte.
    // Resto: top-N opcional (recorte a las `limit` mayores + "Otros").
    // Se hace sobre el resultado crudo (antes de resolver claves) para
    // que las raw_keys —drill-down, CSV, export .md— queden alineadas.
    let collapsed = if bucketed {
        sort_breakdown_by_key(&mut result);
        false
    } else {
        card.limit
            .map(|n| limit_breakdown(&mut result, n, metric_is_additive(&card.metric)))
            .unwrap_or(false)
    };
    // Acumulado (running total): tras fijar el orden, cada valor pasa a
    // ser la suma corrida. No toca las claves, así raw_keys/drill siguen
    // alineados. El caso natural de tesorería ("saldo acumulado por mes").
    if card.cumulative {
        cumulative_breakdown(&mut result);
    }
    let mut raw_keys = breakdown_raw_keys(&result);
    // La fila "Otros" no apunta a un grupo concreto: sentinel vacío para
    // que `drill_msg` la deje no-clickeable. Las series temporales SÍ
    // navegan: la clave es el bucket ("2026-02") y el drill matchea por
    // prefijo sobre la fecha cruda (ver `DrillCtx::prefix`).
    if collapsed {
        if let Some(last) = raw_keys.last_mut() {
            last.clear();
        }
    }
    if let (Some(ref_entity), Some(backend)) = (&card.group_ref, guard.as_ref()) {
        resolve_breakdown_keys(&mut result, backend, ref_entity);
    }
    // Labels legibles de las claves de campo (Select → su label,
    // booleano → Sí/No). No pisa lo resuelto por `group_ref`/`bucket`.
    humanize_breakdown_labels(&mut result, module, card);
    (result, raw_keys)
}

/// El campo de grupo de una métrica de desglose (`GroupBy.field` /
/// `SumBy`·`AvgBy.group`). `None` para escalares.
pub(crate) fn metric_group_field(metric: &nahual_meta_schema::Metric) -> Option<&str> {
    use nahual_meta_schema::Metric;
    match metric {
        Metric::GroupBy { field } => Some(field),
        Metric::SumBy { group, .. }
        | Metric::AvgBy { group, .. }
        | Metric::SumBySeries { group, .. } => Some(group),
        _ => None,
    }
}

/// `true` si el valor de un desglose es aditivo (se puede sumar para el
/// bucket "Otros"): conteos (`GroupBy`) y sumas (`SumBy`). `AvgBy` no.
pub(crate) fn metric_is_additive(metric: &nahual_meta_schema::Metric) -> bool {
    use nahual_meta_schema::Metric;
    !matches!(metric, Metric::AvgBy { .. })
}

/// Claves de grupo de un desglose, en orden (vacío para escalares).
pub(crate) fn breakdown_raw_keys(result: &MetricResult) -> Vec<String> {
    match result {
        MetricResult::Breakdown(rows) => rows.iter().map(|(k, _)| k.clone()).collect(),
        MetricResult::ValueBreakdown(rows) => rows.iter().map(|(k, _)| k.clone()).collect(),
        // Multi-serie no es navegable (drill ambiguo entre group y serie).
        MetricResult::MultiBreakdown { .. } => Vec::new(),
        MetricResult::Scalar(_) => Vec::new(),
    }
}

/// El campo por el que agrupa una métrica de desglose (para el filtro
/// de drill-down). `None` para escalares.
pub(crate) fn drill_field(card: &DashboardCard) -> Option<String> {
    use nahual_meta_schema::Metric;
    match &card.metric {
        Metric::GroupBy { field } => Some(field.clone()),
        Metric::SumBy { group, .. } | Metric::AvgBy { group, .. } => Some(group.clone()),
        _ => None,
    }
}

/// `true` si el módulo tiene una vista `List` para esa entity (destino
/// posible de un drill-down).
pub(crate) fn has_list_for(module: &Module, entity: &str) -> bool {
    module.views.values().any(|v| {
        matches!(v, ModuleView::List(lv) if lv.entity == entity)
    })
}

/// Contexto de drill-down de una card: a dónde navega cada fila del
/// desglose. `field` es el campo de filtro; `raw_keys[i]` el valor real
/// de la fila i; `labels[i]` el texto mostrado (para el chip).
pub(crate) struct DrillCtx {
    entity: String,
    field: String,
    raw_keys: Vec<String>,
    labels: Vec<String>,
    /// Match por prefijo (series temporales): el bucket "2026-02"
    /// recorta a las fechas que empiezan con él.
    prefix: bool,
}

/// Arma el `DrillCtx` de una card si es un desglose y existe una lista
/// de su entity a la que navegar. `raw_keys` son las claves sin
/// resolver; los labels salen del `result` ya resuelto.
pub(crate) fn drill_ctx_for(
    module: &Module,
    card: &DashboardCard,
    result: &MetricResult,
    raw_keys: Vec<String>,
) -> Option<DrillCtx> {
    let field = drill_field(card)?;
    if !has_list_for(module, &card.entity) {
        return None;
    }
    let labels = breakdown_raw_keys(result);
    Some(DrillCtx {
        entity: card.entity.clone(),
        field,
        raw_keys,
        labels,
        prefix: card.bucket.is_some(),
    })
}

/// Clave de grupo de un record para un campo top-level, replicando el
/// `field_as_text` de meta-runtime (lo que produce las claves de los
/// desgloses) — para que el drill-down matchee exactamente.
pub(crate) fn group_key_text(v: &Value, field: &str) -> Option<String> {
    match v.get(field)? {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// Clave de un toggle de reporte en `Model::report_filters`.
pub(crate) fn report_filter_key(view_key: &str, idx: usize) -> String {
    format!("{view_key}#{idx}")
}

/// Filtros de los toggles activos que aplican a una card concreta: un
/// toggle entra si está prendido y su `entity` es `None` o coincide con
/// la de la card.
pub(crate) fn card_active_filters<'a>(
    model: &'a Model,
    view_key: &str,
    rv: &'a ReportView,
    card: &DashboardCard,
) -> Vec<&'a CardFilter> {
    rv.toggles
        .iter()
        .enumerate()
        .filter(|(i, _)| model.report_filters.contains(&report_filter_key(view_key, *i)))
        .filter(|(_, t)| t.entity.as_deref().map_or(true, |e| e == card.entity))
        .map(|(_, t)| &t.filter)
        .collect()
}

/// Labels de los toggles activos de un reporte (para encabezados).
pub(crate) fn active_toggle_labels(model: &Model, view_key: &str, rv: &ReportView) -> Vec<String> {
    rv.toggles
        .iter()
        .enumerate()
        .filter(|(i, _)| model.report_filters.contains(&report_filter_key(view_key, *i)))
        .map(|(_, t)| t.label.clone())
        .collect()
}

/// `true` si el resultado es un desglose (exportable a CSV).
pub(crate) fn is_breakdown(r: &MetricResult) -> bool {
    matches!(
        r,
        MetricResult::Breakdown(_)
            | MetricResult::ValueBreakdown(_)
            | MetricResult::MultiBreakdown { .. }
    )
}

/// Vista `Dashboard`: una grilla de tarjetas de KPI, cada una con su
/// agregado (`Count`/`Sum`/`Avg`/`Min`/`Max`/`GroupBy`/`SumBy`/`AvgBy`)
/// computado sobre los records de su entity.
pub(crate) fn build_dashboard_panel(
    model: &Model,
    mod_idx: usize,
    view_key: &str,
    dv: &DashboardView,
    theme: &Theme,
) -> View<Msg> {
    let module = &model.modules[mod_idx];
    let title = text_line(
        format!("{} · {}", module.label, dv.title),
        16.0,
        theme.fg_text,
    );

    let mut cards: Vec<View<Msg>> = Vec::new();
    for (i, card) in dv.cards.iter().enumerate() {
        let (result, raw_keys) = compute_card_full(model, module, card, &[]);
        // Las cards con desglose ganan un botón de export CSV.
        let on_export = if is_breakdown(&result) {
            Some(Msg::ExportBreakdownCsv {
                module_idx: mod_idx,
                view_key: view_key.to_string(),
                card_idx: i,
            })
        } else {
            None
        };
        let drill = drill_ctx_for(module, card, &result, raw_keys);
        cards.push(dashboard_card(
            &card.label,
            &result,
            &card.format,
            card.chart,
            on_export,
            drill.as_ref(),
            theme,
        ));
    }

    let grid = View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_wrap: llimphi_ui::llimphi_layout::taffy::FlexWrap::Wrap,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        align_content: Some(llimphi_ui::llimphi_layout::taffy::AlignContent::Start),
        gap: Size {
            width: length(12.0),
            height: length(12.0),
        },
        ..Default::default()
    })
    .children(cards);

    column(vec![title, grid], 12.0)
}

/// Una tarjeta del tablero: label + número grande (Scalar) o barras de
/// breakdown (GroupBy).
pub(crate) fn dashboard_card(
    label: &str,
    result: &MetricResult,
    fmt: &ValueFormat,
    chart: ChartKind,
    on_export: Option<Msg>,
    drill: Option<&DrillCtx>,
    theme: &Theme,
) -> View<Msg> {
    let mut children: Vec<View<Msg>> = vec![text_line(label.to_string(), 11.0, theme.fg_muted)];
    // Closure que arma el click de drill-down de la fila `i` (si hay).
    let drill_msg = |i: usize| -> Option<Msg> {
        let d = drill?;
        let value = d.raw_keys.get(i)?.clone();
        // Sentinel vacío = fila agregada ("Otros"): no navega a nada.
        if value.is_empty() {
            return None;
        }
        Some(Msg::DrillDown {
            entity: d.entity.clone(),
            field: d.field.clone(),
            value,
            label: d.labels.get(i).cloned().unwrap_or_default(),
            prefix: d.prefix,
        })
    };

    match result {
        MetricResult::Scalar(s) => {
            // Entero si no tiene parte decimal (Count / sumas enteras).
            let value = if s.fract() == 0.0 {
                Value::from(*s as i64)
            } else {
                Value::from(*s)
            };
            children.push(
                View::new(Style {
                    size: Size {
                        width: percent(1.0_f32),
                        height: length(34.0),
                    },
                    align_items: Some(AlignItems::Center),
                    ..Default::default()
                })
                .text_aligned(
                    format_value(Some(&value), fmt),
                    26.0,
                    theme.accent,
                    Alignment::Start,
                ),
            );
        }
        // Desgloses (GroupBy / SumBy / AvgBy): normalizados a una lista
        // `(label, magnitud, texto)` y pintados según `chart` —barras
        // ASCII (default), torta o dona—.
        MetricResult::Breakdown(_) | MetricResult::ValueBreakdown(_) => {
            let items = breakdown_display(result, fmt);
            if items.is_empty() {
                children.push(text_line("(sin datos)".into(), 11.0, theme.fg_muted));
            } else if matches!(chart, ChartKind::Pie | ChartKind::Donut) {
                let donut = matches!(chart, ChartKind::Donut);
                let slices: Vec<(f64, Color)> = items
                    .iter()
                    .enumerate()
                    .map(|(i, (_, m, _))| (m.abs(), chart_color(i)))
                    .collect();
                children.push(pie_canvas(slices, donut, theme.bg_panel_alt));
                let total: f64 = items.iter().map(|(_, m, _)| m.abs()).sum();
                for (i, (key, m, disp)) in items.iter().enumerate() {
                    let pct = if total > 0.0 { m.abs() / total * 100.0 } else { 0.0 };
                    children.push(legend_row(
                        chart_color(i),
                        key.clone(),
                        format!("{disp} · {pct:.0}%"),
                        drill_msg(i),
                        theme,
                    ));
                }
            } else if matches!(
                chart,
                ChartKind::Columns | ChartKind::Line | ChartKind::StackedColumns
            ) {
                // En una sola dimensión, `stacked_columns` = `columns`.
                let line = matches!(chart, ChartKind::Line);
                let series: Vec<(f64, Color)> = items
                    .iter()
                    .enumerate()
                    .map(|(i, (_, m, _))| (*m, chart_color(i)))
                    .collect();
                children.push(plot_canvas(series, line, theme.border, theme.accent));
                for (i, (key, _, disp)) in items.iter().enumerate() {
                    children.push(legend_row(
                        chart_color(i),
                        key.clone(),
                        disp.clone(),
                        drill_msg(i),
                        theme,
                    ));
                }
            } else {
                // Barras: la longitud escala contra el mayor valor absoluto.
                let value_w = if matches!(result, MetricResult::ValueBreakdown(_)) {
                    72.0
                } else {
                    32.0
                };
                let max = items
                    .iter()
                    .map(|(_, m, _)| m.abs())
                    .fold(0.0_f64, f64::max)
                    .max(1.0);
                for (i, (key, m, disp)) in items.iter().enumerate() {
                    let filled = ((m.abs() / max) * 12.0).round() as usize;
                    let bar = "█".repeat(filled.max(1));
                    children.push(breakdown_row(
                        key.clone(),
                        bar,
                        disp.clone(),
                        value_w,
                        drill_msg(i),
                        theme,
                    ));
                }
            }
        }
        // Desglose de dos dimensiones (`SumBySeries`): multi-línea o
        // columnas agrupadas. Una serie por color; leyenda con el total
        // de cada serie; caption con el orden de los grupos (eje x).
        MetricResult::MultiBreakdown { groups, series } => {
            if groups.is_empty() || series.is_empty() {
                children.push(text_line("(sin datos)".into(), 11.0, theme.fg_muted));
            } else {
                let mode = match chart {
                    ChartKind::Line => MultiMode::Line,
                    ChartKind::StackedColumns => MultiMode::Stacked,
                    _ => MultiMode::Grouped,
                };
                let plot_series: Vec<(Vec<f64>, Color)> = series
                    .iter()
                    .enumerate()
                    .map(|(i, (_, vals))| (vals.clone(), chart_color(i)))
                    .collect();
                children.push(multi_plot_canvas(
                    groups.len(),
                    plot_series,
                    mode,
                    theme.border,
                ));
                // Caption: el eje x (grupos en orden).
                children.push(text_line(groups.join("  ·  "), 10.0, theme.fg_muted));
                // Leyenda: total de cada serie.
                for (i, (name, vals)) in series.iter().enumerate() {
                    let total: f64 = vals.iter().sum();
                    let value = if total.fract() == 0.0 {
                        Value::from(total as i64)
                    } else {
                        Value::from(total)
                    };
                    children.push(legend_row(
                        chart_color(i),
                        name.clone(),
                        format_value(Some(&value), fmt),
                        None,
                        theme,
                    ));
                }
            }
        }
    }

    // Botón de export CSV para los desgloses.
    if let Some(msg) = on_export {
        children.push(button_styled(
            "⤓ CSV",
            btn_style_auto(),
            Alignment::Center,
            &ButtonPalette::from_theme(theme),
            msg,
        ));
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(220.0),
            height: auto(),
        },
        flex_grow: 0.0,
        flex_shrink: 0.0,
        padding: Rect {
            left: length(14.0),
            right: length(14.0),
            top: length(12.0),
            bottom: length(12.0),
        },
        gap: Size {
            width: length(0.0),
            height: length(6.0),
        },
        ..Default::default()
    })
    // Firma visual transversal del kit (gradiente vertical + hairline
    // accent) en vez de un fill plano — para que las stat cards de nakui
    // lean "talladas" igual que el resto del sistema. Reemplaza el fill.
    .paint_with(panel_signature_painter(PanelStyle::from_theme(theme)))
    .radius(PanelStyle::from_theme(theme).radius)
    .clip(true)
    .children(children)
}

/// Vista `Report`: los mismos agregados que un tablero, dispuestos
/// como documento de una columna (título + subtítulo) con un botón
/// "Exportar (.md)" que vuelca el reporte completo a Markdown.
pub(crate) fn build_report_panel(
    model: &Model,
    mod_idx: usize,
    view_key: &str,
    rv: &ReportView,
    theme: &Theme,
) -> View<Msg> {
    let module = &model.modules[mod_idx];
    let mut children: Vec<View<Msg>> = Vec::new();

    let header = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::SpaceBetween),
        ..Default::default()
    })
    .children(vec![
        text_line(format!("{} · {}", module.label, rv.title), 16.0, theme.fg_text),
        button_styled(
            "⤓ Exportar (.md)",
            btn_style(150.0),
            Alignment::Center,
            &accent_btn(theme),
            Msg::ExportReport {
                module_idx: mod_idx,
                view_key: view_key.to_string(),
            },
        ),
    ]);
    children.push(header);
    if let Some(sub) = &rv.subtitle {
        children.push(text_line(sub.clone(), 12.0, theme.fg_muted));
    }

    // Barra de toggles interactivos: cada uno prende/apaga un filtro.
    if !rv.toggles.is_empty() {
        let mut chips: Vec<View<Msg>> = Vec::new();
        for (i, toggle) in rv.toggles.iter().enumerate() {
            let active = model
                .report_filters
                .contains(&report_filter_key(view_key, i));
            let palette = if active {
                accent_btn(theme)
            } else {
                ButtonPalette::from_theme(theme)
            };
            let label = if active {
                format!("● {}", toggle.label)
            } else {
                format!("○ {}", toggle.label)
            };
            chips.push(button_styled(
                label,
                btn_style_auto(),
                Alignment::Center,
                &palette,
                Msg::ToggleReportFilter {
                    view_key: view_key.to_string(),
                    idx: i,
                },
            ));
        }
        children.push(
            View::new(Style {
                flex_direction: FlexDirection::Row,
                flex_wrap: llimphi_ui::llimphi_layout::taffy::FlexWrap::Wrap,
                size: Size {
                    width: percent(1.0_f32),
                    height: auto(),
                },
                gap: Size {
                    width: length(8.0),
                    height: length(8.0),
                },
                ..Default::default()
            })
            .children(chips),
        );
    }

    // Una card por agregado, apiladas en columna (documento).
    for (i, card) in rv.cards.iter().enumerate() {
        let active = card_active_filters(model, view_key, rv, card);
        let (result, raw_keys) = compute_card_full(model, module, card, &active);
        let on_export = if is_breakdown(&result) {
            Some(Msg::ExportBreakdownCsv {
                module_idx: mod_idx,
                view_key: view_key.to_string(),
                card_idx: i,
            })
        } else {
            None
        };
        let drill = drill_ctx_for(module, card, &result, raw_keys);
        children.push(dashboard_card(
            &card.label,
            &result,
            &card.format,
            card.chart,
            on_export,
            drill.as_ref(),
            theme,
        ));
    }

    column(children, 12.0)
}

/// Serializa un reporte completo a Markdown: título, subtítulo, y una
/// sección por card (escalar en negrita o tabla de desglose).
pub(crate) fn report_markdown(model: &Model, module: &Module, view_key: &str, rv: &ReportView) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {} · {}\n\n", module.label, rv.title));
    if let Some(sub) = &rv.subtitle {
        out.push_str(&format!("_{sub}_\n\n"));
    }
    let active_labels = active_toggle_labels(model, view_key, rv);
    if !active_labels.is_empty() {
        out.push_str(&format!("Filtros activos: {}\n\n", active_labels.join(" · ")));
    }
    out.push_str("Generado por nakui.\n\n");
    for card in &rv.cards {
        let active = card_active_filters(model, view_key, rv, card);
        let result = compute_card_result(model, module, card, &active);
        out.push_str(&format!("## {}\n\n", card.label));
        match &result {
            MetricResult::Scalar(s) => {
                let value = if s.fract() == 0.0 {
                    Value::from(*s as i64)
                } else {
                    Value::from(*s)
                };
                out.push_str(&format!("**{}**\n\n", format_value(Some(&value), &card.format)));
            }
            MetricResult::Breakdown(rows) => {
                out.push_str("| Grupo | Cantidad |\n|---|---:|\n");
                for (k, n) in rows {
                    out.push_str(&format!("| {} | {} |\n", md_escape(k), n));
                }
                out.push('\n');
            }
            MetricResult::ValueBreakdown(rows) => {
                out.push_str("| Grupo | Valor |\n|---|---:|\n");
                for (k, v) in rows {
                    let value = if v.fract() == 0.0 {
                        Value::from(*v as i64)
                    } else {
                        Value::from(*v)
                    };
                    out.push_str(&format!(
                        "| {} | {} |\n",
                        md_escape(k),
                        format_value(Some(&value), &card.format)
                    ));
                }
                out.push('\n');
            }
            // Tabla matriz: una columna por serie.
            MetricResult::MultiBreakdown { groups, series } => {
                out.push_str("| Grupo |");
                let mut sep = String::from("|---|");
                for (name, _) in series {
                    out.push_str(&format!(" {} |", md_escape(name)));
                    sep.push_str("---:|");
                }
                out.push('\n');
                out.push_str(&sep);
                out.push('\n');
                for (i, g) in groups.iter().enumerate() {
                    out.push_str(&format!("| {} |", md_escape(g)));
                    for (_, vals) in series {
                        let v = vals.get(i).copied().unwrap_or(0.0);
                        let value = if v.fract() == 0.0 {
                            Value::from(v as i64)
                        } else {
                            Value::from(v)
                        };
                        out.push_str(&format!(" {} |", format_value(Some(&value), &card.format)));
                    }
                    out.push('\n');
                }
                out.push('\n');
            }
        }
    }
    out
}

/// Escapa los `|` de una celda de tabla Markdown.
pub(crate) fn md_escape(s: &str) -> String {
    s.replace('|', "\\|")
}

