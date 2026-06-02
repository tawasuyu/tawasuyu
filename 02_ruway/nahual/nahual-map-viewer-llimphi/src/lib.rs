//! `nahual-map-viewer-llimphi` — visor de mapas (GeoJSON/GPX/KML/PMTiles)
//! sobre Llimphi. **El dominio geoespacial vive en `nahual-geo-core`**
//! (parsers, modelo, proyección, hit-test, ruteo, basemap); este crate
//! sólo lo pinta con vello vía `paint_with`, más la paleta y la leyenda.
//! Cambiar de GUI no pierde nada del dominio (regla #2 del repo).

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Circle, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_text::{draw_block, Alignment, TextBlock};
use llimphi_ui::View;
use std::path::Path;

// Re-exportamos el dominio para no romper a los consumidores (el shell
// usa `nahual_map_viewer_llimphi::{MapView, load_map, ...}`).
pub use nahual_geo_core::*;

/// Color de una posición `t ∈ [0,1]` en una escala secuencial azul→ámbar→rojo
/// (legible y con buen contraste sobre fondo oscuro o claro).
fn scale_color(t: f64) -> Color {
    let t = t.clamp(0.0, 1.0);
    // Tres paradas: azul (40,110,200) → ámbar (240,200,70) → rojo (210,60,50).
    let stops = [
        (40.0, 110.0, 200.0),
        (240.0, 200.0, 70.0),
        (210.0, 60.0, 50.0),
    ];
    let (a, b, local) = if t < 0.5 {
        (stops[0], stops[1], t / 0.5)
    } else {
        (stops[1], stops[2], (t - 0.5) / 0.5)
    };
    let lerp = |x: f64, y: f64| (x + (y - x) * local).round().clamp(0.0, 255.0) as u8;
    Color::from_rgba8(lerp(a.0, b.0), lerp(a.1, b.1), lerp(a.2, b.2), 255)
}

/// Parsea GPX (XML de GPS): waypoints (`<wpt>`) → puntos, rutas (`<rte>`) y
/// segmentos de track (`<trkseg>`) → polilíneas. Los `<name>` de waypoints,
#[derive(Debug, Clone, Copy)]
pub struct MapViewerPalette {
    pub bg: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
    pub fg_error: Color,
    /// Trazo de líneas y bordes de polígono.
    pub stroke: Color,
    /// Relleno de polígonos (se aplica translúcido).
    pub fill: Color,
    /// Disco de los puntos.
    pub point: Color,
    /// Rejilla de coordenadas (se aplica muy tenue).
    pub grid: Color,
    /// Texto de etiquetas y rótulos de la rejilla.
    pub label: Color,
    /// Mapa-base mundial (tierra): se aplica muy tenue de fondo.
    pub land: Color,
}

impl Default for MapViewerPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl MapViewerPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg: t.bg_app,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
            fg_error: t.fg_destructive,
            stroke: t.accent,
            fill: t.accent,
            point: t.fg_text,
            grid: t.fg_muted,
            label: t.fg_text,
            land: t.fg_muted,
        }
    }
}

/// Multiplica el alfa de un color (sin reemplazarlo). Mismo patrón que los
/// widgets de llimphi.
fn with_alpha(c: Color, alpha: f32) -> Color {
    let rgba = c.to_rgba8();
    let a = (alpha.clamp(0.0, 1.0) * 255.0) as u8;
    Color::from_rgba8(rgba.r, rgba.g, rgba.b, a)
}

/// Pinta header (nombre + resumen) + body con el mapa proyectado.
pub fn map_viewer_view<Msg, F>(
    state: &MapPreview,
    path: Option<&Path>,
    palette: &MapViewerPalette,
    view: &MapView,
    on_pick: F,
) -> View<Msg>
where
    Msg: Clone + 'static,
    F: Fn(f32, f32, f32, f32) -> Option<Msg> + Send + Sync + 'static,
{
    let name = path
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().to_string());

    let header_text = match (name.as_deref(), state) {
        (Some(n), MapPreview::Map { data, truncated }) => {
            let bb = data.bbox();
            let bbox_txt = bb
                .map(|b| {
                    format!(
                        " · [{:.3},{:.3} → {:.3},{:.3}]",
                        b.min_lon, b.min_lat, b.max_lon, b.max_lat
                    )
                })
                .unwrap_or_default();
            format!(
                "mapa · {n} · {} pts · {} líneas · {} polígonos{}{}",
                data.points.len(),
                data.lines.len(),
                data.polygons.len(),
                bbox_txt,
                if *truncated { " · (truncado)" } else { "" },
            )
        }
        (Some(n), _) => format!("mapa · {n}"),
        (None, _) => "(seleccioná un .geojson)".to_string(),
    };
    // En modo búsqueda/ruteo, el header refleja el estado.
    let header_text = if view.searching {
        format!("buscar: {}▏", view.query)
    } else if view.routing {
        let dist = if view.route_meters > 0.0 {
            format!(" · {}", fmt_distance(view.route_meters / 1000.0))
        } else {
            String::new()
        };
        format!(
            "ruta · {}/2 puntos{} · (clic origen y destino · r sale)",
            view.route_pins.len(),
            dist
        )
    } else {
        header_text
    };
    let header_color = if view.searching || view.routing {
        palette.fg_text
    } else {
        palette.fg_muted
    };

    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        padding: pad(12.0, 0.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(header_text, 10.0, header_color, Alignment::Start);

    let body = match state {
        MapPreview::Empty => simple_body("—", palette.fg_muted),
        MapPreview::NoGeometry => simple_body("(JSON sin geometrías GeoJSON)", palette.fg_muted),
        MapPreview::TooBig(n) => simple_body(
            &format!("(archivo muy grande: {n} bytes — sin preview)"),
            palette.fg_muted,
        ),
        MapPreview::Error(e) => simple_body(&format!("(no se pudo leer: {e})"), palette.fg_error),
        // El clic sobre el lienzo se reporta como fracción del rect (el host
        // la resuelve con `hit_test`); el resto de las variantes lo ignora.
        MapPreview::Map { data, .. } => {
            map_canvas(data.clone(), *palette, view.clone()).on_click_at(on_pick)
        }
    };

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(6.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg)
    .clip(true)
    .children(vec![header, body])
}

/// Lienzo que proyecta y dibuja las geometrías encajadas en el panel,
/// aplicando la cámara (`zoom`/`pan`) y registrando su rect para el host.
fn map_canvas<Msg>(data: MapData, palette: MapViewerPalette, view: MapView) -> View<Msg>
where
    Msg: Clone + 'static,
{
    let zoom = view.zoom;
    let pan = view.pan;
    let show_base = view.show_base;
    let selected = view.selected;
    let color_field = view.color_field.clone();
    let route_pins = view.route_pins.clone();
    let route_path = view.route_path.clone();
    View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: pad(8.0, 6.0),
        ..Default::default()
    })
    .paint_with(move |scene, ts, rect| {
        // Registrar el rect físico para que el host acote el zoom-por-rueda.
        view.record_rect((rect.x, rect.y, rect.w, rect.h));
        let Some(bb) = data.bbox() else { return };
        if rect.w <= 8.0 || rect.h <= 8.0 {
            return;
        }

        // Proyección equirectangular (corrección por cos(lat)) + cámara
        // (zoom/pan), encapsulada para compartir la matemática exacta con el
        // hit-test del clic.
        let proj = Projection::fit(
            bb,
            (rect.x as f64, rect.y as f64, rect.w as f64, rect.h as f64),
            zoom,
            pan,
        );
        let to_screen = |c: Coord| proj.to_screen(c);
        let (pivot_x, pivot_y) = (proj.pivot_x, proj.pivot_y);
        let scale = proj.scale;

        let stroke_thin = Stroke::new(1.2);
        let stroke_edge = Stroke::new(1.0);
        let stroke_grid = Stroke::new(0.75);
        let fill_col = with_alpha(palette.fill, 0.18);
        let grid_col = with_alpha(palette.grid, 0.22);
        let grid_label_col = with_alpha(palette.label, 0.55);

        let in_panel = |x: f64, y: f64| {
            x >= rect.x as f64
                && x <= (rect.x + rect.w) as f64
                && y >= rect.y as f64
                && y <= (rect.y + rect.h) as f64
        };

        // --- Mapa-base mundial (detrás de todo) ----------------------
        // Países Natural Earth, proyectados con la misma cámara que el dato:
        // al hacer zoom a una región, sólo se ve su parte (el resto, clipeado).
        if show_base {
            let world = world_base();
            let land_fill = with_alpha(palette.land, 0.10);
            let land_stroke = with_alpha(palette.land, 0.32);
            let land_label = with_alpha(palette.land, 0.5);
            let stroke_coast = Stroke::new(0.6);
            for poly in &world.polygons {
                for (i, ring) in poly.iter().enumerate() {
                    let path = ring_path(ring, &to_screen, true);
                    if i == 0 {
                        scene.fill(Fill::NonZero, Affine::IDENTITY, land_fill, None, &path);
                    }
                    scene.stroke(&stroke_coast, Affine::IDENTITY, land_stroke, None, &path);
                }
            }
            // Nombres de país, sólo los que caen dentro del panel (el clip
            // recorta el resto, así que en una vista regional son pocos).
            for label in &world.labels {
                let (x, y) = to_screen(label.at);
                if in_panel(x, y) {
                    let block = TextBlock::simple(&label.text, 9.0, land_label, (x + 2.0, y - 6.0));
                    draw_block(scene, ts, &block);
                }
            }
        }

        // --- Rejilla de coordenadas (detrás del dato) ----------------
        // Líneas de lon/lat a un paso "redondo" con rótulo en grados, para
        // dar contexto geográfico aunque no haya mapa-base.
        let x0 = rect.x as f64;
        let y0 = rect.y as f64;
        let x1 = x0 + rect.w as f64;
        let y1 = y0 + rect.h as f64;
        let lon_step = nice_step(bb.max_lon - bb.min_lon);
        let lat_step = nice_step(bb.max_lat - bb.min_lat);
        for lon in ticks(bb.min_lon, bb.max_lon, lon_step) {
            let (gx, _) = to_screen([lon, bb.max_lat]);
            let mut path = BezPath::new();
            path.move_to((gx, y0));
            path.line_to((gx, y1));
            scene.stroke(&stroke_grid, Affine::IDENTITY, grid_col, None, &path);
            let txt = fmt_deg(lon, lon_step);
            let block = TextBlock::simple(&txt, 9.0, grid_label_col, (gx + 2.0, y1 - 12.0));
            draw_block(scene, ts, &block);
        }
        for lat in ticks(bb.min_lat, bb.max_lat, lat_step) {
            let (_, gy) = to_screen([bb.min_lon, lat]);
            let mut path = BezPath::new();
            path.move_to((x0, gy));
            path.line_to((x1, gy));
            scene.stroke(&stroke_grid, Affine::IDENTITY, grid_col, None, &path);
            let txt = fmt_deg(lat, lat_step);
            let block = TextBlock::simple(&txt, 9.0, grid_label_col, (x0 + 2.0, gy + 1.0));
            draw_block(scene, ts, &block);
        }

        // Choropleth: si hay campo de color activo, rango [min,max] del valor
        // a través de las features (para mapear cada polígono a un color).
        let choro = color_field.as_deref().and_then(|field| {
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            for f in &data.features {
                if let Some(v) = f.number(field) {
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
            }
            (hi > lo).then_some((field, lo, hi))
        });

        // Polígonos: relleno (choropleth o translúcido uniforme) + borde.
        for (pi, poly) in data.polygons.iter().enumerate() {
            // Color de relleno del polígono según el choropleth, si aplica.
            let fill = choro
                .and_then(|(field, lo, hi)| {
                    let fi = *data.polygon_feat.get(pi)?;
                    let v = data.features.get(fi)?.number(field)?;
                    Some(with_alpha(scale_color((v - lo) / (hi - lo)), 0.62))
                })
                .unwrap_or(fill_col);
            for (i, ring) in poly.iter().enumerate() {
                let path = ring_path(ring, &to_screen, true);
                if i == 0 {
                    scene.fill(Fill::NonZero, Affine::IDENTITY, fill, None, &path);
                }
                scene.stroke(&stroke_edge, Affine::IDENTITY, palette.stroke, None, &path);
            }
        }

        // Líneas.
        for line in &data.lines {
            let path = ring_path(line, &to_screen, false);
            scene.stroke(&stroke_thin, Affine::IDENTITY, palette.stroke, None, &path);
        }

        // Puntos: disco pequeño. Un radio levemente mayor si es el único
        // contenido (mapa de un solo punto), para que se vea.
        let r = if data.total_features() == 1 { 4.0 } else { 2.5 };
        for p in &data.points {
            let (x, y) = to_screen(*p);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palette.point,
                None,
                &Circle::new((x, y), r),
            );
        }

        // --- Etiquetas (encima de todo) ------------------------------
        for label in &data.labels {
            let (x, y) = to_screen(label.at);
            // Desplazada arriba-derecha del ancla para no taparla.
            let block = TextBlock::simple(&label.text, 11.0, palette.label, (x + 5.0, y - 14.0));
            draw_block(scene, ts, &block);
        }

        // --- Feature seleccionada (clic): resalte ---------------------
        if let Some(fi) = selected {
            let hl = Color::from_rgba8(255, 196, 64, 255); // ámbar, pop sobre cualquier tema
            let hl_stroke = Stroke::new(2.6);
            for (i, poly) in data.polygons.iter().enumerate() {
                if data.polygon_feat.get(i) == Some(&fi) {
                    for ring in poly {
                        let path = ring_path(ring, &to_screen, true);
                        scene.stroke(&hl_stroke, Affine::IDENTITY, hl, None, &path);
                    }
                }
            }
            for (i, line) in data.lines.iter().enumerate() {
                if data.line_feat.get(i) == Some(&fi) {
                    let path = ring_path(line, &to_screen, false);
                    scene.stroke(&hl_stroke, Affine::IDENTITY, hl, None, &path);
                }
            }
            for (i, p) in data.points.iter().enumerate() {
                if data.point_feat.get(i) == Some(&fi) {
                    let (x, y) = to_screen(*p);
                    scene.fill(
                        Fill::NonZero,
                        Affine::IDENTITY,
                        hl,
                        None,
                        &Circle::new((x, y), 5.0),
                    );
                }
            }
        }

        // --- Ruta calculada + pines de origen/destino ----------------
        if !route_path.is_empty() {
            let route_col = Color::from_rgba8(64, 220, 140, 255); // verde ruta
            let path = ring_path(&route_path, &to_screen, false);
            scene.stroke(&Stroke::new(3.2), Affine::IDENTITY, route_col, None, &path);
        }
        for (i, pin) in route_pins.iter().enumerate() {
            let (x, y) = to_screen(*pin);
            // Origen verde, destino rojo.
            let col = if i == 0 {
                Color::from_rgba8(64, 220, 140, 255)
            } else {
                Color::from_rgba8(235, 90, 70, 255)
            };
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                col,
                None,
                &Circle::new((x, y), 5.5),
            );
            scene.stroke(
                &Stroke::new(1.4),
                Affine::IDENTITY,
                Color::from_rgba8(255, 255, 255, 230),
                None,
                &Circle::new((x, y), 5.5),
            );
        }

        // --- Mobiliario cartográfico (fijo a pantalla) ---------------
        let furn = with_alpha(palette.label, 0.7);
        let furn_line = Stroke::new(1.4);
        let rx = rect.x as f64;
        let ry = rect.y as f64;
        let rw = rect.w as f64;
        let rh = rect.h as f64;

        // Lectura del centro de la vista + zoom (arriba-izquierda):
        // invierte la proyección en el centro del panel.
        let [lon_c, lat_c] = proj.inverse(pivot_x, pivot_y);
        let read = format!("{}  {}   {:.1}×", fmt_lat(lat_c), fmt_lon(lon_c), zoom);
        draw_block(
            scene,
            ts,
            &TextBlock::simple(&read, 9.5, furn, (rx + 12.0, ry + 6.0)),
        );

        // Flecha de norte (arriba-derecha): el norte siempre es arriba.
        let nx = rx + rw - 18.0;
        let ny = ry + 12.0;
        let mut arrow = BezPath::new();
        arrow.move_to((nx, ny + 15.0));
        arrow.line_to((nx, ny));
        arrow.move_to((nx - 4.0, ny + 5.0));
        arrow.line_to((nx, ny));
        arrow.line_to((nx + 4.0, ny + 5.0));
        scene.stroke(&furn_line, Affine::IDENTITY, furn, None, &arrow);
        draw_block(
            scene,
            ts,
            &TextBlock::simple("N", 9.0, furn, (nx - 3.5, ny + 15.0)),
        );

        // Barra de escala (abajo-izquierda): un segmento de distancia
        // redonda, calculado de la proyección a la latitud de la vista.
        // En equirectangular el grado de latitud mide ~constante.
        let km_per_px = 110.574 / (scale * zoom).max(1e-9);
        let nice_km = nice_125(km_per_px * 110.0);
        let bar_px = (nice_km / km_per_px).clamp(20.0, rw * 0.45);
        let bx = rx + 14.0;
        let by = ry + rh - 22.0;
        let mut bar = BezPath::new();
        bar.move_to((bx, by - 5.0));
        bar.line_to((bx, by));
        bar.line_to((bx + bar_px, by));
        bar.line_to((bx + bar_px, by - 5.0));
        scene.stroke(&furn_line, Affine::IDENTITY, furn, None, &bar);
        draw_block(
            scene,
            ts,
            &TextBlock::simple(&fmt_distance(nice_km), 9.0, furn, (bx, by - 17.0)),
        );

        // --- Leyenda del choropleth (abajo-derecha) ------------------
        if let Some((field, lo, hi)) = choro {
            draw_legend(scene, ts, (rx, ry, rw, rh), furn, field, lo, hi);
        }

        // --- Panel de propiedades de la feature seleccionada ---------
        if let Some(fp) = selected.and_then(|fi| data.features.get(fi)) {
            draw_props_panel(scene, ts, (rx, ry, rw, rh), &palette, fp);
        }
    })
}

/// Dibuja la leyenda del choropleth (abajo-derecha): nombre del campo, barra
/// de gradiente azul→ámbar→rojo y el rango `lo – hi`.
fn draw_legend(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    ts: &mut llimphi_ui::llimphi_text::Typesetter,
    rect: (f64, f64, f64, f64),
    color: Color,
    field: &str,
    lo: f64,
    hi: f64,
) {
    use llimphi_ui::llimphi_raster::kurbo::Rect as KRect;
    let (rx, ry, rw, rh) = rect;
    let lw = 130.0_f64.min(rw - 24.0);
    if lw < 60.0 {
        return;
    }
    let lx = rx + rw - lw - 12.0;
    let ly = ry + rh - 34.0;
    let segs = 24;
    let seg_w = lw / segs as f64;
    for s in 0..segs {
        let t = (s as f64 + 0.5) / segs as f64;
        let x0 = lx + s as f64 * seg_w;
        let bar = KRect::new(x0, ly, x0 + seg_w + 0.6, ly + 8.0);
        scene.fill(Fill::NonZero, Affine::IDENTITY, scale_color(t), None, &bar);
    }
    scene.stroke(
        &Stroke::new(0.8),
        Affine::IDENTITY,
        color,
        None,
        &KRect::new(lx, ly, lx + lw, ly + 8.0),
    );
    draw_block(
        scene,
        ts,
        &TextBlock::simple(&clip_text(field, 26), 9.0, color, (lx, ly - 12.0)),
    );
    let range = format!("{} – {}", fmt_num(lo), fmt_num(hi));
    draw_block(
        scene,
        ts,
        &TextBlock::simple(&range, 8.5, color, (lx, ly + 9.0)),
    );
}

/// Formatea un número: entero si es exacto, dos decimales si no.
fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v:.2}")
    }
}

/// Dibuja un panel con las propiedades de la feature seleccionada en el
/// borde derecho del lienzo. Cabecera con el nombre + hasta [`PANEL_ROWS`]
/// pares clave→valor.
fn draw_props_panel(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    ts: &mut llimphi_ui::llimphi_text::Typesetter,
    rect: (f64, f64, f64, f64),
    palette: &MapViewerPalette,
    fp: &FeatureProps,
) {
    use llimphi_ui::llimphi_raster::kurbo::RoundedRect;

    const PANEL_ROWS: usize = 12;
    let (rx, ry, rw, rh) = rect;
    let pw = 220.0_f64.min(rw - 16.0);
    if pw < 80.0 {
        return;
    }
    let rows = fp.props.len().min(PANEL_ROWS);
    let header = fp.name.clone().unwrap_or_else(|| "(feature)".to_string());
    let ph = 14.0 + 16.0 + rows as f64 * 13.0 + 8.0;
    let px = rx + rw - pw - 8.0;
    let py = (ry + 30.0).min(ry + rh - ph - 8.0).max(ry + 8.0);

    let bg = with_alpha(palette.bg, 0.92);
    let border = with_alpha(palette.grid, 0.5);
    let panel = RoundedRect::new(px, py, px + pw, py + ph, 5.0);
    scene.fill(Fill::NonZero, Affine::IDENTITY, bg, None, &panel);
    scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, border, None, &panel);

    let pad = 8.0;
    draw_block(
        scene,
        ts,
        &TextBlock::simple(
            &clip_text(&header, 30),
            11.5,
            palette.label,
            (px + pad, py + 6.0),
        ),
    );
    let key_col = with_alpha(palette.fg_muted, 0.95);
    for (i, (k, v)) in fp.props.iter().take(PANEL_ROWS).enumerate() {
        let y = py + 24.0 + i as f64 * 13.0;
        let line = format!("{}: {}", clip_text(k, 16), clip_text(v, 22));
        draw_block(
            scene,
            ts,
            &TextBlock::simple(&line, 9.5, key_col, (px + pad, y)),
        );
    }
    if fp.props.len() > PANEL_ROWS {
        let y = py + 24.0 + PANEL_ROWS as f64 * 13.0;
        let more = format!("… +{} más", fp.props.len() - PANEL_ROWS);
        draw_block(
            scene,
            ts,
            &TextBlock::simple(&more, 9.0, key_col, (px + pad, y)),
        );
    }
}

/// Recorta un texto a `max` caracteres con elipsis.
fn clip_text(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// Paso "redondo" (1·2·5 × 10ⁿ) para una rejilla que cubra `span` con unas
/// ~4–8 divisiones. Devuelve un paso positivo aun para spans degenerados.
fn nice_step(span: f64) -> f64 {
    let span = span.abs();
    if span <= 1e-9 {
        return 1.0;
    }
    let target = span / 6.0;
    let mag = 10f64.powf(target.log10().floor());
    let norm = target / mag; // 1..10
    let step = if norm < 1.5 {
        1.0
    } else if norm < 3.5 {
        2.0
    } else if norm < 7.5 {
        5.0
    } else {
        10.0
    };
    step * mag
}

/// Redondea a un valor "lindo" (1·2·5·10 × 10ⁿ) cercano a `x`, para la barra
/// de escala. Siempre positivo.
fn nice_125(x: f64) -> f64 {
    if !(x > 0.0) {
        return 1.0;
    }
    let mag = 10f64.powf(x.log10().floor());
    let n = x / mag;
    let pick = if n < 1.5 {
        1.0
    } else if n < 3.0 {
        2.0
    } else if n < 7.0 {
        5.0
    } else {
        10.0
    };
    pick * mag
}

/// Formatea una distancia: km (entero o un decimal) o metros si < 1 km.
fn fmt_distance(km: f64) -> String {
    if km >= 1.0 {
        if (km - km.round()).abs() < 1e-9 {
            format!("{} km", km as i64)
        } else {
            format!("{km:.1} km")
        }
    } else {
        format!("{} m", (km * 1000.0).round() as i64)
    }
}

/// Latitud con hemisferio (`N`/`S`).
fn fmt_lat(lat: f64) -> String {
    let h = if lat >= 0.0 { 'N' } else { 'S' };
    format!("{:.2}°{h}", lat.abs())
}

/// Longitud con hemisferio (`E`/`O`).
fn fmt_lon(lon: f64) -> String {
    let h = if lon >= 0.0 { 'E' } else { 'O' };
    format!("{:.2}°{h}", lon.abs())
}

/// Múltiplos de `step` dentro de `[lo, hi]` (incluidos), redondeando el
/// primero hacia arriba. Capada por seguridad para no iterar de más.
fn ticks(lo: f64, hi: f64, step: f64) -> Vec<f64> {
    let mut out = Vec::new();
    if step <= 0.0 || !lo.is_finite() || !hi.is_finite() {
        return out;
    }
    let first = (lo / step).ceil() * step;
    let mut v = first;
    while v <= hi + step * 1e-6 && out.len() < 64 {
        out.push(v);
        v += step;
    }
    out
}

/// Formatea un grado con la cantidad de decimales que el paso amerita
/// (pasos chicos → más decimales), con sufijo `°`.
fn fmt_deg(value: f64, step: f64) -> String {
    let decimals = if step >= 1.0 {
        0
    } else {
        // -log10(step), acotado a [1, 4].
        (-step.log10().floor() as i32).clamp(1, 4) as usize
    };
    format!("{value:.decimals$}°")
}

/// Construye un `BezPath` en coordenadas de pantalla a partir de un anillo.
/// Si `close`, cierra el contorno (para relleno/borde de polígono).
fn ring_path(ring: &[Coord], to_screen: &impl Fn(Coord) -> (f64, f64), close: bool) -> BezPath {
    let mut path = BezPath::new();
    let mut it = ring.iter();
    if let Some(first) = it.next() {
        let (x, y) = to_screen(*first);
        path.move_to((x, y));
        for c in it {
            let (x, y) = to_screen(*c);
            path.line_to((x, y));
        }
        if close {
            path.close_path();
        }
    }
    path
}

/// Body de una sola línea (estados Empty/NoGeometry/TooBig/Error).
fn simple_body<Msg>(text: &str, color: Color) -> View<Msg>
where
    Msg: Clone + 'static,
{
    View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: pad(14.0, 8.0),
        ..Default::default()
    })
    .text_aligned(text.to_string(), 12.0, color, Alignment::Start)
}

/// Padding horizontal `h` + vertical `v`.
fn pad(h: f32, v: f32) -> Rect<llimphi_ui::llimphi_layout::taffy::LengthPercentage> {
    Rect {
        left: length(h),
        right: length(h),
        top: length(v),
        bottom: length(v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_color_extremos_y_medio() {
        // Azul en 0, rojo en 1, ámbar al medio (sin pánico en bordes).
        let lo = scale_color(0.0).to_rgba8();
        let hi = scale_color(1.0).to_rgba8();
        assert!(lo.b > lo.r); // azulado
        assert!(hi.r > hi.b); // rojizo
        let _ = scale_color(0.5);
        // fuera de rango se acota.
        assert_eq!(
            scale_color(-1.0).to_rgba8().b,
            scale_color(0.0).to_rgba8().b
        );
    }

    #[test]
    fn nice_step_es_redondo() {
        assert_eq!(nice_step(60.0), 10.0);
        assert_eq!(nice_step(12.0), 2.0);
        assert_eq!(nice_step(3.0), 0.5);
        assert!(nice_step(0.0) > 0.0); // degenerado no rompe
    }

    #[test]
    fn ticks_dentro_del_rango() {
        let t = ticks(-3.0, 7.0, 2.0);
        assert_eq!(t, vec![-2.0, 0.0, 2.0, 4.0, 6.0]);
        assert!(ticks(0.0, 1.0, 0.0).is_empty()); // paso 0 no itera
    }

    #[test]
    fn fmt_deg_decimales_segun_paso() {
        assert_eq!(fmt_deg(10.0, 5.0), "10°");
        assert_eq!(fmt_deg(-16.5, 0.5), "-16.5°");
        assert_eq!(fmt_deg(0.25, 0.1), "0.2°");
    }

    #[test]
    fn nice_125_redondea() {
        assert_eq!(nice_125(1.0), 1.0);
        assert_eq!(nice_125(1.7), 2.0);
        assert_eq!(nice_125(4.0), 5.0);
        assert_eq!(nice_125(800.0), 1000.0);
        assert_eq!(nice_125(0.0), 1.0); // degenerado
    }
    #[test]
    fn fmt_distancia_km_y_m() {
        assert_eq!(fmt_distance(5.0), "5 km");
        assert_eq!(fmt_distance(2.5), "2.5 km");
        assert_eq!(fmt_distance(0.5), "500 m");
    }
    #[test]
    fn fmt_coordenadas_con_hemisferio() {
        assert_eq!(fmt_lat(-16.5), "16.50°S");
        assert_eq!(fmt_lat(40.0), "40.00°N");
        assert_eq!(fmt_lon(-70.65), "70.65°O");
        assert_eq!(fmt_lon(2.35), "2.35°E");
    }
}
