//! Tile AstroCarto — MC/IC + Asc/Desc líneas sobre un mapa equirectangular.
//!
//! MVP sin fondo de continentes — solo grilla lat/long y las líneas de los
//! cuerpos clásicos. La aproximación supone latitud eclíptica β=0 para
//! todos los cuerpos (válido para AstroCarto a este zoom; Luna y Plutón
//! se separan unos grados pero la silueta de líneas es la misma). La
//! obliquidad usa ε₂₀₀₀ = 23.4393° fijo — el error a 100 años es <0.01°.

use cosmos_model::Chart;
use cosmos_render::{LayerKind, RenderModel};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, Size, Style};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::View;

use crate::model::Msg;
use crate::view::line;

const ASTROCARTO_OBLIQUITY: f64 = 23.4393;
const ASTROCARTO_W: f32 = 320.0;
const ASTROCARTO_H: f32 = 160.0;

fn julian_day_utc(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: f64) -> f64 {
    let (y, m) = if month <= 2 {
        (year - 1, (month + 12) as i32)
    } else {
        (year, month as i32)
    };
    let a = (y as f64 / 100.0).floor();
    let b = 2.0 - a + (a / 4.0).floor();
    let jd0 = (365.25 * (y as f64 + 4716.0)).floor()
        + (30.6001 * (m as f64 + 1.0)).floor()
        + day as f64
        + b
        - 1524.5;
    let frac = (hour as f64 + minute as f64 / 60.0 + second / 3600.0) / 24.0;
    jd0 + frac
}

/// GMST en grados [0, 360) — Meeus 12.4.
fn gmst_deg(jd_ut: f64) -> f64 {
    let t = (jd_ut - 2451545.0) / 36525.0;
    let g = 280.46061837
        + 360.98564736629 * (jd_ut - 2451545.0)
        + 0.000387933 * t * t
        - t * t * t / 38710000.0;
    g.rem_euclid(360.0)
}

/// Conversión ecliptica → ecuatorial con β=0 fijo. Retorna (RA°, Dec°).
fn ecliptic_to_equatorial(lon_deg: f64) -> (f64, f64) {
    let l = lon_deg.to_radians();
    let e = ASTROCARTO_OBLIQUITY.to_radians();
    let ra = (l.sin() * e.cos()).atan2(l.cos()).to_degrees().rem_euclid(360.0);
    let dec = (e.sin() * l.sin()).asin().to_degrees();
    (ra, dec)
}

/// Color por cuerpo en el AstroCarto. Hue distintivo para que las líneas
/// se diferencien aun cuando se cruzan.
fn color_de_cuerpo(name: &str) -> Color {
    match name {
        "sun" => Color::from_rgba8(255, 200, 60, 255),
        "moon" => Color::from_rgba8(200, 210, 220, 255),
        "mercury" => Color::from_rgba8(180, 180, 180, 255),
        "venus" => Color::from_rgba8(120, 220, 130, 255),
        "mars" => Color::from_rgba8(230, 90, 90, 255),
        "jupiter" => Color::from_rgba8(240, 170, 80, 255),
        "saturn" => Color::from_rgba8(180, 150, 90, 255),
        "uranus" => Color::from_rgba8(100, 220, 220, 255),
        "neptune" => Color::from_rgba8(100, 130, 230, 255),
        "pluto" => Color::from_rgba8(170, 90, 130, 255),
        _ => Color::from_rgba8(140, 140, 140, 255),
    }
}

/// Proyección equirectangular a coordenadas locales del canvas.
fn project_lon_lat(lon_deg: f64, lat_deg: f64) -> (f32, f32) {
    let x = ((lon_deg + 180.0) / 360.0) as f32 * ASTROCARTO_W;
    let y = ((90.0 - lat_deg) / 180.0) as f32 * ASTROCARTO_H;
    (x.clamp(0.0, ASTROCARTO_W), y.clamp(0.0, ASTROCARTO_H))
}

pub(crate) fn tile_astrocarto(
    chart: &Chart,
    render: &RenderModel,
    theme: &Theme,
    zoom: f32,
    pan: (f32, f32),
    rect_cell: std::sync::Arc<std::sync::Mutex<Option<(f32, f32, f32, f32)>>>,
) -> View<Msg> {
    let bd = &chart.birth_data;
    // Local → UTC.
    let total_minutes_local = bd.hour as i64 * 60 + bd.minute as i64;
    let total_minutes_utc = total_minutes_local - bd.tz_offset_minutes as i64;
    let h_utc = total_minutes_utc as f64 / 60.0;
    let jd = julian_day_utc(bd.year, bd.month, bd.day, 0, 0, bd.second) + h_utc / 24.0;
    let gmst = gmst_deg(jd);
    // Cosas owned para meter dentro de la closure 'static.
    let natal_lat = bd.latitude_deg;
    let natal_lon = bd.longitude_deg;

    // Cuerpos natales con su longitud eclíptica.
    let bodies: Vec<(String, f64)> = render
        .layers
        .iter()
        .filter(|l| l.module_id == "natal" && matches!(l.kind, LayerKind::Bodies))
        .flat_map(|l| l.glyphs.iter())
        .map(|g| (g.symbol.clone(), g.deg as f64))
        .collect();

    let bg = theme.bg_panel_alt;
    let grid = theme.fg_muted;
    let zoom = zoom.max(0.1) as f64;
    let pan = (pan.0 as f64, pan.1 as f64);
    let canvas = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(0.0_f32),
        },
        flex_grow: 1.0,
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(bg)
    .radius(3.0)
    .clip(true)
    // Arrastrar panea el mapa (la rueda hace zoom vía App::on_wheel).
    .draggable_at(|phase, dx, dy, _lx, _ly| match phase {
        llimphi_ui::DragPhase::Move => Some(Msg::WheelPan(dx, dy)),
        llimphi_ui::DragPhase::End => None,
    })
    .paint_with(move |scene, ts, rect: llimphi_ui::PaintRect| {
        use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Point, Stroke};
        use llimphi_ui::llimphi_raster::peniko::Color as PColor;
        use llimphi_ui::llimphi_text::{draw_layout, layout_block, Alignment, TextBlock};
        // Deja el rect del lienzo para que `on_wheel` haga zoom al cursor.
        if let Ok(mut g) = rect_cell.lock() {
            *g = Some((rect.x, rect.y, rect.w, rect.h));
        }
        // Aspect-fit centrado + zoom/paneo del usuario.
        let scale_x = rect.w as f64 / ASTROCARTO_W as f64;
        let scale_y = rect.h as f64 / ASTROCARTO_H as f64;
        let scale = scale_x.min(scale_y) * zoom;
        let disp_w = ASTROCARTO_W as f64 * scale;
        let disp_h = ASTROCARTO_H as f64 * scale;
        let off_x = rect.x as f64 + (rect.w as f64 - disp_w) * 0.5 + pan.0;
        let off_y = rect.y as f64 + (rect.h as f64 - disp_h) * 0.5 + pan.1;
        let xform = Affine::translate((off_x, off_y)) * Affine::scale(scale);
        // Grosor de trazo medido en PÍXELES de pantalla: las líneas no
        // engordan con el zoom (el `scale` las inflaría), apenas crecen
        // un pelo (zoom^0.15) para acompañar el acercamiento. Como la
        // escena va escalada por `scale`, dividimos por `scale` para que
        // el ancho final en pantalla sea el pedido.
        let px_w = move |screen_px: f64| screen_px * zoom.powf(0.15) / scale;

        // Mapa de fondo: continentes (world-countries.geojson vía
        // nahual-geo-core). Relleno tenue de tierra + contorno de costas.
        let land_fill = PColor::from_rgba8(
            (grid.components[0] * 255.0) as u8,
            (grid.components[1] * 255.0) as u8,
            (grid.components[2] * 255.0) as u8,
            38,
        );
        let coast = PColor::from_rgba8(
            (grid.components[0] * 255.0) as u8,
            (grid.components[1] * 255.0) as u8,
            (grid.components[2] * 255.0) as u8,
            150,
        );
        for poly in &nahual_geo_core::world_base().polygons {
            for ring in poly {
                if ring.len() < 2 {
                    continue;
                }
                let mut path = BezPath::new();
                for (i, c) in ring.iter().enumerate() {
                    let (x, y) = project_lon_lat(c[0], c[1]);
                    if i == 0 {
                        path.move_to((x as f64, y as f64));
                    } else {
                        path.line_to((x as f64, y as f64));
                    }
                }
                path.close_path();
                scene.fill(
                    llimphi_ui::llimphi_raster::peniko::Fill::NonZero,
                    xform,
                    land_fill,
                    None,
                    &path,
                );
                scene.stroke(&Stroke::new(px_w(0.6)), xform, coast, None, &path);
            }
        }

        // Grilla (graticule). El acercamiento ABRE detalle: aparece una
        // grilla fina entre las líneas mayores. Mayores cada 30°/30°;
        // las finas cada 10° (zoom ≥ 2.5) o 5° (zoom ≥ 5).
        let grid_color = PColor::from_rgba8(
            (grid.components[0] * 255.0) as u8,
            (grid.components[1] * 255.0) as u8,
            (grid.components[2] * 255.0) as u8,
            80,
        );
        let minor_color = PColor::from_rgba8(
            (grid.components[0] * 255.0) as u8,
            (grid.components[1] * 255.0) as u8,
            (grid.components[2] * 255.0) as u8,
            38,
        );
        let minor_step = if zoom >= 5.0 {
            5.0_f64
        } else if zoom >= 2.5 {
            10.0
        } else {
            0.0
        };
        // Paralelos finos.
        if minor_step > 0.0 {
            let mut lat = -85.0_f64;
            while lat <= 85.0 {
                if (lat / 30.0).fract().abs() > 1e-6 {
                    let (_, y) = project_lon_lat(0.0, lat);
                    let mut p = BezPath::new();
                    p.move_to((0.0, y as f64));
                    p.line_to((ASTROCARTO_W as f64, y as f64));
                    scene.stroke(&Stroke::new(px_w(0.4)), xform, minor_color, None, &p);
                }
                lat += minor_step;
            }
            let mut lon = -180.0_f64;
            while lon <= 180.0 {
                if (lon / 30.0).fract().abs() > 1e-6 {
                    let (x, _) = project_lon_lat(lon, 0.0);
                    let mut p = BezPath::new();
                    p.move_to((x as f64, 0.0));
                    p.line_to((x as f64, ASTROCARTO_H as f64));
                    scene.stroke(&Stroke::new(px_w(0.4)), xform, minor_color, None, &p);
                }
                lon += minor_step;
            }
        }
        // Paralelos mayores cada 30°.
        for lat in [-60.0_f64, -30.0, 0.0, 30.0, 60.0] {
            let (_, y) = project_lon_lat(0.0, lat);
            let mut p = BezPath::new();
            p.move_to((0.0, y as f64));
            p.line_to((ASTROCARTO_W as f64, y as f64));
            let w = if lat.abs() < 0.5 { px_w(0.9) } else { px_w(0.5) };
            scene.stroke(&Stroke::new(w), xform, grid_color, None, &p);
        }
        // Meridianos mayores cada 30°.
        let mut lon = -150.0_f64;
        while lon <= 180.0 {
            let (x, _) = project_lon_lat(lon, 0.0);
            let mut p = BezPath::new();
            p.move_to((x as f64, 0.0));
            p.line_to((x as f64, ASTROCARTO_H as f64));
            let w = if lon.abs() < 0.5 { px_w(0.9) } else { px_w(0.5) };
            scene.stroke(&Stroke::new(w), xform, grid_color, None, &p);
            lon += 30.0;
        }

        for (name, ecl_lon) in &bodies {
            let (ra, dec) = ecliptic_to_equatorial(*ecl_lon);
            let mc_lon = wrap_lon(ra - gmst);
            let ic_lon = wrap_lon(mc_lon + 180.0);
            let body_color = color_de_cuerpo(name);

            // MC: línea vertical a lo largo del canvas.
            let (x_mc, _) = project_lon_lat(mc_lon, 0.0);
            let mut p = BezPath::new();
            p.move_to((x_mc as f64, 0.0));
            p.line_to((x_mc as f64, ASTROCARTO_H as f64));
            scene.stroke(&Stroke::new(px_w(1.5)), xform, body_color, None, &p);

            // IC: línea vertical punteada para distinguir.
            let (x_ic, _) = project_lon_lat(ic_lon, 0.0);
            let mut p = BezPath::new();
            p.move_to((x_ic as f64, 0.0));
            p.line_to((x_ic as f64, ASTROCARTO_H as f64));
            scene.stroke(
                &Stroke::new(px_w(1.1)).with_dashes(0.0, [px_w(4.0), px_w(4.0)]),
                xform,
                body_color,
                None,
                &p,
            );

            // Asc/Desc: curvas paramétricas en latitud.
            if dec.abs() < 89.9 {
                let mut rise = BezPath::new();
                let mut set = BezPath::new();
                let mut rise_started = false;
                let mut set_started = false;
                let dec_r = dec.to_radians();
                let mut phi_deg = -85.0_f64;
                while phi_deg <= 85.0 {
                    let phi_r = phi_deg.to_radians();
                    let cos_h = -phi_r.tan() * dec_r.tan();
                    if cos_h.abs() <= 1.0 {
                        let h_deg = cos_h.acos().to_degrees();
                        let lon_r = wrap_lon(ra - h_deg - gmst);
                        let lon_s = wrap_lon(ra + h_deg - gmst);
                        let (xr, yr) = project_lon_lat(lon_r, phi_deg);
                        let (xs, ys) = project_lon_lat(lon_s, phi_deg);
                        if rise_started {
                            rise.line_to((xr as f64, yr as f64));
                        } else {
                            rise.move_to((xr as f64, yr as f64));
                            rise_started = true;
                        }
                        if set_started {
                            set.line_to((xs as f64, ys as f64));
                        } else {
                            set.move_to((xs as f64, ys as f64));
                            set_started = true;
                        }
                    } else if rise_started || set_started {
                        // Cruzamos región circumpolar — corta la línea.
                        rise_started = false;
                        set_started = false;
                    }
                    phi_deg += 3.0;
                }
                if rise_started {
                    scene.stroke(&Stroke::new(px_w(0.9)), xform, body_color, None, &rise);
                }
                if set_started {
                    scene.stroke(&Stroke::new(px_w(0.9)), xform, body_color, None, &set);
                }
            }
        }

        // Marca del lugar de nacimiento.
        let (px, py) = project_lon_lat(natal_lon, natal_lat);
        let mark = llimphi_ui::llimphi_raster::kurbo::Circle::new(
            (px as f64, py as f64),
            px_w(3.0),
        );
        scene.fill(
            llimphi_ui::llimphi_raster::peniko::Fill::NonZero,
            xform,
            PColor::from_rgba8(255, 255, 255, 230),
            None,
            &mark,
        );

        // Etiquetas de coordenadas — tamaño FIJO en pantalla (no escalan
        // con el zoom): se dibujan en coordenadas de pantalla aplicando
        // `xform` al punto del mapa. Paralelos sobre el borde izquierdo,
        // meridianos sobre el borde inferior del mapa.
        let label_col = PColor::from_rgba8(
            (grid.components[0] * 255.0) as u8,
            (grid.components[1] * 255.0) as u8,
            (grid.components[2] * 255.0) as u8,
            210,
        );
        let draw_label = |scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
                              ts: &mut llimphi_ui::llimphi_text::Typesetter,
                              wx: f64,
                              wy: f64,
                              text: &str| {
            let sp = xform * Point::new(wx, wy);
            let block = TextBlock {
                text,
                size_px: 9.5,
                color: label_col,
                origin: (sp.x + 2.0, sp.y - 5.0),
                max_width: None,
                alignment: Alignment::Start,
                line_height: 1.0,
                italic: false,
                font_family: None,
            };
            let layout = layout_block(ts, &block);
            draw_layout(scene, &layout, label_col, block.origin);
        };
        for lat in [-60.0_f64, -30.0, 0.0, 30.0, 60.0] {
            let (_, y) = project_lon_lat(2.0, lat);
            let txt = if lat.abs() < 0.5 {
                "Ec.".to_string()
            } else if lat > 0.0 {
                format!("{}°N", lat as i32)
            } else {
                format!("{}°S", (-lat) as i32)
            };
            draw_label(scene, ts, 2.0, y as f64, &txt);
        }
        for lon in [-120.0_f64, -60.0, 0.0, 60.0, 120.0] {
            let (x, _) = project_lon_lat(lon, -86.0);
            let txt = if lon.abs() < 0.5 {
                "0°".to_string()
            } else if lon > 0.0 {
                format!("{}°E", lon as i32)
            } else {
                format!("{}°O", (-lon) as i32)
            };
            draw_label(scene, ts, x as f64, 158.0, &txt);
        }
    });

    // Columna a alto completo: el lienzo ocupa todo el espacio (base más
    // grande), la leyenda abajo.
    let legend = line(
        rimay_localize::t("cosmos-astrocarto-leyenda"),
        9.0,
        theme.fg_muted,
    );
    View::new(Style {
        flex_direction: llimphi_ui::llimphi_layout::taffy::prelude::FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        padding: llimphi_ui::llimphi_layout::taffy::Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(8.0_f32),
            bottom: length(6.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(4.0_f32),
        },
        ..Default::default()
    })
    .children(vec![canvas, legend])
}

fn wrap_lon(lon: f64) -> f64 {
    let l = lon.rem_euclid(360.0);
    if l > 180.0 {
        l - 360.0
    } else {
        l
    }
}
