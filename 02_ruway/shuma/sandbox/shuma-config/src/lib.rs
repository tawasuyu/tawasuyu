//! `shuma-config` — el fichero de configuración personal del shell.
//!
//! Se carga al arrancar desde `$XDG_CONFIG_HOME/shuma/shumarc.toml`
//! (típicamente `~/.config/shuma/shumarc.toml` en Linux). Si no existe
//! o no se pudo parsear, el shell arranca con [`Config::default`] —
//! aquí no hay nada crítico, sólo preferencias del usuario.
//!
//! Esquema mínimo:
//!
//! ```toml
//! # ---- Aliases ----
//! # Se expanden ANTES del tokenizer: la primera palabra de la línea,
//! # si coincide, se reemplaza por el cuerpo.
//! [aliases]
//! ll = "ls -la"
//! gs = "git status --short"
//!
//! # ---- Variables de entorno ----
//! # Se exportan al proceso del shell al cargar; los procesos hijos las
//! # heredan.
//! [env]
//! EDITOR = "hx"
//!
//! # ---- Prompt ----
//! # Segmentos en orden. Tokens soportados:
//! #   "cwd", "git", "exit", "time", o cualquier literal.
//! [prompt]
//! segments = ["cwd", "git", "exit"]
//!
//! # ---- Historial durable ----
//! [history]
//! dedup = "ignore_consecutive"  # none | ignore_consecutive | erase_dups
//!
//! # ---- Captura de salida ----
//! [capture]
//! limit_mb = 8
//! spill = false
//! ```

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Política de deduplicación, paralela a la de `shuma-history` pero
/// codificada como string en el fichero TOML para que el rc sea legible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DedupPolicy {
    None,
    #[default]
    IgnoreConsecutive,
    EraseDups,
}

/// Configuración del historial durable.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HistoryConfig {
    #[serde(default)]
    pub dedup: DedupPolicy,
}

/// Configuración de la política de captura de salida por sesión.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureConfig {
    /// Tope en MiB; `0` = sin tope.
    #[serde(default = "default_limit_mb")]
    pub limit_mb: usize,
    /// Si la salida que excede el tope se vuelca a disco.
    #[serde(default)]
    pub spill: bool,
}

fn default_limit_mb() -> usize {
    8
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self { limit_mb: 8, spill: false }
    }
}

/// Configuración del prompt — segmentos en orden.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptConfig {
    #[serde(default = "default_segments")]
    pub segments: Vec<String>,
}

fn default_segments() -> Vec<String> {
    vec!["cwd".into()]
}

impl Default for PromptConfig {
    fn default() -> Self {
        Self { segments: default_segments() }
    }
}

/// Configuración del scrollback del surface (Fase 5.7+ del SDD-TERMINAL).
/// `limit_mb` cap en memoria, `spill` activa el archivo de archive para
/// líneas que se recortan del frente. `spill_path` vacío = elegido
/// automáticamente bajo `$XDG_RUNTIME_DIR/shuma-<pid>.spill`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrollbackConfig {
    /// Cap del scrollback en MiB. `0` = sin cap (peligroso para sesiones
    /// largas — la memoria crece sin tope).
    #[serde(default = "default_scrollback_mb")]
    pub limit_mb: usize,
    /// Si las líneas recortadas se archivan a un spill file en disco.
    #[serde(default)]
    pub spill: bool,
    /// Path del spill file. Vacío = elegido automáticamente.
    #[serde(default)]
    pub spill_path: String,
}

fn default_scrollback_mb() -> usize {
    4
}

impl Default for ScrollbackConfig {
    fn default() -> Self {
        Self {
            limit_mb: default_scrollback_mb(),
            spill: false,
            spill_path: String::new(),
        }
    }
}

/// Configuración completa cargada del `.shumarc.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Config {
    /// Aliases: la primera palabra de una línea se reemplaza por el cuerpo.
    #[serde(default)]
    pub aliases: HashMap<String, String>,
    /// Variables de entorno a exportar al proceso del shell al cargar.
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub prompt: PromptConfig,
    #[serde(default)]
    pub history: HistoryConfig,
    #[serde(default)]
    pub capture: CaptureConfig,
    #[serde(default)]
    pub scrollback: ScrollbackConfig,
}

impl Config {
    /// Ruta por defecto: `$XDG_CONFIG_HOME/shuma/shumarc.toml`. `None` si
    /// el SO no expone un directorio de configuración.
    pub fn default_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "shuma")
            .map(|d| d.config_dir().join("shumarc.toml"))
    }

    /// Directorio donde el shell busca completions extendidas:
    /// `$XDG_CONFIG_HOME/shuma/completions/`. Cada archivo `<cmd>.toml`
    /// declara las flags de un comando — el shell las suma a la tabla
    /// estática de [`shuma_line::flag_hints`].
    pub fn completions_dir() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "shuma")
            .map(|d| d.config_dir().join("completions"))
    }

    /// Carga la configuración del path indicado. Si el fichero no existe
    /// devuelve [`Config::default`] sin error (caso normal en arranque
    /// limpio).
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::Io(path.to_path_buf(), e))?;
        toml::from_str(&text).map_err(|e| ConfigError::Parse(path.to_path_buf(), e))
    }

    /// Carga la configuración del path por defecto. Errores blandos
    /// (parse, IO) se devuelven; ausencia del fichero da `default`.
    pub fn load_default() -> Result<Self, ConfigError> {
        match Self::default_path() {
            Some(p) => Self::load(p),
            None => Ok(Self::default()),
        }
    }

    /// Aplica las variables de entorno declaradas al proceso actual.
    /// Pensado para llamarse una vez al arrancar el shell; los procesos
    /// hijos heredan el entorno y verán los valores.
    pub fn apply_env(&self) {
        for (k, v) in &self.env {
            // SAFETY: setenv no es seguro en presencia de hilos concurrentes
            // que lean getenv. El shell la llama una vez en el hilo principal,
            // antes de spawnear ningún subproceso, así que es válido.
            // SAFETY (Rust 2024): `set_var` es unsafe sólo bajo
            // edición 2024; en 2021 sigue siendo seguro.
            std::env::set_var(k, v);
        }
    }

    /// Expande aliases en una línea: si la **primera palabra** coincide
    /// con un alias, se reemplaza por su cuerpo. El resto de la línea
    /// queda intacto.
    ///
    /// Convención simple — sin parámetros posicionales, sin recursión
    /// (un alias se expande una vez, no se persigue el resultado).
    pub fn expand_aliases<'a>(&self, line: &'a str) -> std::borrow::Cow<'a, str> {
        let trimmed = line.trim_start();
        let leading_ws = line.len() - trimmed.len();
        let (head, rest) = match trimmed.find(char::is_whitespace) {
            Some(i) => (&trimmed[..i], &trimmed[i..]),
            None => (trimmed, ""),
        };
        if let Some(body) = self.aliases.get(head) {
            let mut out = String::with_capacity(line.len() + body.len());
            out.push_str(&line[..leading_ws]);
            out.push_str(body);
            out.push_str(rest);
            std::borrow::Cow::Owned(out)
        } else {
            std::borrow::Cow::Borrowed(line)
        }
    }
}

// ─── Grupos de environment (la config del sidebar del shell) ───────────
//
// Un grupo nombrado de variables que se activa/desactiva en bloque desde
// la UI (`env.json` en el config dir). El builtin `:env` escribe al grupo
// «general»; la app puede definir grupos por proyecto/credenciales/etc.

/// Grupo de variables de entorno activable en bloque.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvGroup {
    pub name: String,
    /// Si el grupo está aplicado al proceso (los hijos lo heredan).
    #[serde(default)]
    pub active: bool,
    /// Pares `(NOMBRE, valor)` en orden estable.
    #[serde(default)]
    pub vars: Vec<(String, String)>,
}

impl EnvGroup {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), active: true, vars: Vec::new() }
    }

    /// Inserta o reemplaza una variable del grupo.
    pub fn upsert(&mut self, name: &str, value: &str) {
        match self.vars.iter_mut().find(|(n, _)| n == name) {
            Some((_, v)) => *v = value.to_string(),
            None => self.vars.push((name.to_string(), value.to_string())),
        }
    }

    /// Borra una variable. Devuelve `true` si existía.
    pub fn remove(&mut self, name: &str) -> bool {
        let antes = self.vars.len();
        self.vars.retain(|(n, _)| n != name);
        self.vars.len() != antes
    }
}

/// `$XDG_CONFIG_HOME/shuma/env.json` — el archivo de grupos.
pub fn env_groups_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "shuma").map(|d| d.config_dir().join("env.json"))
}

/// `$XDG_CONFIG_HOME/shuma/macros.toml` — el libro de macros (`:macro`). El
/// tipo (`shuma_intent::MacroBook`) vive en otro crate; acá sólo la ruta.
pub fn macros_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "shuma").map(|d| d.config_dir().join("macros.toml"))
}

/// Lee los grupos. Archivo ausente o corrupto → lista vacía (sin error:
/// es config de conveniencia, el shell arranca igual).
pub fn load_env_groups() -> Vec<EnvGroup> {
    env_groups_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persiste los grupos (atómico: tmp + rename).
pub fn save_env_groups(groups: &[EnvGroup]) -> std::io::Result<()> {
    let Some(path) = env_groups_path() else {
        return Ok(());
    };
    let json = serde_json::to_string_pretty(groups)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

/// Aplica/levanta un grupo del ambiente del proceso. `on = true` exporta
/// todas sus variables; `false` las remueve. Los hijos nuevos lo heredan.
pub fn apply_env_group(group: &EnvGroup, on: bool) {
    for (k, v) in &group.vars {
        if on {
            std::env::set_var(k, v);
        } else {
            std::env::remove_var(k);
        }
    }
}

/// Upsert **quirúrgico** de `key = value_raw` en la sección `[section]`
/// del archivo TOML en `path`: edita el TEXTO (preserva comentarios y el
/// resto de las secciones), crea el archivo y/o la sección si faltan.
/// `value_raw` va literal — el caller decide el formato TOML (`"texto"`
/// con [`toml_string`], `true`, `64`).
pub fn upsert_key(path: &Path, section: &str, key: &str, value_raw: &str) -> std::io::Result<()> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let header = format!("[{section}]");
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let nueva = format!("{key} = {value_raw}");

    // Buscar la sección.
    let sec_idx = lines.iter().position(|l| l.trim() == header);
    match sec_idx {
        Some(si) => {
            // Rango de la sección: desde si+1 hasta el próximo header.
            let fin = lines[si + 1..]
                .iter()
                .position(|l| l.trim_start().starts_with('['))
                .map(|o| si + 1 + o)
                .unwrap_or(lines.len());
            // ¿La clave ya existe adentro? → reemplazo in-place.
            for l in lines[si + 1..fin].iter_mut() {
                let lt = l.trim_start();
                if let Some(eq) = lt.find('=') {
                    if lt[..eq].trim() == key {
                        *l = nueva;
                        let out = lines.join("\n") + "\n";
                        return write_atomico(path, &out);
                    }
                }
            }
            // No existe: insertar al final de la sección (antes de líneas
            // en blanco que la separen de la próxima).
            let mut ins = fin;
            while ins > si + 1 && lines[ins - 1].trim().is_empty() {
                ins -= 1;
            }
            lines.insert(ins, nueva);
        }
        None => {
            if !lines.is_empty() && !lines.last().map(|l| l.is_empty()).unwrap_or(true) {
                lines.push(String::new());
            }
            lines.push(header);
            lines.push(nueva);
        }
    }
    let out = lines.join("\n") + "\n";
    write_atomico(path, &out)
}

/// Borra `key` de la sección `[section]`. Devuelve `true` si existía.
pub fn remove_key(path: &Path, section: &str, key: &str) -> std::io::Result<bool> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(false);
    };
    let header = format!("[{section}]");
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let Some(si) = lines.iter().position(|l| l.trim() == header) else {
        return Ok(false);
    };
    let fin = lines[si + 1..]
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .map(|o| si + 1 + o)
        .unwrap_or(lines.len());
    let antes = lines.len();
    let mut i = si + 1;
    let mut fin = fin;
    while i < fin {
        let lt = lines[i].trim_start();
        let es_clave = lt
            .find('=')
            .map(|eq| lt[..eq].trim() == key)
            .unwrap_or(false);
        if es_clave {
            lines.remove(i);
            fin -= 1;
        } else {
            i += 1;
        }
    }
    if lines.len() == antes {
        return Ok(false);
    }
    let out = lines.join("\n") + "\n";
    write_atomico(path, &out)?;
    Ok(true)
}

/// Serializa un string como TOML basic string (comillas + escapes).
pub fn toml_string(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

/// Escritura atómica: tmp + rename, para no dejar un rc a medias si el
/// proceso muere en medio del write.
fn write_atomico(path: &Path, contenido: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, contenido)?;
    std::fs::rename(&tmp, path)
}

impl From<DedupPolicy> for &'static str {
    fn from(p: DedupPolicy) -> Self {
        match p {
            DedupPolicy::None => "none",
            DedupPolicy::IgnoreConsecutive => "ignore_consecutive",
            DedupPolicy::EraseDups => "erase_dups",
        }
    }
}

/// Expande `$VAR` y `${VAR}` en un texto contra `getenv`. Si la variable
/// no existe, se sustituye por cadena vacía — convención bash. Las
/// barras `\$` escapan el signo.
pub fn expand_env(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\\' && i + 1 < bytes.len() && bytes[i + 1] == b'$' {
            out.push('$');
            i += 2;
            continue;
        }
        if c != b'$' {
            out.push(bytes[i] as char);
            i += 1;
            continue;
        }
        // `$VAR` o `${VAR}`.
        let (name_end, with_braces) = if i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            // `${VAR}` — buscar la `}` que cierra.
            match s[i + 2..].find('}') {
                Some(off) => (i + 2 + off, true),
                None => {
                    out.push('$');
                    i += 1;
                    continue;
                }
            }
        } else {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len()
                && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_')
            {
                end += 1;
            }
            if end == start {
                // `$` solo: literal.
                out.push('$');
                i += 1;
                continue;
            }
            (end, false)
        };
        let name_start = if with_braces { i + 2 } else { i + 1 };
        let name = &s[name_start..name_end];
        if let Ok(val) = std::env::var(name) {
            out.push_str(&val);
        }
        i = name_end + if with_braces { 1 } else { 0 };
    }
    out
}

/// Completion declarada por el usuario para un comando concreto.
/// Esquema mínimo en `<cmd>.toml`:
///
/// ```toml
/// flags = ["--foo", "--bar=", "-x"]
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CommandCompletion {
    #[serde(default)]
    pub flags: Vec<String>,
}

impl CommandCompletion {
    /// Carga `<dir>/<cmd>.toml` si existe, o devuelve `None`. Si el
    /// archivo está roto, también `None` — completions son nice-to-have,
    /// no deben caer el shell.
    pub fn load(dir: &Path, command: &str) -> Option<Self> {
        let path = dir.join(format!("{command}.toml"));
        let text = std::fs::read_to_string(path).ok()?;
        toml::from_str(&text).ok()
    }

    /// Carga *todas* las completions de un directorio en un HashMap.
    /// Útil para precargar al arrancar el shell (un read_dir + N lecturas
    /// pequeñas; barato comparado con el coste de un fork).
    pub fn load_all(dir: &Path) -> HashMap<String, Self> {
        let mut out = HashMap::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return out;
        };
        for e in entries.flatten() {
            let path = e.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Ok(c) = toml::from_str::<CommandCompletion>(&text) {
                    out.insert(stem.to_string(), c);
                }
            }
        }
        out
    }
}

/// Errores al cargar la configuración.
#[derive(Debug)]
pub enum ConfigError {
    Io(PathBuf, std::io::Error),
    Parse(PathBuf, toml::de::Error),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(p, e) => write!(f, "lectura de {}: {}", p.display(), e),
            ConfigError::Parse(p, e) => write!(f, "parseo de {}: {}", p.display(), e),
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn missing_file_yields_default() {
        let d = tempdir().unwrap();
        let c = Config::load(d.path().join("nope.toml")).unwrap();
        assert_eq!(c, Config::default());
    }

    #[test]
    fn parses_a_full_example() {
        let d = tempdir().unwrap();
        let path = d.path().join("shumarc.toml");
        std::fs::write(
            &path,
            r#"
[aliases]
ll = "ls -la"
gs = "git status"

[env]
EDITOR = "hx"

[prompt]
segments = ["cwd", "git", "exit"]

[history]
dedup = "erase_dups"

[capture]
limit_mb = 16
spill = true
"#,
        )
        .unwrap();
        let c = Config::load(&path).unwrap();
        assert_eq!(c.aliases.get("ll").map(|s| s.as_str()), Some("ls -la"));
        assert_eq!(c.env.get("EDITOR").map(|s| s.as_str()), Some("hx"));
        assert_eq!(c.prompt.segments, vec!["cwd", "git", "exit"]);
        assert_eq!(c.history.dedup, DedupPolicy::EraseDups);
        assert_eq!(c.capture.limit_mb, 16);
        assert!(c.capture.spill);
    }

    #[test]
    fn partial_toml_falls_back_to_defaults() {
        // Sólo aliases — el resto debe defaultear, no fallar.
        let d = tempdir().unwrap();
        let path = d.path().join("shumarc.toml");
        std::fs::write(&path, "[aliases]\nll = \"ls -la\"\n").unwrap();
        let c = Config::load(&path).unwrap();
        assert_eq!(c.aliases.len(), 1);
        assert_eq!(c.prompt, PromptConfig::default());
        assert_eq!(c.capture, CaptureConfig::default());
    }

    #[test]
    fn alias_expansion_replaces_first_word_only() {
        let mut c = Config::default();
        c.aliases.insert("ll".into(), "ls -la".into());
        assert_eq!(c.expand_aliases("ll"), "ls -la");
        assert_eq!(c.expand_aliases("ll src/"), "ls -la src/");
        // `ll` en el medio no es un alias.
        assert_eq!(c.expand_aliases("echo ll"), "echo ll");
    }

    #[test]
    fn alias_preserves_leading_whitespace() {
        let mut c = Config::default();
        c.aliases.insert("ll".into(), "ls -la".into());
        // Un comando indentado mantiene su indentación tras expandir.
        assert_eq!(c.expand_aliases("  ll src/"), "  ls -la src/");
    }

    #[test]
    fn alias_does_not_recurse() {
        // No queremos que un alias expandido se vuelva a expandir —
        // evita bucles infinitos triviales (ll=ls, ls=ll).
        let mut c = Config::default();
        c.aliases.insert("a".into(), "b".into());
        c.aliases.insert("b".into(), "c".into());
        assert_eq!(c.expand_aliases("a"), "b");
    }

    #[test]
    fn expand_env_substitutes_vars() {
        // Usamos una var artificial para no colisionar con el entorno real.
        // SAFETY: ver `Config::apply_env`; en tests de un solo hilo es OK.
        std::env::set_var("SHUMA_TEST_VAR", "valor");
        assert_eq!(expand_env("$SHUMA_TEST_VAR"), "valor");
        assert_eq!(expand_env("${SHUMA_TEST_VAR}/bin"), "valor/bin");
        // Variable inexistente → cadena vacía.
        std::env::remove_var("SHUMA_TEST_NOPE");
        assert_eq!(expand_env("x=$SHUMA_TEST_NOPE!"), "x=!");
        // `\$` se escapa.
        assert_eq!(expand_env("precio \\$5"), "precio $5");
    }

    #[test]
    fn expand_env_keeps_dollar_alone() {
        std::env::remove_var("SHUMA_TEST_FOO");
        assert_eq!(expand_env("$ "), "$ ");
        assert_eq!(expand_env("$"), "$");
    }

    #[test]
    fn completion_loads_per_command_file() {
        let d = tempdir().unwrap();
        std::fs::write(
            d.path().join("mytool.toml"),
            "flags = [\"--foo\", \"--bar=\"]\n",
        )
        .unwrap();
        let c = CommandCompletion::load(d.path(), "mytool").unwrap();
        assert_eq!(c.flags, vec!["--foo", "--bar="]);
        // Comando inexistente → None.
        assert!(CommandCompletion::load(d.path(), "nope").is_none());
    }

    #[test]
    fn completion_loads_all_in_dir() {
        let d = tempdir().unwrap();
        std::fs::write(d.path().join("alfa.toml"), "flags = [\"--a\"]\n").unwrap();
        std::fs::write(d.path().join("beta.toml"), "flags = [\"--b\"]\n").unwrap();
        std::fs::write(d.path().join("ignored.txt"), "no soy toml").unwrap();
        let all = CommandCompletion::load_all(d.path());
        assert_eq!(all.len(), 2);
        assert!(all.contains_key("alfa"));
        assert!(all.contains_key("beta"));
        assert!(!all.contains_key("ignored"));
    }

    #[test]
    fn corrupt_completion_file_is_skipped() {
        let d = tempdir().unwrap();
        std::fs::write(d.path().join("bad.toml"), "not = valid = toml").unwrap();
        std::fs::write(d.path().join("good.toml"), "flags = [\"--ok\"]\n").unwrap();
        let all = CommandCompletion::load_all(d.path());
        assert!(all.contains_key("good"));
        assert!(!all.contains_key("bad"));
    }

    #[test]
    fn upsert_key_crea_archivo_y_seccion() {
        let d = tempdir().unwrap();
        let p = d.path().join("rc.toml");
        upsert_key(&p, "env", "EDITOR", &toml_string("hx")).unwrap();
        let c = Config::load(&p).unwrap();
        assert_eq!(c.env.get("EDITOR").map(String::as_str), Some("hx"));
    }

    #[test]
    fn upsert_key_reemplaza_sin_tocar_el_resto() {
        let d = tempdir().unwrap();
        let p = d.path().join("rc.toml");
        std::fs::write(
            &p,
            "# mi rc\n[aliases]\ngs = \"git status\"\n\n[env]\n# comentario\nEDITOR = \"vi\"\nPAGER = \"less\"\n",
        )
        .unwrap();
        upsert_key(&p, "env", "EDITOR", &toml_string("hx")).unwrap();
        let texto = std::fs::read_to_string(&p).unwrap();
        assert!(texto.contains("# mi rc"), "preserva comentarios");
        assert!(texto.contains("# comentario"));
        assert!(texto.contains("gs = \"git status\""));
        let c = Config::load(&p).unwrap();
        assert_eq!(c.env.get("EDITOR").map(String::as_str), Some("hx"));
        assert_eq!(c.env.get("PAGER").map(String::as_str), Some("less"));
    }

    #[test]
    fn upsert_key_agrega_a_seccion_existente() {
        let d = tempdir().unwrap();
        let p = d.path().join("rc.toml");
        std::fs::write(&p, "[env]\nA = \"1\"\n\n[history]\nmax = 10\n").unwrap();
        upsert_key(&p, "env", "B", &toml_string("2")).unwrap();
        let c = Config::load(&p).unwrap();
        assert_eq!(c.env.get("A").map(String::as_str), Some("1"));
        assert_eq!(c.env.get("B").map(String::as_str), Some("2"));
    }

    #[test]
    fn remove_key_borra_y_reporta() {
        let d = tempdir().unwrap();
        let p = d.path().join("rc.toml");
        std::fs::write(&p, "[env]\nA = \"1\"\nB = \"2\"\n").unwrap();
        assert!(remove_key(&p, "env", "A").unwrap());
        assert!(!remove_key(&p, "env", "A").unwrap());
        let c = Config::load(&p).unwrap();
        assert!(c.env.get("A").is_none());
        assert_eq!(c.env.get("B").map(String::as_str), Some("2"));
    }

    #[test]
    fn toml_string_escapa() {
        assert_eq!(toml_string("hola"), "\"hola\"");
        // El formato exacto puede variar (basic vs literal string); lo que
        // importa es que el TOML resultante parsea de vuelta al mismo valor.
        let raw = toml_string("con \"comillas\"");
        let parsed: toml::Value = format!("v = {raw}").parse().unwrap();
        assert_eq!(parsed["v"].as_str(), Some("con \"comillas\""));
    }
}
