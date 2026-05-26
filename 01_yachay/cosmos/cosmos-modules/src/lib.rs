//! `cosmos_app-modules` — registry de módulos astrológicos.
//!
//! Cada tipo de astrología (natal, tránsito, progresión, sinastría,
//! Uraniano, …) es un **módulo** que declara:
//!
//! - Qué `Layer`s aporta al `RenderModel`.
//! - Qué `Control`s expone al panel inferior (toggles, sliders, selects).
//! - Hotkeys opcionales.
//! - Si su cómputo es lazy (sólo cuando se activa) o eager.
//!
//! El registry es un `Vec<&dyn Module>` estático: el canvas consulta
//! "para esta `ChartKind`, ¿qué módulos están disponibles?" y el panel
//! pinta sus controles. Activar / desactivar persiste en
//! `ModuleState` (en la store).
//!
//! Esta fase 1 trae el trait + un módulo `NatalModule` de placeholder.
//! En fases posteriores agregamos Transit, Progression, Synastry,
//! Composite, SolarArc, Uranian, FixedStars, Dignities, Lots…

#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

use serde::{Deserialize, Serialize};

use cosmos_engine::Layer;
use cosmos_model::{Chart, ChartKind};

// =====================================================================
// Trait Module
// =====================================================================

/// Una capa de astrología enchufable.
///
/// `Send + Sync` para que el registry sea estático y se pueda consultar
/// desde cualquier thread (el cómputo pesado va a un background executor).
pub trait Module: Send + Sync {
    /// Identidad estable del módulo. Coincide con `ModuleState.module_id`
    /// en la store.
    fn id(&self) -> &'static str;

    /// Etiqueta amigable para el panel.
    fn label(&self) -> &'static str;

    /// Breve descripción para tooltip.
    fn description(&self) -> &'static str;

    /// Para qué tipos de carta tiene sentido este módulo. El panel filtra
    /// con esto al armar la lista de toggles disponibles.
    fn applies_to(&self, kind: ChartKind) -> bool;

    /// Si el módulo está activado por default al crear una carta.
    fn enabled_by_default(&self) -> bool {
        false
    }

    /// Controles que aporta al panel inferior.
    fn controls(&self) -> Vec<Control> {
        Vec::new()
    }

    /// Computa las capas que este módulo aporta al RenderModel de
    /// `chart`. La engine la llama solo si el módulo está activado
    /// para esa carta.
    ///
    /// Devuelve `Vec` (no Option) — un módulo puede no aportar capas
    /// si su config interna lo apaga (ej. "Uranian: mostrar simetría
    /// = false"); en ese caso retorna `Vec::new()`.
    fn compute_layers(&self, chart: &Chart, config: &serde_json::Value) -> Vec<Layer>;
}

// =====================================================================
// Controls expuestos al panel
// =====================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Control {
    Toggle {
        key: String,
        label: String,
        default: bool,
        hotkey: Option<String>,
    },
    Slider {
        key: String,
        label: String,
        min: f64,
        max: f64,
        step: f64,
        default: f64,
    },
    Select {
        key: String,
        label: String,
        options: Vec<SelectOption>,
        default: String,
    },
    /// Texto libre — útil para etiquetas, comentarios.
    TextInput {
        key: String,
        label: String,
        default: String,
    },
    /// Picker dinámico de una carta de la DB. Las opciones las inyecta
    /// el host (Shell) en el panel — el módulo solo declara la
    /// existencia del control. Valor emitido en `ControlChanged` =
    /// `Value::String(chart_id)` cuando se selecciona, `Value::Null`
    /// cuando se vuelve a "automático".
    ChartPicker {
        key: String,
        label: String,
    },
    /// Botón sin estado — el click dispara un `PanelEvent::Action`
    /// con `key`. El panel lo pinta como pill clickeable. Útil para
    /// "Guardar como carta libre" en los módulos overlay con
    /// transformación (RS, progresión, solar arc, GR).
    Action {
        key: String,
        label: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
}

// =====================================================================
// Registry
// =====================================================================

/// Lista estática de módulos disponibles. La app los registra al boot.
pub struct Registry {
    modules: Vec<Box<dyn Module>>,
}

impl Registry {
    /// Registry con todos los módulos built-in. La app llama esto al
    /// boot y luego usa `find()` / `for_kind()` para consultar.
    pub fn with_builtins() -> Self {
        let mut r = Self { modules: Vec::new() };
        r.register(Box::new(natal::NatalModule));
        r.register(Box::new(transit::TransitModule));
        r.register(Box::new(progression::ProgressionModule));
        r.register(Box::new(solar_arc::SolarArcModule));
        r.register(Box::new(synastry::SynastryModule));
        r.register(Box::new(planetary_return::PlanetaryReturnModule));
        r.register(Box::new(midpoints::MidpointsModule));
        r.register(Box::new(composite::CompositeModule));
        r.register(Box::new(uranian::UranianModule));
        r.register(Box::new(lots::LotsModule));
        r.register(Box::new(fixed_stars::FixedStarsModule));
        r.register(Box::new(topocentric::TopocentricModule));
        r.register(Box::new(primary_directions::PrimaryDirectionsModule));
        r
    }

    pub fn register(&mut self, m: Box<dyn Module>) {
        self.modules.push(m);
    }

    pub fn all(&self) -> &[Box<dyn Module>] {
        &self.modules
    }

    pub fn find(&self, id: &str) -> Option<&dyn Module> {
        self.modules
            .iter()
            .find(|m| m.id() == id)
            .map(|m| m.as_ref())
    }

    pub fn for_kind(&self, kind: ChartKind) -> Vec<&dyn Module> {
        self.modules
            .iter()
            .filter(|m| m.applies_to(kind))
            .map(|m| m.as_ref())
            .collect()
    }
}

// =====================================================================
// NatalModule — placeholder fase 1
// =====================================================================

pub mod natal {
    use super::*;
    use cosmos_engine::compute_mock;

    pub struct NatalModule;

    impl Module for NatalModule {
        fn id(&self) -> &'static str {
            "natal"
        }
        fn label(&self) -> &'static str {
            "Carta natal"
        }
        fn description(&self) -> &'static str {
            "Posiciones natales, casas y aspectos."
        }
        fn applies_to(&self, kind: ChartKind) -> bool {
            matches!(kind, ChartKind::Natal)
        }
        fn enabled_by_default(&self) -> bool {
            true
        }

        fn controls(&self) -> Vec<Control> {
            vec![
                Control::Toggle {
                    key: "show_sign_dial".into(),
                    label: "Dial zodiacal".into(),
                    default: true,
                    hotkey: Some("D".into()),
                },
                Control::Toggle {
                    key: "show_houses".into(),
                    label: "Casas".into(),
                    default: true,
                    hotkey: Some("H".into()),
                },
                Control::Toggle {
                    key: "show_aspects".into(),
                    label: "Aspectos".into(),
                    default: true,
                    hotkey: Some("X".into()),
                },
                Control::Toggle {
                    key: "show_bodies".into(),
                    label: "Cuerpos".into(),
                    default: true,
                    hotkey: Some("P".into()),
                },
                Control::Toggle {
                    key: "show_coords".into(),
                    label: "Coordenadas (grado°min')".into(),
                    default: true,
                    hotkey: Some("C".into()),
                },
                // Filtros de aspectos: cambian QUÉ se computa, no QUÉ
                // se pinta del render. Recompose al togglear.
                Control::Toggle {
                    key: "aspect_majors".into(),
                    label: "Mayores (☌ ☍ △ □ ⚹)".into(),
                    default: true,
                    hotkey: None,
                },
                Control::Toggle {
                    key: "aspect_minors".into(),
                    label: "Menores (quincunx, semi-…)".into(),
                    default: false,
                    hotkey: None,
                },
                Control::Slider {
                    key: "orb_multiplier".into(),
                    label: "Multiplicador de orbe".into(),
                    min: 0.25,
                    max: 2.5,
                    step: 0.25,
                    default: 1.0,
                },
                Control::Toggle {
                    key: "show_dignities".into(),
                    label: "Dignidades esenciales (+ · − *)".into(),
                    default: false,
                    hotkey: None,
                },
                Control::Slider {
                    key: "harmonic".into(),
                    label: "Armónico".into(),
                    min: 1.0,
                    // 1-32: el rango del espectro de fuerza armónica.
                    max: 32.0,
                    step: 1.0,
                    default: 1.0,
                },
            ]
        }

        fn compute_layers(&self, chart: &Chart, _cfg: &serde_json::Value) -> Vec<Layer> {
            // Fase 1: delega al mock de la engine para que la UI tenga
            // algo que pintar. Fase 3 reemplaza con `engine::compute`
            // contra `eternal-astrology`.
            compute_mock(chart).layers
        }
    }
}

// =====================================================================
// TransitModule — overlay del cielo del momento sobre la carta natal
// =====================================================================

pub mod transit {
    use super::*;

    /// Anillo externo con las posiciones planetarias del **instante
    /// actual** (reloj de pared) sobre el sujeto natal, más las
    /// cross-aspects natal × transit. La engine despacha al pipeline
    /// `PipelineRequest::Transit` cuando este módulo está activo en el
    /// `module_configs` del shell.
    pub struct TransitModule;

    impl Module for TransitModule {
        fn id(&self) -> &'static str {
            "transit"
        }
        fn label(&self) -> &'static str {
            "Tránsitos"
        }
        fn description(&self) -> &'static str {
            "Cielo del momento sobre la natal + cross aspects."
        }
        fn applies_to(&self, kind: ChartKind) -> bool {
            // Por ahora solo overlay sobre cartas natales — más adelante
            // podríamos overlayar tránsitos sobre Progresiones, etc.
            matches!(kind, ChartKind::Natal)
        }
        fn enabled_by_default(&self) -> bool {
            false
        }
        fn controls(&self) -> Vec<Control> {
            vec![
                Control::Toggle {
                    key: "enabled".into(),
                    label: "Activar".into(),
                    default: false,
                    hotkey: Some("T".into()),
                },
                Control::Action {
                    key: "save_as_free".into(),
                    label: rimay_localize::t("cosmos-btn-save-transit"),
                },
            ]
        }
        fn compute_layers(&self, _chart: &Chart, _cfg: &serde_json::Value) -> Vec<Layer> {
            // Las capas de tránsito se construyen en la engine vía
            // `PipelineRequest::Transit` porque necesitan acceso a la
            // NatalChart cruda + EphemerisSession. Este método queda
            // como no-op — el módulo es puramente declarativo.
            Vec::new()
        }
    }
}

// =====================================================================
// ProgressionModule — progresión secundaria (día por año)
// =====================================================================

pub mod progression {
    use super::*;

    /// Anillo interno con la carta progresada (método secundario,
    /// "un día de efemérides = un año de vida") + cross aspects natal ×
    /// progresada. La engine lo despacha vía
    /// `PipelineRequest::SecondaryProgression { target_age_years }`.
    pub struct ProgressionModule;

    impl Module for ProgressionModule {
        fn id(&self) -> &'static str {
            "progression"
        }
        fn label(&self) -> &'static str {
            "Progresión secundaria"
        }
        fn description(&self) -> &'static str {
            "Día-por-año: avanza la carta a la edad actual."
        }
        fn applies_to(&self, kind: ChartKind) -> bool {
            matches!(kind, ChartKind::Natal)
        }
        fn enabled_by_default(&self) -> bool {
            false
        }
        fn controls(&self) -> Vec<Control> {
            vec![
                Control::Toggle {
                    key: "enabled".into(),
                    label: "Activar".into(),
                    default: false,
                    hotkey: None,
                },
                // El default (30.0) es un placeholder — el shell empuja
                // la edad actual del sujeto al cargar una carta vía
                // panel.set_slider("progression", "target_age_years",
                // current_age).
                Control::Slider {
                    key: "target_age_years".into(),
                    label: "Edad objetivo (años)".into(),
                    min: 0.0,
                    max: 120.0,
                    step: 0.25,
                    default: 30.0,
                },
                Control::Action {
                    key: "save_as_free".into(),
                    label: rimay_localize::t("cosmos-btn-save-progressed"),
                },
            ]
        }
        fn compute_layers(&self, _chart: &Chart, _cfg: &serde_json::Value) -> Vec<Layer> {
            Vec::new()
        }
    }
}

// =====================================================================
// SynastryModule — bi-wheel con otra carta hermana del contacto actual
// =====================================================================

pub mod synastry {
    use super::*;

    /// Pone la carta del partner en el anillo externo (compartido con
    /// Transit — mutuamente excluyentes) y dibuja las cross aspects
    /// natal × partner. El shell elige el partner: la primera carta
    /// hermana del mismo contacto. Si no hay hermana, el request se
    /// salta silenciosamente.
    pub struct SynastryModule;

    impl Module for SynastryModule {
        fn id(&self) -> &'static str {
            "synastry"
        }
        fn label(&self) -> &'static str {
            "Sinastría"
        }
        fn description(&self) -> &'static str {
            "Bi-wheel con la primera carta hermana del contacto."
        }
        fn applies_to(&self, kind: ChartKind) -> bool {
            matches!(kind, ChartKind::Natal)
        }
        fn enabled_by_default(&self) -> bool {
            false
        }
        fn controls(&self) -> Vec<Control> {
            vec![
                Control::Toggle {
                    key: "enabled".into(),
                    label: "Activar".into(),
                    default: false,
                    hotkey: None,
                },
                Control::ChartPicker {
                    key: "partner_chart_id".into(),
                    label: "Partner".into(),
                },
            ]
        }
        fn compute_layers(&self, _chart: &Chart, _cfg: &serde_json::Value) -> Vec<Layer> {
            Vec::new()
        }
    }
}

// =====================================================================
// PlanetaryReturnModule — retornos de cualquier cuerpo a su pos natal
// =====================================================================

pub mod planetary_return {
    use super::*;

    /// Computa la carta natal completa al instante del próximo retorno
    /// del cuerpo elegido. Sun = anual (cumpleaños), Moon = mensual,
    /// Júpiter/Saturno = generacionales. Comparte el outer ring con
    /// Transit y Synastry — mutuamente excluyentes a nivel de Shell.
    pub struct PlanetaryReturnModule;

    impl Module for PlanetaryReturnModule {
        fn id(&self) -> &'static str {
            "planetary_return"
        }
        fn label(&self) -> &'static str {
            "Retornos planetarios"
        }
        fn description(&self) -> &'static str {
            "Carta del próximo retorno (Sol, Luna, Júpiter, Saturno…)."
        }
        fn applies_to(&self, kind: ChartKind) -> bool {
            matches!(kind, ChartKind::Natal)
        }
        fn enabled_by_default(&self) -> bool {
            false
        }
        fn controls(&self) -> Vec<Control> {
            vec![
                Control::Toggle {
                    key: "enabled".into(),
                    label: "Activar".into(),
                    default: false,
                    hotkey: None,
                },
                Control::Select {
                    key: "body".into(),
                    label: "Cuerpo".into(),
                    default: "sun".into(),
                    options: vec![
                        SelectOption { value: "sun".into(), label: "Sol".into() },
                        SelectOption { value: "moon".into(), label: "Luna".into() },
                        SelectOption { value: "mercury".into(), label: "Mercurio".into() },
                        SelectOption { value: "venus".into(), label: "Venus".into() },
                        SelectOption { value: "mars".into(), label: "Marte".into() },
                        SelectOption { value: "jupiter".into(), label: "Júpiter".into() },
                        SelectOption { value: "saturn".into(), label: "Saturno".into() },
                        SelectOption { value: "uranus".into(), label: "Urano".into() },
                        SelectOption { value: "neptune".into(), label: "Neptuno".into() },
                        SelectOption { value: "pluto".into(), label: "Plutón".into() },
                    ],
                },
                Control::Slider {
                    key: "target_age_years".into(),
                    label: "Edad del retorno".into(),
                    min: 0.0,
                    max: 120.0,
                    step: 1.0,
                    default: 30.0,
                },
                // Offset adicional para Moon return (saltar ~28d entre
                // retornos lunares) o ajuste fino del Solar return.
                Control::Slider {
                    key: "shift_days".into(),
                    label: "Shift días (lunar nav)".into(),
                    min: -180.0,
                    max: 180.0,
                    step: 1.0,
                    default: 0.0,
                },
                // Botón: captura la carta del retorno actual (cuerpo +
                // edad) como FreeChart con label `{contacto} rs-{N}`
                // (o `lunar-{N}` etc. según el cuerpo). El usuario
                // luego decide si guardarla en un contacto.
                Control::Action {
                    key: "save_as_free".into(),
                    label: rimay_localize::t("cosmos-btn-save-return"),
                },
            ]
        }
        fn compute_layers(&self, _chart: &Chart, _cfg: &serde_json::Value) -> Vec<Layer> {
            Vec::new()
        }
    }
}

// =====================================================================
// CompositeModule — carta compuesta (midpoint Davison) con un partner
// =====================================================================

pub mod composite {
    use super::*;

    /// Carta compuesta entre la natal y otra carta — cada placement es
    /// el midpoint angular del par. Mismo ChartPicker que sinastría
    /// para elegir el partner.
    pub struct CompositeModule;

    impl Module for CompositeModule {
        fn id(&self) -> &'static str {
            "composite"
        }
        fn label(&self) -> &'static str {
            "Composite"
        }
        fn description(&self) -> &'static str {
            "Carta compuesta con otro sujeto (midpoint Davison)."
        }
        fn applies_to(&self, kind: ChartKind) -> bool {
            matches!(kind, ChartKind::Natal)
        }
        fn enabled_by_default(&self) -> bool {
            false
        }
        fn controls(&self) -> Vec<Control> {
            vec![
                Control::Toggle {
                    key: "enabled".into(),
                    label: "Activar".into(),
                    default: false,
                    hotkey: None,
                },
                Control::ChartPicker {
                    key: "partner_chart_id".into(),
                    label: "Partner".into(),
                },
            ]
        }
        fn compute_layers(&self, _chart: &Chart, _cfg: &serde_json::Value) -> Vec<Layer> {
            Vec::new()
        }
    }
}

// =====================================================================
// SolarArcModule — Solar Arc dirigido (true progressed Sun)
// =====================================================================

pub mod solar_arc {
    use super::*;

    /// Cada planeta y cusp natal se desplaza por el mismo arco
    /// (≈ 1° por año de vida, calculado como el delta del Sol
    /// progresado secundario). Anillo interno bien adentro + cross
    /// aspects natal × dirigida.
    pub struct SolarArcModule;

    impl Module for SolarArcModule {
        fn id(&self) -> &'static str {
            "solar_arc"
        }
        fn label(&self) -> &'static str {
            "Solar Arc"
        }
        fn description(&self) -> &'static str {
            "Dirección por arco solar — uniforme, ≈1°/año."
        }
        fn applies_to(&self, kind: ChartKind) -> bool {
            matches!(kind, ChartKind::Natal)
        }
        fn enabled_by_default(&self) -> bool {
            false
        }
        fn controls(&self) -> Vec<Control> {
            vec![
                Control::Toggle {
                    key: "enabled".into(),
                    label: "Activar".into(),
                    default: false,
                    hotkey: None,
                },
                Control::Slider {
                    key: "target_age_years".into(),
                    label: "Edad objetivo (años)".into(),
                    min: 0.0,
                    max: 120.0,
                    step: 0.25,
                    default: 30.0,
                },
            ]
        }
        fn compute_layers(&self, _chart: &Chart, _cfg: &serde_json::Value) -> Vec<Layer> {
            Vec::new()
        }
    }
}

// =====================================================================
// MidpointsModule — puntos medios entre cuerpos natales (Sol/Luna)
// =====================================================================

pub mod midpoints {
    use super::*;

    /// Computa midpoints entre los cuerpos natales (filtrado a los que
    /// involucran Sol o Luna, ~10 puntos) y los renderea como pequeños
    /// puntos en un anillo interior. Hovering muestra los dos cuerpos
    /// que originan el midpoint.
    pub struct MidpointsModule;

    impl Module for MidpointsModule {
        fn id(&self) -> &'static str {
            "midpoints"
        }
        fn label(&self) -> &'static str {
            "Midpoints"
        }
        fn description(&self) -> &'static str {
            "Puntos medios que involucran al Sol o a la Luna."
        }
        fn applies_to(&self, kind: ChartKind) -> bool {
            matches!(kind, ChartKind::Natal)
        }
        fn enabled_by_default(&self) -> bool {
            false
        }
        fn controls(&self) -> Vec<Control> {
            vec![Control::Toggle {
                key: "enabled".into(),
                label: "Activar".into(),
                default: false,
                hotkey: None,
            }]
        }
        fn compute_layers(&self, _chart: &Chart, _cfg: &serde_json::Value) -> Vec<Layer> {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_finds_builtins() {
        let r = Registry::with_builtins();
        assert!(r.find("natal").is_some());
        assert!(r.find("transit").is_some());
        assert!(r.find("progression").is_some());
        assert!(r.find("solar_arc").is_some());
        assert!(r.find("synastry").is_some());
        assert!(r.find("planetary_return").is_some());
        assert!(r.find("midpoints").is_some());
        assert!(r.find("composite").is_some());
        assert!(r.find("uranian").is_some());
        assert!(r.find("lots").is_some());
        assert!(r.find("fixed_stars").is_some());
        // Natal kind tiene 11 módulos aplicables.
        assert_eq!(r.for_kind(ChartKind::Natal).len(), 11);
        assert!(r.for_kind(ChartKind::Synastry).is_empty());
    }
}

// =====================================================================
// LotsModule — Lots helenísticos (Fortune, Spirit, Eros, …)
// =====================================================================

pub mod lots {
    use super::*;

    /// Calcula los 7 Lots arábigos clásicos via eternal-astrology y
    /// los renderea como pequeños labels en un ring justo debajo de
    /// los cuerpos natales. Hover muestra el nombre completo.
    pub struct LotsModule;

    impl Module for LotsModule {
        fn id(&self) -> &'static str {
            "lots"
        }
        fn label(&self) -> &'static str {
            "Lots (helenísticos)"
        }
        fn description(&self) -> &'static str {
            "Fortune, Spirit, Eros, Necessity, Courage, Victory, Nemesis."
        }
        fn applies_to(&self, kind: ChartKind) -> bool {
            matches!(kind, ChartKind::Natal)
        }
        fn enabled_by_default(&self) -> bool {
            false
        }
        fn controls(&self) -> Vec<Control> {
            vec![Control::Toggle {
                key: "enabled".into(),
                label: "Activar".into(),
                default: false,
                hotkey: None,
            }]
        }
        fn compute_layers(&self, _chart: &Chart, _cfg: &serde_json::Value) -> Vec<Layer> {
            Vec::new()
        }
    }
}

// =====================================================================
// FixedStarsModule — 9 estrellas astrológicamente notables
// =====================================================================

pub mod fixed_stars {
    use super::*;

    /// 9 estrellas fijas (Aldebaran, Regulus, Antares, Fomalhaut,
    /// Spica, Sirius, Algol, Vega, Pollux) con posición tropical
    /// aproximada (J2000 + precesión simple). Marcadores chicos en el
    /// margen exterior del sign dial.
    pub struct FixedStarsModule;

    impl Module for FixedStarsModule {
        fn id(&self) -> &'static str {
            "fixed_stars"
        }
        fn label(&self) -> &'static str {
            "Estrellas fijas"
        }
        fn description(&self) -> &'static str {
            "9 estrellas notables — conjunciones con planetas natales."
        }
        fn applies_to(&self, kind: ChartKind) -> bool {
            matches!(kind, ChartKind::Natal)
        }
        fn enabled_by_default(&self) -> bool {
            false
        }
        fn controls(&self) -> Vec<Control> {
            vec![Control::Toggle {
                key: "enabled".into(),
                label: "Activar".into(),
                default: false,
                hotkey: None,
            }]
        }
        fn compute_layers(&self, _chart: &Chart, _cfg: &serde_json::Value) -> Vec<Layer> {
            Vec::new()
        }
    }
}

// =====================================================================
// UranianModule — ejes del dial uraniano de 90° (versión textual)
// =====================================================================

pub mod uranian {
    use super::*;

    /// Detecta "ejes" del dial uraniano: grupos de cuerpos natales cuya
    /// longitud módulo 90 cae dentro de una tolerancia. Los grupos
    /// resultantes se listan en el footer del canvas. La visualización
    /// geométrica del dial completo de 90° queda para una fase futura.
    pub struct UranianModule;

    impl Module for UranianModule {
        fn id(&self) -> &'static str {
            "uranian"
        }
        fn label(&self) -> &'static str {
            "Uraniano (90°)"
        }
        fn description(&self) -> &'static str {
            "Ejes del dial uraniano — cuerpos en la misma posición mod 90."
        }
        fn applies_to(&self, kind: ChartKind) -> bool {
            matches!(kind, ChartKind::Natal)
        }
        fn enabled_by_default(&self) -> bool {
            false
        }
        fn controls(&self) -> Vec<Control> {
            vec![Control::Toggle {
                key: "enabled".into(),
                label: "Activar".into(),
                default: false,
                hotkey: None,
            }]
        }
        fn compute_layers(&self, _chart: &Chart, _cfg: &serde_json::Value) -> Vec<Layer> {
            Vec::new()
        }
    }
}

// =====================================================================
// TopocentricModule — capa "ascensional" (paralaje + Polich-Page)
// =====================================================================

pub mod topocentric {
    use super::*;

    /// Capa topocéntrica que convive con la natal geocéntrica: cada
    /// planeta se re-proyecta a longitud eclíptica topocéntrica (con
    /// paralaje horizontal por cuerpo) y las casas se calculan con el
    /// sistema Polich-Page. El shift es visible en la Luna (~1°),
    /// modesto en interiores cerca de oposición, e imperceptible en
    /// exteriores. La engine despacha al pipeline
    /// `PipelineRequest::Topocentric` cuando este módulo está activo.
    pub struct TopocentricModule;

    impl Module for TopocentricModule {
        fn id(&self) -> &'static str {
            "topocentric"
        }
        fn label(&self) -> &'static str {
            "Topocéntrico (ascensional)"
        }
        fn description(&self) -> &'static str {
            "Paralaje horizontal por cuerpo + casas Polich-Page."
        }
        fn applies_to(&self, kind: ChartKind) -> bool {
            matches!(kind, ChartKind::Natal)
        }
        fn enabled_by_default(&self) -> bool {
            true
        }
        fn controls(&self) -> Vec<Control> {
            vec![Control::Toggle {
                key: "enabled".into(),
                label: "Activar".into(),
                default: true,
                hotkey: None,
            }]
        }
        fn compute_layers(&self, _chart: &Chart, _cfg: &serde_json::Value) -> Vec<Layer> {
            Vec::new()
        }
    }
}

// =====================================================================
// PrimaryDirectionsModule — GR dual-ring (Direct + Converse)
// =====================================================================

pub mod primary_directions {
    use super::*;

    /// Direcciones Primarias del Sistema GR (García Rosas): cada
    /// cuerpo natal se proyecta en dos rings — directa (rotación
    /// diurna forward) y conversa (rotación inversa). El usuario
    /// scrubea `target_age_years` para ver el movimiento en vivo.
    /// Útil para rectificación: un evento real debe coincidir con
    /// arcos directos y conversos consistentes si la hora natal es
    /// correcta.
    pub struct PrimaryDirectionsModule;

    impl Module for PrimaryDirectionsModule {
        fn id(&self) -> &'static str {
            "primary_directions"
        }
        fn label(&self) -> &'static str {
            "Direcciones primarias (GR)"
        }
        fn description(&self) -> &'static str {
            "Dual-ring directas + conversas para rectificación en vivo."
        }
        fn applies_to(&self, kind: ChartKind) -> bool {
            matches!(kind, ChartKind::Natal)
        }
        fn enabled_by_default(&self) -> bool {
            false
        }
        fn controls(&self) -> Vec<Control> {
            vec![
                Control::Toggle {
                    key: "enabled".into(),
                    label: "Activar".into(),
                    default: false,
                    hotkey: None,
                },
                Control::Slider {
                    key: "target_age_years".into(),
                    label: "Edad (años)".into(),
                    min: 0.0,
                    max: 120.0,
                    step: 0.05,
                    default: 30.0,
                },
                Control::Select {
                    key: "key".into(),
                    label: "Clave (arco/año)".into(),
                    default: "naibod".into(),
                    options: vec![
                        SelectOption {
                            value: "naibod".into(),
                            label: "Naibod (0°59'08\"/año)".into(),
                        },
                        SelectOption {
                            value: "ptolemy".into(),
                            label: "Ptolomeo (1°/año)".into(),
                        },
                    ],
                },
                // --- Rectificador automático ---
                // Tres edades de eventos conocidos de la vida del
                // sujeto; `0` = ranura sin usar. El barrido GR busca la
                // hora de nacimiento que mejor las explica.
                Control::Slider {
                    key: "evento_1".into(),
                    label: "Evento 1 · edad".into(),
                    min: 0.0,
                    max: 90.0,
                    step: 1.0,
                    default: 0.0,
                },
                Control::Slider {
                    key: "evento_2".into(),
                    label: "Evento 2 · edad".into(),
                    min: 0.0,
                    max: 90.0,
                    step: 1.0,
                    default: 0.0,
                },
                Control::Slider {
                    key: "evento_3".into(),
                    label: "Evento 3 · edad".into(),
                    min: 0.0,
                    max: 90.0,
                    step: 1.0,
                    default: 0.0,
                },
                Control::Action {
                    key: "rectificar".into(),
                    label: "Rectificar hora".into(),
                },
                Control::TextInput {
                    key: "resultado".into(),
                    label: "Resultado".into(),
                    default: "—".into(),
                },
            ]
        }
        fn compute_layers(&self, _chart: &Chart, _cfg: &serde_json::Value) -> Vec<Layer> {
            Vec::new()
        }
    }
}
