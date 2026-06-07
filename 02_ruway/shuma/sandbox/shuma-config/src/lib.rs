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
}
