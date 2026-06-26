//! `pluma-estilo` — el estilo *persistible* de un lienzo.
//!
//! Hasta ahora el aspecto de un cuerpo (lienzo) vivía sólo en el render: ni
//! `NarrativeAtom` ni `Cuerpo` llevaban color, fuente o tamaño, y el store no
//! guardaba nada de eso. Este crate define el modelo de estilo que sí se
//! guarda en disco, en tres granularidades que se solapan por precedencia:
//!
//! 1. **lienzo entero** — [`EstiloLienzo::base`], se aplica a todo el cuerpo.
//! 2. **por zona** — [`EstiloLienzo::por_zona`], override sobre una zona
//!    (el agrupamiento de átomos que ya maneja `CuerpoIde`); índice de zona →
//!    estilo.
//! 3. **por span** — [`EstiloLienzo::por_span`], override sobre un rango de
//!    caracteres dentro de un átomo concreto (lo que pide "estilizar la
//!    selección de texto"). Clave = id del átomo, valor = lista de spans.
//!
//! La precedencia es `base < zona < span`: cada nivel **mergea** sobre el
//! anterior, y como cada [`EstiloTexto`] es parcial (todos sus campos son
//! `Option`), tocar una sola propiedad no pisa las demás.
//!
//! El crate es deliberadamente delgado: sólo `serde` + `uuid`, sin ninguna
//! dependencia de UI. La conversión a los `TextSpan` que consume el editor
//! Llimphi vive en `pluma-editor-llimphi`, que es quien conoce el layout
//! átomo↔línea.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Color RGBA empaquetado, sin depender de ningún crate gráfico. El canal
/// alfa permite resaltados de fondo translúcidos. La conversión a
/// `peniko::Color` la hace el frontend.
pub type Rgba = [u8; 4];

/// Un estilo de texto **parcial**: cada propiedad es opcional, así un
/// override sólo toca lo que define y deja pasar el resto del nivel inferior.
/// Es el espejo serde-friendly de `llimphi_text::TextSpanStyle` más un color
/// de fondo propio (resaltado), que `TextSpan` no lleva.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct EstiloTexto {
    /// Color del glifo (RGBA). `None` = heredar.
    pub color_fg: Option<Rgba>,
    /// Color de fondo / resaltado (RGBA). `None` = sin resaltado.
    pub color_bg: Option<Rgba>,
    /// Familia tipográfica (nombre tal como lo conoce el motor de texto).
    pub font_family: Option<String>,
    /// Tamaño en px.
    pub size_px: Option<f32>,
    /// Peso (400 normal, 700 negrita).
    pub weight: Option<f32>,
    /// Itálica.
    pub italic: Option<bool>,
    /// Subrayado.
    pub underline: Option<bool>,
    /// Tachado.
    pub strikethrough: Option<bool>,
}

impl EstiloTexto {
    /// Estilo vacío — todo hereda. Igual que `Default`, pero `const`.
    pub const VACIO: EstiloTexto = EstiloTexto {
        color_fg: None,
        color_bg: None,
        font_family: None,
        size_px: None,
        weight: None,
        italic: None,
        underline: None,
        strikethrough: None,
    };

    /// `true` si no define ninguna propiedad (heredaría todo).
    pub fn es_vacio(&self) -> bool {
        self.color_fg.is_none()
            && self.color_bg.is_none()
            && self.font_family.is_none()
            && self.size_px.is_none()
            && self.weight.is_none()
            && self.italic.is_none()
            && self.underline.is_none()
            && self.strikethrough.is_none()
    }

    /// Devuelve `self` con las propiedades de `override_` aplicadas encima:
    /// donde `override_` define algo, gana; donde no, se conserva lo de
    /// `self`. No muta — devuelve el merge.
    pub fn merge(&self, override_: &EstiloTexto) -> EstiloTexto {
        EstiloTexto {
            color_fg: override_.color_fg.or(self.color_fg),
            color_bg: override_.color_bg.or(self.color_bg),
            font_family: override_
                .font_family
                .clone()
                .or_else(|| self.font_family.clone()),
            size_px: override_.size_px.or(self.size_px),
            weight: override_.weight.or(self.weight),
            italic: override_.italic.or(self.italic),
            underline: override_.underline.or(self.underline),
            strikethrough: override_.strikethrough.or(self.strikethrough),
        }
    }

    /// Aplica `override_` **in situ** sobre `self` (misma semántica que
    /// [`merge`](Self::merge) pero mutando). Útil para acumular cambios de
    /// un panel propiedad-a-propiedad.
    pub fn aplicar(&mut self, override_: &EstiloTexto) {
        *self = self.merge(override_);
    }
}

/// Un override sobre un rango de **caracteres** `[ini, fin)` dentro del
/// contenido de un átomo. Los offsets son índices de `char` (no de byte) —
/// estables frente a multibyte y fáciles de calcular desde una selección;
/// el frontend traduce a bytes al construir el `TextSpan`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpanEstilo {
    pub ini: usize,
    pub fin: usize,
    pub estilo: EstiloTexto,
}

impl SpanEstilo {
    pub fn nuevo(ini: usize, fin: usize, estilo: EstiloTexto) -> Self {
        Self { ini, fin, estilo }
    }

    /// `true` si el rango está vacío o invertido (no aporta nada).
    pub fn es_degenerado(&self) -> bool {
        self.fin <= self.ini
    }
}

/// El estilo completo de un lienzo: base + overrides por zona + overrides por
/// span. Persistido por `pluma-store` con clave = id del cuerpo.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct EstiloLienzo {
    /// Estilo de todo el lienzo.
    pub base: EstiloTexto,
    /// Override por índice de zona (el orden de zonas lo define `CuerpoIde`).
    pub por_zona: BTreeMap<usize, EstiloTexto>,
    /// Override por átomo: spans de caracteres dentro de su contenido.
    pub por_span: BTreeMap<Uuid, Vec<SpanEstilo>>,
}

impl EstiloLienzo {
    /// Lienzo sin estilo: todo hereda del default del editor.
    pub fn nuevo() -> Self {
        Self::default()
    }

    /// `true` si no hay nada configurado (render idéntico al default).
    pub fn es_vacio(&self) -> bool {
        self.base.es_vacio() && self.por_zona.is_empty() && self.por_span.is_empty()
    }

    /// Estilo efectivo de una zona = `base` mergeada con el override de la
    /// zona, si lo hay.
    pub fn estilo_de_zona(&self, zona: usize) -> EstiloTexto {
        match self.por_zona.get(&zona) {
            Some(z) => self.base.merge(z),
            None => self.base.clone(),
        }
    }

    /// Spans de un átomo (vacío si no tiene).
    pub fn spans_de(&self, atom: Uuid) -> &[SpanEstilo] {
        self.por_span.get(&atom).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Mergea `delta` sobre el estilo base del lienzo entero.
    pub fn set_base(&mut self, delta: &EstiloTexto) {
        self.base.aplicar(delta);
    }

    /// Mergea `delta` sobre el override de la zona `zona` (creándolo si no
    /// existía).
    pub fn set_zona(&mut self, zona: usize, delta: &EstiloTexto) {
        self.por_zona.entry(zona).or_default().aplicar(delta);
    }

    /// Agrega un span de override `[ini, fin)` con `delta` al átomo `atom`.
    /// Los spans se acumulan en orden de inserción: al resolver, los
    /// posteriores mergean sobre los anteriores en el solapamiento. Rangos
    /// degenerados se ignoran.
    pub fn set_span(&mut self, atom: Uuid, ini: usize, fin: usize, delta: EstiloTexto) {
        if fin <= ini || delta.es_vacio() {
            return;
        }
        self.por_span
            .entry(atom)
            .or_default()
            .push(SpanEstilo::nuevo(ini, fin, delta));
    }

    /// Quita todos los spans de un átomo (limpiar selección estilizada).
    pub fn limpiar_spans(&mut self, atom: Uuid) {
        self.por_span.remove(&atom);
    }

    /// El mayor `size_px` usado en cualquier nivel (base, zonas, spans), si
    /// alguno define tamaño. Sirve para que el editor agrande su alto de línea
    /// y las fuentes grandes no se solapen con la línea siguiente.
    pub fn max_size_px(&self) -> Option<f32> {
        let mut m: Option<f32> = self.base.size_px;
        let upd = |m: &mut Option<f32>, s: Option<f32>| {
            if let Some(s) = s {
                *m = Some(m.map_or(s, |x| x.max(s)));
            }
        };
        for e in self.por_zona.values() {
            upd(&mut m, e.size_px);
        }
        for v in self.por_span.values() {
            for sp in v {
                upd(&mut m, sp.estilo.size_px);
            }
        }
        m
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn rojo() -> Rgba {
        [255, 0, 0, 255]
    }
    fn azul() -> Rgba {
        [0, 0, 255, 255]
    }

    #[test]
    fn merge_override_gana_y_conserva_lo_no_definido() {
        let base = EstiloTexto {
            color_fg: Some(rojo()),
            size_px: Some(13.0),
            weight: Some(400.0),
            ..Default::default()
        };
        // Override sólo cambia el color: tamaño y peso se conservan.
        let delta = EstiloTexto {
            color_fg: Some(azul()),
            ..Default::default()
        };
        let r = base.merge(&delta);
        assert_eq!(r.color_fg, Some(azul()));
        assert_eq!(r.size_px, Some(13.0));
        assert_eq!(r.weight, Some(400.0));
    }

    #[test]
    fn merge_no_pisa_con_none() {
        let base = EstiloTexto {
            italic: Some(true),
            ..Default::default()
        };
        let r = base.merge(&EstiloTexto::VACIO);
        assert_eq!(r.italic, Some(true));
    }

    #[test]
    fn es_vacio_detecta_ambos_estados() {
        assert!(EstiloTexto::VACIO.es_vacio());
        assert!(!EstiloTexto {
            underline: Some(true),
            ..Default::default()
        }
        .es_vacio());
    }

    #[test]
    fn estilo_de_zona_mergea_sobre_base() {
        let mut e = EstiloLienzo::nuevo();
        e.set_base(&EstiloTexto {
            color_fg: Some(rojo()),
            size_px: Some(13.0),
            ..Default::default()
        });
        e.set_zona(
            2,
            &EstiloTexto {
                size_px: Some(20.0),
                ..Default::default()
            },
        );
        // Zona 0 sin override → sólo base.
        let z0 = e.estilo_de_zona(0);
        assert_eq!(z0.size_px, Some(13.0));
        assert_eq!(z0.color_fg, Some(rojo()));
        // Zona 2 → tamaño override, color heredado de base.
        let z2 = e.estilo_de_zona(2);
        assert_eq!(z2.size_px, Some(20.0));
        assert_eq!(z2.color_fg, Some(rojo()));
    }

    #[test]
    fn set_span_acumula_e_ignora_degenerados() {
        let mut e = EstiloLienzo::nuevo();
        let atom = Uuid::new_v4();
        e.set_span(
            atom,
            0,
            5,
            EstiloTexto {
                weight: Some(700.0),
                ..Default::default()
            },
        );
        // Rango degenerado: no se agrega.
        e.set_span(atom, 5, 5, EstiloTexto { italic: Some(true), ..Default::default() });
        // Delta vacío: no se agrega.
        e.set_span(atom, 0, 3, EstiloTexto::VACIO);
        assert_eq!(e.spans_de(atom).len(), 1);
        assert_eq!(e.spans_de(atom)[0].estilo.weight, Some(700.0));
    }

    #[test]
    fn limpiar_spans_borra_el_atomo() {
        let mut e = EstiloLienzo::nuevo();
        let atom = Uuid::new_v4();
        e.set_span(atom, 0, 2, EstiloTexto { italic: Some(true), ..Default::default() });
        assert!(!e.spans_de(atom).is_empty());
        e.limpiar_spans(atom);
        assert!(e.spans_de(atom).is_empty());
    }

    #[test]
    fn max_size_px_toma_el_mayor_de_todos_los_niveles() {
        let mut e = EstiloLienzo::nuevo();
        assert_eq!(e.max_size_px(), None);
        e.set_base(&EstiloTexto { size_px: Some(13.0), ..Default::default() });
        e.set_zona(0, &EstiloTexto { size_px: Some(20.0), ..Default::default() });
        let atom = Uuid::new_v4();
        e.set_span(atom, 0, 3, EstiloTexto { size_px: Some(48.0), ..Default::default() });
        assert_eq!(e.max_size_px(), Some(48.0));
    }

    #[test]
    fn es_vacio_lienzo() {
        let mut e = EstiloLienzo::nuevo();
        assert!(e.es_vacio());
        e.set_base(&EstiloTexto { italic: Some(true), ..Default::default() });
        assert!(!e.es_vacio());
    }
}
