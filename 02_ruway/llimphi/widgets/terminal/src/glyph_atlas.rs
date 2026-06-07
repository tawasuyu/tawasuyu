//! Atlas de glifos para el render GPU-directo de la grilla del modo TUI
//! (Fase 4 del SDD-TERMINAL). Pura CPU: rasteriza cada char a una celda
//! del atlas con `fontdue` y devuelve coords UV para que el shader de
//! quads instanciados las samplee.
//!
//! ## Diseño
//!
//! - **Grilla fija de celdas**: el atlas es una imagen `atlas_w × atlas_h`
//!   en escala de grises (1 byte por pixel: cobertura del glifo). Cada
//!   celda mide `cell_w × cell_h` px y aloja UN glifo. `cols × rows`
//!   celdas totales (computable desde el tamaño y el font size).
//! - **Mapa `char → slot`**: cargado on-demand (primera vez que se pide
//!   un char se rasteriza y se asigna la próxima celda libre). Sin LRU
//!   por ahora — atlas grande de entrada (suficiente para ASCII +
//!   símbolos comunes); si se llena, crece duplicando alto.
//! - **Bytes RAW**: el caller decide cuándo subir a GPU (toda la imagen
//!   o sólo el rect del slot recién agregado, vía `dirty_rect`). Esto
//!   mantiene el atlas **agnóstico de wgpu** (testeable headless).
//!
//! ## Métricas
//!
//! Las celdas son del **tamaño máximo** del glifo (incluye padding para
//! que el render del shader pueda ofset-ear el origen del baseline sin
//! cortar). El caller (el pipeline) usa `metrics_for` para alinear cada
//! quad al baseline correcto dentro de la fila.

use fontdue::{Font, FontSettings};

/// Slot de un glifo en el atlas. Coords en píxeles del atlas (no UV
/// normalizadas — el caller las divide por `(atlas_w, atlas_h)` al subirlas
/// al shader). El offset `(xmin, ymin)` es del bitmap respecto del origen
/// del cell — el shader lo aplica al posicionar el quad de salida.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphSlot {
    /// x del píxel superior-izquierdo del glifo (dentro de su celda).
    pub px: u32,
    /// y idem.
    pub py: u32,
    /// Ancho del bitmap del glifo (≤ cell_w).
    pub w: u32,
    /// Alto del bitmap (≤ cell_h).
    pub h: u32,
    /// Offset horizontal del glifo respecto del origen del cell (typically 0).
    pub xmin: i32,
    /// Offset vertical — `metrics.ymin` de fontdue (positivo = baseline arriba).
    pub ymin: i32,
    /// Advance horizontal (para mono, igual a `metrics.advance_width` o ~cell_w).
    pub advance: f32,
}

/// Rect en píxeles del atlas: `(x, y, w, h)`. Empacado por
/// `add_dirty_rect` para que el caller sepa qué subir a GPU.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DirtyRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Atlas de glifos rasterizados sobre una textura grayscale.
pub struct GlyphAtlas {
    font: Font,
    /// Tamaño del font en píxeles (input al rasterizer).
    font_size: f32,
    /// Tamaño de cada celda en píxeles (ancho/alto del cell, no del glifo).
    cell_w: u32,
    cell_h: u32,
    /// Columnas/filas vigentes del atlas.
    cols: u32,
    rows: u32,
    /// Bytes del atlas grayscale (`atlas_w * atlas_h` bytes, row-major).
    pixels: Vec<u8>,
    /// Mapeo `char → slot_index_lineal` (filled on demand).
    map: std::collections::HashMap<char, u32>,
    /// Próximo slot libre (lineal `0..cols*rows`). `None` cuando está lleno.
    next_slot: Option<u32>,
    /// Rect vigente que cambió desde la última `take_dirty`. `None` = nada
    /// que subir. Acumula con union; el caller llama `take_dirty()` después
    /// de subir y resetea.
    dirty: Option<DirtyRect>,
}

impl GlyphAtlas {
    /// Construye el atlas con `font_bytes` (TTF/OTF), `font_size_px` y un
    /// número inicial de `cols`/`rows`. El alto de cada cell sale del font
    /// (`line_metrics`), el ancho del max(advance, 'M'). Si el font no
    /// parsea, devuelve `None` — el caller decide el fallback.
    pub fn new(font_bytes: &[u8], font_size_px: f32, cols: u32, rows: u32) -> Option<Self> {
        let font = Font::from_bytes(font_bytes, FontSettings::default()).ok()?;
        // Cell metrics: alto del line (ascent - descent + line_gap),
        // ancho del advance del 'M' (proxy para mono). Padding 1 px por
        // lado para que glifos con bearing negativo no sangren.
        let line = font.horizontal_line_metrics(font_size_px)?;
        let cell_h = (line.new_line_size.ceil() as u32).max(1) + 2;
        // Para mono asumimos que 'M' marca el ancho de cell. Si el font no
        // tiene 'M', cae a advance del primer glifo no-cero o a 8 px.
        let m_metrics = font.metrics('M', font_size_px);
        let cell_w = (m_metrics.advance_width.ceil() as u32).max(1) + 2;
        let atlas_w = cell_w * cols;
        let atlas_h = cell_h * rows;
        Some(Self {
            font,
            font_size: font_size_px,
            cell_w,
            cell_h,
            cols,
            rows,
            pixels: vec![0u8; (atlas_w * atlas_h) as usize],
            map: std::collections::HashMap::new(),
            next_slot: Some(0),
            dirty: None,
        })
    }

    /// Tamaño total del atlas en píxeles.
    pub fn size(&self) -> (u32, u32) {
        (self.cell_w * self.cols, self.cell_h * self.rows)
    }

    /// Tamaño de cada cell (ancho × alto, px).
    pub fn cell_size(&self) -> (u32, u32) {
        (self.cell_w, self.cell_h)
    }

    /// Buffer crudo del atlas (grayscale 1 byte por pixel, row-major,
    /// stride = `atlas_w` bytes). Inmutable — el caller sube esto a la
    /// textura GPU directamente.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Si hay un rect modificado desde la última llamada, lo devuelve y
    /// resetea. Patrón "consume on read": el caller que llama esto es el
    /// que está por hacer el upload a GPU.
    pub fn take_dirty(&mut self) -> Option<DirtyRect> {
        self.dirty.take()
    }

    /// Devuelve el slot del glifo `ch`. Si no estaba cacheado, lo rasteriza
    /// y le asigna la próxima celda libre (marcando el rect como dirty).
    /// Si el atlas está lleno devuelve `None` (el caller puede llamar
    /// `grow()` y reintentar).
    pub fn glyph_for(&mut self, ch: char) -> Option<GlyphSlot> {
        if let Some(&slot) = self.map.get(&ch) {
            return Some(self.slot_at(slot, ch));
        }
        let slot = self.next_slot?;
        self.rasterize_to(ch, slot);
        self.map.insert(ch, slot);
        let next = slot + 1;
        self.next_slot = if next < self.cols * self.rows { Some(next) } else { None };
        Some(self.slot_at(slot, ch))
    }

    /// Duplica el alto del atlas (`rows *= 2`) para hacer más espacio. El
    /// buffer se extiende con ceros; los glifos viejos quedan donde
    /// estaban; `next_slot` apunta a la primera celda nueva. El rect
    /// dirty se setea sobre la mitad nueva. Es la estrategia más simple
    /// que mantiene los slots viejos válidos sin re-empacar.
    pub fn grow(&mut self) {
        let old_rows = self.rows;
        let new_rows = old_rows.saturating_mul(2).max(old_rows + 1);
        let (atlas_w, _) = self.size();
        let old_pixels = std::mem::take(&mut self.pixels);
        self.rows = new_rows;
        let new_atlas_h = self.cell_h * new_rows;
        self.pixels = vec![0u8; (atlas_w * new_atlas_h) as usize];
        // Copy old block at the top.
        self.pixels[..old_pixels.len()].copy_from_slice(&old_pixels);
        // El próximo slot arranca en la primera celda nueva.
        self.next_slot = Some(self.cols * old_rows);
        // Toda la mitad nueva está sucia (zeros, pero el caller necesita
        // saber que el atlas creció para re-subir si quiere texturas
        // ajustadas; en práctica se re-aloca la textura GPU al detectar
        // size change).
        self.add_dirty(DirtyRect {
            x: 0,
            y: self.cell_h * old_rows,
            w: atlas_w,
            h: self.cell_h * (new_rows - old_rows),
        });
    }

    /// Cantidad de glifos cacheados hasta ahora (informativo).
    pub fn cached_count(&self) -> usize {
        self.map.len()
    }

    /// Capacidad total del atlas en celdas (`cols * rows`).
    pub fn capacity(&self) -> u32 {
        self.cols * self.rows
    }

    // ── helpers privados ──────────────────────────────────────────────

    fn slot_at(&self, slot: u32, ch: char) -> GlyphSlot {
        let col = slot % self.cols;
        let row = slot / self.cols;
        let (m, _) = self.font.rasterize(ch, self.font_size);
        GlyphSlot {
            px: col * self.cell_w,
            py: row * self.cell_h,
            w: m.width as u32,
            h: m.height as u32,
            xmin: m.xmin,
            ymin: m.ymin,
            advance: m.advance_width,
        }
    }

    fn rasterize_to(&mut self, ch: char, slot: u32) {
        let (m, bitmap) = self.font.rasterize(ch, self.font_size);
        let col = slot % self.cols;
        let row = slot / self.cols;
        let px = col * self.cell_w;
        let py = row * self.cell_h;
        let (atlas_w, _) = self.size();
        // Blit del bitmap a (px, py). El glifo puede ser más chico que la
        // celda — el resto queda en 0 (transparente para el shader).
        let bw = m.width as u32;
        let bh = m.height as u32;
        for y in 0..bh.min(self.cell_h) {
            for x in 0..bw.min(self.cell_w) {
                let src = (y * bw + x) as usize;
                let dst = ((py + y) * atlas_w + (px + x)) as usize;
                if src < bitmap.len() && dst < self.pixels.len() {
                    self.pixels[dst] = bitmap[src];
                }
            }
        }
        self.add_dirty(DirtyRect {
            x: px,
            y: py,
            w: self.cell_w,
            h: self.cell_h,
        });
    }

    fn add_dirty(&mut self, r: DirtyRect) {
        self.dirty = Some(match self.dirty {
            None => r,
            Some(prev) => union_rects(prev, r),
        });
    }
}

fn union_rects(a: DirtyRect, b: DirtyRect) -> DirtyRect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    let r = (a.x + a.w).max(b.x + b.w);
    let bo = (a.y + a.h).max(b.y + b.h);
    DirtyRect {
        x,
        y,
        w: r - x,
        h: bo - y,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONO: &[u8] = llimphi_ui::llimphi_text::MONO_FONT_BYTES;

    fn atlas() -> GlyphAtlas {
        GlyphAtlas::new(MONO, 14.0, 16, 4).expect("font parses")
    }

    #[test]
    fn new_compone_dimensiones_segun_font_y_cols_rows() {
        let a = atlas();
        let (cw, ch) = a.cell_size();
        let (w, h) = a.size();
        assert!(cw > 0 && ch > 0);
        assert_eq!(w, cw * 16);
        assert_eq!(h, ch * 4);
        assert_eq!(a.capacity(), 64);
        assert_eq!(a.cached_count(), 0);
    }

    #[test]
    fn primer_glyph_for_rasteriza_y_marca_dirty() {
        let mut a = atlas();
        let s = a.glyph_for('A').expect("slot");
        assert_eq!(s.px, 0);
        assert_eq!(s.py, 0);
        assert!(s.w > 0 && s.h > 0);
        assert_eq!(a.cached_count(), 1);
        let dirty = a.take_dirty().expect("dirty");
        assert_eq!(dirty.x, 0);
        assert_eq!(dirty.y, 0);
        // Tras consumir, sin dirty pendiente.
        assert!(a.take_dirty().is_none());
    }

    #[test]
    fn segundo_glyph_va_a_la_proxima_celda() {
        let mut a = atlas();
        let _ = a.glyph_for('A').unwrap();
        let _ = a.take_dirty();
        let s = a.glyph_for('B').unwrap();
        let (cw, _) = a.cell_size();
        assert_eq!(s.px, cw);
        assert_eq!(s.py, 0);
        assert_eq!(a.cached_count(), 2);
    }

    #[test]
    fn lookup_repetido_no_aumenta_la_cache() {
        let mut a = atlas();
        let s1 = a.glyph_for('A').unwrap();
        let _ = a.take_dirty();
        let s2 = a.glyph_for('A').unwrap();
        assert_eq!(s1, s2);
        assert_eq!(a.cached_count(), 1);
        // Lookup cacheado no marca dirty.
        assert!(a.take_dirty().is_none());
    }

    #[test]
    fn fila_se_envuelve_a_la_siguiente_al_completar_columnas() {
        let mut a = atlas();
        let (cw, ch) = a.cell_size();
        for c in 'a'..='z' {
            let _ = a.glyph_for(c);
        }
        // Tras 16 columnas se va a la segunda fila.
        let s_q = a.glyph_for('a').unwrap(); // ya está; mismo slot.
        assert_eq!((s_q.px, s_q.py), (0, 0));
        // El char 17 (índice 16 en 0-based) cayó en (col=0, row=1).
        let s17 = a.glyph_for(('a' as u32 + 16) as u8 as char).unwrap();
        assert_eq!((s17.px, s17.py), (0, ch));
        // El char 18 cae en (col=1, row=1).
        let s18 = a.glyph_for(('a' as u32 + 17) as u8 as char).unwrap();
        assert_eq!((s18.px, s18.py), (cw, ch));
    }

    #[test]
    fn glyph_for_devuelve_none_cuando_lleno() {
        let mut a = GlyphAtlas::new(MONO, 14.0, 2, 2).unwrap(); // capacidad 4
        for c in ['A', 'B', 'C', 'D'] {
            assert!(a.glyph_for(c).is_some(), "{c}");
        }
        // El quinto no entra.
        assert!(a.glyph_for('E').is_none());
        assert_eq!(a.cached_count(), 4);
    }

    #[test]
    fn grow_duplica_rows_y_libera_celdas() {
        let mut a = GlyphAtlas::new(MONO, 14.0, 2, 2).unwrap(); // capacidad 4
        for c in ['A', 'B', 'C', 'D'] {
            a.glyph_for(c).unwrap();
        }
        assert!(a.glyph_for('E').is_none());
        a.grow();
        assert_eq!(a.capacity(), 8);
        // Glifos viejos siguen en su slot original.
        let s_a = a.glyph_for('A').unwrap();
        assert_eq!((s_a.px, s_a.py), (0, 0));
        // 'E' entra en la mitad nueva (slot 4 → col 0, row 2).
        let (_, ch) = a.cell_size();
        let s_e = a.glyph_for('E').unwrap();
        assert_eq!((s_e.px, s_e.py), (0, ch * 2));
    }

    #[test]
    fn dirty_acumula_union_hasta_take() {
        let mut a = atlas();
        a.glyph_for('A').unwrap();
        a.glyph_for('B').unwrap();
        a.glyph_for('C').unwrap();
        let d = a.take_dirty().unwrap();
        let (cw, ch) = a.cell_size();
        // Tres celdas en fila: x=0..3*cw, y=0..ch.
        assert_eq!(d.x, 0);
        assert_eq!(d.y, 0);
        assert_eq!(d.w, cw * 3);
        assert_eq!(d.h, ch);
    }

    #[test]
    fn pixels_buffer_se_llena_con_algo_distinto_de_cero_tras_rasterizar() {
        let mut a = atlas();
        a.glyph_for('A').unwrap();
        // Algún pixel del primer cell debe ser no-cero (alpha del glifo).
        let (cw, ch) = a.cell_size();
        let (atlas_w, _) = a.size();
        let mut any = false;
        for y in 0..ch {
            for x in 0..cw {
                if a.pixels()[((y * atlas_w) + x) as usize] != 0 {
                    any = true;
                    break;
                }
            }
            if any {
                break;
            }
        }
        assert!(any, "el cell de 'A' debe tener pixels rasterizados");
    }
}
