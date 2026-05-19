//! Barra core — modelo agnóstico de taskbar.
//!
//! Provee la lista de `Task`, los helpers de sanitización para atributos
//! HTML, y `render_html` puro. El binding DOM vive en `barra-web`.

/// Una tarea (cajita) en la barra.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    pub id: String,
    pub label: String,
    pub active: bool,
}

impl Task {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self { id: id.into(), label: label.into(), active: false }
    }
    pub fn active(mut self) -> Self {
        self.active = true;
        self
    }
}

/// Renderiza un slice de tareas a markup HTML. Sanitiza IDs y escapa
/// labels. La salida es la lista de `<li>` que el host inyecta en su `<ul>`.
pub fn render_html(tasks: &[Task]) -> String {
    let mut html = String::new();
    for t in tasks {
        let id_safe = sanitize_attr(&t.id);
        let label_safe = escape_text(&t.label);
        let active_cls = if t.active { " active" } else { "" };
        html.push_str(&format!(
            "<li><button class=\"taskbar-item{active_cls}\" data-task=\"{id_safe}\" type=\"button\">\
             <span class=\"taskbar-item-dot\" aria-hidden=\"true\"></span>{label_safe}</button></li>"
        ));
    }
    html
}

/// Filtra a `[a-zA-Z0-9_-]` para uso seguro en atributos HTML.
pub fn sanitize_attr(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

/// HTML-escape de texto para insertarlo en posiciones de contenido.
pub fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_builder_defaults_inactive() {
        let t = Task::new("aire", "AIRE");
        assert!(!t.active);
        assert!(Task::new("f", "F").active().active);
    }

    #[test]
    fn sanitize_attr_strips_unsafe() {
        assert_eq!(sanitize_attr("aire"), "aire");
        assert_eq!(sanitize_attr("a-b_c"), "a-b_c");
        assert_eq!(sanitize_attr("ai<re>"), "aire");
        assert_eq!(sanitize_attr("a\"b"), "ab");
    }

    #[test]
    fn escape_text_escapes_html() {
        assert_eq!(escape_text("AIRE"), "AIRE");
        assert_eq!(escape_text("<script>"), "&lt;script&gt;");
        assert_eq!(escape_text("a & b"), "a &amp; b");
    }

    #[test]
    fn render_html_emits_active_class() {
        let tasks = [
            Task::new("aire", "AIRE"),
            Task::new("fuego", "FUEGO").active(),
        ];
        let html = render_html(&tasks);
        assert!(html.contains("data-task=\"aire\""));
        assert!(html.contains("data-task=\"fuego\""));
        assert!(html.contains("taskbar-item active"));
    }

    #[test]
    fn render_html_escapes_label_and_sanitizes_id() {
        let tasks = [Task::new("a<b", "x<script>y")];
        let html = render_html(&tasks);
        assert!(html.contains("data-task=\"ab\""));
        assert!(html.contains("x&lt;script&gt;y"));
        assert!(!html.contains("<script>"));
    }
}
