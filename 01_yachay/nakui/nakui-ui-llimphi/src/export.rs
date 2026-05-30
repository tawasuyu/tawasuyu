//! Exportación a archivos en el cwd: reporte a Markdown, desglose de una
//! card a CSV, y la lista activa a CSV. Más los helpers de path/toast
//! compartidos. La serialización en sí vive en `nahual-meta-runtime`
//! (`to_csv`, `breakdown_to_csv`) y en `tablero::report_markdown`.

use super::*;

/// Exporta un `View::Report` completo a Markdown en el cwd, respetando
/// los toggles de filtro activos.
pub(crate) fn export_report_md(m: &Model, module_idx: usize, view_key: &str) -> Toast {
    let Some(module) = m.modules.get(module_idx) else {
        return err_toast("módulo fuera de rango");
    };
    let Some(ModuleView::Report(rv)) = module.views.get(view_key) else {
        return err_toast("no encontré el reporte a exportar");
    };
    let md = report_markdown(m, module, view_key, rv);
    let path = export_path_ext(&rv.title, "md");
    match std::fs::write(&path, md) {
        Ok(()) => Toast {
            kind: BannerKind::Success,
            text: format!("exporté el reporte a {}", path.display()),
        },
        Err(e) => err_toast(&format!("no pude exportar el reporte: {e}")),
    }
}

/// Exporta el desglose de una card (de un tablero o reporte) a CSV.
pub(crate) fn export_breakdown_csv(
    m: &Model,
    module_idx: usize,
    view_key: &str,
    card_idx: usize,
) -> Toast {
    let Some(module) = m.modules.get(module_idx) else {
        return err_toast("módulo fuera de rango");
    };
    // Los reportes aplican sus toggles activos (los que matchean la
    // entity de la card) al CSV; los tableros no tienen toggles.
    let (card, active): (&DashboardCard, Vec<&CardFilter>) = match module.views.get(view_key) {
        Some(ModuleView::Dashboard(dv)) => match dv.cards.get(card_idx) {
            Some(c) => (c, Vec::new()),
            None => return err_toast("tarjeta fuera de rango"),
        },
        Some(ModuleView::Report(rv)) => match rv.cards.get(card_idx) {
            Some(c) => (c, card_active_filters(m, view_key, rv, c)),
            None => return err_toast("tarjeta fuera de rango"),
        },
        _ => return err_toast("la vista no tiene tarjetas"),
    };
    let result = compute_card_result(m, module, card, &active);
    let (gh, vh) = breakdown_headers(card);
    let Some(csv) = breakdown_to_csv(&result, &gh, &vh) else {
        return err_toast("esta tarjeta no es un desglose");
    };
    let path = export_path_ext(&card.label, "csv");
    match std::fs::write(&path, csv) {
        Ok(()) => Toast {
            kind: BannerKind::Success,
            text: format!("exporté «{}» a {}", card.label, path.display()),
        },
        Err(e) => err_toast(&format!("no pude exportar CSV: {e}")),
    }
}

/// Encabezados (grupo, valor) del CSV de un desglose, derivados de la
/// métrica de la card.
pub(crate) fn breakdown_headers(card: &DashboardCard) -> (String, String) {
    use nahual_meta_schema::Metric;
    match &card.metric {
        Metric::GroupBy { field } => (field.clone(), "Cantidad".to_string()),
        Metric::SumBy { group, value } => (group.clone(), format!("Suma de {value}")),
        Metric::AvgBy { group, value } => (group.clone(), format!("Promedio de {value}")),
        _ => ("Grupo".to_string(), "Valor".to_string()),
    }
}

pub(crate) fn err_toast(text: &str) -> Toast {
    Toast {
        kind: BannerKind::Error,
        text: text.to_string(),
    }
}

pub(crate) fn export_path(entity: &str) -> std::path::PathBuf {
    export_path_ext(entity, "csv")
}

/// Como [`export_path`] pero con extensión arbitraria. El `stem` se
/// normaliza a kebab seguro para el filesystem.
pub(crate) fn export_path_ext(stem: &str, ext: &str) -> std::path::PathBuf {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let safe: String = stem
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let name = format!("{safe}-{secs}.{ext}");
    std::env::current_dir()
        .map(|d| d.join(&name))
        .unwrap_or_else(|_| std::path::PathBuf::from(name))
}

/// Exporta la lista activa (filas filtradas/ordenadas, todas las
/// columnas con sus valores renderizados) a un CSV en el cwd; devuelve
/// un toast con el resultado.
pub(crate) fn export_active_list_csv(m: &Model, entity: &str) -> Toast {
    let Some(lv) = active_list_view(m, entity) else {
        return Toast {
            kind: BannerKind::Error,
            text: "no encontré la lista activa para exportar".into(),
        };
    };
    let Ok(backend) = m.backend.lock() else {
        return Toast {
            kind: BannerKind::Error,
            text: "backend lock envenenado".into(),
        };
    };
    let rows = list_filtered_sorted(
        &backend,
        lv,
        &m.list_search.text(),
        &m.list_sort,
        m.drill.as_ref(),
    );
    let headers: Vec<String> = lv.columns.iter().map(|c| c.label.clone()).collect();
    let data: Vec<Vec<String>> = rows
        .iter()
        .map(|(_, v)| {
            lv.columns
                .iter()
                .map(|c| cell_display(&backend, c, lookup_field(v, &c.field)))
                .collect()
        })
        .collect();
    drop(backend);

    let csv = to_csv(&headers, &data);
    let path = export_path(entity);
    match std::fs::write(&path, csv) {
        Ok(()) => Toast {
            kind: BannerKind::Success,
            text: format!("exporté {} fila(s) a {}", rows.len(), path.display()),
        },
        Err(e) => Toast {
            kind: BannerKind::Error,
            text: format!("no pude exportar CSV: {e}"),
        },
    }
}
