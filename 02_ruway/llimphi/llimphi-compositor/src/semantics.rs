//! Modelo de **semántica accesible** de un nodo. Es el dato que el runtime
//! traduce a un árbol [AccessKit](https://accesskit.dev) por frame para
//! alimentar lectores de pantalla (NVDA, VoiceOver, Orca, TalkBack) y otras
//! ayudas técnicas — TTS, navegación por voz, switch control.
//!
//! Este módulo es **pura data**: define los tipos sin acoplarse al crate
//! `accesskit`. La conversión a `accesskit::Node` vive en `llimphi-ui::a11y`
//! (iter 2 del plan), donde el cableado del adapter winit ya importa la
//! librería. Tener acá solo el modelo permite:
//!
//! - Compilar el compositor con o sin la integración AccessKit habilitada.
//! - Testear semántica a nivel "qué declaran los widgets" sin levantar un
//!   adapter ni un lector real.
//! - Mantener la API estable aunque cambien versiones de `accesskit`.
//!
//! ## Cuándo declarar semántica
//!
//! - **Siempre** en controles interactivos: botones, inputs, checkboxes, tabs,
//!   ítems de menú, sliders. Sin rol declarado, el lector no sabe que el nodo
//!   ES un botón aunque tenga `on_click`.
//! - **Para texto significativo** que no es un botón: títulos (`Heading`),
//!   etiquetas asociadas, valores (`Label` / `Static`). El text de un nodo se
//!   lee igual aunque no tenga `semantics`, pero un rol explícito mejora la
//!   navegación por rol de los lectores.
//! - **Para grouping**: tabbar, dock, toolbars, listas — `Role::Group` o un
//!   rol específico (`TabList`, `Menu`, `Toolbar`) ayuda a saltar bloques.
//!
//! ## Cuándo NO declarar
//!
//! Decorativo puro (un divider, un fondo con gradiente, una sombra) **no debe**
//! declarar semántica — los lectores ya filtran texto vacío, pero un rol
//! superfluo (`Role::Group` en cada `View` envoltorio) ensucia la navegación.

use std::sync::Arc;

/// Rol semántico del nodo. Los nombres y la granularidad siguen los roles de
/// AccessKit / ARIA. Subset acotado: agregamos lo que falte cuando aparezca un
/// caller real (regla del repo — no diseñamos para lo hipotético).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// Botón clickeable. El lector dice "botón <label>" + el flag `pressed`
    /// para botones de toggle.
    Button,
    /// Campo de texto editable (single-line o multi-line). Combinable con
    /// `value` (texto actual) y los flags `readonly`/`required`.
    TextInput,
    /// Título de sección (h1..h6 en HTML). El `value` puede llevar el nivel
    /// como string ("1", "2", …) si la app lo necesita; v1 no lo distingue.
    Heading,
    /// Casilla de verificación. Combina con `checked`.
    Checkbox,
    /// Texto estático significativo (no interactivo, no título). Si solo es
    /// decorativo, no declarar semántica.
    Label,
    /// Hipervínculo / acción que navega a otra ubicación.
    Link,
    /// Ítem de un menú (context-menu, menubar, dropdown).
    MenuItem,
    /// Pestaña de un tabbar / segmented control.
    Tab,
    /// Imagen significativa. El `label` actúa como alt-text.
    Image,
    /// Control deslizable continuo (volumen, brillo, range). Combinable con
    /// `value` (string del valor actual) — los rangos numéricos se modelan
    /// más fino en iter posteriores si hace falta.
    Slider,
    /// Agrupador genérico (toolbar, panel, sección). Sirve para que los
    /// lectores ofrezcan "saltar al siguiente grupo".
    Group,
}

/// Banderas booleanas del nodo accesible. Todas opcionales (`None` = no aplica,
/// que es distinto de "aplica pero es false"). Mantienelas en None salvo que el
/// widget realmente las exponga — los lectores diferencian "no es checkable" de
/// "es checkable y no checked".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SemanticsFlags {
    /// Estado de un checkbox / radio / toggle button.
    pub checked: Option<bool>,
    /// Estado on/off de un botón de toggle (separado de `checked` porque ARIA
    /// los distingue: un toggle `<button>` usa `aria-pressed`, una checkbox
    /// `aria-checked`).
    pub pressed: Option<bool>,
    /// Para acordeones, menús, tree-rows que se expanden.
    pub expanded: Option<bool>,
    /// El control está deshabilitado (no responde a input).
    pub disabled: Option<bool>,
    /// Sólo lectura (típicamente input de texto que no se edita).
    pub readonly: Option<bool>,
    /// Campo requerido (formularios).
    pub required: Option<bool>,
}

impl SemanticsFlags {
    pub const EMPTY: Self = Self {
        checked: None,
        pressed: None,
        expanded: None,
        disabled: None,
        readonly: None,
        required: None,
    };
}

/// Especificación semántica completa de un nodo. Lo que el runtime traduce a
/// un `accesskit::Node` cada frame.
///
/// `label` es lo que el lector enuncia primero (el "nombre accesible"). Si el
/// nodo ya tiene un `text` visible y significativo, podés dejar `label = None`
/// y el runtime usará ese texto como nombre — pero declararlo explícito es más
/// robusto (e.g. un botón con sólo un ícono necesita label porque no hay texto
/// visible).
///
/// `value` es el dato dinámico (texto del input, valor del slider). El lector
/// suele leer label + value juntos: "Volumen, 70".
///
/// `description` es contexto adicional ("Disminuye el volumen del sistema").
/// Los lectores lo leen tras una pausa o con un atajo distinto; usalo para
/// info que ayude PERO no sobreloadées (los usuarios de TTS perciben ruido
/// más que falta de info).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SemanticsSpec {
    pub role: Option<Role>,
    pub label: Option<Arc<str>>,
    pub description: Option<Arc<str>>,
    pub value: Option<Arc<str>>,
    pub flags: SemanticsFlags,
}

impl SemanticsSpec {
    /// Especificación con sólo el rol fijado. Atajo común; los demás campos
    /// quedan `None` y los flags vacíos.
    pub fn role(role: Role) -> Self {
        Self {
            role: Some(role),
            ..Self::default()
        }
    }

    /// Pone `label` (consumiendo cualquier valor previo).
    pub fn with_label(mut self, s: impl Into<Arc<str>>) -> Self {
        self.label = Some(s.into());
        self
    }

    /// Pone `description`.
    pub fn with_description(mut self, s: impl Into<Arc<str>>) -> Self {
        self.description = Some(s.into());
        self
    }

    /// Pone `value`.
    pub fn with_value(mut self, s: impl Into<Arc<str>>) -> Self {
        self.value = Some(s.into());
        self
    }

    /// Pone `flags.checked = Some(v)`.
    pub fn with_checked(mut self, v: bool) -> Self {
        self.flags.checked = Some(v);
        self
    }
    pub fn with_pressed(mut self, v: bool) -> Self {
        self.flags.pressed = Some(v);
        self
    }
    pub fn with_expanded(mut self, v: bool) -> Self {
        self.flags.expanded = Some(v);
        self
    }
    pub fn with_disabled(mut self, v: bool) -> Self {
        self.flags.disabled = Some(v);
        self
    }
    pub fn with_readonly(mut self, v: bool) -> Self {
        self.flags.readonly = Some(v);
        self
    }
    pub fn with_required(mut self, v: bool) -> Self {
        self.flags.required = Some(v);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_es_todo_none_y_flags_empty() {
        let s = SemanticsSpec::default();
        assert!(s.role.is_none());
        assert!(s.label.is_none());
        assert!(s.value.is_none());
        assert_eq!(s.flags, SemanticsFlags::EMPTY);
    }

    #[test]
    fn role_builder_pone_solo_el_rol() {
        let s = SemanticsSpec::role(Role::Button);
        assert_eq!(s.role, Some(Role::Button));
        assert!(s.label.is_none());
        assert!(s.value.is_none());
        assert_eq!(s.flags, SemanticsFlags::EMPTY);
    }

    #[test]
    fn with_label_y_with_value_componen() {
        let s = SemanticsSpec::role(Role::Slider)
            .with_label("Volumen")
            .with_value("70");
        assert_eq!(s.role, Some(Role::Slider));
        assert_eq!(s.label.as_deref(), Some("Volumen"));
        assert_eq!(s.value.as_deref(), Some("70"));
    }

    #[test]
    fn flags_con_with_son_independientes() {
        let s = SemanticsSpec::role(Role::Checkbox)
            .with_checked(true)
            .with_required(true);
        assert_eq!(s.flags.checked, Some(true));
        assert_eq!(s.flags.required, Some(true));
        assert!(s.flags.disabled.is_none(), "no setear flags no tocados");
    }
}
