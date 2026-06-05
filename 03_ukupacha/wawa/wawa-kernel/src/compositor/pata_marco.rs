// =============================================================================
//  renaser :: kernel/src/compositor/pata_marco.rs — Fase 9 :: el marco (pata)
// -----------------------------------------------------------------------------
//  El kernel consume el MISMO `pata-core` que el frontend Llimphi en Linux: el
//  esquema declarativo (`Config` → barras + widgets), su geometría
//  (`layout::resolve`: config + pantalla → superficies colocadas) y el modelo de
//  widgets (`WidgetSpec` → `Widget` → `WidgetView`). Acá el kernel arma el
//  `Config` del marco y un `WidgetCtx` desde sus propios datos (la RAM del heap),
//  y traduce todo a sus primitivas de dibujo (`grafico::Lienzo` +
//  `texto::rasterizar`). Es la regla del repo hecha carne: un modelo, dos
//  pinceles.
//
//  Hoy pinta una **barra de menú** completa (resuelta por `pata_core::resolve`)
//  con sus tres slots: el `start_button` a la izquierda y el medidor de RAM
//  (dato real del heap) a la derecha. El config se arma en memoria; cuando llegue
//  por akasha, sólo cambia la fuente. El ruteo de input al `start_button` y más
//  widgets (reloj wall-clock, etc.) llegan después.
// =============================================================================

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use spin::{Mutex, Once};

use pata_core::layout::Rect as MarcoRect;
use pata_core::widget::{build_all, WidgetCtx, WidgetView};
use pata_core::wire::WireConfig;
use pata_core::{resolve, Anchor, Config, Surface, SurfaceKind, WidgetSpec};

use format::Hash;

use crate::almacen;
use crate::grafico::{Color, Lienzo, RegionPantalla};
use crate::texto;

/// Ancho de la barrita de un medidor, en px.
const BARRA_W: usize = 56;
/// Alto de la barrita de un medidor, en px.
const BARRA_H: usize = 6;
/// Separación entre piezas de un widget y entre widgets, en px.
const GAP: usize = 8;
/// Padding interno de la barra (borde → primer widget), en px.
const PAD: usize = 12;
/// Tamaño de fuente de las leyendas, en px.
const TAM: f32 = 16.0;
/// Grosor de la barra de menú del kernel, en px. El compositor **reserva** esta
/// franja (la resta de `area_apps`) para que las ventanas no queden bajo la
/// barra; debe coincidir con el `thickness` del Top bar del config (de ahí que
/// el config lo tome de acá). Cuando el config sea dinámico, la reserva deberá
/// leer el grosor resuelto en vez de esta constante.
pub(crate) const ALTO_BARRA: usize = 32;
/// El mismo grosor como `f32`, para el `thickness` del config.
const BARRA_ALTO: f32 = ALTO_BARRA as f32;

/// El `Config` del marco **por defecto** — declarado como en Linux (mismos
/// `kind`s). Una barra superior con el botón de inicio a la izquierda y el
/// medidor de RAM a la derecha. Es la semilla que se graba en akasha la primera
/// vez; el fallback si el grafo no la tiene o no deserializa.
fn marco_por_defecto() -> Config {
    let mut top = Surface::bar(Anchor::Top);
    top.thickness = BARRA_ALTO;
    top.start = vec![WidgetSpec::new("start_button")];
    top.end = vec![WidgetSpec::new("ram_meter")];
    let mut cfg = Config::default();
    cfg.surfaces.push(top);
    cfg
}

/// La celda del config del marco activo, cacheada tras el primer uso. Un proceso
/// de userspace puede **proponer** un config nuevo vía `sys_marco_proponer` (ver
/// [`proponer`]), que reemplaza su contenido — de ahí que sea un `Mutex` y no un
/// `Once` inmutable.
static MARCO: Once<Mutex<Config>> = Once::new();

/// La celda del config del marco. La inicializa [`cargar_inicial`] perezosamente;
/// la comparten el render (lectura) y el syscall de propuesta (escritura).
fn marco_cell() -> &'static Mutex<Config> {
    MARCO.call_once(|| Mutex::new(cargar_inicial()))
}

/// Carga el marco al arranque, **desde akasha**:
/// - si el manifiesto enlaza un marco (propuesto en un boot anterior y
///   persistido), lo lee del grafo y lo usa;
/// - si no, siembra el [`marco_por_defecto`] como nodo del grafo y **reancla el
///   manifiesto** a él (para que el próximo boot lo encuentre).
///
/// Cualquier fallo cae al default en memoria: el marco nunca se queda sin config.
fn cargar_inicial() -> Config {
    if let Some(hash) = crate::manifiesto::marco_activo() {
        if let Some(cfg) = leer_del_grafo(hash) {
            return cfg;
        }
    }
    let def = marco_por_defecto();
    if let Some(hash) = grabar_en_grafo(&def) {
        let _ = crate::manifiesto::enlazar_marco(hash);
    }
    def
}

/// Graba `cfg` en el grafo (postcard sobre su espejo [`WireConfig`]) y devuelve su
/// hash. `None` si el grafo no está listo o el codec falla.
fn grabar_en_grafo(cfg: &Config) -> Option<Hash> {
    let wire = WireConfig::from(cfg);
    let bytes = postcard::to_allocvec(&wire).ok()?;
    almacen::almacenar(bytes, Vec::new()).ok()
}

/// Lee el nodo `hash` del grafo y lo deserializa como marco. `None` si no está o
/// no es un [`WireConfig`] válido.
fn leer_del_grafo(hash: Hash) -> Option<Config> {
    let objeto = almacen::recuperar(&hash).ok()??;
    let wire: WireConfig = postcard::from_bytes(&objeto.datos).ok()?;
    Some(Config::from(wire))
}

/// El alto (px) que el marco **reserva** en la cima del área de apps: la suma de
/// los grosores de las barras `Bar` superiores no-`autohide` del config activo.
/// El compositor lo descuenta de `area_apps` para que las ventanas no queden bajo
/// la barra. Lee el config **resuelto** —si una app propone una barra de otro
/// alto vía `sys_marco_proponer`, la reserva lo sigue—, no una constante.
pub(crate) fn alto_reservado() -> usize {
    let cfg = marco_cell().lock();
    cfg.surfaces
        .iter()
        .filter(|s| s.kind == SurfaceKind::Bar && !s.autohide && s.anchor == Anchor::Top)
        .map(|s| s.thickness.max(0.0) as usize)
        .sum()
}

/// **Propone** un config nuevo desde userspace: los `bytes` son un [`WireConfig`]
/// serializado con postcard (el espejo postcard-safe). El kernel lo deserializa
/// (validándolo), lo re-serializa canónico y lo **graba en el grafo**
/// direccionado por contenido, y reemplaza el marco activo — el próximo frame
/// pinta el nuevo. Lo invoca la capacidad WASM `sys_marco_proponer`. Devuelve
/// error si los bytes no son un `WireConfig` válido o el grafo no acepta el nodo;
/// en ese caso el marco activo no cambia.
pub(crate) fn proponer(bytes: &[u8]) -> Result<(), &'static str> {
    let wire: WireConfig =
        postcard::from_bytes(bytes).map_err(|_| "marco :: config propuesto invalido")?;
    // Re-serializar desde el wire ya parseado garantiza un nodo canónico.
    let canon = postcard::to_allocvec(&wire).map_err(|_| "marco :: serializacion fallida")?;
    let hash = almacen::almacenar(canon, Vec::new())?;
    // Persistir: reanclar el manifiesto al nodo nuevo, así el marco propuesto
    // sobrevive al reinicio (no sólo a la sesión).
    crate::manifiesto::enlazar_marco(hash)?;
    *marco_cell().lock() = Config::from(wire);
    Ok(())
}

/// El snapshot del sistema que el kernel le entrega a los widgets de `pata-core`.
/// Hoy sólo la RAM (usado/total del heap); el resto queda en cero (un widget que
/// dependa de un campo en cero se ve vacío, no rompe).
pub(crate) fn ctx_kernel() -> WidgetCtx {
    let (usado, total) = crate::memory::allocator::stats();
    let total = total.max(1);
    let mib = 1024 * 1024;
    WidgetCtx {
        ram: (usado as f32 / total as f32).clamp(0.0, 1.0),
        ram_used_mb: (usado / mib) as u32,
        ram_total_mb: (total / mib) as u32,
        ..WidgetCtx::default()
    }
}

/// Mezcla `fondo`→`tinta` según una cobertura de glifo `0..=255` (anti-aliasing).
fn mezclar(fondo: Color, tinta: Color, cobertura: u8) -> Color {
    let c = cobertura as u16;
    let inv = 255 - c;
    let mez = |f: u8, t: u8| ((f as u16 * inv + t as u16 * c) / 255) as u8;
    Color {
        r: mez(fondo.r, tinta.r),
        g: mez(fondo.g, tinta.g),
        b: mez(fondo.b, tinta.b),
    }
}

/// Ancho en px de una cadena al tamaño [`TAM`] (suma de avances de glifo).
fn medir_texto(s: &str) -> usize {
    s.chars()
        .map(|c| texto::rasterizar(c, TAM).0.advance_width as usize)
        .sum()
}

/// Funde una cadena sobre el lienzo en `(x, base_y)`, sobre un fondo conocido —
/// mismo patrón que `consola::pintar_etiqueta`, sin estado de pluma.
fn dibujar_texto(lienzo: &mut Lienzo, x: usize, base_y: usize, s: &str, fondo: Color, tinta: Color) {
    let mut cursor = x;
    for caracter in s.chars() {
        let (m, cob) = texto::rasterizar(caracter, TAM);
        let inicio_x = cursor as isize + m.xmin as isize;
        let inicio_y = base_y as isize - m.ymin as isize - m.height as isize;
        for fila in 0..m.height {
            for col in 0..m.width {
                let opacidad = cob[fila * m.width + col];
                if opacidad == 0 {
                    continue;
                }
                let px = inicio_x + col as isize;
                let py = inicio_y + fila as isize;
                if px < 0 || py < 0 {
                    continue;
                }
                lienzo.pintar_pixel(px as usize, py as usize, mezclar(fondo, tinta, opacidad));
            }
        }
        cursor += m.advance_width as usize;
    }
}

/// Ancho en px que ocupará un view-model al pintarse.
fn medir_vista(v: &WidgetView) -> usize {
    match v {
        WidgetView::Empty => 0,
        WidgetView::Text(t) | WidgetView::Placeholder(t) => medir_texto(t),
        WidgetView::Meter { label, caption, .. } => {
            let mut w = 0;
            if let Some(l) = label {
                w += medir_texto(l) + GAP;
            }
            w += BARRA_W + GAP + medir_texto(caption);
            w
        }
        WidgetView::Workspaces { count, .. } => {
            let n = *count as usize;
            let celdas: usize = (1..=*count).map(ancho_celda_ws).sum();
            celdas + GAP_CELDA * n.saturating_sub(1)
        }
    }
}

/// Padding horizontal de una celda del switcher (a cada lado del número).
const PAD_CELDA: usize = 6;
/// Separación entre celdas del switcher (más compacta que [`GAP`]).
const GAP_CELDA: usize = 4;

/// Ancho en px de la celda del escritorio `n`: el número más el padding.
fn ancho_celda_ws(n: u8) -> usize {
    medir_texto(&format!("{n}")) + PAD_CELDA * 2
}

/// El ancho total (con gaps) de una secuencia de vistas.
fn ancho_total(vistas: &[WidgetView]) -> usize {
    let suma: usize = vistas.iter().map(medir_vista).sum();
    suma + GAP * vistas.len().saturating_sub(1)
}

/// Pinta un view-model en `(x, …)` dentro de `region`, sobre `fondo`. Devuelve el
/// ancho pintado (para avanzar al siguiente widget).
fn pintar_vista(lienzo: &mut Lienzo, v: &WidgetView, x: usize, region: RegionPantalla, fondo: Color) -> usize {
    let base_y = region.y + (region.alto + 14) / 2;
    match v {
        WidgetView::Empty => 0,
        WidgetView::Text(t) | WidgetView::Placeholder(t) => {
            dibujar_texto(lienzo, x, base_y, t, fondo, Color::TEXTO);
            medir_texto(t)
        }
        WidgetView::Meter { label, fraction, caption } => {
            let mut cur = x;
            if let Some(l) = label {
                dibujar_texto(lienzo, cur, base_y, l, fondo, Color::TEXTO);
                cur += medir_texto(l) + GAP;
            }
            // Pista (track) + relleno proporcional. El framebuffer es color
            // sólido (no hay gradiente como en Llimphi/vello).
            let barra_y = region.y + region.alto.saturating_sub(BARRA_H) / 2;
            lienzo.rellenar_rect(cur, barra_y, BARRA_W, BARRA_H, Color::SIN_FOCO);
            let relleno = (BARRA_W as f32 * fraction.clamp(0.0, 1.0)) as usize;
            lienzo.rellenar_rect(cur, barra_y, relleno, BARRA_H, Color::FOCO);
            cur += BARRA_W + GAP;
            dibujar_texto(lienzo, cur, base_y, caption, fondo, Color::TEXTO);
            cur += medir_texto(caption);
            cur - x
        }
        WidgetView::Workspaces { active, count, occupied } => {
            // Una celda por escritorio: la activa con fondo de foco, las ocupadas
            // con el gris de "sin foco", las vacías sobre el fondo de la barra.
            // El framebuffer no rutea clicks — es display; el cambio sigue por
            // atajo o por la celda en el frontend Llimphi.
            let mut cur = x;
            let celda_h = region.alto.saturating_sub(8);
            let cy = region.y + 4;
            for n in 1..=*count {
                let num = format!("{n}");
                let w = medir_texto(&num) + PAD_CELDA * 2;
                let ocupado = occupied & (1u16 << (n as u16 - 1)) != 0;
                let bg = if n == *active {
                    Color::FOCO
                } else if ocupado {
                    Color::SIN_FOCO
                } else {
                    fondo
                };
                lienzo.rellenar_rect(cur, cy, w, celda_h, bg);
                dibujar_texto(lienzo, cur + PAD_CELDA, base_y, &num, bg, Color::TEXTO);
                cur += w + GAP_CELDA;
            }
            cur.saturating_sub(x).saturating_sub(GAP_CELDA)
        }
    }
}

/// Pinta una secuencia de vistas de izquierda a derecha desde `x0`.
fn pintar_secuencia(lienzo: &mut Lienzo, vistas: &[WidgetView], x0: usize, region: RegionPantalla, fondo: Color) {
    let mut x = x0;
    for v in vistas {
        let w = pintar_vista(lienzo, v, x, region, fondo);
        x += w + GAP;
    }
}

/// Construye, `tick`ea y proyecta a `WidgetView` los widgets de un slot.
fn vistas(specs: &[WidgetSpec], ctx: &WidgetCtx) -> Vec<WidgetView> {
    let mut widgets = build_all(specs);
    for w in &mut widgets {
        w.tick(ctx);
    }
    widgets.iter().map(|w| w.view()).collect()
}

/// Pinta una barra resuelta: fondo + separador + sus tres slots
/// (start a la izquierda, center centrado, end a la derecha).
fn pintar_barra(lienzo: &mut Lienzo, rect: MarcoRect, s: &Surface, ctx: &WidgetCtx, fondo: Color) {
    let (x, y) = (rect.x.max(0) as usize, rect.y.max(0) as usize);
    let (ancho, alto) = (rect.w.max(0) as usize, rect.h.max(0) as usize);
    if ancho == 0 || alto == 0 {
        return;
    }
    let region = RegionPantalla { x, y, ancho, alto };
    // Fondo de la barra + línea de separación inferior (cromo).
    lienzo.rellenar_rect(x, y, ancho, alto, fondo);
    lienzo.rellenar_rect(x, y + alto.saturating_sub(1), ancho, 1, Color::SIN_FOCO);

    let vs_start = vistas(&s.start, ctx);
    let vs_center = vistas(&s.center, ctx);
    let vs_end = vistas(&s.end, ctx);

    // start: pegado al borde izquierdo.
    pintar_secuencia(lienzo, &vs_start, x + PAD, region, fondo);
    // center: centrado en el ancho de la barra.
    let wc = ancho_total(&vs_center);
    let cx = x + ancho.saturating_sub(wc) / 2;
    pintar_secuencia(lienzo, &vs_center, cx, region, fondo);
    // end: pegado al borde derecho.
    let we = ancho_total(&vs_end);
    let ex = x + ancho.saturating_sub(PAD + we);
    pintar_secuencia(lienzo, &vs_end, ex, region, fondo);
}

/// Pinta el **marco completo** de `pata` sobre `area`: resuelve el `Config` del
/// kernel con `pata_core::resolve` (la geometría canónica que comparten Linux y
/// wawa) y pinta cada barra resuelta en su rect, con sus tres slots. La llama el
/// compositor (`consola::recomponer`) tras componer el escritorio.
pub(crate) fn pintar_marco(lienzo: &mut Lienzo, area: RegionPantalla) {
    let cfg = marco_cell().lock();
    let pantalla = MarcoRect::new(
        area.x as i32,
        area.y as i32,
        area.ancho as i32,
        area.alto as i32,
    );
    let frame = resolve(&cfg, pantalla);
    let ctx = ctx_kernel();
    for placed in &frame.surfaces {
        let s = &cfg.surfaces[placed.index];
        if s.kind != SurfaceKind::Bar || !placed.rect.es_visible() {
            continue;
        }
        pintar_barra(lienzo, placed.rect, s, &ctx, Color::PANEL);
    }
}

/// El rectángulo **clickeable** del `start_button` del marco sobre `area`, o
/// `None` si el config no lo tiene. Lo usa el ratón del compositor para abrir el
/// launcher al clickear el ⊞. Resuelve el mismo `Config` que [`pintar_marco`] y
/// ubica el primer widget del slot `start` de la barra (que es el botón), con un
/// target generoso (el padding + el glifo). Espeja la posición que pinta
/// [`pintar_barra`] (start pegado al borde izquierdo, en `bar.x + PAD`).
pub(crate) fn start_button_rect(area: RegionPantalla) -> Option<RegionPantalla> {
    let cfg = marco_cell().lock();
    let pantalla = MarcoRect::new(
        area.x as i32,
        area.y as i32,
        area.ancho as i32,
        area.alto as i32,
    );
    let frame = resolve(&cfg, pantalla);
    let ctx = ctx_kernel();
    for placed in &frame.surfaces {
        let s = &cfg.surfaces[placed.index];
        if s.kind != SurfaceKind::Bar || !placed.rect.es_visible() {
            continue;
        }
        match s.start.first() {
            Some(spec) if spec.kind == "start_button" => {}
            _ => continue,
        }
        let ancho_glifo = vistas(&s.start, &ctx)
            .first()
            .map(medir_vista)
            .unwrap_or(0);
        let bx = placed.rect.x.max(0) as usize;
        let by = placed.rect.y.max(0) as usize;
        let bh = placed.rect.h.max(0) as usize;
        // Desde el borde izquierdo (incluye el padding) hasta pasado el glifo.
        return Some(RegionPantalla {
            x: bx,
            y: by,
            ancho: PAD + ancho_glifo + GAP,
            alto: bh,
        });
    }
    None
}
