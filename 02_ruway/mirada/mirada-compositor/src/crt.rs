//! Estado de la transición **CRT** (apagado/encendido «TV antigua») de la
//! pantalla por inactividad: al apagar, la imagen colapsa a una línea horizontal
//! → un punto → negro (el clásico tubo de rayos catódicos al cortarse); al
//! despertar, lo inverso. Es puro tiempo + geometría (un escalado **no uniforme**
//! del contenido hacia el centro) — sin shader: el render del backend DRM
//! envuelve la escena compuesta en un `RescaleRenderElement` con la escala que da
//! [`CrtAnim::scale`], igual que la lupa pero encogiendo. Sólo DRM (winit no
//! tiene DPMS). Pura y testeada; el «porqué» vive en `PLAN.md` §«Animaciones de
//! transición».

/// Duración de la transición CRT, en segundos (~el rango que pide el PLAN).
pub(crate) const CRT_SECS: f32 = 0.32;

/// Grosor mínimo de la «línea» del tubo, como fracción del lado: el contenido
/// nunca llega a 0 en la fase de línea (si no, desaparecería antes del punto).
const THIN: f32 = 0.006;

/// Fracción del progreso de colapso dedicada a aplastar la **vertical** (a una
/// línea); el resto aplasta la **horizontal** (la línea a un punto).
const Y_PHASE: f32 = 0.7;

/// Hacia dónde va la transición.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CrtKind {
    /// Apagar: la pantalla colapsa (pantalla llena → línea → punto → negro).
    Collapse,
    /// Encender: la inversa (punto → línea → pantalla llena).
    Restore,
}

/// El progreso de una transición CRT: cuánto lleva (`t ∈ [0,1]`) y hacia dónde.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CrtAnim {
    pub kind: CrtKind,
    t: f32,
    dur: f32,
}

impl CrtAnim {
    /// Arranca un **colapso** (apagar la pantalla).
    pub fn collapse() -> Self {
        Self { kind: CrtKind::Collapse, t: 0.0, dur: CRT_SECS.max(0.001) }
    }

    /// Arranca una **restauración** (encender la pantalla).
    pub fn restore() -> Self {
        Self { kind: CrtKind::Restore, t: 0.0, dur: CRT_SECS.max(0.001) }
    }

    /// Avanza `dt` segundos. Devuelve `true` cuando la transición terminó
    /// (`t >= 1`): el llamante apaga el DPMS (colapso) o sigue al render normal
    /// (restauración).
    pub fn advance(&mut self, dt: f32) -> bool {
        self.t = (self.t + dt.max(0.0) / self.dur).min(1.0);
        self.t >= 1.0
    }

    /// Progreso de **colapso** `p ∈ [0,1]` (0 = pantalla llena, 1 = punto/negro),
    /// independiente del sentido: en `Restore` el tiempo corre pero el colapso va
    /// de 1 a 0 (se expande).
    fn collapse_progress(&self) -> f32 {
        match self.kind {
            CrtKind::Collapse => self.t,
            CrtKind::Restore => 1.0 - self.t,
        }
    }

    /// La escala `(sx, sy)` que el render aplica al contenido (alrededor del
    /// centro). Dos fases sobre el progreso de colapso `p`: primero aplasta la
    /// vertical a una línea (`sy: 1 → THIN`, `sx = 1`), luego la línea a un punto
    /// (`sx: 1 → 0`, `sy = THIN`). En reposo (`p = 0`) es `(1, 1)` — identidad.
    pub fn scale(&self) -> (f32, f32) {
        let p = self.collapse_progress();
        if p <= 0.0 {
            return (1.0, 1.0);
        }
        if p < Y_PHASE {
            let k = smoothstep(p / Y_PHASE); // 0 → 1
            let sy = 1.0 + (THIN - 1.0) * k; // 1 → THIN
            (1.0, sy)
        } else {
            let k = smoothstep((p - Y_PHASE) / (1.0 - Y_PHASE)); // 0 → 1
            let sx = (1.0 - k).max(0.0); // 1 → 0
            (sx, THIN)
        }
    }

    /// Alfa del **flash** blanco (el destello del tubo): una campana centrada en
    /// el instante en que se forma la línea (`p ≈ Y_PHASE`), tope `0.5`. `0` lejos.
    pub fn flash_alpha(&self) -> f32 {
        let p = self.collapse_progress();
        let d = (p - Y_PHASE).abs();
        ((1.0 - d / 0.18).clamp(0.0, 1.0)) * 0.5
    }
}

/// Smoothstep clásico (`3x²−2x³`), acotado a `[0,1]`.
fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colapso_arranca_lleno_y_termina_en_punto() {
        let a = CrtAnim::collapse();
        // En t=0: identidad (pantalla llena).
        assert_eq!(a.scale(), (1.0, 1.0));
        // Al final: un punto (ambas escalas ~0).
        let mut a = CrtAnim::collapse();
        assert!(a.advance(CRT_SECS));
        let (sx, sy) = a.scale();
        assert!(sx <= 0.001, "sx={sx}");
        assert!(sy <= THIN + 0.001, "sy={sy}");
    }

    #[test]
    fn fase_y_primero_aplasta_la_vertical_dejando_el_ancho() {
        let mut a = CrtAnim::collapse();
        a.advance(CRT_SECS * (Y_PHASE * 0.5)); // mitad de la fase Y
        let (sx, sy) = a.scale();
        assert!((sx - 1.0).abs() < 1e-6, "el ancho se conserva en la fase Y: {sx}");
        assert!(sy < 1.0 && sy > THIN, "la altura ya bajó pero no es la línea: {sy}");
    }

    #[test]
    fn restore_es_el_espejo_temporal_del_colapso() {
        // Restore en t=0 está en el punto; al final, lleno.
        let r = CrtAnim::restore();
        let (sx, sy) = r.scale();
        assert!(sx <= 0.001 && sy <= THIN + 0.001, "restore arranca en el punto");
        let mut r = CrtAnim::restore();
        assert!(r.advance(CRT_SECS));
        assert_eq!(r.scale(), (1.0, 1.0));
    }

    #[test]
    fn el_flash_pica_al_formarse_la_linea_y_es_cero_en_los_extremos() {
        // En p≈Y_PHASE el flash es máximo; en p=0 (lleno) es 0.
        let lleno = CrtAnim::collapse();
        assert_eq!(lleno.flash_alpha(), 0.0);
        let mut linea = CrtAnim::collapse();
        linea.advance(CRT_SECS * Y_PHASE);
        assert!(linea.flash_alpha() > 0.4, "pico de flash en la línea");
    }

    #[test]
    fn advance_satura_en_uno() {
        let mut a = CrtAnim::collapse();
        assert!(a.advance(CRT_SECS * 10.0)); // dt enorme → completa
        // Idempotente tras completar.
        assert!(a.advance(0.1));
    }
}
