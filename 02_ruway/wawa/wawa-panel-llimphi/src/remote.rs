//! Sesiones remotas (waypipe) del diente **Inicio**: la lista de sesiones del
//! `startup` de mirada (`config.ron`) y el editor de **una** sesión que se abre
//! en una subventana (overlay) del panel.
//!
//! La lógica vive acá (pura, sin UI): la sección-lista, el borrador editable y
//! su `Schema` de allichay, y la detección de `waypipe`/`ssh` en el PATH. El
//! armado del overlay (scrim + caja) y el ruteo de mensajes los hace `main.rs`,
//! que es quien tiene a mano el `Model`, el `Msg` y los tipos de vista.
//!
//! El editor edita un [`mirada_brain::StartupApp`]; al guardar, `main.rs` lo
//! mete en `mirada.startup` y persiste — mirada lo lanza al arrancar (envuelto
//! en `waypipe ssh` si hay host) y lo ubica por las reglas derivadas. Ver
//! `mirada_brain::config` (StartupApp / waypipe_command).

use allichay::{EnumOption, Field, FieldValue, Schema, Section};
use mirada_brain::StartupApp;

/// `true` si `name` está en alguno de los directorios del `PATH`. Mismo criterio
/// que `shuma` usa para detectar binarios (docker/podman): existencia del
/// archivo, sin chequear el bit de ejecución (suficiente para un aviso).
fn binary_in_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(name).exists()))
        .unwrap_or(false)
}

/// Disponibilidad de las herramientas que necesita una sesión remota:
/// `(waypipe, ssh)`. Ambas deben estar en esta máquina (y waypipe también en el
/// host remoto, lo que no podemos comprobar desde acá).
pub fn tooling() -> (bool, bool) {
    (binary_in_path("waypipe"), binary_in_path("ssh"))
}

/// El aviso sobre waypipe/ssh para mostrar arriba de la lista.
pub fn tooling_warning() -> String {
    match tooling() {
        (false, _) => "⚠ waypipe no está en el PATH — instalalo en ESTA máquina y en el host \
                       remoto, o las sesiones remotas no abrirán."
            .to_string(),
        (true, false) => {
            "⚠ ssh no está en el PATH — waypipe lo necesita para abrir el túnel.".to_string()
        }
        (true, true) => "✓ waypipe y ssh disponibles (recordá que waypipe también \
                         debe estar en el host remoto)."
            .to_string(),
    }
}

/// Las opciones de compresión del túnel waypipe (slug → rótulo). El slug vacío
/// es «el default de waypipe» (no pasa la bandera).
fn compress_options() -> Vec<EnumOption> {
    vec![
        EnumOption::new("", "(default de waypipe)"),
        EnumOption::new("none", "sin compresión"),
        EnumOption::new("lz4", "lz4 — rápido, baja latencia"),
        EnumOption::new("zstd", "zstd — comprime más (enlaces flacos)"),
    ]
}

/// Recorta un comando largo para el rótulo de la lista.
fn short(cmd: &str) -> String {
    let c = cmd.trim();
    if c.chars().count() > 28 {
        format!("{}…", c.chars().take(27).collect::<String>())
    } else {
        c.to_string()
    }
}

/// La sección-lista del diente Inicio: el aviso de waypipe + un botón por sesión
/// (abre el editor en la subventana) + el botón «nueva». Los ids de los botones
/// codifican la acción y el índice (`editar:3`), que `main.rs` parsea.
pub fn sessions_section(startup: &[StartupApp]) -> Section {
    let mut sec = Section::new("remote::sesiones", "Sesiones remotas (waypipe)")
        .icon("🔗")
        .help(
            "Apps de OTRA máquina que mirada abre al iniciar sesión, vía waypipe+ssh, y \
             aterrizan en su escritorio como una app local. Se guardan en el `startup` de \
             config.ron. Tocá una para editarla en su ventana, o «＋» para crear.",
        );
    sec = sec.field(Field::display("waypipe", "Estado", tooling_warning()));
    for (i, a) in startup.iter().enumerate() {
        let host = if a.remote.trim().is_empty() {
            "local".to_string()
        } else {
            a.remote.clone()
        };
        let esc = if a.workspace == 0 {
            "—".to_string()
        } else {
            a.workspace.to_string()
        };
        let label = format!("✎  {}   ·   {}   ·   esc {}", short(&a.command), host, esc);
        sec = sec.field(Field::button(format!("editar:{i}"), label));
    }
    sec.field(Field::button("nueva", "＋  nueva sesión remota"))
}

/// El editor de **una** sesión (lo que vive en la subventana). `idx` es el
/// índice en `mirada.startup` que se edita; `None` = sesión nueva (se agrega al
/// guardar). `draft` es el borrador que la UI muta campo a campo.
#[derive(Debug, Clone)]
pub struct RemoteEdit {
    pub idx: Option<usize>,
    pub draft: StartupApp,
}

impl RemoteEdit {
    /// Editor para una sesión nueva (borrador vacío).
    pub fn nueva() -> Self {
        Self { idx: None, draft: StartupApp::default() }
    }

    /// Editor para la sesión existente `app` en el índice `idx`.
    pub fn editar(idx: usize, app: &StartupApp) -> Self {
        Self { idx: Some(idx), draft: app.clone() }
    }

    /// El `Schema` de allichay que la subventana renderiza: un formulario con los
    /// campos de la sesión, una vista previa del comando resultante y los botones
    /// guardar / borrar / cancelar.
    pub fn schema(&self) -> Schema {
        let d = &self.draft;
        let titulo = if self.idx.is_some() {
            "Editar sesión remota"
        } else {
            "Nueva sesión remota"
        };
        let mut sec = Section::new("rsesion::form", titulo)
            .field(
                Field::text("command", "Comando", d.command.clone())
                    .help("La app a lanzar (programa + args). Si el host queda vacío, es local."),
            )
            .field(
                Field::text("remote", "Host  ([user@]host)", d.remote.clone())
                    .help("Vacío = app local. Con host = se envuelve en `waypipe ssh`."),
            )
            .field(
                Field::text(
                    "ssh_port",
                    "Puerto ssh (vacío/22 = default)",
                    if d.ssh_port == 0 { String::new() } else { d.ssh_port.to_string() },
                )
                .help("Puerto del host remoto (ssh -p)."),
            )
            .field(
                Field::text("ssh_key", "Clave ssh -i (vacío = default)", d.ssh_key.clone())
                    .help("Ruta a la clave privada; vacío = la que elija ssh (agente/~/.ssh)."),
            )
            .field(
                Field::slider_int("workspace", "Escritorio (0 = el activo)", d.workspace as i64, 0, 9),
            )
            .field(
                Field::text("app_id", "app_id (necesario para anclar)", d.app_id.clone())
                    .help("Con qué app_id se reconoce la ventana al abrir, para fijar escritorio/flotante."),
            )
            .field(
                Field::dropdown("compress", "Compresión waypipe", d.compress.clone(), compress_options())
                    .help("Baja latencia/ancho de banda en enlaces lentos."),
            )
            .field(
                Field::toggle("video", "Vídeo (H.264/VP9)", d.video)
                    .help("Codifica las superficies como vídeo — mucho menos ancho de banda en ventanas grandes."),
            )
            .field(Field::slider_int("threads", "Hilos waypipe (0 = auto)", d.threads as i64, 0, 16))
            .field(Field::toggle("floating", "Flotante", d.floating))
            .field(Field::toggle("fullscreen", "Pantalla completa", d.fullscreen))
            .field(Field::display("preview", "Comando resultante", d.shell_command()))
            .field(Field::button("guardar", "✔  Guardar"));
        if self.idx.is_some() {
            sec = sec.field(Field::button("borrar", "✕  Borrar sesión"));
        }
        sec = sec.field(Field::button("cancelar", "Cancelar"));
        Schema { sections: vec![sec] }
    }

    /// Aplica el cambio de un campo (por su `leaf`) al borrador. Los botones
    /// (guardar/borrar/cancelar) NO se manejan acá — los intercepta `main.rs`.
    pub fn apply(&mut self, leaf: &str, value: FieldValue) {
        let d = &mut self.draft;
        match leaf {
            "command" => {
                if let Some(s) = value.as_str() {
                    d.command = s.to_string();
                }
            }
            "remote" => {
                if let Some(s) = value.as_str() {
                    d.remote = s.to_string();
                }
            }
            "ssh_port" => {
                if let Some(s) = value.as_str() {
                    d.ssh_port = s.trim().parse().unwrap_or(0);
                }
            }
            "ssh_key" => {
                if let Some(s) = value.as_str() {
                    d.ssh_key = s.to_string();
                }
            }
            "workspace" => {
                if let Some(i) = value.as_int() {
                    d.workspace = i.max(0) as usize;
                }
            }
            "app_id" => {
                if let Some(s) = value.as_str() {
                    d.app_id = s.to_string();
                }
            }
            "compress" => {
                if let Some(s) = value.as_str() {
                    d.compress = s.to_string();
                }
            }
            "video" => {
                if let Some(b) = value.as_bool() {
                    d.video = b;
                }
            }
            "threads" => {
                if let Some(i) = value.as_int() {
                    d.threads = i.max(0) as u32;
                }
            }
            "floating" => {
                if let Some(b) = value.as_bool() {
                    d.floating = b;
                }
            }
            "fullscreen" => {
                if let Some(b) = value.as_bool() {
                    d.fullscreen = b;
                }
            }
            _ => {}
        }
    }

    /// El texto actual de un campo de texto (para sembrar el buffer de edición al
    /// enfocarlo). Devuelve vacío para campos no-texto.
    pub fn text_value(&self, leaf: &str) -> String {
        let d = &self.draft;
        match leaf {
            "command" => d.command.clone(),
            "remote" => d.remote.clone(),
            "ssh_port" => {
                if d.ssh_port == 0 {
                    String::new()
                } else {
                    d.ssh_port.to_string()
                }
            }
            "ssh_key" => d.ssh_key.clone(),
            "app_id" => d.app_id.clone(),
            _ => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editar_aplica_campos_y_arma_comando() {
        let mut e = RemoteEdit::nueva();
        e.apply("command", FieldValue::Text("foot".into()));
        e.apply("remote", FieldValue::Text("sergio@servidor".into()));
        e.apply("ssh_port", FieldValue::Text("2222".into()));
        e.apply("compress", FieldValue::Enum("zstd".into()));
        e.apply("video", FieldValue::Bool(true));
        e.apply("workspace", FieldValue::Int(3));
        e.apply("app_id", FieldValue::Text("foot".into()));
        assert_eq!(
            e.draft.shell_command(),
            "waypipe --compress=zstd --video ssh -p 2222 sergio@servidor foot"
        );
        // Con app_id + workspace, la sesión produce una regla de anclaje.
        assert!(e.draft.placement_rule().is_some());
    }

    #[test]
    fn schema_lista_los_campos_y_botones_segun_modo() {
        // Nueva: sin botón borrar.
        let nueva = RemoteEdit::nueva().schema();
        let ids: Vec<&str> = nueva.sections[0].fields.iter().map(|f| f.id.as_str()).collect();
        assert!(ids.contains(&"command") && ids.contains(&"guardar") && ids.contains(&"cancelar"));
        assert!(!ids.contains(&"borrar"));
        // Editar: con botón borrar.
        let editar = RemoteEdit::editar(0, &StartupApp::default()).schema();
        let ids: Vec<&str> = editar.sections[0].fields.iter().map(|f| f.id.as_str()).collect();
        assert!(ids.contains(&"borrar"));
    }

    #[test]
    fn la_seccion_lista_un_boton_por_sesion_mas_nueva() {
        let startup = vec![
            StartupApp { command: "foot".into(), remote: "h".into(), ..Default::default() },
            StartupApp { command: "mpv".into(), ..Default::default() },
        ];
        let sec = sessions_section(&startup);
        let ids: Vec<&str> = sec.fields.iter().map(|f| f.id.as_str()).collect();
        // aviso + 2 sesiones + nueva.
        assert!(ids.contains(&"waypipe"));
        assert!(ids.contains(&"editar:0") && ids.contains(&"editar:1"));
        assert!(ids.contains(&"nueva"));
    }
}
