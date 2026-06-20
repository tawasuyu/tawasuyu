//! Modo `Recorrido` — presentación espacial sobre lienzo infinito (tipo Prezi).
//!
//! Un `Recorrido` coloca `Marco`s en coordenadas de mundo y define una **ruta**
//! ordenada (`pasos`) que la cámara recorre: avanzar un paso encuadra el marco
//! destino animando zoom/pan/giro desde la cámara actual. Entre pasos el usuario
//! puede volar libre (drag = pan, wheel = zoom-a-cursor).
//!
//! El strip lineal de [`crate::DeckState`] es el caso degenerado: marcos del
//! mismo tamaño en fila, zoom fijo, sin giro. Aquí el lienzo es 2D.
//!
//! Como toda pieza `*-core` del repo, esto es una máquina de estados pura: el
//! host traduce pointer/wheel/teclado → llamadas, y tick'ea la animación con
//! [`RecorridoState::avanzar`]; no hay render ni reloj propio.

use crate::camara::{Camara, Ease, Rect};

/// Duración por defecto del vuelo entre dos pasos, en segundos.
pub const DURACION_PASO_S: f64 = 0.8;

/// Tamaño mínimo (mundo) al redimensionar un marco — evita marcos degenerados.
pub const MIN_MARCO: f64 = 20.0;

pub type MarcoId = u64;

/// Qué pinta el host dentro de un marco. El core es agnóstico: guarda una
/// referencia o etiqueta y deja la resolución (cuerpo, subgrafo de átomos,
/// imagen, página de deck) al frontend vía `pluma-render-plan` u otro.
// Sin `Eq`: `Croquis` lleva `f32` (geometría normalizada), que no es `Eq`.
#[derive(Clone, Debug, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ContenidoMarco {
    #[default]
    Vacio,
    /// Texto plano de una línea — títulos de sección, hitos del recorrido.
    Etiqueta(String),
    /// Contenido de "slide": título opcional + párrafos. Agnóstico (sólo
    /// strings); un adaptador convierte un cuerpo/subgrafo de pluma a esto.
    Texto { titulo: Option<String>, parrafos: Vec<String> },
    /// Imagen rasterizada: bytes **codificados** (PNG/JPEG/WebP) + dimensiones
    /// en px. El core es agnóstico — guarda los bytes sin decodificar y deja la
    /// rasterización al frontend; `ancho`/`alto` permiten encuadrar/aspectar el
    /// marco sin tener que decodificar.
    Imagen { bytes: Vec<u8>, ancho: u32, alto: u32 },
    /// Referencia opaca que el host resuelve (hash BLAKE3, id de cuerpo, ruta…).
    Ref(String),
    /// **Croquis**: un título opcional + una lista de cajas (wireframe). Cada
    /// caja es `[x, y, w, h]` **normalizada** a `0..1` del marco. Sirve para
    /// "slides diagrama" — un esquema de rectángulos (p. ej. la miniatura de un
    /// escritorio teselado en el editor de recorrido de mirada). El frontend lo
    /// pinta como cajas dentro del marco; el core sólo guarda la geometría.
    Croquis { titulo: Option<String>, cajas: Vec<[f32; 4]> },
}

/// Un marco colocado en el lienzo: su rectángulo en coordenadas de mundo, su
/// giro propio y su contenido.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Marco {
    pub id: MarcoId,
    pub rect: Rect,
    pub rot_rad: f64,
    pub contenido: ContenidoMarco,
}

impl Marco {
    pub fn new(id: MarcoId, rect: Rect, contenido: ContenidoMarco) -> Self {
        Self { id, rect, rot_rad: 0.0, contenido }
    }

    pub fn con_giro(mut self, rot_rad: f64) -> Self {
        self.rot_rad = rot_rad;
        self
    }

    /// Cámara que encuadra este marco en `panel`.
    pub fn fit(&self, panel: Rect) -> Camara {
        Camara::fit(self.rect, self.rot_rad, panel)
    }

    /// Bounding box axis-aligned (en coordenadas de mundo) que contiene al
    /// marco **ya girado** alrededor de su centro. Para un marco recto coincide
    /// con su `rect`; para uno girado lo envuelve sin recortar esquinas. Base de
    /// [`Recorrido::bbox`] → vista general.
    pub fn aabb(&self) -> Rect {
        let (cx, cy) = self.rect.centro();
        let (hw, hh) = (self.rect.w * 0.5, self.rect.h * 0.5);
        let (s, c) = self.rot_rad.sin_cos();
        // Las cuatro esquinas relativas al centro, giradas; el AABB lo fija la
        // mayor extensión en cada eje (simétrico, así que basta el máximo).
        let ex = (hw * c).abs() + (hh * s).abs();
        let ey = (hw * s).abs() + (hh * c).abs();
        Rect::new(cx - ex, cy - ey, ex * 2.0, ey * 2.0)
    }

    /// `true` si el punto de mundo `p` cae dentro del marco, considerando su
    /// giro propio (deshace la rotación con que se dibuja antes del aabb test).
    pub fn contiene(&self, p: (f64, f64)) -> bool {
        let (cx, cy) = self.rect.centro();
        // Inversa de la rotación de dibujo: local = centro + R(-rot)·(p-centro).
        let (s, c) = (-self.rot_rad).sin_cos();
        let dx = p.0 - cx;
        let dy = p.1 - cy;
        let lx = dx * c - dy * s + cx;
        let ly = dx * s + dy * c + cy;
        lx >= self.rect.x
            && lx <= self.rect.x + self.rect.w
            && ly >= self.rect.y
            && ly <= self.rect.y + self.rect.h
    }
}

/// Lienzo + ruta narrativa. `pasos` es una secuencia de `MarcoId` (puede
/// repetir un marco, saltarse otros, o recorrerlos en cualquier orden).
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Recorrido {
    pub marcos: Vec<Marco>,
    pub pasos: Vec<MarcoId>,
}

impl Recorrido {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn agregar_marco(&mut self, marco: Marco) -> MarcoId {
        let id = marco.id;
        self.marcos.push(marco);
        id
    }

    pub fn marco(&self, id: MarcoId) -> Option<&Marco> {
        self.marcos.iter().find(|m| m.id == id)
    }

    /// Marco bajo un punto de mundo, si hay — el de más arriba (último
    /// dibujado) gana cuando se solapan. Para hit-test de autoría.
    pub fn marco_en_punto(&self, p: (f64, f64)) -> Option<MarcoId> {
        self.marcos.iter().rev().find(|m| m.contiene(p)).map(|m| m.id)
    }

    /// Traslada el marco `id` por un delta de mundo `(dx, dy)`. No-op si el id
    /// no existe. Para arrastrar marcos en modo edición.
    pub fn mover_marco(&mut self, id: MarcoId, dx: f64, dy: f64) {
        if let Some(m) = self.marcos.iter_mut().find(|m| m.id == id) {
            m.rect.x += dx;
            m.rect.y += dy;
        }
    }

    /// Elimina el marco `id` y purga **todos** los pasos que lo referencian
    /// (manteniendo el resto del guion). Devuelve `true` si el marco existía.
    /// Para autoría — deja la ruta consistente sin ids colgantes.
    pub fn eliminar_marco(&mut self, id: MarcoId) -> bool {
        let antes = self.marcos.len();
        self.marcos.retain(|m| m.id != id);
        let elimino = self.marcos.len() != antes;
        if elimino {
            self.pasos.retain(|p| *p != id);
        }
        elimino
    }

    /// Redimensiona el marco `id` a `w`×`h` (clamp a [`MIN_MARCO`]), conservando
    /// su esquina superior-izquierda. No-op si el id no existe.
    pub fn redimensionar_marco(&mut self, id: MarcoId, w: f64, h: f64) {
        if let Some(m) = self.marcos.iter_mut().find(|m| m.id == id) {
            m.rect.w = w.max(MIN_MARCO);
            m.rect.h = h.max(MIN_MARCO);
        }
    }

    /// Suma `delta_rad` al giro propio del marco `id`. No-op si no existe.
    pub fn rotar_marco(&mut self, id: MarcoId, delta_rad: f64) {
        if let Some(m) = self.marcos.iter_mut().find(|m| m.id == id) {
            m.rot_rad += delta_rad;
        }
    }

    /// Reordena el guion: mueve el paso en el índice `desde` a la posición
    /// `hasta` (clamp al final). No-op si `desde` está fuera de rango.
    pub fn mover_paso(&mut self, desde: usize, hasta: usize) {
        if desde >= self.pasos.len() {
            return;
        }
        let id = self.pasos.remove(desde);
        self.pasos.insert(hasta.min(self.pasos.len()), id);
    }

    /// Marco al que apunta el paso `idx` (resolviendo el id contra `marcos`).
    pub fn marco_en_paso(&self, idx: usize) -> Option<&Marco> {
        self.pasos.get(idx).and_then(|id| self.marco(*id))
    }

    pub fn n_pasos(&self) -> usize {
        self.pasos.len()
    }

    /// Bounding box de **todos** los marcos (cada uno por su [`Marco::aabb`], así
    /// los girados entran enteros). `None` si no hay marcos. Es el encuadre de la
    /// *vista general* — el zoom-out narrativo que muestra el mapa completo.
    pub fn bbox(&self) -> Option<Rect> {
        let mut it = self.marcos.iter().map(Marco::aabb);
        let primero = it.next()?;
        let (mut min_x, mut min_y) = (primero.x, primero.y);
        let (mut max_x, mut max_y) = (primero.x + primero.w, primero.y + primero.h);
        for r in it {
            min_x = min_x.min(r.x);
            min_y = min_y.min(r.y);
            max_x = max_x.max(r.x + r.w);
            max_y = max_y.max(r.y + r.h);
        }
        Some(Rect::new(min_x, min_y, max_x - min_x, max_y - min_y))
    }

    /// Auto-layout: coloca una secuencia de contenidos en una rejilla y arma
    /// la ruta en orden de lectura (fila por fila). Es el "dame N piezas →
    /// dame un recorrido listo" — el frontend sólo pinta y vuela. Los ids se
    /// asignan `1..=n` en orden.
    pub fn en_rejilla(contenidos: Vec<ContenidoMarco>, opts: RejillaOpts) -> Recorrido {
        let cols = opts.cols.max(1);
        let mut rec = Recorrido::new();
        for (i, c) in contenidos.into_iter().enumerate() {
            let col = (i % cols) as f64;
            let row = (i / cols) as f64;
            let x = col * (opts.marco_w + opts.gap_x);
            let y = row * (opts.marco_h + opts.gap_y);
            let id = (i + 1) as MarcoId;
            rec.agregar_marco(Marco::new(id, Rect::new(x, y, opts.marco_w, opts.marco_h), c));
            rec.pasos.push(id);
        }
        rec
    }
}

#[cfg(feature = "serde")]
impl Recorrido {
    /// Serializa el recorrido (marcos + ruta) a su forma binaria `postcard` —
    /// el codec nativo del workspace (mismo que `format`/`akasha`/`pluma-cuerpo`).
    /// Persiste sólo el modelo de datos; el estado de interacción
    /// ([`RecorridoState`]) es efímero y no se guarda.
    pub fn serializar(&self) -> Result<Vec<u8>, &'static str> {
        postcard::to_allocvec(self).map_err(|_| "recorrido :: serializacion fallida")
    }

    /// Reconstruye un recorrido desde su forma binaria `postcard`.
    pub fn deserializar(bytes: &[u8]) -> Result<Recorrido, &'static str> {
        postcard::from_bytes::<Recorrido>(bytes).map_err(|_| "recorrido :: deserializacion fallida")
    }
}

/// Parámetros del auto-layout en rejilla de [`Recorrido::en_rejilla`].
#[derive(Clone, Copy, Debug)]
pub struct RejillaOpts {
    pub cols: usize,
    pub marco_w: f64,
    pub marco_h: f64,
    pub gap_x: f64,
    pub gap_y: f64,
}

impl Default for RejillaOpts {
    fn default() -> Self {
        Self { cols: 3, marco_w: 640.0, marco_h: 400.0, gap_x: 220.0, gap_y: 180.0 }
    }
}

/// Animación de cámara en curso entre dos encuadres.
#[derive(Clone, Copy, Debug)]
struct Vuelo {
    desde: Camara,
    hasta: Camara,
    /// Tiempo transcurrido / duración total, en segundos.
    t: f64,
    dur: f64,
    ease: Ease,
}

/// Máquina de interacción del recorrido: cámara viva + paso actual + vuelo en
/// curso + estado de arrastre para el paneo libre.
#[derive(Clone, Debug)]
pub struct RecorridoState {
    pub camara: Camara,
    /// Índice del paso actual dentro de `Recorrido::pasos`.
    pub paso: usize,
    vuelo: Option<Vuelo>,
    arrastre: Option<(f64, f64)>,
}

impl Default for RecorridoState {
    fn default() -> Self {
        Self { camara: Camara::default(), paso: 0, vuelo: None, arrastre: None }
    }
}

impl RecorridoState {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` si hay un vuelo de cámara animándose.
    pub fn animando(&self) -> bool {
        self.vuelo.is_some()
    }

    // ---- Roam libre -------------------------------------------------------

    /// Inicio de arrastre para panear. Cancela cualquier vuelo en curso (el
    /// usuario toma el control manual).
    pub fn pointer_down(&mut self, x: f64, y: f64) {
        self.vuelo = None;
        self.arrastre = Some((x, y));
    }

    /// Movimiento de puntero: si hay arrastre activo, panea la cámara por el
    /// delta y devuelve `true` (el host debe repintar).
    pub fn pointer_move(&mut self, x: f64, y: f64) -> bool {
        let Some((px, py)) = self.arrastre else { return false };
        self.camara.pan(x - px, y - py);
        self.arrastre = Some((x, y));
        true
    }

    pub fn pointer_up(&mut self) {
        self.arrastre = None;
    }

    /// Paneo por delta de pantalla — para hosts que ya entregan el delta del
    /// arrastre (p. ej. `llimphi-ui::draggable`, que da `(dx, dy)` por evento).
    /// Cancela el vuelo (control manual). Alternativa a `pointer_down/move/up`.
    pub fn arrastrar_delta(&mut self, dx: f64, dy: f64) {
        self.vuelo = None;
        self.camara.pan(dx, dy);
    }

    /// Wheel: zoom-a-cursor inmediato. Cancela el vuelo (control manual).
    pub fn wheel(&mut self, mult: f64, cursor: (f64, f64), panel: Rect) {
        self.vuelo = None;
        self.camara.zoom_a_cursor(mult, cursor, panel);
    }

    // ---- Reproducción guiada ---------------------------------------------

    /// Arranca un vuelo desde la cámara actual hasta encuadrar el paso `idx`.
    /// No hace nada si el índice o su marco no existen. Fija `paso = idx`.
    pub fn ir_a_paso(&mut self, rec: &Recorrido, idx: usize, panel: Rect) {
        let Some(marco) = rec.marco_en_paso(idx) else { return };
        self.paso = idx;
        self.iniciar_vuelo(marco.fit(panel), DURACION_PASO_S);
    }

    /// Avanza al paso siguiente (clamp al final). Devuelve `true` si arrancó
    /// un vuelo nuevo.
    pub fn siguiente(&mut self, rec: &Recorrido, panel: Rect) -> bool {
        if rec.n_pasos() == 0 || self.paso + 1 >= rec.n_pasos() {
            return false;
        }
        self.ir_a_paso(rec, self.paso + 1, panel);
        true
    }

    /// Retrocede al paso anterior (clamp en 0). Devuelve `true` si arrancó un
    /// vuelo nuevo.
    pub fn anterior(&mut self, rec: &Recorrido, panel: Rect) -> bool {
        if self.paso == 0 {
            return false;
        }
        self.ir_a_paso(rec, self.paso - 1, panel);
        true
    }

    /// Vuela a la **vista general**: aleja la cámara hasta encuadrar todos los
    /// marcos (recta, sin giro), el gesto-firma de Prezi "alejarse para ver el
    /// mapa". No toca `paso` — `siguiente`/`anterior` siguen desde donde iban.
    /// Devuelve `true` si arrancó un vuelo (`false` si el lienzo está vacío).
    pub fn vista_general(&mut self, rec: &Recorrido, panel: Rect) -> bool {
        let Some(bbox) = rec.bbox() else { return false };
        self.iniciar_vuelo(Camara::fit(bbox, 0.0, panel), DURACION_PASO_S);
        true
    }

    /// Salto instantáneo (sin vuelo) al encuadre del paso `idx` — útil para
    /// reposicionar tras un resize, o para "jump to" sin animación.
    pub fn saltar_a_paso(&mut self, rec: &Recorrido, idx: usize, panel: Rect) {
        let Some(marco) = rec.marco_en_paso(idx) else { return };
        self.paso = idx;
        self.vuelo = None;
        self.camara = marco.fit(panel);
    }

    fn iniciar_vuelo(&mut self, hasta: Camara, dur: f64) {
        if dur <= 0.0 {
            self.camara = hasta;
            self.vuelo = None;
            return;
        }
        self.vuelo = Some(Vuelo { desde: self.camara, hasta, t: 0.0, dur, ease: Ease::default() });
    }

    /// Avanza la animación `dt` segundos. Devuelve `true` mientras siga
    /// animando (el host repite el tick); `false` cuando ya no hay vuelo.
    /// El host la llama desde un timer (p. ej. `Handle::spawn_periodic`).
    pub fn avanzar(&mut self, dt: f64) -> bool {
        let Some(mut v) = self.vuelo else { return false };
        v.t += dt;
        if v.t >= v.dur {
            self.camara = v.hasta;
            self.vuelo = None;
            return false;
        }
        self.camara = Camara::interpolar(&v.desde, &v.hasta, v.t / v.dur, v.ease);
        self.vuelo = Some(v);
        true
    }
}

/// Reproducción automática ("modo presentador"): tras aterrizar en un paso
/// espera `dwell_s` segundos y avanza solo al siguiente. Al llegar al final,
/// vuelve al inicio si `bucle`, o se detiene. Máquina de tiempo **pura** — el
/// host la tickea junto a [`RecorridoState::avanzar`]; no tiene reloj propio.
#[derive(Clone, Copy, Debug)]
pub struct Autoplay {
    /// Segundos de permanencia en cada paso una vez que el vuelo aterrizó.
    pub dwell_s: f64,
    /// Si al final vuelve al primer paso (`true`) o se detiene (`false`).
    pub bucle: bool,
    activo: bool,
    espera: f64,
}

/// Dwell por defecto del modo presentador, en segundos.
pub const DWELL_S: f64 = 2.5;

impl Default for Autoplay {
    fn default() -> Self {
        Self { dwell_s: DWELL_S, bucle: true, activo: false, espera: 0.0 }
    }
}

impl Autoplay {
    pub fn new(dwell_s: f64, bucle: bool) -> Self {
        Self { dwell_s, bucle, activo: false, espera: 0.0 }
    }

    pub fn activo(&self) -> bool {
        self.activo
    }

    /// Arranca la reproducción (resetea el contador de permanencia).
    pub fn play(&mut self) {
        self.activo = true;
        self.espera = 0.0;
    }

    pub fn pausa(&mut self) {
        self.activo = false;
    }

    /// Alterna play/pausa. Devuelve el nuevo estado.
    pub fn toggle(&mut self) -> bool {
        if self.activo {
            self.pausa();
        } else {
            self.play();
        }
        self.activo
    }

    /// Tick del modo presentador. No hace nada si está pausado. Mientras el
    /// vuelo se anima, espera (no acumula dwell). Una vez quieto, acumula `dt`;
    /// al superar `dwell_s` avanza un paso (o vuelve al inicio si `bucle`; o se
    /// detiene). Devuelve `true` si disparó un avance este tick.
    pub fn tick(&mut self, dt: f64, state: &mut RecorridoState, rec: &Recorrido, panel: Rect) -> bool {
        if !self.activo || rec.n_pasos() == 0 {
            return false;
        }
        if state.animando() {
            self.espera = 0.0;
            return false;
        }
        self.espera += dt;
        if self.espera < self.dwell_s {
            return false;
        }
        self.espera = 0.0;
        if state.paso + 1 < rec.n_pasos() {
            state.siguiente(rec, panel);
        } else if self.bucle {
            state.ir_a_paso(rec, 0, panel);
        } else {
            self.activo = false;
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANEL: Rect = Rect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 };

    fn recorrido_demo() -> Recorrido {
        let mut r = Recorrido::new();
        r.agregar_marco(Marco::new(1, Rect::new(0.0, 0.0, 400.0, 300.0), ContenidoMarco::Etiqueta("a".into())));
        r.agregar_marco(Marco::new(2, Rect::new(2000.0, 0.0, 200.0, 150.0), ContenidoMarco::Etiqueta("b".into())));
        r.agregar_marco(Marco::new(3, Rect::new(1000.0, 1000.0, 800.0, 600.0), ContenidoMarco::Etiqueta("c".into())));
        r.pasos = vec![1, 2, 3];
        r
    }

    #[test]
    fn en_rejilla_coloca_y_rutea_en_orden_de_lectura() {
        let contenidos = vec![
            ContenidoMarco::Etiqueta("a".into()),
            ContenidoMarco::Etiqueta("b".into()),
            ContenidoMarco::Etiqueta("c".into()),
            ContenidoMarco::Etiqueta("d".into()),
        ];
        let opts = RejillaOpts { cols: 2, marco_w: 100.0, marco_h: 50.0, gap_x: 20.0, gap_y: 10.0 };
        let rec = Recorrido::en_rejilla(contenidos, opts);
        assert_eq!(rec.marcos.len(), 4);
        // Ruta secuencial 1..=4.
        assert_eq!(rec.pasos, vec![1, 2, 3, 4]);
        // Índice 0 en (0,0); índice 1 en la columna siguiente; índice 2 baja de fila.
        assert_eq!(rec.marco(1).unwrap().rect, Rect::new(0.0, 0.0, 100.0, 50.0));
        assert_eq!(rec.marco(2).unwrap().rect, Rect::new(120.0, 0.0, 100.0, 50.0));
        assert_eq!(rec.marco(3).unwrap().rect, Rect::new(0.0, 60.0, 100.0, 50.0));
    }

    #[test]
    fn marco_en_punto_devuelve_el_de_arriba() {
        let r = recorrido_demo();
        // Punto dentro del marco 1 (0,0,400,300).
        assert_eq!(r.marco_en_punto((10.0, 10.0)), Some(1));
        // Punto dentro del marco 2 (2000,0,200,150).
        assert_eq!(r.marco_en_punto((2050.0, 50.0)), Some(2));
        // Punto en el vacío.
        assert_eq!(r.marco_en_punto((-500.0, -500.0)), None);
    }

    #[test]
    fn marco_en_punto_respeta_giro() {
        let mut r = Recorrido::new();
        // Marco cuadrado centrado en (0,0), girado 45°.
        r.agregar_marco(
            Marco::new(7, Rect::new(-50.0, -50.0, 100.0, 100.0), ContenidoMarco::Vacio)
                .con_giro(std::f64::consts::FRAC_PI_4),
        );
        // El centro siempre está dentro.
        assert_eq!(r.marco_en_punto((0.0, 0.0)), Some(7));
        // Una esquina del aabb sin girar (49,49) queda FUERA del cuadrado girado
        // (su semidiagonal es ~70.7, pero el lado rotado pasa antes por los ejes).
        assert_eq!(r.marco_en_punto((49.0, 49.0)), None);
        // Sobre el eje X a distancia 60 < semidiagonal: dentro del rombo.
        assert_eq!(r.marco_en_punto((60.0, 0.0)), Some(7));
    }

    #[test]
    fn marco_con_imagen_es_agnostico_al_hit_test() {
        // El core no decodifica la imagen: guarda bytes + dims y el hit-test
        // sigue dependiendo sólo de la geometría del marco.
        let mut r = Recorrido::new();
        let img = ContenidoMarco::Imagen { bytes: vec![0xDE, 0xAD, 0xBE, 0xEF], ancho: 320, alto: 240 };
        r.agregar_marco(Marco::new(5, Rect::new(0.0, 0.0, 400.0, 300.0), img.clone()));
        assert_eq!(r.marco_en_punto((100.0, 100.0)), Some(5));
        assert_eq!(r.marco_en_punto((9999.0, 0.0)), None);
        // La variante conserva bytes y dimensiones tal cual.
        assert_eq!(r.marco(5).unwrap().contenido, img);
    }

    #[test]
    fn mover_marco_traslada_el_rect() {
        let mut r = recorrido_demo();
        r.mover_marco(1, 100.0, -40.0);
        assert_eq!(r.marco(1).unwrap().rect, Rect::new(100.0, -40.0, 400.0, 300.0));
        // Id inexistente: no-op.
        r.mover_marco(999, 1.0, 1.0);
    }

    #[test]
    fn eliminar_marco_purga_la_ruta() {
        let mut r = recorrido_demo(); // marcos 1,2,3; pasos [1,2,3]
        r.pasos = vec![1, 2, 3, 2]; // el 2 aparece dos veces en el guion
        assert!(r.eliminar_marco(2));
        assert!(r.marco(2).is_none());
        assert_eq!(r.pasos, vec![1, 3], "se purgan TODAS las apariciones del 2");
        // Id inexistente: no-op, devuelve false.
        assert!(!r.eliminar_marco(999));
        assert_eq!(r.marcos.len(), 2);
    }

    #[test]
    fn redimensionar_marco_clampa_al_minimo() {
        let mut r = recorrido_demo();
        r.redimensionar_marco(1, 500.0, 5.0); // alto por debajo del mínimo
        let m = r.marco(1).unwrap();
        assert_eq!(m.rect.w, 500.0);
        assert_eq!(m.rect.h, MIN_MARCO);
        // Conserva la esquina sup-izq (marco 1 estaba en 0,0).
        assert_eq!((m.rect.x, m.rect.y), (0.0, 0.0));
    }

    #[test]
    fn rotar_marco_acumula() {
        let mut r = recorrido_demo();
        r.rotar_marco(1, 0.3);
        r.rotar_marco(1, 0.2);
        assert!((r.marco(1).unwrap().rot_rad - 0.5).abs() < 1e-12);
        r.rotar_marco(404, 1.0); // inexistente: no-op
    }

    #[test]
    fn mover_paso_reordena_el_guion() {
        let mut r = recorrido_demo(); // pasos [1,2,3]
        r.mover_paso(0, 2); // lleva el primero al final
        assert_eq!(r.pasos, vec![2, 3, 1]);
        r.mover_paso(2, 0); // y de vuelta al inicio
        assert_eq!(r.pasos, vec![1, 2, 3]);
        // hasta fuera de rango → clamp al final; desde fuera de rango → no-op.
        r.mover_paso(0, 99);
        assert_eq!(r.pasos, vec![2, 3, 1]);
        r.mover_paso(99, 0);
        assert_eq!(r.pasos, vec![2, 3, 1]);
    }

    #[test]
    fn marco_en_paso_resuelve_id() {
        let r = recorrido_demo();
        assert_eq!(r.marco_en_paso(1).unwrap().id, 2);
        assert!(r.marco_en_paso(9).is_none());
    }

    #[test]
    fn pan_libre_mueve_y_cancela_vuelo() {
        let r = recorrido_demo();
        let mut s = RecorridoState::new();
        s.ir_a_paso(&r, 1, PANEL);
        assert!(s.animando());
        s.pointer_down(100.0, 100.0);
        assert!(!s.animando(), "el drag cancela el vuelo");
        assert!(s.pointer_move(130.0, 100.0));
        s.pointer_up();
        assert!(!s.pointer_move(200.0, 200.0), "sin arrastre no panea");
    }

    #[test]
    fn arrastrar_delta_panea_y_cancela_vuelo() {
        let r = recorrido_demo();
        let mut s = RecorridoState::new();
        s.ir_a_paso(&r, 1, PANEL);
        s.camara.zoom = 2.0;
        let antes = s.camara.world_to_screen((0.0, 0.0), PANEL);
        s.arrastrar_delta(40.0, -20.0);
        assert!(!s.animando());
        let despues = s.camara.world_to_screen((0.0, 0.0), PANEL);
        assert!((despues.0 - antes.0 - 40.0).abs() < 1e-9);
        assert!((despues.1 - antes.1 + 20.0).abs() < 1e-9);
    }

    #[test]
    fn wheel_hace_zoom_y_cancela_vuelo() {
        let r = recorrido_demo();
        let mut s = RecorridoState::new();
        s.ir_a_paso(&r, 1, PANEL);
        let z = s.camara.zoom;
        s.wheel(1.1, (400.0, 300.0), PANEL);
        assert!(!s.animando());
        assert!((s.camara.zoom - z * 1.1).abs() < 1e-9);
    }

    #[test]
    fn siguiente_y_anterior_respetan_bordes() {
        let r = recorrido_demo();
        let mut s = RecorridoState::new();
        assert!(!s.anterior(&r, PANEL), "ya en el primero");
        assert!(s.siguiente(&r, PANEL));
        assert_eq!(s.paso, 1);
        assert!(s.siguiente(&r, PANEL));
        assert_eq!(s.paso, 2);
        assert!(!s.siguiente(&r, PANEL), "ya en el último");
        assert!(s.anterior(&r, PANEL));
        assert_eq!(s.paso, 1);
    }

    #[test]
    fn avanzar_completa_el_vuelo_y_aterriza_exacto() {
        let r = recorrido_demo();
        let mut s = RecorridoState::new();
        s.ir_a_paso(&r, 2, PANEL);
        let objetivo = r.marco_en_paso(2).unwrap().fit(PANEL);
        // Tickea en pasos hasta que el vuelo termina.
        let mut iter = 0;
        while s.avanzar(0.1) {
            iter += 1;
            assert!(iter < 1000, "el vuelo no converge");
        }
        // Al terminar aterriza EXACTAMENTE en el encuadre objetivo.
        assert_eq!(s.camara, objetivo);
        assert!(!s.animando());
        // Un avanzar extra no hace nada.
        assert!(!s.avanzar(0.1));
    }

    #[test]
    fn aabb_recto_coincide_con_rect_y_girado_lo_envuelve() {
        // Marco recto: el aabb es su propio rect.
        let m = Marco::new(1, Rect::new(10.0, 20.0, 100.0, 60.0), ContenidoMarco::Vacio);
        assert_eq!(m.aabb(), Rect::new(10.0, 20.0, 100.0, 60.0));
        // Cuadrado 100×100 centrado en (0,0) girado 45°: su aabb es el cuadrado
        // que lo circunscribe, lado = diagonal = 100·√2 ≈ 141.42.
        let g = Marco::new(2, Rect::new(-50.0, -50.0, 100.0, 100.0), ContenidoMarco::Vacio)
            .con_giro(std::f64::consts::FRAC_PI_4);
        let bb = g.aabb();
        let lado = 100.0 * std::f64::consts::SQRT_2;
        assert!((bb.w - lado).abs() < 1e-6 && (bb.h - lado).abs() < 1e-6, "{bb:?}");
        assert!((bb.centro().0).abs() < 1e-9 && (bb.centro().1).abs() < 1e-9);
    }

    #[test]
    fn bbox_une_todos_los_marcos() {
        let r = recorrido_demo();
        // Marcos: (0,0,400,300), (2000,0,200,150), (1000,1000,800,600).
        let bb = r.bbox().unwrap();
        assert_eq!(bb, Rect::new(0.0, 0.0, 2200.0, 1600.0));
        assert!(Recorrido::new().bbox().is_none(), "lienzo vacío no tiene bbox");
    }

    #[test]
    fn vista_general_vuela_a_encuadrar_todo_sin_tocar_el_paso() {
        let r = recorrido_demo();
        let mut s = RecorridoState::new();
        s.ir_a_paso(&r, 2, PANEL);
        while s.avanzar(0.1) {}
        assert_eq!(s.paso, 2);
        assert!(s.vista_general(&r, PANEL));
        assert!(s.animando());
        // No cambió el paso narrativo.
        assert_eq!(s.paso, 2);
        while s.avanzar(0.1) {}
        // Aterriza en el fit del bbox completo, recto.
        let objetivo = Camara::fit(r.bbox().unwrap(), 0.0, PANEL);
        assert_eq!(s.camara, objetivo);
        // Lienzo vacío: no-op.
        assert!(!RecorridoState::new().vista_general(&Recorrido::new(), PANEL));
    }

    /// Tickea estado + autoplay hasta que el autoplay dispare un avance (o se
    /// agote el presupuesto de iteraciones). Simula el bucle del host.
    fn correr_hasta_avance(ap: &mut Autoplay, s: &mut RecorridoState, r: &Recorrido) -> bool {
        for _ in 0..100_000 {
            s.avanzar(1.0 / 60.0);
            if ap.tick(1.0 / 60.0, s, r, PANEL) {
                return true;
            }
        }
        false
    }

    #[test]
    fn autoplay_pausado_no_hace_nada() {
        let r = recorrido_demo();
        let mut s = RecorridoState::new();
        let mut ap = Autoplay::new(0.5, false);
        assert!(!ap.activo());
        // Sin play, mil ticks no mueven el paso.
        for _ in 0..1000 {
            assert!(!ap.tick(1.0 / 60.0, &mut s, &r, PANEL));
        }
        assert_eq!(s.paso, 0);
    }

    #[test]
    fn autoplay_avanza_tras_el_dwell_esperando_a_que_aterrice_el_vuelo() {
        let r = recorrido_demo();
        let mut s = RecorridoState::new();
        let mut ap = Autoplay::new(0.5, false);
        ap.play();
        assert!(ap.toggle() == false, "toggle desde activo pausa");
        ap.play();
        // Mientras el contador de dwell no llega, no avanza; el avance ocurre
        // recién tras ~0.5s quietos.
        assert!(correr_hasta_avance(&mut ap, &mut s, &r));
        assert_eq!(s.paso, 1);
        // Y respeta que el vuelo aterrice antes de contar el siguiente dwell.
        assert!(correr_hasta_avance(&mut ap, &mut s, &r));
        assert_eq!(s.paso, 2);
    }

    #[test]
    fn autoplay_al_final_sin_bucle_se_detiene_y_con_bucle_reinicia() {
        let r = recorrido_demo(); // 3 pasos
        // Sin bucle: tras el último, se desactiva solo.
        let mut s = RecorridoState::new();
        let mut ap = Autoplay::new(0.2, false);
        ap.play();
        correr_hasta_avance(&mut ap, &mut s, &r); // → paso 1
        correr_hasta_avance(&mut ap, &mut s, &r); // → paso 2 (último)
        assert_eq!(s.paso, 2);
        // En el último, el dwell vence pero no avanza: se apaga.
        let arranco = correr_hasta_avance(&mut ap, &mut s, &r);
        assert!(!arranco && !ap.activo(), "sin bucle se detiene en el final");
        // Con bucle: del último vuelve al inicio.
        let mut s = RecorridoState::new();
        s.saltar_a_paso(&r, 2, PANEL);
        let mut ap = Autoplay::new(0.2, true);
        ap.play();
        assert!(correr_hasta_avance(&mut ap, &mut s, &r));
        assert_eq!(s.paso, 0, "con bucle reinicia");
    }

    #[cfg(feature = "serde")]
    #[test]
    fn roundtrip_postcard_preserva_marcos_ruta_y_contenido() {
        let mut r = recorrido_demo();
        // Un marco con giro, imagen (bytes crudos) y texto, para cubrir variantes.
        r.agregar_marco(
            Marco::new(4, Rect::new(10.0, 20.0, 300.0, 200.0), ContenidoMarco::Imagen {
                bytes: vec![1, 2, 3, 4, 5],
                ancho: 64,
                alto: 48,
            })
            .con_giro(0.37),
        );
        r.agregar_marco(Marco::new(
            5,
            Rect::new(0.0, 0.0, 100.0, 100.0),
            ContenidoMarco::Texto { titulo: Some("T".into()), parrafos: vec!["p1".into(), "p2".into()] },
        ));
        r.pasos = vec![1, 2, 3, 4, 5];
        let bytes = r.serializar().unwrap();
        let r2 = Recorrido::deserializar(&bytes).unwrap();
        assert_eq!(r2.pasos, r.pasos);
        assert_eq!(r2.marcos.len(), r.marcos.len());
        // El marco girado con imagen conserva geometría, giro y bytes.
        let m4 = r2.marco(4).unwrap();
        assert_eq!(m4.rect, Rect::new(10.0, 20.0, 300.0, 200.0));
        assert!((m4.rot_rad - 0.37).abs() < 1e-12);
        assert_eq!(
            m4.contenido,
            ContenidoMarco::Imagen { bytes: vec![1, 2, 3, 4, 5], ancho: 64, alto: 48 }
        );
        // Bytes corruptos no panican: error controlado.
        assert!(Recorrido::deserializar(&[0xFF]).is_err());
    }

    #[test]
    fn saltar_a_paso_es_instantaneo() {
        let r = recorrido_demo();
        let mut s = RecorridoState::new();
        s.saltar_a_paso(&r, 2, PANEL);
        assert!(!s.animando());
        assert_eq!(s.paso, 2);
        assert_eq!(s.camara, r.marco_en_paso(2).unwrap().fit(PANEL));
    }
}
