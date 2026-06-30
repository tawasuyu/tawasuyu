//! `tullpu-icon-core` — el hueso del generador de íconos.
//!
//! La tubería del generador converge en **un artefacto declarativo**, el
//! [`IconSpec`]: una descripción de un ícono como *composición de primitivas
//! vectoriales* sobre la grilla canónica 24×24 (la misma de `llimphi-icons`).
//! Un `IconSpec` se **compila determinísticamente** a `Vec<ParamsVector>` de
//! `tullpu-core` — el formato vectorial nativo del editor — y de ahí cae en
//! cualquiera de los dos stacks vectoriales de la suite:
//!
//! ```text
//!                        IconSpec  (declarativo, serde)
//!                           │  compilar(resolver)  ── DETERMINISTA
//!             ┌─────────────┼──────────────┐
//!             ▼             ▼               ▼
//!      Vec<ParamsVector>  Vec<BezPath>     SVG
//!       (este crate)      (llimphi-icons)  (foreign-svg)
//! ```
//!
//! El **motor híbrido** se apoya en esto: la parte paramétrica construye el
//! `IconSpec` con recetas (vocabulario de [`Forma`] + color de marca); la parte
//! IA sólo tiene que *proponer* un `IconSpec` (no dibujar píxeles), y el
//! compilador lo materializa en vectores limpios y **recolorables**.
//!
//! Este crate define **sólo el modelo y la compilación a vectores**. Sin
//! rasterización (vive en `tullpu-ops`), sin export PNG/SVG (en `tullpu-render`
//! / `foreign-svg`), sin IA (en `pixel-verbo` / la futura fachada `-llm`), sin
//! Llimphi.
//!
//! ## Recoloreo: `Color::Corriente`
//!
//! Igual que `currentColor` de SVG, [`Color::Corriente`] deja el color *sin
//! resolver* en el spec. El consumidor —un theme de Llimphi, el widget que
//! pinta el diente, o el `--color` del CLI— lo resuelve al pintar vía
//! [`ResolverColor`]. Así un mismo ícono se recolorea por contexto sin
//! re-generar geometría.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use tullpu_core::{ComandoPath, EstiloTrazo, Gradiente, ParamsVector, ReglaRelleno};

/// Lado por defecto de la grilla de diseño, en px-imagen. Coincide con la
/// convención 24×24 de `llimphi-icons` (paths trazados sobre esa caja y
/// escalados al pintar).
pub const GRILLA: f32 = 24.0;

// =============================================================================
//  Color — RGBA concreto, color de marca, o "corriente" (sin resolver)
// =============================================================================

/// Cómo se determina un color del ícono. Permite specs **portables**: la
/// geometría queda fija pero el color puede diferirse al consumidor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Color {
    /// Color RGBA8 literal.
    Rgba([u8; 4]),
    /// Color de **marca** de una app/dominio (p.ej. `"cosmos"`), resuelto por el
    /// consumidor contra su catálogo de marcas. Espeja `llimphi-icons::AppIcon::brand`.
    Marca(String),
    /// Sin resolver — `currentColor`. El theme/widget/CLI inyecta el color al
    /// pintar. Es el modo que mantiene los íconos de UI recolorables.
    Corriente,
}

/// Resuelve un [`Color`] a RGBA8 concreto en el momento de compilar a vectores.
/// La implementación trivial [`ColorFijo`] cubre el caso "todo en `Corriente` es
/// este color"; un consumidor real mapea también `Marca(_)` a su paleta.
pub trait ResolverColor {
    fn resolver(&self, color: &Color) -> [u8; 4];
}

/// Resolutor mínimo: `Rgba` pasa tal cual; `Corriente` y cualquier `Marca`
/// caen al color `corriente` provisto. Útil para CLI/tests sin catálogo de marcas.
#[derive(Debug, Clone, Copy)]
pub struct ColorFijo {
    pub corriente: [u8; 4],
}

impl ColorFijo {
    pub fn nuevo(corriente: [u8; 4]) -> Self {
        Self { corriente }
    }
}

impl ResolverColor for ColorFijo {
    fn resolver(&self, color: &Color) -> [u8; 4] {
        match color {
            Color::Rgba(c) => *c,
            Color::Marca(_) | Color::Corriente => self.corriente,
        }
    }
}

// =============================================================================
//  Pintura — cómo se rellena/traza una primitiva
// =============================================================================

/// Cómo se pinta una [`Forma`]. Mapea directo a los campos de relleno/trazo de
/// `ParamsVector`. El ancho de trazo está en px-grilla.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Pintura {
    /// Relleno sólido, sin contorno.
    Relleno(Color),
    /// Sólo contorno (la mayoría de los íconos de `llimphi-icons` son así).
    Trazo { color: Color, ancho: f32 },
    /// Relleno y contorno.
    RellenoYTrazo { relleno: Color, trazo: Color, ancho: f32 },
    /// Relleno por gradiente (lineal/radial). El gradiente se expresa en
    /// coords de la grilla; las paradas llevan RGBA8 literal.
    Gradiente(Gradiente),
}

// =============================================================================
//  Forma — el vocabulario de primitivas
// =============================================================================

/// Primitiva geométrica del ícono. Cada variante se traduce a una lista de
/// [`ComandoPath`] reutilizando los constructores de `tullpu_core::ParamsVector`,
/// salvo `Path` que pasa comandos crudos (p.ej. los que produce `foreign-svg`).
/// Coordenadas en px sobre la grilla `lienzo` del [`IconSpec`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Forma {
    Rect { x: f32, y: f32, w: f32, h: f32 },
    RectRedondeado { x: f32, y: f32, w: f32, h: f32, r: f32 },
    Elipse { cx: f32, cy: f32, rx: f32, ry: f32 },
    Circulo { cx: f32, cy: f32, r: f32 },
    PoligonoRegular { cx: f32, cy: f32, r: f32, lados: u32 },
    Estrella { cx: f32, cy: f32, r_ext: f32, r_int: f32, puntas: u32 },
    /// Segmento recto (sólo cobra sentido con `Pintura::Trazo`).
    Linea { x1: f32, y1: f32, x2: f32, y2: f32 },
    /// Comandos crudos — la vía de escape para geometría arbitraria (import SVG,
    /// glifos, salida de la IA). El sub-path se cierra/no según sus comandos.
    Path { comandos: Vec<ComandoPath> },
}

impl Forma {
    /// Lista de comandos de la primitiva. Reutiliza los constructores del core
    /// para no duplicar el cálculo de Bézier; el color es irrelevante aquí (se
    /// fija después), así que se pasa opaco y se descartan los campos de paint.
    fn comandos(&self) -> Vec<ComandoPath> {
        const OPACO: [u8; 4] = [0, 0, 0, 255];
        match *self {
            Forma::Rect { x, y, w, h } => ParamsVector::rectangulo(x, y, w, h, OPACO).comandos,
            Forma::RectRedondeado { x, y, w, h, r } => {
                ParamsVector::rect_redondeado(x, y, w, h, r, OPACO).comandos
            }
            Forma::Elipse { cx, cy, rx, ry } => ParamsVector::elipse(cx, cy, rx, ry, OPACO).comandos,
            Forma::Circulo { cx, cy, r } => ParamsVector::elipse(cx, cy, r, r, OPACO).comandos,
            Forma::PoligonoRegular { cx, cy, r, lados } => {
                ParamsVector::poligono_regular(cx, cy, r, lados, OPACO).comandos
            }
            Forma::Estrella { cx, cy, r_ext, r_int, puntas } => {
                ParamsVector::estrella(cx, cy, r_ext, r_int, puntas, OPACO).comandos
            }
            Forma::Linea { x1, y1, x2, y2 } => {
                vec![ComandoPath::MoverA { x: x1, y: y1 }, ComandoPath::LineaA { x: x2, y: y2 }]
            }
            Forma::Path { ref comandos } => comandos.clone(),
        }
    }
}

// =============================================================================
//  Capa e IconSpec
// =============================================================================

/// Una capa del ícono: una [`Forma`] con su [`Pintura`], opcionalmente
/// transformada por una afín `[a,b,c,d,e,f]` (misma convención que
/// `ParamsVector::transformar`: `[scaleX, skewY, skewX, scaleY, transX, transY]`).
/// Las capas se pintan en orden, de atrás hacia adelante.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Capa {
    pub forma: Forma,
    pub pintura: Pintura,
    /// Afín opcional aplicada a la geometría (no al gradiente, que se expresa
    /// directamente en coords del lienzo).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<[f32; 6]>,
    /// Regla de relleno para sub-paths cruzados/anidados.
    #[serde(default = "regla_por_defecto")]
    pub regla: ReglaRelleno,
    /// Estilo extendido de trazo (cap/join/dash); `None` = defaults de tullpu.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estilo_trazo: Option<EstiloTrazo>,
}

fn regla_por_defecto() -> ReglaRelleno {
    ReglaRelleno::NoCero
}

impl Capa {
    /// Capa de relleno sólido sin transform ni estilo.
    pub fn rellena(forma: Forma, color: Color) -> Self {
        Self {
            forma,
            pintura: Pintura::Relleno(color),
            transform: None,
            regla: ReglaRelleno::NoCero,
            estilo_trazo: None,
        }
    }

    /// Capa sólo-trazo (el patrón de la mayoría de íconos de UI).
    pub fn trazada(forma: Forma, color: Color, ancho: f32) -> Self {
        Self {
            forma,
            pintura: Pintura::Trazo { color, ancho },
            transform: None,
            regla: ReglaRelleno::NoCero,
            estilo_trazo: None,
        }
    }

    /// Compila esta capa a un `ParamsVector` resolviendo colores con `resolver`.
    pub fn compilar(&self, resolver: &impl ResolverColor) -> ParamsVector {
        let mut pv = ParamsVector {
            comandos: self.forma.comandos(),
            relleno: None,
            gradiente: None,
            regla: self.regla,
            trazo: None,
            ancho_trazo: 0.0,
            estilo_trazo: self.estilo_trazo.clone(),
        };
        // La geometría se transforma; el gradiente se deja en coords del lienzo.
        if let Some(m) = self.transform {
            pv.transformar(m);
        }
        match &self.pintura {
            Pintura::Relleno(c) => pv.relleno = Some(resolver.resolver(c)),
            Pintura::Trazo { color, ancho } => {
                pv.trazo = Some(resolver.resolver(color));
                pv.ancho_trazo = *ancho;
            }
            Pintura::RellenoYTrazo { relleno, trazo, ancho } => {
                pv.relleno = Some(resolver.resolver(relleno));
                pv.trazo = Some(resolver.resolver(trazo));
                pv.ancho_trazo = *ancho;
            }
            Pintura::Gradiente(g) => pv.gradiente = Some(g.clone()),
        }
        pv
    }
}

/// Descripción declarativa completa de un ícono. Es el artefacto que la parte
/// paramétrica construye y la parte IA propone; serializable a JSON para
/// persistir/transportar/cachear.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IconSpec {
    /// Nombre semántico (clave de caché, nombre de archivo de salida, etc.).
    pub nombre: String,
    /// Lado de la grilla de diseño en px. Por defecto [`GRILLA`] (24).
    #[serde(default = "grilla_por_defecto")]
    pub lienzo: f32,
    /// Capas en orden de pintado (atrás → adelante).
    pub capas: Vec<Capa>,
}

fn grilla_por_defecto() -> f32 {
    GRILLA
}

impl IconSpec {
    /// Crea un spec en la grilla canónica 24×24.
    pub fn nuevo(nombre: impl Into<String>, capas: Vec<Capa>) -> Self {
        Self { nombre: nombre.into(), lienzo: GRILLA, capas }
    }

    /// Compila a la lista de `ParamsVector` nativos de tullpu, en orden de
    /// pintado. Determinista: el mismo spec + el mismo resolver dan el mismo
    /// resultado, bit a bit.
    pub fn compilar(&self, resolver: &impl ResolverColor) -> Vec<ParamsVector> {
        self.capas.iter().map(|c| c.compilar(resolver)).collect()
    }

    /// Factor de escala para rasterizar la grilla a un lado de `px` píxeles.
    /// El CLI lo usa para construir la afín de export.
    pub fn escala_para(&self, px: f32) -> f32 {
        if self.lienzo > 0.0 {
            px / self.lienzo
        } else {
            1.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_plus() -> IconSpec {
        // "Más": dos segmentos trazados en color corriente.
        IconSpec::nuevo(
            "plus",
            vec![
                Capa::trazada(Forma::Linea { x1: 12.0, y1: 5.0, x2: 12.0, y2: 19.0 }, Color::Corriente, 2.0),
                Capa::trazada(Forma::Linea { x1: 5.0, y1: 12.0, x2: 19.0, y2: 12.0 }, Color::Corriente, 2.0),
            ],
        )
    }

    #[test]
    fn compila_determinista() {
        let spec = spec_plus();
        let r = ColorFijo::nuevo([10, 20, 30, 255]);
        let a = spec.compilar(&r);
        let b = spec.compilar(&r);
        assert_eq!(a, b, "la compilación debe ser determinista");
        assert_eq!(a.len(), 2);
        // Sólo-trazo: sin relleno, con color resuelto y ancho.
        assert_eq!(a[0].relleno, None);
        assert_eq!(a[0].trazo, Some([10, 20, 30, 255]));
        assert_eq!(a[0].ancho_trazo, 2.0);
    }

    #[test]
    fn corriente_se_resuelve_al_color_provisto() {
        let cap = Capa::rellena(Forma::Circulo { cx: 12.0, cy: 12.0, r: 8.0 }, Color::Corriente);
        let pv = cap.compilar(&ColorFijo::nuevo([200, 100, 50, 255]));
        assert_eq!(pv.relleno, Some([200, 100, 50, 255]));
        // El círculo es una elipse: 1 MoverA + 4 cúbicas + Cerrar.
        assert_eq!(pv.comandos.len(), 6);
    }

    #[test]
    fn transform_mueve_la_geometria() {
        let cap = Capa::rellena(Forma::Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }, Color::Rgba([1, 2, 3, 4]));
        let mut trasladada = cap.clone();
        trasladada.transform = Some([1.0, 0.0, 0.0, 1.0, 5.0, 7.0]); // +5,+7
        let r = ColorFijo::nuevo([0, 0, 0, 255]);
        let base = cap.compilar(&r);
        let mov = trasladada.compilar(&r);
        match (base.comandos[0], mov.comandos[0]) {
            (ComandoPath::MoverA { x: bx, y: by }, ComandoPath::MoverA { x: mx, y: my }) => {
                assert_eq!((mx - bx, my - by), (5.0, 7.0));
            }
            _ => panic!("primer comando debería ser MoverA"),
        }
        // El relleno literal no lo toca el resolver.
        assert_eq!(mov.relleno, Some([1, 2, 3, 4]));
    }

    #[test]
    fn json_round_trip() {
        let spec = spec_plus();
        let txt = serde_json::to_string(&spec).expect("serializa");
        let back: IconSpec = serde_json::from_str(&txt).expect("deserializa");
        assert_eq!(spec, back);
    }
}
