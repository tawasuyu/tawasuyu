//! Tipos base del núcleo geoespacial: coordenadas, cajas envolventes, etiquetas,
//! propiedades de features y el modelo de datos plano listo para proyectar.

use std::sync::{Arc, Mutex};

/// Una coordenada geográfica `[lon, lat]` en grados. La `z` (altitud) de
/// GeoJSON, si viene, se ignora.
pub type Coord = [f64; 2];

/// Un anillo o polilínea: secuencia de coordenadas.
pub type Ring = Vec<Coord>;

/// Caja envolvente en grados: `(min_lon, min_lat, max_lon, max_lat)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BBox {
    pub min_lon: f64,
    pub min_lat: f64,
    pub max_lon: f64,
    pub max_lat: f64,
}

impl BBox {
    /// Caja vacía/invertida: lista para acumular con [`expand`](Self::expand).
    pub fn empty() -> Self {
        BBox {
            min_lon: f64::INFINITY,
            min_lat: f64::INFINITY,
            max_lon: f64::NEG_INFINITY,
            max_lat: f64::NEG_INFINITY,
        }
    }

    pub fn expand(&mut self, [lon, lat]: Coord) {
        self.min_lon = self.min_lon.min(lon);
        self.min_lat = self.min_lat.min(lat);
        self.max_lon = self.max_lon.max(lon);
        self.max_lat = self.max_lat.max(lat);
    }

    /// `true` si nunca se expandió (no hubo coordenadas).
    pub fn is_empty(&self) -> bool {
        self.min_lon > self.max_lon || self.min_lat > self.max_lat
    }
}

/// Una etiqueta: el nombre de una feature anclado a un punto representativo
/// (el punto mismo, el medio de una línea, el centroide de un polígono).
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub at: Coord,
    pub text: String,
}

/// Tope de etiquetas a retener — más que esto satura el panel de texto.
pub const MAX_LABELS: usize = 200;

/// Tope de propiedades retenidas por feature (para inspección/choropleth).
pub const MAX_PROPS: usize = 80;

/// Tope de vértices a retener. Cortar datasets enormes mantiene el panel
/// instantáneo (vello rebuild es barato hasta ~500 K primitivos/frame).
pub const MAX_VERTICES: usize = 200_000;

/// Propiedades de una feature, retenidas para inspección (clic) y estilo por
/// valor (choropleth). `props` son pares clave→valor ya stringificados (orden
/// de aparición); `numbers` son sólo las numéricas, para escalas de color.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FeatureProps {
    pub name: Option<String>,
    pub props: Vec<(String, String)>,
    pub numbers: Vec<(String, f64)>,
}

impl FeatureProps {
    /// Valor numérico de una propiedad por nombre, si existe.
    pub fn number(&self, key: &str) -> Option<f64> {
        self.numbers.iter().find(|(k, _)| k == key).map(|(_, v)| *v)
    }
}

/// Geometrías aplanadas listas para proyectar y pintar. Las geometrías
/// GeoJSON anidadas (multi-, colecciones) se desarman a estas tres listas.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MapData {
    /// Puntos sueltos (`Point`/`MultiPoint`).
    pub points: Vec<Coord>,
    /// Polilíneas (`LineString`/`MultiLineString`).
    pub lines: Vec<Ring>,
    /// Polígonos: cada uno es una lista de anillos; el primero es el
    /// contorno exterior y los siguientes, huecos. (`Polygon`/`MultiPolygon`.)
    pub polygons: Vec<Vec<Ring>>,
    /// Nombres de features (de `properties.nombre`/`name`/…) anclados a un
    /// punto representativo, para rotular el mapa.
    pub labels: Vec<Label>,
    /// Propiedades por feature. Los índices `*_feat` apuntan acá.
    pub features: Vec<FeatureProps>,
    /// Índice de feature de cada punto (paralelo a `points`).
    pub point_feat: Vec<usize>,
    /// Índice de feature de cada línea (paralelo a `lines`).
    pub line_feat: Vec<usize>,
    /// Índice de feature de cada polígono (paralelo a `polygons`).
    pub polygon_feat: Vec<usize>,
    /// Caja envolvente fija (basemap PMTiles): ancla la proyección a un marco
    /// estable para que el mapa no salte mientras llegan tiles. Si es `None`,
    /// la bbox se calcula del contenido.
    pub bbox_override: Option<BBox>,
}

impl MapData {
    /// Cantidad total de vértices retenidos.
    pub fn vertex_count(&self) -> usize {
        self.points.len()
            + self.lines.iter().map(Vec::len).sum::<usize>()
            + self
                .polygons
                .iter()
                .flat_map(|p| p.iter().map(Vec::len))
                .sum::<usize>()
    }

    /// Caja envolvente: el override fijo si está, o la de todo el contenido.
    pub fn bbox(&self) -> Option<BBox> {
        if self.bbox_override.is_some() {
            return self.bbox_override;
        }
        let mut bb = BBox::empty();
        for p in &self.points {
            bb.expand(*p);
        }
        for l in &self.lines {
            for c in l {
                bb.expand(*c);
            }
        }
        for poly in &self.polygons {
            for ring in poly {
                for c in ring {
                    bb.expand(*c);
                }
            }
        }
        if bb.is_empty() {
            None
        } else {
            Some(bb)
        }
    }

    pub fn total_features(&self) -> usize {
        self.points.len() + self.lines.len() + self.polygons.len()
    }

    /// Anexa otro `MapData`, reindexando sus features (para fusionar varios
    /// tiles en un solo mapa).
    pub fn append(&mut self, other: MapData) {
        let base = self.features.len();
        self.features.extend(other.features);
        self.labels.extend(other.labels);
        self.points.extend(other.points);
        self.point_feat
            .extend(other.point_feat.into_iter().map(|f| f + base));
        self.lines.extend(other.lines);
        self.line_feat
            .extend(other.line_feat.into_iter().map(|f| f + base));
        self.polygons.extend(other.polygons);
        self.polygon_feat
            .extend(other.polygon_feat.into_iter().map(|f| f + base));
    }
}

/// Estado del visor. Replica la forma de los otros para que el shell lo
/// trate igual.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum MapPreview {
    /// Sin archivo seleccionado.
    #[default]
    Empty,
    /// GeoJSON parseado a geometrías (posiblemente truncado por
    /// [`MAX_VERTICES`]).
    Map { data: MapData, truncated: bool },
    /// Parseó como JSON pero no contiene ninguna geometría reconocible.
    NoGeometry,
    /// Excede el tope de tamaño.
    TooBig(u64),
    /// E/S o parseo falló.
    Error(String),
}

/// Transformación de cámara del mapa: zoom (factor) + pan (desplazamiento en
/// píxeles físicos de pantalla). El host la guarda y la muta con la rueda y
/// el arrastre; el canvas la aplica al proyectar, anclando el zoom al centro
/// del panel.
///
/// La celda `rect` la **escribe el canvas** en cada paint con su rectángulo
/// físico, y la **lee el host** ([`MapView::contains`]) para acotar el
/// zoom-por-rueda al área del mapa (sin robarle el scroll a la lista).
#[derive(Clone)]
pub struct MapView {
    pub zoom: f64,
    pub pan: (f64, f64),
    /// Dibujar el mapa-base mundial de fondo.
    pub show_base: bool,
    /// Índice de la feature seleccionada (clic) en `MapData.features`, si la hay.
    pub selected: Option<usize>,
    /// Campo numérico por el que colorear los polígonos (choropleth). `None`
    /// = relleno uniforme.
    pub color_field: Option<String>,
    /// Modo búsqueda activo (captura el teclado para escribir la consulta).
    pub searching: bool,
    /// Consulta de búsqueda en curso.
    pub query: String,
    /// Modo ruteo activo (los clics fijan origen/destino).
    pub routing: bool,
    /// Puntos de ruta marcados por el usuario (0..2), en lon/lat.
    pub route_pins: Vec<Coord>,
    /// Ruta calculada (polilínea a dibujar), vacía si no hay.
    pub route_path: Vec<Coord>,
    /// Longitud de la ruta calculada, en metros.
    pub route_meters: f64,
    pub(crate) rect: Arc<Mutex<Option<(f32, f32, f32, f32)>>>,
}

impl Default for MapView {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: (0.0, 0.0),
            show_base: true,
            selected: None,
            color_field: None,
            searching: false,
            query: String::new(),
            routing: false,
            route_pins: Vec::new(),
            route_path: Vec::new(),
            route_meters: 0.0,
            rect: Arc::new(Mutex::new(None)),
        }
    }
}

impl MapView {
    /// Límites de zoom: ni tan lejos que desaparezca, ni tan cerca que se
    /// pierda en aritmética.
    pub const ZOOM_MIN: f64 = 0.2;
    pub const ZOOM_MAX: f64 = 64.0;

    /// Vuelve al encuadre inicial (zoom 1, sin pan) y limpia la selección.
    /// Conserva la celda del rect para no perder el gateo entre selecciones.
    pub fn reset(&mut self) {
        self.zoom = 1.0;
        self.pan = (0.0, 0.0);
        self.selected = None;
        self.searching = false;
        self.query.clear();
        self.routing = false;
        self.clear_route();
    }

    /// Limpia los puntos y la ruta calculada (no toca el modo).
    pub fn clear_route(&mut self) {
        self.route_pins.clear();
        self.route_path.clear();
        self.route_meters = 0.0;
    }

    /// Acumula un desplazamiento (de un arrastre), en píxeles físicos.
    pub fn pan_by(&mut self, dx: f64, dy: f64) {
        self.pan.0 += dx;
        self.pan.1 += dy;
    }

    /// Multiplica el zoom (acotado). El pan no se toca: el zoom queda
    /// anclado al centro del panel.
    pub fn zoom_by(&mut self, factor: f64) {
        if factor.is_finite() && factor > 0.0 {
            self.zoom = (self.zoom * factor).clamp(Self::ZOOM_MIN, Self::ZOOM_MAX);
        }
    }

    /// Zoom anclado a un punto de pantalla `(cx, cy)` (físicos): el lugar bajo
    /// el cursor queda fijo. Si todavía no se pintó (sin rect), cae a
    /// [`zoom_by`] (zoom al centro).
    pub fn zoom_at(&mut self, factor: f64, cx: f32, cy: f32) {
        if !(factor.is_finite() && factor > 0.0) {
            return;
        }
        let Some((rx, ry, rw, rh)) = self.rect.lock().ok().and_then(|g| *g) else {
            self.zoom_by(factor);
            return;
        };
        let pivot_x = rx as f64 + rw as f64 * 0.5;
        let pivot_y = ry as f64 + rh as f64 * 0.5;
        let z0 = self.zoom;
        let z1 = (z0 * factor).clamp(Self::ZOOM_MIN, Self::ZOOM_MAX);
        if (z1 - z0).abs() < f64::EPSILON {
            return;
        }
        // Mantener fijo el punto bajo el cursor:
        //   pan1 = pan0 - (c - pivot - pan0) * (z1 - z0) / z0
        let k = (z1 - z0) / z0;
        self.pan.0 -= (cx as f64 - pivot_x - self.pan.0) * k;
        self.pan.1 -= (cy as f64 - pivot_y - self.pan.1) * k;
        self.zoom = z1;
    }

    /// Alterna el mapa-base de fondo.
    pub fn toggle_base(&mut self) {
        self.show_base = !self.show_base;
    }

    /// `true` si `(x, y)` (físicos) cae dentro del último rect pintado por el
    /// canvas. `false` si todavía no se pintó.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        match self.rect.lock().ok().and_then(|g| *g) {
            Some((rx, ry, rw, rh)) => x >= rx && x <= rx + rw && y >= ry && y <= ry + rh,
            None => false,
        }
    }

    /// Último rect físico pintado por el canvas (si ya se pintó alguna vez).
    pub fn rect(&self) -> Option<(f32, f32, f32, f32)> {
        self.rect.lock().ok().and_then(|g| *g)
    }

    /// Registra el rect físico del canvas. Lo llama el propio canvas en cada
    /// paint; también lo usan herramientas/tests para dirigir el viewport
    /// headless (sin un paint real).
    pub fn record_rect(&self, r: (f32, f32, f32, f32)) {
        if let Ok(mut g) = self.rect.lock() {
            *g = Some(r);
        }
    }
}
