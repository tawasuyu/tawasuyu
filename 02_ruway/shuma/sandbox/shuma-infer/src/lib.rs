//! `shuma-infer` — el motor de inferencia de intenciones secuenciales.
//!
//! El shell observa cómo trabajas. Cuando una *coreografía* de comandos
//! se repite —`cd` a un proyecto, `git pull`, `cargo build`— este motor
//! la detecta, la abstrae (los argumentos que cambian se vuelven
//! variables) y la ofrece como un patrón reutilizable. Automatización
//! que nace de la repetición orgánica, no de escribir scripts.
//!
//! Es agnóstico y determinista: recibe el historial reducido a
//! [`CommandRecord`]s y devuelve [`EmergingPattern`]s. No toca disco, ni
//! la red, ni ningún frontend — el shell se encarga de eso.
//!
//! ```text
//!   historial ──► detect_patterns ──► [EmergingPattern]
//!     · firma de binarios (ventana deslizante)
//!     · sólo ventanas 100% exitosas
//!     · abstracción: args que varían → Varies
//!     · se quedan los patrones maximales
//! ```

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Un comando ejecutado, reducido a lo que importa para inferir.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandRecord {
    /// El binario invocado — la primera palabra de la línea.
    pub binary: String,
    /// Los argumentos, en orden.
    pub args: Vec<String>,
    /// Directorio en que se ejecutó.
    pub cwd: String,
    /// Si terminó con éxito (código 0).
    pub success: bool,
}

impl CommandRecord {
    /// Reduce una línea de comando a un registro. La división es simple
    /// (`split_whitespace`) — suficiente para comparar firmas.
    pub fn parse(line: &str, cwd: impl Into<String>, success: bool) -> Self {
        let mut words = line.split_whitespace().map(str::to_string);
        let binary = words.next().unwrap_or_default();
        Self { binary, args: words.collect(), cwd: cwd.into(), success }
    }
}

/// Ajustes del detector.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct InferConfig {
    /// Largo mínimo de una secuencia para considerarla patrón.
    pub min_len: usize,
    /// Largo máximo de ventana a buscar.
    pub max_len: usize,
    /// Cuántas veces debe repetirse una firma para emerger.
    pub min_occurrences: usize,
}

impl Default for InferConfig {
    fn default() -> Self {
        Self { min_len: 2, max_len: 5, min_occurrences: 2 }
    }
}

/// Los argumentos de un paso del patrón, tras la abstracción.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepArgs {
    /// Los argumentos son idénticos en todas las ocurrencias.
    Fixed(Vec<String>),
    /// Los argumentos cambian entre ocurrencias — son una variable.
    Varies,
}

/// Un paso abstracto del patrón: el binario + sus argumentos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatternStep {
    pub binary: String,
    pub args: StepArgs,
}

impl PatternStep {
    /// Renderiza el paso para mostrarlo — `"git pull"`, `"cd <…>"`.
    pub fn render(&self) -> String {
        match &self.args {
            StepArgs::Fixed(a) if a.is_empty() => self.binary.clone(),
            StepArgs::Fixed(a) => format!("{} {}", self.binary, a.join(" ")),
            StepArgs::Varies => format!("{} <…>", self.binary),
        }
    }
}

/// Un patrón de comandos que emergió del historial.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmergingPattern {
    /// Firma: la secuencia de binarios.
    pub signature: Vec<String>,
    /// Pasos abstractos — para mostrar al usuario.
    pub steps: Vec<PatternStep>,
    /// Las líneas reales de la ocurrencia más reciente — ejecutables.
    pub example: Vec<String>,
    /// Cuántas veces apareció el patrón.
    pub occurrences: usize,
    /// Directorios donde arrancó el patrón, sin repetir.
    pub directories: Vec<String>,
}

impl EmergingPattern {
    /// Puntaje de interés: más largo y más frecuente, más arriba.
    pub fn score(&self) -> usize {
        self.occurrences * self.signature.len()
    }

    /// Nombre sugerido para el patrón — los binarios significativos
    /// (sin el `cd` inicial) unidos por `+`.
    pub fn suggested_name(&self) -> String {
        let significant: Vec<&str> = self
            .signature
            .iter()
            .filter(|b| b.as_str() != "cd")
            .map(String::as_str)
            .collect();
        if significant.is_empty() {
            self.signature.join("+")
        } else {
            significant.join("+")
        }
    }
}

/// `true` si `needle` aparece como sub-secuencia contigua de `haystack`.
fn contains_subslice(haystack: &[String], needle: &[String]) -> bool {
    needle.len() <= haystack.len() && haystack.windows(needle.len()).any(|w| w == needle)
}

/// Construye el patrón abstracto a partir de su firma y las posiciones
/// donde ocurrió.
fn build_pattern(
    history: &[CommandRecord],
    signature: &[String],
    starts: &[usize],
) -> EmergingPattern {
    let len = signature.len();
    let mut steps = Vec::with_capacity(len);
    for i in 0..len {
        // Argumentos de este paso a lo largo de todas las ocurrencias.
        let first = &history[starts[0] + i].args;
        let all_same = starts.iter().all(|&s| &history[s + i].args == first);
        let args = if all_same {
            StepArgs::Fixed(first.clone())
        } else {
            StepArgs::Varies
        };
        steps.push(PatternStep { binary: signature[i].clone(), args });
    }

    // La ocurrencia más reciente da las líneas reales, ejecutables.
    let last = *starts.iter().max().expect("hay ocurrencias");
    let example: Vec<String> = (0..len)
        .map(|i| {
            let c = &history[last + i];
            if c.args.is_empty() {
                c.binary.clone()
            } else {
                format!("{} {}", c.binary, c.args.join(" "))
            }
        })
        .collect();

    let mut directories: Vec<String> = Vec::new();
    for &s in starts {
        let d = &history[s].cwd;
        if !directories.contains(d) {
            directories.push(d.clone());
        }
    }

    EmergingPattern {
        signature: signature.to_vec(),
        steps,
        example,
        occurrences: starts.len(),
        directories,
    }
}

/// Detecta los patrones de comandos repetidos en `history`.
///
/// Sólo cuentan las ventanas cuyos comandos terminaron todos con éxito.
/// Se devuelven los patrones *maximales* (uno contenido en otro más
/// largo no se reporta), ordenados por puntaje descendente.
pub fn detect_patterns(history: &[CommandRecord], cfg: &InferConfig) -> Vec<EmergingPattern> {
    // firma → posiciones de inicio de las ventanas que la producen.
    let mut windows: BTreeMap<Vec<String>, Vec<usize>> = BTreeMap::new();
    for len in cfg.min_len..=cfg.max_len {
        if history.len() < len {
            break;
        }
        for start in 0..=history.len() - len {
            let win = &history[start..start + len];
            if !win.iter().all(|c| c.success) {
                continue; // una ventana con un fallo no es un patrón
            }
            let signature: Vec<String> = win.iter().map(|c| c.binary.clone()).collect();
            windows.entry(signature).or_default().push(start);
        }
    }

    // Firmas que se repiten lo suficiente.
    let qualifying: Vec<(Vec<String>, Vec<usize>)> = windows
        .into_iter()
        .filter(|(_, starts)| starts.len() >= cfg.min_occurrences)
        .collect();

    // Sólo las maximales: una firma contenida en otra más larga que
    // también califica se descarta (la larga la subsume).
    let mut patterns: Vec<EmergingPattern> = qualifying
        .iter()
        .filter(|(sig, _)| {
            !qualifying
                .iter()
                .any(|(other, _)| other.len() > sig.len() && contains_subslice(other, sig))
        })
        .map(|(sig, starts)| build_pattern(history, sig, starts))
        .collect();

    patterns.sort_by(|a, b| {
        b.score()
            .cmp(&a.score())
            .then(a.signature.cmp(&b.signature))
    });
    patterns
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Atajo: un `CommandRecord` exitoso.
    fn ok(line: &str, cwd: &str) -> CommandRecord {
        CommandRecord::parse(line, cwd, true)
    }

    #[test]
    fn parse_splits_binary_and_args() {
        let r = CommandRecord::parse("git commit -m mensaje", "/p", true);
        assert_eq!(r.binary, "git");
        assert_eq!(r.args, vec!["commit", "-m", "mensaje"]);
    }

    #[test]
    fn detects_a_repeated_sequence() {
        // cd → git pull → cargo build, dos veces, en dos directorios.
        let history = vec![
            ok("cd /proj/a", "/home"),
            ok("git pull", "/proj/a"),
            ok("cargo build", "/proj/a"),
            ok("cd /proj/b", "/home"),
            ok("git pull", "/proj/b"),
            ok("cargo build", "/proj/b"),
        ];
        let patterns = detect_patterns(&history, &InferConfig::default());
        assert_eq!(patterns.len(), 1);
        let p = &patterns[0];
        assert_eq!(p.signature, vec!["cd", "git", "cargo"]);
        assert_eq!(p.occurrences, 2);
    }

    #[test]
    fn abstracts_varying_arguments() {
        let history = vec![
            ok("cd /proj/a", "/home"),
            ok("git pull", "/proj/a"),
            ok("cd /proj/b", "/home"),
            ok("git pull", "/proj/b"),
        ];
        let patterns = detect_patterns(&history, &InferConfig::default());
        let p = &patterns[0];
        // El `cd` cambia de argumento → Varies; `git pull` es constante.
        assert_eq!(p.steps[0].args, StepArgs::Varies);
        assert_eq!(p.steps[1].args, StepArgs::Fixed(vec!["pull".into()]));
        assert_eq!(p.steps[0].render(), "cd <…>");
        assert_eq!(p.steps[1].render(), "git pull");
    }

    #[test]
    fn example_is_the_most_recent_occurrence() {
        let history = vec![
            ok("cd /proj/a", "/home"),
            ok("git pull", "/proj/a"),
            ok("cd /proj/b", "/home"),
            ok("git pull", "/proj/b"),
        ];
        let p = &detect_patterns(&history, &InferConfig::default())[0];
        // Las líneas reales y ejecutables de la última ocurrencia.
        assert_eq!(p.example, vec!["cd /proj/b", "git pull"]);
    }

    #[test]
    fn a_failed_command_breaks_the_pattern() {
        let history = vec![
            ok("cd /proj/a", "/home"),
            ok("git pull", "/proj/a"),
            ok("cd /proj/b", "/home"),
            CommandRecord::parse("git pull", "/proj/b", false), // falló
        ];
        // Sólo una ventana [cd, git] exitosa → no se repite → sin patrón.
        assert!(detect_patterns(&history, &InferConfig::default()).is_empty());
    }

    #[test]
    fn no_repetition_yields_no_patterns() {
        let history = vec![
            ok("ls", "/a"),
            ok("pwd", "/a"),
            ok("date", "/a"),
        ];
        assert!(detect_patterns(&history, &InferConfig::default()).is_empty());
    }

    #[test]
    fn longer_pattern_subsumes_its_subsequences() {
        // [cd, git, cargo] repetido → no se reporta también [cd, git].
        let history = vec![
            ok("cd /a", "/h"),
            ok("git pull", "/a"),
            ok("cargo build", "/a"),
            ok("cd /b", "/h"),
            ok("git pull", "/b"),
            ok("cargo build", "/b"),
        ];
        let patterns = detect_patterns(&history, &InferConfig::default());
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].signature.len(), 3);
    }

    #[test]
    fn directories_are_collected() {
        let history = vec![
            ok("cd /a", "/home"),
            ok("git pull", "/a"),
            ok("cd /b", "/work"),
            ok("git pull", "/b"),
        ];
        let p = &detect_patterns(&history, &InferConfig::default())[0];
        assert_eq!(p.directories, vec!["/home", "/work"]);
    }

    #[test]
    fn suggested_name_drops_the_cd() {
        let history = vec![
            ok("cd /a", "/h"),
            ok("git pull", "/a"),
            ok("cargo build", "/a"),
            ok("cd /b", "/h"),
            ok("git pull", "/b"),
            ok("cargo build", "/b"),
        ];
        let p = &detect_patterns(&history, &InferConfig::default())[0];
        assert_eq!(p.suggested_name(), "git+cargo");
    }

    #[test]
    fn score_ranks_longer_and_more_frequent_higher() {
        let short = EmergingPattern {
            signature: vec!["a".into(), "b".into()],
            steps: vec![],
            example: vec![],
            occurrences: 2,
            directories: vec![],
        };
        let long = EmergingPattern {
            signature: vec!["a".into(), "b".into(), "c".into()],
            steps: vec![],
            example: vec![],
            occurrences: 3,
            directories: vec![],
        };
        assert!(long.score() > short.score());
    }
}
