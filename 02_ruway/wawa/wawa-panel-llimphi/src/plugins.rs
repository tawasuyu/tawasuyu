//! Editor visual de **plugins de mirada** del diente **Inicio**: lista los
//! plugins WASM instalados en `~/.config/mirada/plugins` y edita —visualmente—
//! las reglas del **asignador** (el enrutador de apps), que se guardan en el
//! campo `config:` de su manifest `.ron`.
//!
//! La lógica vive acá (pura, sin UI): leer los manifests, el borrador de reglas y
//! su `Schema` de allichay, y la reescritura del `config:` del `.ron`. El armado
//! del overlay y el ruteo de mensajes los hace `main.rs` (igual que [`remote`]).
//!
//! El host de plugins **recarga en caliente** el directorio, así que al guardar,
//! mirada reaplica las reglas sin reiniciar. El `config:` NO entra en la firma
//! del plugin, así que editarlo no la invalida.
//!
//! [`remote`]: crate::remote

use std::path::PathBuf;

use allichay::{Field, FieldValue, Schema, Section};
use serde::Deserialize;

/// El directorio de plugins de mirada del usuario.
pub fn plugins_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    PathBuf::from(home).join(".config/mirada/plugins")
}

/// El tipo de plugin (espejo de `mirada_plugin_host::PluginKind`, sólo para leer
/// el manifest).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Kind {
    Layout,
    Reactor,
}

/// Los campos del manifest que nos interesan (lo demás —firma— se deja intacto).
#[derive(Debug, Deserialize)]
struct RawManifest {
    kind: Kind,
    #[serde(default)]
    caps: Vec<String>,
    #[serde(default)]
    priority: i32,
    #[serde(default)]
    config: String,
}

/// Un plugin instalado, tal como lo lee el panel.
#[derive(Debug, Clone)]
pub struct PluginInfo {
    /// Ruta del `.ron` (donde se reescribe el `config`).
    pub path: PathBuf,
    /// Nombre = stem del archivo (`asignador.ron` → `asignador`).
    pub name: String,
    pub kind: Kind,
    pub caps: Vec<String>,
    pub priority: i32,
    pub config: String,
}

impl PluginInfo {
    pub fn kind_str(&self) -> &'static str {
        match self.kind {
            Kind::Layout => "layout",
            Kind::Reactor => "reactor",
        }
    }

    /// `true` si el panel sabe editar visualmente la config de este plugin. Hoy,
    /// sólo el **asignador** (el formato de reglas `app_id → escritorio/float`).
    pub fn editable(&self) -> bool {
        self.name == "asignador"
    }
}

/// Lee los plugins del directorio (ignora `trust.ron` y los `.ron` ilegibles).
/// Ordenados por nombre.
pub fn list_plugins() -> Vec<PluginInfo> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(plugins_dir()) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("ron") {
            continue;
        }
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
        if stem == "trust" {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(raw) = ron::from_str::<RawManifest>(&text) else {
            continue;
        };
        out.push(PluginInfo {
            path,
            name: stem,
            kind: raw.kind,
            caps: raw.caps,
            priority: raw.priority,
            config: raw.config,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// La sección-lista del diente Inicio: un renglón por plugin. El asignador (y
/// futuros editables) trae un botón `plugin:{i}` que abre el editor; el resto se
/// muestra informativo. Vacío ⇒ un aviso de cómo sembrarlos.
pub fn plugins_section(plugins: &[PluginInfo]) -> Section {
    let mut sec = Section::new("plugins::lista", "Plugins de mirada")
        .icon("🧩")
        .help(
            "Plugins WASM del Cerebro de mirada (~/.config/mirada/plugins), activos en la \
             sesión «mirada · plugins». El asignador enruta cada ventana por su app a un \
             escritorio y/o la flota — editá sus reglas acá. Se recargan en caliente.",
        );
    if plugins.is_empty() {
        return sec.field(Field::display(
            "vacio",
            "Estado",
            "No hay plugins instalados. Sembralos con install-mirada-dm.sh, o copiá los de \
             mirada-plugin-host/assets a ~/.config/mirada/plugins."
                .to_string(),
        ));
    }
    for (i, p) in plugins.iter().enumerate() {
        let caps = if p.caps.is_empty() { "—".to_string() } else { p.caps.join(", ") };
        if p.editable() {
            let n = parse_rules(&p.config).len();
            let label = format!("✎  {}   ·   {} regla(s)   ·   [{}]", p.name, n, caps);
            sec = sec.field(Field::button(format!("plugin:{i}"), label));
        } else {
            sec = sec.field(Field::display(
                format!("info:{i}"),
                p.name.clone(),
                format!("{} · prioridad {} · [{}]", p.kind_str(), p.priority, caps),
            ));
        }
    }
    sec
}

/// Una regla de enrutado del asignador (espejo editable de su DSL).
#[derive(Debug, Clone, Default)]
pub struct AppRule {
    /// Substring del `app_id` a buscar.
    pub app: String,
    /// Escritorio destino (1..9; `0` = ninguno).
    pub ws: u8,
    /// Flotar la ventana.
    pub float: bool,
}

/// El editor de reglas de **un** plugin (lo que vive en la subventana).
#[derive(Debug, Clone)]
pub struct PluginEdit {
    /// El `.ron` que se reescribe al guardar.
    pub path: PathBuf,
    pub name: String,
    pub rules: Vec<AppRule>,
}

impl PluginEdit {
    /// Abre el editor para `info`, parseando sus reglas actuales.
    pub fn open(info: &PluginInfo) -> Self {
        Self { path: info.path.clone(), name: info.name.clone(), rules: parse_rules(&info.config) }
    }

    pub fn add_rule(&mut self) {
        self.rules.push(AppRule::default());
    }

    pub fn del_rule(&mut self, i: usize) {
        if i < self.rules.len() {
            self.rules.remove(i);
        }
    }

    /// El texto de config que producen las reglas actuales (el formato del
    /// asignador). Reglas con app vacía se descartan.
    pub fn serialize(&self) -> String {
        let mut out = String::new();
        for r in &self.rules {
            let app = r.app.trim();
            if app.is_empty() {
                continue;
            }
            out.push_str(app);
            if (1..=9).contains(&r.ws) {
                out.push(' ');
                out.push_str(&r.ws.to_string());
            }
            if r.float {
                out.push_str(" float");
            }
            out.push('\n');
        }
        out
    }

    /// El `Schema` de allichay de la subventana: una fila por regla (app +
    /// escritorio + flotar + quitar), un botón «agregar», la vista previa de la
    /// config resultante y guardar / cancelar.
    pub fn schema(&self) -> Schema {
        let mut sec = Section::new("plugin::form", format!("Reglas — {}", self.name)).help(
            "Una regla por app: si el app_id CONTIENE el texto, la ventana va al escritorio \
             elegido (0 = ninguno) y/o flota. Se aplica al abrir cada ventana; gana la primera \
             que case.",
        );
        for (i, r) in self.rules.iter().enumerate() {
            sec = sec
                .field(Field::text(
                    format!("rule:{i}:app"),
                    format!("Regla {} · app_id contiene", i + 1),
                    r.app.clone(),
                ))
                .field(Field::slider_int(
                    format!("rule:{i}:ws"),
                    "    → escritorio (0 = ninguno)",
                    r.ws as i64,
                    0,
                    9,
                ))
                .field(Field::toggle(format!("rule:{i}:float"), "    flotar", r.float))
                .field(Field::button(format!("rule:{i}:del"), "    ✕  quitar regla"));
        }
        sec = sec
            .field(Field::button("add", "＋  agregar regla"))
            .field(Field::display("preview", "config resultante", self.serialize()))
            .field(Field::button("guardar", "✔  Guardar (recarga en caliente)"))
            .field(Field::button("cancelar", "Cancelar"));
        Schema { sections: vec![sec] }
    }

    /// Aplica el cambio de un campo (`rule:{i}:{app|ws|float}`) al borrador. Los
    /// botones (add/del/guardar/cancelar) los intercepta `main.rs`.
    pub fn apply(&mut self, leaf: &str, value: FieldValue) {
        let parts: Vec<&str> = leaf.split(':').collect();
        if parts.len() != 3 || parts[0] != "rule" {
            return;
        }
        let Ok(i) = parts[1].parse::<usize>() else {
            return;
        };
        let Some(r) = self.rules.get_mut(i) else {
            return;
        };
        match parts[2] {
            "app" => {
                if let Some(s) = value.as_str() {
                    r.app = s.to_string();
                }
            }
            "ws" => {
                if let Some(n) = value.as_int() {
                    r.ws = n.clamp(0, 9) as u8;
                }
            }
            "float" => {
                if let Some(b) = value.as_bool() {
                    r.float = b;
                }
            }
            _ => {}
        }
    }

    /// El texto actual de un campo de texto (para sembrar el buffer de edición al
    /// enfocarlo). Sólo los `rule:{i}:app` son de texto.
    pub fn text_value(&self, leaf: &str) -> String {
        let parts: Vec<&str> = leaf.split(':').collect();
        if parts.len() == 3 && parts[0] == "rule" && parts[2] == "app" {
            if let Ok(i) = parts[1].parse::<usize>() {
                return self.rules.get(i).map(|r| r.app.clone()).unwrap_or_default();
            }
        }
        String::new()
    }

    /// Reescribe el `config:` del `.ron` con las reglas actuales, dejando el
    /// resto del manifest (firma incluida) intacto.
    pub fn save(&self) -> std::io::Result<()> {
        let text = std::fs::read_to_string(&self.path)?;
        let nuevo = set_config_field(&text, &self.serialize());
        std::fs::write(&self.path, nuevo)
    }
}

/// Parsea el texto de config (formato del asignador) a reglas. Líneas vacías o
/// `#…` se ignoran. Conserva el orden.
fn parse_rules(config: &str) -> Vec<AppRule> {
    let mut rules = Vec::new();
    for line in config.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut toks = line.split_whitespace();
        let Some(app) = toks.next() else { continue };
        let mut ws = 0u8;
        let mut float = false;
        for t in toks {
            if t.eq_ignore_ascii_case("float") {
                float = true;
            } else if let Ok(n) = t.parse::<u8>() {
                if (1..=9).contains(&n) {
                    ws = n;
                }
            }
        }
        rules.push(AppRule { app: app.to_string(), ws, float });
    }
    rules
}

/// Escapa una cadena para un string RON (`"..."`): backslash, comilla, salto y tab.
fn escape_ron(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            _ => o.push(c),
        }
    }
    o
}

/// Reescribe el campo `config:` del texto RON de un manifest con `config` nuevo,
/// dejando todo lo demás igual. Si ya hay una línea `config:`, la reemplaza; si
/// no, la inserta antes del `)` de cierre. El valor va en una sola línea con los
/// saltos escapados como `\n`.
fn set_config_field(ron: &str, config: &str) -> String {
    let linea = format!("    config: \"{}\",", escape_ron(config));
    let mut out: Vec<String> = Vec::new();
    let mut reemplazada = false;
    for line in ron.lines() {
        if line.trim_start().starts_with("config:") {
            out.push(linea.clone());
            reemplazada = true;
        } else {
            out.push(line.to_string());
        }
    }
    let mut text = out.join("\n");
    if ron.ends_with('\n') {
        text.push('\n');
    }
    if !reemplazada {
        if let Some(pos) = text.rfind(')') {
            text.insert_str(pos, &format!("{linea}\n"));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_y_serializa_reglas_redondea() {
        let cfg = "# comentario\nfirefox 2\npavucontrol float\ncalc 5 float\n";
        let rules = parse_rules(cfg);
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].app, "firefox");
        assert_eq!(rules[0].ws, 2);
        assert!(!rules[0].float);
        assert!(rules[1].float && rules[1].ws == 0);
        assert!(rules[2].ws == 5 && rules[2].float);

        let edit = PluginEdit { path: PathBuf::new(), name: "asignador".into(), rules };
        // Round-trip sin los comentarios (el editor maneja las reglas).
        assert_eq!(edit.serialize(), "firefox 2\npavucontrol float\ncalc 5 float\n");
    }

    #[test]
    fn serialize_descarta_reglas_sin_app() {
        let edit = PluginEdit {
            path: PathBuf::new(),
            name: "asignador".into(),
            rules: vec![
                AppRule { app: "  ".into(), ws: 3, float: false },
                AppRule { app: "foot".into(), ws: 1, float: false },
            ],
        };
        assert_eq!(edit.serialize(), "foot 1\n");
    }

    #[test]
    fn apply_rutea_los_campos_de_una_regla() {
        let mut edit = PluginEdit {
            path: PathBuf::new(),
            name: "asignador".into(),
            rules: vec![AppRule::default()],
        };
        edit.apply("rule:0:app", FieldValue::Text("firefox".into()));
        edit.apply("rule:0:ws", FieldValue::Int(4));
        edit.apply("rule:0:float", FieldValue::Bool(true));
        assert_eq!(edit.rules[0].app, "firefox");
        assert_eq!(edit.rules[0].ws, 4);
        assert!(edit.rules[0].float);
        assert_eq!(edit.serialize(), "firefox 4 float\n");
    }

    #[test]
    fn add_y_del_regla() {
        let mut edit =
            PluginEdit { path: PathBuf::new(), name: "asignador".into(), rules: vec![] };
        edit.add_rule();
        edit.add_rule();
        assert_eq!(edit.rules.len(), 2);
        edit.del_rule(0);
        assert_eq!(edit.rules.len(), 1);
        edit.del_rule(9); // fuera de rango = no-op
        assert_eq!(edit.rules.len(), 1);
    }

    #[test]
    fn set_config_field_reemplaza_la_linea_existente() {
        let ron = "(\n    wasm: \"a.wasm\",\n    kind: Reactor,\n    config: \"viejo\",\n)\n";
        let nuevo = set_config_field(ron, "firefox 2\nfoot 1\n");
        assert!(nuevo.contains("config: \"firefox 2\\nfoot 1\\n\","));
        assert!(!nuevo.contains("viejo"));
        // El resto queda intacto.
        assert!(nuevo.contains("kind: Reactor,") && nuevo.contains("wasm: \"a.wasm\","));
        // Sigue siendo un manifest válido (re-deserializa).
        let raw: RawManifest = ron::from_str(&nuevo).unwrap();
        assert_eq!(raw.config, "firefox 2\nfoot 1\n");
    }

    #[test]
    fn set_config_field_inserta_si_falta() {
        let ron = "(\n    wasm: \"a.wasm\",\n    kind: Reactor,\n)\n";
        let nuevo = set_config_field(ron, "foot 1\n");
        let raw: RawManifest = ron::from_str(&nuevo).unwrap();
        assert_eq!(raw.config, "foot 1\n");
    }

    #[test]
    fn editable_solo_el_asignador() {
        let asg = PluginInfo {
            path: PathBuf::new(),
            name: "asignador".into(),
            kind: Kind::Reactor,
            caps: vec!["actions".into()],
            priority: 0,
            config: String::new(),
        };
        assert!(asg.editable());
        let dw = PluginInfo { name: "dwindle".into(), kind: Kind::Layout, ..asg.clone() };
        assert!(!dw.editable());
    }

    #[test]
    fn la_seccion_lista_boton_para_editables_y_display_para_el_resto() {
        let plugins = vec![
            PluginInfo {
                path: PathBuf::new(),
                name: "asignador".into(),
                kind: Kind::Reactor,
                caps: vec!["actions".into()],
                priority: 0,
                config: "firefox 2\n".into(),
            },
            PluginInfo {
                path: PathBuf::new(),
                name: "dwindle".into(),
                kind: Kind::Layout,
                caps: vec!["layout".into()],
                priority: 20,
                config: String::new(),
            },
        ];
        let sec = plugins_section(&plugins);
        let ids: Vec<&str> = sec.fields.iter().map(|f| f.id.as_str()).collect();
        assert!(ids.contains(&"plugin:0"), "el asignador es un botón editable");
        assert!(ids.contains(&"info:1"), "dwindle es informativo");
    }
}
