use serde::{Deserialize, Serialize};

/// Rol semántico de un calendario — independiza la UI del nombre concreto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CalendarRole {
    Personal,
    Work,
    Holidays,
    Birthdays,
    Other,
}

impl CalendarRole {
    /// Orden de presentación canónico.
    pub fn sort_key(self) -> u8 {
        match self {
            CalendarRole::Personal => 0,
            CalendarRole::Work => 1,
            CalendarRole::Holidays => 2,
            CalendarRole::Birthdays => 3,
            CalendarRole::Other => 4,
        }
    }
}

/// Un calendario (una colección CalDAV). El `id` es la ruta/identificador del
/// servidor; `color` es un hex `#rrggbb` opcional para pintarlo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Calendar {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
    pub role: CalendarRole,
}

impl Calendar {
    /// Construye un calendario infiriendo el rol del nombre.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        let name = name.into();
        let role = role_from_name(&name);
        Self { id: id.into(), name, color: None, role }
    }

    pub fn with_color(mut self, color: impl Into<String>) -> Self {
        self.color = Some(color.into());
        self
    }
}

/// Infiere el rol a partir del nombre (inglés/español comunes).
fn role_from_name(name: &str) -> CalendarRole {
    let lower = name.trim().to_ascii_lowercase();
    if lower.contains("trabajo") || lower.contains("work") || lower.contains("oficina") {
        CalendarRole::Work
    } else if lower.contains("feriado") || lower.contains("holiday") || lower.contains("festivo") {
        CalendarRole::Holidays
    } else if lower.contains("cumple") || lower.contains("birthday") {
        CalendarRole::Birthdays
    } else if lower.contains("personal") || lower.contains("casa") || lower.contains("home") {
        CalendarRole::Personal
    } else {
        CalendarRole::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infiere_rol() {
        assert_eq!(Calendar::new("c1", "Trabajo").role, CalendarRole::Work);
        assert_eq!(Calendar::new("c2", "Cumpleaños").role, CalendarRole::Birthdays);
        assert_eq!(Calendar::new("c3", "Proyecto X").role, CalendarRole::Other);
    }

    #[test]
    fn color_opcional() {
        let c = Calendar::new("c1", "Personal").with_color("#3b82f6");
        assert_eq!(c.color.as_deref(), Some("#3b82f6"));
        assert_eq!(c.role, CalendarRole::Personal);
    }
}
