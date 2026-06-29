//! tullpu-paint -- kernel de pintura buffer-puro, agnóstico de GUI.
//!
//! Primitivas deterministas sobre buffers Rgba8 (`[u8]`, 4 bytes/píxel) y
//! máscaras alfa de un canal (1 byte/píxel): recortes, rellenos de rect,
//! flood fill, estampado de disco del pincel, líneas, degradés, espejo de
//! ejes, composición src-over y rotaciones de 90°. Sin `Model`, sin almacén,
//! sin historial, sin render: sólo bytes adentro -> bytes afuera (algunas
//! mutan `buf` in-place). Extraído de `tullpu-app-llimphi/src/ops.rs`
//! (regla #2: el motor de pintura vive en un core agnóstico, no en el
//! frontend) -- behavior-preserving.

/// Construye un buffer Rgba8 de `w × h` lleno con `rgba`. Pura. Salvo
/// errores de overflow (improbables en tamaños sanos), el `w * h * 4`
/// nunca pasa de unos MB para los lienzos típicos de tullpu.
pub fn buffer_relleno(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let mut v = Vec::with_capacity((w as usize) * (h as usize) * 4);
    for _ in 0..(w as usize * h as usize) {
        v.extend_from_slice(&rgba);
    }
    v
}

/// Calcula el bounding box (half-open `(x0, y0, x1, y1)`) de los píxeles
/// con alfa > 0 en un buffer Rgba8 `w × h`. Devuelve `None` si todos
/// los píxeles son transparentes (no hay nada para encerrar). Pura.
pub fn bbox_no_transparente(data: &[u8], w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
    if w == 0 || h == 0 || data.len() != (w as usize) * (h as usize) * 4 {
        return None;
    }
    let mut min_x = u32::MAX;
    let mut min_y = u32::MAX;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    let mut found = false;
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            // Alfa estricto > 0; algunos pipelines premultiplican y dejan
            // valores 1..3 en bordes — eso sigue contando como "tinta".
            if data[i + 3] > 0 {
                found = true;
                if x < min_x {
                    min_x = x;
                }
                if y < min_y {
                    min_y = y;
                }
                if x > max_x {
                    max_x = x;
                }
                if y > max_y {
                    max_y = y;
                }
            }
        }
    }
    if !found {
        return None;
    }
    // Convención half-open: x1/y1 son exclusivos. Suma 1 al máximo
    // observado para que `x1 - x0` sea el ancho efectivo.
    Some((min_x, min_y, max_x + 1, max_y + 1))
}

/// Recorta un buffer Rgba8 `w × h` al rect half-open
/// `(x0, y0, x1, y1)` y devuelve un buffer del nuevo tamaño
/// `(x1 - x0) × (y1 - y0)`. Asume el rect dentro de los bounds
/// (validación aguas arriba). Pura.
pub fn recortar_buffer(src: &[u8], w: u32, x0: u32, y0: u32, x1: u32, y1: u32) -> Vec<u8> {
    recortar_buffer_bpp(src, w, x0, y0, x1, y1, 4)
}

/// Variante de [`recortar_buffer`] parametrizada por bytes-por-píxel:
/// `bpp=4` para buffers Rgba8, `bpp=1` para máscaras alfa de un canal.
/// La aritmética de filas es idéntica salvo el factor de canal.
pub fn recortar_buffer_bpp(
    src: &[u8],
    w: u32,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    bpp: usize,
) -> Vec<u8> {
    let w = w as usize;
    let new_w = (x1 - x0) as usize;
    let new_h = (y1 - y0) as usize;
    let mut out = Vec::with_capacity(new_w * new_h * bpp);
    for y in y0..y1 {
        let row_start = (y as usize * w + x0 as usize) * bpp;
        let row_end = row_start + new_w * bpp;
        out.extend_from_slice(&src[row_start..row_end]);
    }
    out
}

/// Pone `[0, 0, 0, 0]` (transparente full) en cada píxel del rect
/// half-open `(x0, y0, x1, y1)` de un buffer Rgba8 `w × h`. Devuelve
/// un buffer nuevo del mismo tamaño con el resto intacto. Pura.
/// Pre: rect dentro de bounds (validación aguas arriba).
pub fn limpiar_rect_en_buffer(
    src: &[u8],
    w: u32,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
) -> Vec<u8> {
    let mut out = src.to_vec();
    let w = w as usize;
    for y in y0..y1 {
        let row = y as usize * w;
        for x in x0..x1 {
            let i = (row + x as usize) * 4;
            out[i] = 0;
            out[i + 1] = 0;
            out[i + 2] = 0;
            out[i + 3] = 0;
        }
    }
    out
}

/// Pone `rgba` en cada píxel del rect half-open `(x0, y0, x1, y1)` de
/// un buffer Rgba8 `w × h`. Devuelve un buffer nuevo del mismo tamaño
/// con el resto intacto. Pura. Pre: rect dentro de bounds.
pub fn rellenar_rect_en_buffer(
    src: &[u8],
    w: u32,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    rgba: [u8; 4],
) -> Vec<u8> {
    let mut out = src.to_vec();
    let w = w as usize;
    for y in y0..y1 {
        let row = y as usize * w;
        for x in x0..x1 {
            let i = (row + x as usize) * 4;
            out[i..i + 4].copy_from_slice(&rgba);
        }
    }
    out
}

/// Construye un buffer Rgba8 `w × h` todo transparente excepto el rect
/// half-open `(x0, y0, x1, y1)`, donde copia los píxeles de `src`. Es
/// el complemento de [`limpiar_rect_en_buffer`]: aquél conserva el
/// afuera y borra el rect; éste borra el afuera y conserva el rect.
/// Devuelve también si quedó algún píxel con alfa > 0 dentro del rect
/// (`false` ⇒ nada visible que copiar). Pura. Pre: rect dentro de
/// bounds (validación aguas arriba).
pub fn extraer_rect_a_buffer(
    src: &[u8],
    w: u32,
    h: u32,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
) -> (Vec<u8>, bool) {
    let w = w as usize;
    let mut out = vec![0u8; w * h as usize * 4];
    let mut hubo_contenido = false;
    for y in y0..y1 {
        let row = y as usize * w;
        for x in x0..x1 {
            let i = (row + x as usize) * 4;
            out[i..i + 4].copy_from_slice(&src[i..i + 4]);
            if src[i + 3] != 0 {
                hubo_contenido = true;
            }
        }
    }
    (out, hubo_contenido)
}

/// Recorta el rect half-open `(x0, y0, x1, y1)` de un buffer Rgba8
/// `w × *` a un buffer **tight** de `(x1-x0) × (y1-y0)` (NO del tamaño
/// del origen). Devuelve también si quedó algún píxel con alfa > 0
/// (`false` ⇒ nada visible). Pura. Pre: rect dentro de bounds.
pub fn recortar_subbuffer(
    src: &[u8],
    w: u32,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
) -> (Vec<u8>, bool) {
    let sw = w as usize;
    let rw = (x1 - x0) as usize;
    let rh = (y1 - y0) as usize;
    let mut out = Vec::with_capacity(rw * rh * 4);
    let mut hubo = false;
    for y in y0..y1 {
        let row = y as usize * sw;
        for x in x0..x1 {
            let i = (row + x as usize) * 4;
            out.extend_from_slice(&src[i..i + 4]);
            if src[i + 3] != 0 {
                hubo = true;
            }
        }
    }
    (out, hubo)
}

/// Compone un `clip` tight de `clip_w × clip_h` sobre un lienzo fresco
/// transparente de `canvas_w × canvas_h`, con la esquina superior
/// izquierda en `(dx, dy)`. Los píxeles del clip que caigan fuera del
/// lienzo se descartan (blit con recorte por-píxel). Reemplazo directo,
/// no alfa-compositing — el clip pisa lo que haya debajo (el lienzo
/// destino arranca transparente, así que da igual). Pura.
pub fn componer_clip_en_canvas(
    clip: &[u8],
    clip_w: u32,
    clip_h: u32,
    canvas_w: u32,
    canvas_h: u32,
    dx: u32,
    dy: u32,
) -> Vec<u8> {
    let cw = canvas_w as usize;
    let mut out = vec![0u8; cw * canvas_h as usize * 4];
    let clip_w = clip_w as usize;
    for cy in 0..clip_h as usize {
        let ty = dy as usize + cy;
        if ty >= canvas_h as usize {
            break;
        }
        for cx in 0..clip_w {
            let tx = dx as usize + cx;
            if tx >= cw {
                continue;
            }
            let si = (cy * clip_w + cx) * 4;
            let di = (ty * cw + tx) * 4;
            out[di..di + 4].copy_from_slice(&clip[si..si + 4]);
        }
    }
    out
}

/// Flood fill (balde) sobre un buffer Rgba8 `w × h`. Desde la semilla
/// `(sx, sy)` expande en 4-conexión a todos los píxeles cuyo color esté
/// dentro de `tol` (suma de |Δ| RGBA) respecto al color semilla, y los
/// pinta de `nuevo`. Si `bounds` es `Some((x0,y0,x1,y1))` el relleno
/// queda confinado a ese rect half-open (los bordes actúan como muro) y
/// una semilla fuera de él no rellena nada. Devuelve `Some(buffer)` si
/// cambió algún píxel, `None` si no (semilla fuera, o región ya del
/// color destino). Pura. La tolerancia se chequea SIEMPRE contra el
/// color original del píxel (el pintado ocurre sólo al visitarlo, así
/// que un vecino nunca se evalúa con un color ya modificado).
pub fn flood_fill(
    src: &[u8],
    w: u32,
    h: u32,
    sx: u32,
    sy: u32,
    nuevo: [u8; 4],
    tol: u32,
    bounds: Option<(u32, u32, u32, u32)>,
) -> Option<Vec<u8>> {
    let w_us = w as usize;
    let (bx0, by0, bx1, by1) = bounds.unwrap_or((0, 0, w, h));
    // Recortar bounds al canvas por si vinieran sobredimensionados.
    let bx1 = bx1.min(w);
    let by1 = by1.min(h);
    if sx < bx0 || sx >= bx1 || sy < by0 || sy >= by1 {
        return None;
    }
    let idx = |x: u32, y: u32| ((y as usize) * w_us + x as usize) * 4;
    let si = idx(sx, sy);
    let seed = [src[si], src[si + 1], src[si + 2], src[si + 3]];
    let dentro_tol = |c: &[u8]| -> bool {
        let d = (c[0] as i32 - seed[0] as i32).unsigned_abs()
            + (c[1] as i32 - seed[1] as i32).unsigned_abs()
            + (c[2] as i32 - seed[2] as i32).unsigned_abs()
            + (c[3] as i32 - seed[3] as i32).unsigned_abs();
        d <= tol
    };
    let mut out = src.to_vec();
    let mut visto = vec![false; w_us * h as usize];
    let mut pila = vec![(sx, sy)];
    let mut cambio = false;
    while let Some((x, y)) = pila.pop() {
        let vi = y as usize * w_us + x as usize;
        if visto[vi] {
            continue;
        }
        visto[vi] = true;
        let i = vi * 4;
        if !dentro_tol(&out[i..i + 4]) {
            continue;
        }
        let actual = [out[i], out[i + 1], out[i + 2], out[i + 3]];
        if actual != nuevo {
            out[i..i + 4].copy_from_slice(&nuevo);
            cambio = true;
        }
        if x + 1 < bx1 {
            pila.push((x + 1, y));
        }
        if x > bx0 {
            pila.push((x - 1, y));
        }
        if y + 1 < by1 {
            pila.push((x, y + 1));
        }
        if y > by0 {
            pila.push((x, y - 1));
        }
    }
    if cambio {
        Some(out)
    } else {
        None
    }
}

/// Selección por inundación (varita mágica **contigua**): BFS desde
/// `(sx, sy)` que marca cada píxel conectado cuyo color esté dentro de `tol`
/// (suma de |Δ| sobre RGBA, métrica `0..=1020`) respecto al píxel semilla.
/// Devuelve una **máscara** de un canal `W·H` (255 = seleccionado, 0 = no),
/// o `None` si la semilla cae fuera del canvas. A diferencia de
/// [`flood_fill`] no recolorea nada: produce la región, que la app guarda
/// como máscara de selección. Pura; 4-conectividad como `flood_fill`.
pub fn flood_mascara(src: &[u8], w: u32, h: u32, sx: u32, sy: u32, tol: u32) -> Option<Vec<u8>> {
    let w_us = w as usize;
    let h_us = h as usize;
    if sx >= w || sy >= h {
        return None;
    }
    let idx4 = |x: u32, y: u32| ((y as usize) * w_us + x as usize) * 4;
    let si = idx4(sx, sy);
    let seed = [src[si], src[si + 1], src[si + 2], src[si + 3]];
    let dentro_tol = |i4: usize| -> bool {
        let d = (src[i4] as i32 - seed[0] as i32).unsigned_abs()
            + (src[i4 + 1] as i32 - seed[1] as i32).unsigned_abs()
            + (src[i4 + 2] as i32 - seed[2] as i32).unsigned_abs()
            + (src[i4 + 3] as i32 - seed[3] as i32).unsigned_abs();
        d <= tol
    };
    let mut mascara = vec![0u8; w_us * h_us];
    let mut visto = vec![false; w_us * h_us];
    let mut pila = vec![(sx, sy)];
    while let Some((x, y)) = pila.pop() {
        let vi = y as usize * w_us + x as usize;
        if visto[vi] {
            continue;
        }
        visto[vi] = true;
        if !dentro_tol(vi * 4) {
            continue;
        }
        mascara[vi] = 255;
        if x + 1 < w {
            pila.push((x + 1, y));
        }
        if x > 0 {
            pila.push((x - 1, y));
        }
        if y + 1 < h {
            pila.push((x, y + 1));
        }
        if y > 0 {
            pila.push((x, y - 1));
        }
    }
    Some(mascara)
}

/// Rasteriza un polígono (lista de vértices en coords-imagen) a una máscara
/// de un canal `W·H` por relleno scanline con regla **par-impar** (even-odd):
/// 255 dentro del polígono, 0 fuera. Es la base de la herramienta lazo —
/// el cierre del polígono es implícito (último vértice ↔ primero). Con < 3
/// vértices devuelve una máscara vacía. Pura.
pub fn poligono_a_mascara(pts: &[(i32, i32)], w: u32, h: u32) -> Vec<u8> {
    let mut m = vec![0u8; (w as usize) * (h as usize)];
    if pts.len() < 3 {
        return m;
    }
    let w_i = w as i32;
    let h_i = h as i32;
    for y in 0..h_i {
        let yf = y as f32 + 0.5;
        // Intersecciones de la scanline con cada arista.
        let mut xs: Vec<f32> = Vec::new();
        for i in 0..pts.len() {
            let (x0, y0) = (pts[i].0 as f32, pts[i].1 as f32);
            let j = (i + 1) % pts.len();
            let (x1, y1) = (pts[j].0 as f32, pts[j].1 as f32);
            // Arista cruza la scanline (medio-abierta para no contar vértices 2×).
            if (y0 <= yf && y1 > yf) || (y1 <= yf && y0 > yf) {
                let t = (yf - y0) / (y1 - y0);
                xs.push(x0 + t * (x1 - x0));
            }
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mut k = 0;
        while k + 1 < xs.len() {
            let xa = xs[k].ceil().max(0.0) as i32;
            let xb = (xs[k + 1].floor() as i32).min(w_i - 1);
            let mut x = xa;
            while x <= xb {
                if x >= 0 && x < w_i {
                    m[(y * w_i + x) as usize] = 255;
                }
                x += 1;
            }
            k += 2;
        }
    }
    m
}

/// Factor de cobertura del pincel en `[0,1]` para un píxel a distancia
/// `d` del centro, con radio `r` y `dureza` en `[0,1]`. Dentro del núcleo
/// `dureza·r` es 1.0; entre ahí y `r` cae linealmente a 0; fuera de `r`
/// es 0. `r == 0` (1 px) o `dureza == 1` (borde duro) → 1.0 dentro del
/// disco. Pura.
pub fn cobertura_pincel(d: f32, r: f32, dureza: f32) -> f32 {
    if d > r {
        return 0.0;
    }
    if r <= 0.0 || dureza >= 1.0 {
        return 1.0;
    }
    let inner = dureza * r;
    if d <= inner {
        1.0
    } else {
        ((r - d) / (r - inner)).clamp(0.0, 1.0)
    }
}

/// Estampa un disco de radio `radio` centrado en `(cx, cy)` sobre un
/// buffer Rgba8 `w × h`, con `dureza` controlando el degradé del borde
/// (1.0 = duro; <1.0 = el alfa cae hacia el borde, ver [`cobertura_pincel`]).
/// Si `borrar`, reduce el alfa destino por la cobertura (goma suave); si
/// no, compone `color` (con su alfa escalado por la cobertura) src-over
/// ([`mezclar_src_over`]). Recorta al canvas y, si `bounds` es `Some`, al
/// rect half-open. Pura (muta `buf`); `cx, cy` pueden caer fuera.
#[allow(clippy::too_many_arguments)]
pub fn estampar_disco(
    buf: &mut [u8],
    w: u32,
    h: u32,
    cx: i32,
    cy: i32,
    radio: i32,
    color: [u8; 4],
    borrar: bool,
    dureza: f32,
    bounds: Option<(u32, u32, u32, u32)>,
) {
    let (bx0, by0, bx1, by1) = bounds.unwrap_or((0, 0, w, h));
    let bx1 = bx1.min(w) as i32;
    let by1 = by1.min(h) as i32;
    let bx0 = bx0 as i32;
    let by0 = by0 as i32;
    let r2 = radio * radio;
    let rf = radio as f32;
    for dy in -radio..=radio {
        let y = cy + dy;
        if y < by0 || y >= by1 {
            continue;
        }
        for dx in -radio..=radio {
            let x = cx + dx;
            if x < bx0 || x >= bx1 {
                continue;
            }
            if dx * dx + dy * dy <= r2 {
                let d = ((dx * dx + dy * dy) as f32).sqrt();
                let cob = cobertura_pincel(d, rf, dureza);
                if cob <= 0.0 {
                    continue;
                }
                let i = ((y as usize) * w as usize + x as usize) * 4;
                if borrar {
                    // Goma: baja el alfa destino por la cobertura.
                    let a = buf[i + 3] as f32;
                    buf[i + 3] = (a * (1.0 - cob)) as u8;
                } else {
                    let a = (color[3] as f32 * cob).round() as u8;
                    mezclar_src_over(
                        &mut buf[i..i + 4],
                        [color[0], color[1], color[2], a],
                    );
                }
            }
        }
    }
}

/// Estampa discos a lo largo del segmento `(x0, y0) → (x1, y1)`, uno por
/// cada paso entero del eje más largo, de modo que el trazo quede
/// continuo (sin huecos para `radio ≥ 1`). Pura (muta `buf`). Ver
/// [`estampar_disco`] para `borrar`.
#[allow(clippy::too_many_arguments)]
pub fn trazar_linea_pincel(
    buf: &mut [u8],
    w: u32,
    h: u32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    radio: i32,
    color: [u8; 4],
    borrar: bool,
    dureza: f32,
    bounds: Option<(u32, u32, u32, u32)>,
) {
    let n = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
    for k in 0..=n {
        let t = k as f32 / n as f32;
        let x = x0 + ((x1 - x0) as f32 * t).round() as i32;
        let y = y0 + ((y1 - y0) as f32 * t).round() as i32;
        estampar_disco(buf, w, h, x, y, radio, color, borrar, dureza, bounds);
    }
}

/// Refleja `(x, y)` en un lienzo `w × h` según `(flip_x, flip_y)`
/// (espejo sobre el eje central de cada dimensión). Pura.
pub fn aplicar_eje(
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    flip: (bool, bool),
) -> (i32, i32) {
    let nx = if flip.0 { w as i32 - 1 - x } else { x };
    let ny = if flip.1 { h as i32 - 1 - y } else { y };
    (nx, ny)
}

/// Rellena un degradé lineal sobre un buffer Rgba8 `w × h`: para cada
/// píxel proyecta su centro sobre el eje `(ax,ay) → (bx,by)`, obtiene
/// `t ∈ [0,1]` (clamp), y compone src-over `color` con su alfa escalado
/// por `(1 - t)` — `t=0` en el ancla (color pleno), `t=1` en el extremo
/// (transparente). Si el eje tiene longitud cero, `t=0` en todo el área
/// (relleno sólido). Acotado a `bounds` (half-open) si `Some`. Devuelve
/// un buffer nuevo. Pura.
#[allow(clippy::too_many_arguments)]
pub fn rellenar_gradiente(
    src: &[u8],
    w: u32,
    h: u32,
    ax: f32,
    ay: f32,
    bx: f32,
    by: f32,
    color: [u8; 4],
    bounds: Option<(u32, u32, u32, u32)>,
) -> Vec<u8> {
    let mut out = src.to_vec();
    let dx = bx - ax;
    let dy = by - ay;
    let len2 = dx * dx + dy * dy;
    let (bx0, by0, bx1, by1) = bounds.unwrap_or((0, 0, w, h));
    let bx1 = bx1.min(w);
    let by1 = by1.min(h);
    for y in by0..by1 {
        for x in bx0..bx1 {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let t = if len2 <= 0.0 {
                0.0
            } else {
                (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0)
            };
            let a = (color[3] as f32 * (1.0 - t)).round() as u8;
            let i = ((y as usize) * w as usize + x as usize) * 4;
            mezclar_src_over(
                &mut out[i..i + 4],
                [color[0], color[1], color[2], a],
            );
        }
    }
    out
}

/// Estampa un disco de radio `radio` en `(cx, cy)` sobre un buffer de
/// máscara de un canal `w × h`, llevando cada píxel cubierto hacia `valor`
/// (255 revela, 0 oculta) por su cobertura: `m = m + (valor - m)·cob`.
/// Recorta al canvas y a `bounds` (half-open) si `Some`. Pura (muta `buf`).
#[allow(clippy::too_many_arguments)]
pub fn estampar_disco_mascara(
    buf: &mut [u8],
    w: u32,
    h: u32,
    cx: i32,
    cy: i32,
    radio: i32,
    valor: u8,
    dureza: f32,
    bounds: Option<(u32, u32, u32, u32)>,
) {
    let (bx0, by0, bx1, by1) = bounds.unwrap_or((0, 0, w, h));
    let bx1 = bx1.min(w) as i32;
    let by1 = by1.min(h) as i32;
    let bx0 = bx0 as i32;
    let by0 = by0 as i32;
    let r2 = radio * radio;
    let rf = radio as f32;
    for dy in -radio..=radio {
        let y = cy + dy;
        if y < by0 || y >= by1 {
            continue;
        }
        for dx in -radio..=radio {
            let x = cx + dx;
            if x < bx0 || x >= bx1 {
                continue;
            }
            if dx * dx + dy * dy <= r2 {
                let d = ((dx * dx + dy * dy) as f32).sqrt();
                let cob = cobertura_pincel(d, rf, dureza);
                if cob <= 0.0 {
                    continue;
                }
                let i = (y as usize) * w as usize + x as usize;
                let m = buf[i] as f32;
                buf[i] = (m + (valor as f32 - m) * cob).round() as u8;
            }
        }
    }
}

/// Versión máscara de [`trazar_linea_pincel`]: estampa discos de máscara a
/// lo largo del segmento `(x0,y0) → (x1,y1)`. Pura (muta `buf`).
#[allow(clippy::too_many_arguments)]
pub fn trazar_linea_mascara(
    buf: &mut [u8],
    w: u32,
    h: u32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    radio: i32,
    valor: u8,
    dureza: f32,
    bounds: Option<(u32, u32, u32, u32)>,
) {
    let n = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
    for k in 0..=n {
        let t = k as f32 / n as f32;
        let x = x0 + ((x1 - x0) as f32 * t).round() as i32;
        let y = y0 + ((y1 - y0) as f32 * t).round() as i32;
        estampar_disco_mascara(buf, w, h, x, y, radio, valor, dureza, bounds);
    }
}

/// Flood fill de un canal: desde `(sx, sy)` expande en 4-conexión a los
/// píxeles cuyo valor de máscara difiera del semilla en ≤ `tol`, y los
/// fija a `valor`. Confinado a `bounds` si `Some`. Devuelve `Some(buf)` si
/// cambió algo. Análogo a [`flood_fill`] pero sobre un solo byte. Pura.
pub fn flood_fill_mascara(
    src: &[u8],
    w: u32,
    h: u32,
    sx: u32,
    sy: u32,
    valor: u8,
    tol: u32,
    bounds: Option<(u32, u32, u32, u32)>,
) -> Option<Vec<u8>> {
    let w_us = w as usize;
    let (bx0, by0, bx1, by1) = bounds.unwrap_or((0, 0, w, h));
    let bx1 = bx1.min(w);
    let by1 = by1.min(h);
    if sx < bx0 || sx >= bx1 || sy < by0 || sy >= by1 {
        return None;
    }
    let seed = src[sy as usize * w_us + sx as usize];
    let mut out = src.to_vec();
    let mut visto = vec![false; w_us * h as usize];
    let mut pila = vec![(sx, sy)];
    let mut cambio = false;
    while let Some((x, y)) = pila.pop() {
        let vi = y as usize * w_us + x as usize;
        if visto[vi] {
            continue;
        }
        visto[vi] = true;
        let d = (out[vi] as i32 - seed as i32).unsigned_abs();
        if d > tol {
            continue;
        }
        if out[vi] != valor {
            out[vi] = valor;
            cambio = true;
        }
        if x + 1 < bx1 {
            pila.push((x + 1, y));
        }
        if x > bx0 {
            pila.push((x - 1, y));
        }
        if y + 1 < by1 {
            pila.push((x, y + 1));
        }
        if y > by0 {
            pila.push((x, y - 1));
        }
    }
    if cambio {
        Some(out)
    } else {
        None
    }
}

/// Degradé sobre máscara: para cada píxel proyecta su centro sobre el eje
/// `(ax,ay) → (bx,by)`, obtiene `t ∈ [0,1]` y lleva el píxel hacia `valor`
/// con peso `(1 - t)` — pleno en el ancla, sin efecto en el extremo (calco
/// del degradé Rgba8, que se desvanece a transparente). Confinado a
/// `bounds`. Devuelve un buffer nuevo. Pura.
#[allow(clippy::too_many_arguments)]
pub fn rellenar_gradiente_mascara(
    src: &[u8],
    w: u32,
    h: u32,
    ax: f32,
    ay: f32,
    bx: f32,
    by: f32,
    valor: u8,
    bounds: Option<(u32, u32, u32, u32)>,
) -> Vec<u8> {
    let mut out = src.to_vec();
    let dx = bx - ax;
    let dy = by - ay;
    let len2 = dx * dx + dy * dy;
    let (bx0, by0, bx1, by1) = bounds.unwrap_or((0, 0, w, h));
    let bx1 = bx1.min(w);
    let by1 = by1.min(h);
    for y in by0..by1 {
        for x in bx0..bx1 {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let t = if len2 <= 0.0 {
                0.0
            } else {
                (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0)
            };
            let peso = 1.0 - t;
            let i = (y as usize) * w as usize + x as usize;
            let m = out[i] as f32;
            out[i] = (m + (valor as f32 - m) * peso).round() as u8;
        }
    }
    out
}

/// Compone (alpha src-over, Rgba8 NO premultiplicado) un `clip` de
/// `clip_w × clip_h` sobre `dst` (`dst_w × dst_h`) con la esquina
/// superior izquierda en el offset CON SIGNO `(dx, dy)`. Los píxeles del
/// clip que caen fuera de `dst` se descartan. A diferencia de
/// [`componer_clip_en_canvas`] (que parte de un lienzo fresco y pisa),
/// éste preserva y compone sobre el contenido previo de `dst` — sirve
/// para "dejar caer" píxeles movidos encima de lo que ya hay. Pura.
pub fn blit_alpha_sobre(
    dst: &[u8],
    dst_w: u32,
    dst_h: u32,
    clip: &[u8],
    clip_w: u32,
    clip_h: u32,
    dx: i32,
    dy: i32,
) -> Vec<u8> {
    let mut out = dst.to_vec();
    let dw = dst_w as i32;
    let dh = dst_h as i32;
    let cw = clip_w as usize;
    for cy in 0..clip_h as i32 {
        let ty = dy + cy;
        if ty < 0 || ty >= dh {
            continue;
        }
        for cx in 0..clip_w as i32 {
            let tx = dx + cx;
            if tx < 0 || tx >= dw {
                continue;
            }
            let si = ((cy as usize) * cw + cx as usize) * 4;
            let di = ((ty as usize) * dst_w as usize + tx as usize) * 4;
            let src = [clip[si], clip[si + 1], clip[si + 2], clip[si + 3]];
            mezclar_src_over(&mut out[di..di + 4], src);
        }
    }
    out
}

/// Compone `src` (Rgba8 NO premultiplicado) sobre el píxel destino
/// `dst` (slice de 4 bytes) con la fórmula src-over, redondeo entero
/// `/255`. Fast-path: alfa 0 no hace nada, alfa 255 pisa. Pura sobre el
/// slice. Es el núcleo compartido por `blit_alpha_sobre` (Fase 41) y el
/// pincel con alpha (Fase 46).
pub fn mezclar_src_over(dst: &mut [u8], src: [u8; 4]) {
    let sa = src[3] as u32;
    if sa == 0 {
        return;
    }
    if sa == 255 {
        dst.copy_from_slice(&src);
        return;
    }
    let da = dst[3] as u32;
    let da_eff = da * (255 - sa) / 255;
    let oa = sa + da_eff;
    for k in 0..3 {
        let num = src[k] as u32 * sa + dst[k] as u32 * da_eff;
        dst[k] = if oa == 0 { 0 } else { (num / oa) as u8 };
    }
    dst[3] = oa as u8;
}

/// Rota 90° en sentido horario un buffer Rgba8 `w × h`. El buffer
/// resultante tiene el mismo conteo de bytes pero su layout corresponde
/// a dimensiones `h × w` (el ancho del destino = el alto del origen).
/// Pura. Pre: `src.len() == w*h*4` (la validación va aguas arriba).
///
/// Mapeo: src `(x, y)` → dst `(h-1-y, x)` con `w_new = h`.
pub fn rotar_buffer_90_cw(src: &[u8], w: u32, h: u32) -> Vec<u8> {
    rotar_buffer_90_cw_bpp(src, w, h, 4)
}

/// Variante de [`rotar_buffer_90_cw`] parametrizada por bytes-por-píxel
/// (`4` Rgba8, `1` máscara alfa). Mismo mapeo geométrico.
pub fn rotar_buffer_90_cw_bpp(src: &[u8], w: u32, h: u32, bpp: usize) -> Vec<u8> {
    let w = w as usize;
    let h = h as usize;
    let mut out = vec![0u8; src.len()];
    let w_new = h;
    for y in 0..h {
        for x in 0..w {
            let i_src = (y * w + x) * bpp;
            let i_dst = (x * w_new + (h - 1 - y)) * bpp;
            out[i_dst..i_dst + bpp].copy_from_slice(&src[i_src..i_src + bpp]);
        }
    }
    out
}

/// Rota 90° en sentido antihorario. Mapeo: src `(x, y)` → dst
/// `(y, w-1-x)` con `w_new = h`. Inversa exacta de `rotar_buffer_90_cw`.
pub fn rotar_buffer_90_ccw(src: &[u8], w: u32, h: u32) -> Vec<u8> {
    rotar_buffer_90_ccw_bpp(src, w, h, 4)
}

/// Variante de [`rotar_buffer_90_ccw`] parametrizada por bytes-por-píxel.
pub fn rotar_buffer_90_ccw_bpp(src: &[u8], w: u32, h: u32, bpp: usize) -> Vec<u8> {
    let w = w as usize;
    let h = h as usize;
    let mut out = vec![0u8; src.len()];
    let w_new = h;
    for y in 0..h {
        for x in 0..w {
            let i_src = (y * w + x) * bpp;
            let i_dst = ((w - 1 - x) * w_new + y) * bpp;
            out[i_dst..i_dst + bpp].copy_from_slice(&src[i_src..i_src + bpp]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // px(buf, w, x, y) → los 4 bytes Rgba8 en (x, y)
    fn px(buf: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * w + x) * 4) as usize;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    }

    #[test]
    fn flood_mascara_selecciona_region_contigua_por_color() {
        // 4×1: [rojo, rojo, azul, rojo]. Desde x=0 con tol baja sólo agarra
        // los dos rojos contiguos; el rojo aislado tras el azul NO entra.
        let mut src = Vec::new();
        for c in [[255, 0, 0, 255], [255, 0, 0, 255], [0, 0, 255, 255], [255, 0, 0, 255]] {
            src.extend_from_slice(&c);
        }
        let m = flood_mascara(&src, 4, 1, 0, 0, 16).unwrap();
        assert_eq!(m, vec![255, 255, 0, 0], "región contigua de rojos");
    }

    #[test]
    fn flood_mascara_semilla_fuera_es_none() {
        let src = vec![0u8; 4 * 4];
        assert!(flood_mascara(&src, 2, 2, 5, 5, 0).is_none());
    }

    #[test]
    fn poligono_triangulo_rellena_interior() {
        // Triángulo grande en 8×8 que cubre la esquina inferior-izquierda.
        let pts = [(0, 0), (7, 0), (0, 7)];
        let m = poligono_a_mascara(&pts, 8, 8);
        // (1,1) está dentro (x+y < 7); (6,6) fuera.
        assert_eq!(m[1 * 8 + 1], 255, "interior seleccionado");
        assert_eq!(m[6 * 8 + 6], 0, "exterior libre");
    }

    #[test]
    fn poligono_degenerado_es_vacio() {
        assert!(poligono_a_mascara(&[(0, 0), (1, 1)], 4, 4).iter().all(|&v| v == 0));
    }

    #[test]
    fn poligono_cuadrado_llena_su_area() {
        // Cuadrado [1,4)×[1,4) en 6×6.
        let pts = [(1, 1), (4, 1), (4, 4), (1, 4)];
        let m = poligono_a_mascara(&pts, 6, 6);
        let dentro = m[2 * 6 + 2];
        let fuera = m[0];
        assert_eq!(dentro, 255);
        assert_eq!(fuera, 0);
    }

    #[test]
    fn flood_mascara_tol_alta_agarra_todo() {
        // tol máxima (1020) ⇒ todos los píxeles conectados entran.
        let src = vec![10, 20, 30, 255, 200, 100, 50, 255, 0, 0, 0, 0, 255, 255, 255, 255];
        let m = flood_mascara(&src, 4, 1, 0, 0, 1020).unwrap();
        assert_eq!(m, vec![255, 255, 255, 255]);
    }

    #[test]
    fn buffer_relleno_repite_el_color() {
        let b = buffer_relleno(2, 3, [10, 20, 30, 40]);
        assert_eq!(b.len(), 2 * 3 * 4);
        assert!(b.chunks_exact(4).all(|c| c == [10, 20, 30, 40]));
    }

    #[test]
    fn bbox_envuelve_solo_lo_opaco_y_es_half_open() {
        // 4×4 transparente con un solo píxel opaco en (1,2)
        let mut b = vec![0u8; 4 * 4 * 4];
        let i = (2 * 4 + 1) * 4;
        b[i + 3] = 255;
        assert_eq!(bbox_no_transparente(&b, 4, 4), Some((1, 2, 2, 3)));
        assert_eq!(bbox_no_transparente(&vec![0u8; 4 * 4 * 4], 4, 4), None);
    }

    #[test]
    fn recortar_buffer_extrae_el_rect() {
        // gradiente por columna: cada píxel marca su x en el canal R
        let w = 4;
        let mut b = vec![0u8; (w * 2 * 4) as usize];
        for y in 0..2 {
            for x in 0..w {
                b[((y * w + x) * 4) as usize] = x as u8;
            }
        }
        let out = recortar_buffer(&b, w, 1, 0, 3, 2);
        assert_eq!(out.len(), 2 * 2 * 4);
        assert_eq!(px(&out, 2, 0, 0)[0], 1);
        assert_eq!(px(&out, 2, 1, 1)[0], 2);
    }

    #[test]
    fn limpiar_y_rellenar_rect_son_complementarios() {
        let base = buffer_relleno(3, 3, [9, 9, 9, 255]);
        let limpio = limpiar_rect_en_buffer(&base, 3, 1, 1, 2, 2);
        assert_eq!(px(&limpio, 3, 1, 1), [0, 0, 0, 0]);
        assert_eq!(px(&limpio, 3, 0, 0), [9, 9, 9, 255]); // afuera intacto
        let rojo = rellenar_rect_en_buffer(&base, 3, 1, 1, 2, 2, [255, 0, 0, 255]);
        assert_eq!(px(&rojo, 3, 1, 1), [255, 0, 0, 255]);
        assert_eq!(px(&rojo, 3, 0, 0), [9, 9, 9, 255]);
    }

    #[test]
    fn mezclar_src_over_fast_paths_y_mezcla() {
        let mut d = [10, 20, 30, 255];
        mezclar_src_over(&mut d, [0, 0, 0, 0]); // alfa 0 → no toca
        assert_eq!(d, [10, 20, 30, 255]);
        mezclar_src_over(&mut d, [1, 2, 3, 255]); // alfa 255 → pisa
        assert_eq!(d, [1, 2, 3, 255]);
    }

    #[test]
    fn flood_fill_respeta_tolerancia_y_bounds() {
        // mitad izquierda negra, mitad derecha blanca (4×1)
        let mut b = vec![0u8, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255];
        let out = flood_fill(&b, 4, 1, 0, 0, [1, 2, 3, 255], 0, None).unwrap();
        assert_eq!(px(&out, 4, 0, 0), [1, 2, 3, 255]);
        assert_eq!(px(&out, 4, 1, 0), [1, 2, 3, 255]);
        assert_eq!(px(&out, 4, 2, 0), [255, 255, 255, 255]); // muro de color
        // semilla fuera de bounds → None
        b[0] = 0;
        assert!(flood_fill(&b, 4, 1, 3, 0, [0, 0, 0, 255], 0, Some((0, 0, 2, 1))).is_none());
    }

    #[test]
    fn cobertura_pincel_borde_duro_vs_suave() {
        assert_eq!(cobertura_pincel(0.0, 4.0, 1.0), 1.0);
        assert_eq!(cobertura_pincel(4.1, 4.0, 1.0), 0.0); // fuera del radio
        // dureza 0: cae lineal del centro al borde
        assert!((cobertura_pincel(2.0, 4.0, 0.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn estampar_disco_pinta_el_centro() {
        let mut b = buffer_relleno(8, 8, [0, 0, 0, 0]);
        estampar_disco(&mut b, 8, 8, 4, 4, 2, [255, 0, 0, 255], false, 1.0, None);
        assert_eq!(px(&b, 8, 4, 4), [255, 0, 0, 255]);
        assert_eq!(px(&b, 8, 0, 0)[3], 0); // esquina lejana intacta
    }

    #[test]
    fn rotar_90_cw_y_ccw_son_inversas() {
        // 2×3 con R = índice lineal
        let (w, h) = (2u32, 3u32);
        let mut b = vec![0u8; (w * h * 4) as usize];
        for k in 0..(w * h) {
            b[(k * 4) as usize] = k as u8;
        }
        let cw = rotar_buffer_90_cw(&b, w, h);
        let back = rotar_buffer_90_ccw(&cw, h, w); // dims rotadas
        assert_eq!(back, b);
    }

    #[test]
    fn aplicar_eje_espeja_sobre_el_centro() {
        assert_eq!(aplicar_eje(0, 0, 4, 4, (true, false)), (3, 0));
        assert_eq!(aplicar_eje(0, 0, 4, 4, (false, true)), (0, 3));
        assert_eq!(aplicar_eje(1, 1, 4, 4, (false, false)), (1, 1));
    }
}
