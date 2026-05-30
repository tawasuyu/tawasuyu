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
use crate::view::{line, tile_container};

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

pub(crate) fn tile_astrocarto(chart: &Chart, render: &RenderModel, theme: &Theme) -> View<Msg> {
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
    let canvas = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(ASTROCARTO_H + 4.0),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(bg)
    .radius(3.0)
    .paint_with(move |scene, _ts, rect: llimphi_ui::PaintRect| {
        use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Stroke};
        use llimphi_ui::llimphi_raster::peniko::Color as PColor;
        // Aspect-fit centrado del canvas lógico ASTROCARTO_WxH al rect.
        let scale_x = rect.w as f64 / ASTROCARTO_W as f64;
        let scale_y = rect.h as f64 / ASTROCARTO_H as f64;
        let scale = scale_x.min(scale_y);
        let disp_w = ASTROCARTO_W as f64 * scale;
        let disp_h = ASTROCARTO_H as f64 * scale;
        let off_x = rect.x as f64 + (rect.w as f64 - disp_w) * 0.5;
        let off_y = rect.y as f64 + (rect.h as f64 - disp_h) * 0.5;
        let xform = Affine::translate((off_x, off_y)) * Affine::scale(scale);

        // Grilla: ecuador, ±30°, ±60°.
        let grid_color = PColor::from_rgba8(
            (grid.components[0] * 255.0) as u8,
            (grid.components[1] * 255.0) as u8,
            (grid.components[2] * 255.0) as u8,
            80,
        );
        for lat in [-60.0_f64, -30.0, 0.0, 30.0, 60.0] {
            let (_, y) = project_lon_lat(0.0, lat);
            let mut p = BezPath::new();
            p.move_to((0.0, y as f64));
            p.line_to((ASTROCARTO_W as f64, y as f64));
            scene.stroke(&Stroke::new(0.5), xform, grid_color, None, &p);
        }
        // Líneas verticales cada 60° de longitud.
        for lon in [-120.0_f64, -60.0, 0.0, 60.0, 120.0] {
            let (x, _) = project_lon_lat(lon, 0.0);
            let mut p = BezPath::new();
            p.move_to((x as f64, 0.0));
            p.line_to((x as f64, ASTROCARTO_H as f64));
            scene.stroke(&Stroke::new(0.5), xform, grid_color, None, &p);
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
            scene.stroke(&Stroke::new(1.4), xform, body_color, None, &p);

            // IC: línea vertical punteada para distinguir.
            let (x_ic, _) = project_lon_lat(ic_lon, 0.0);
            let mut p = BezPath::new();
            p.move_to((x_ic as f64, 0.0));
            p.line_to((x_ic as f64, ASTROCARTO_H as f64));
            scene.stroke(
                &Stroke::new(1.0).with_dashes(0.0, [4.0, 4.0]),
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
                    scene.stroke(&Stroke::new(0.8), xform, body_color, None, &rise);
                }
                if set_started {
                    scene.stroke(&Stroke::new(0.8), xform, body_color, None, &set);
                }
            }
        }

        // Marca del lugar de nacimiento.
        let (px, py) = project_lon_lat(natal_lon, natal_lat);
        let mark = llimphi_ui::llimphi_raster::kurbo::Circle::new(
            (px as f64, py as f64),
            3.0,
        );
        scene.fill(
            llimphi_ui::llimphi_raster::peniko::Fill::NonZero,
            xform,
            PColor::from_rgba8(255, 255, 255, 230),
            None,
            &mark,
        );
    });

    tile_container(
        vec![
            canvas,
            line(
                rimay_localize::t("cosmos-astrocarto-leyenda"),
                9.0,
                theme.fg_muted,
            ),
        ],
        theme,
    )
}

fn wrap_lon(lon: f64) -> f64 {
    let l = lon.rem_euclid(360.0);
    if l > 180.0 {
        l - 360.0
    } else {
        l
    }
}
