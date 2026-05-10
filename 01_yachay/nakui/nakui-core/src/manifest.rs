use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub module: String,
    /// Schema files that compose this module's KCL surface. Paths are
    /// resolved relative to the module directory; cross-module references
    /// use `"../other_module/schema.k"`. Defaults to `["schema.k"]` when
    /// the field is absent — the single-file case.
    #[serde(default)]
    pub schemas: Vec<String>,
    pub morphisms: Vec<MorphismSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MorphismSpec {
    pub name: String,
    pub inputs: Vec<MorphismInput>,
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    #[serde(default)]
    pub invariants: Invariants,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub script: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MorphismInput {
    pub role: String,
    pub entity: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Invariants {
    /// Sum-conservation rules. The total Δ of (entity, field) across the ops
    /// produced by the morphism must be zero — optionally bucketed by another
    /// field on the entity (e.g. group_by="currency" so USD and EUR are
    /// independent ledgers).
    #[serde(default)]
    pub conserve: Vec<ConserveRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConserveRule {
    pub entity: String,
    pub field: String,
    #[serde(default)]
    pub group_by: Option<String>,
}

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("io reading manifest: {0}")]
    Io(#[from] std::io::Error),
    #[error("parsing manifest json: {0}")]
    Parse(#[from] serde_json::Error),
}

/// Errors raised by `Manifest::validate`. Each variant flags a specific
/// semantic issue caught before the kernel ever runs the module — these
/// are the contract between manifest authors (humans or AI) and Nakui.
#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("morphism name `{0}` declared more than once")]
    DuplicateMorphism(String),
    #[error("morphism `{morphism}`: input role `{role}` declared more than once")]
    DuplicateRole { morphism: String, role: String },
    #[error(
        "morphism `{morphism}`: input entity `{entity}` is not declared in any schema file (known: {known:?})"
    )]
    InputUnknownEntity {
        morphism: String,
        entity: String,
        known: Vec<String>,
    },
    #[error(
        "morphism `{morphism}`: writes token `{token}` references unknown role `{role}` (declared roles: {roles:?})"
    )]
    WritesUnknownRole {
        morphism: String,
        token: String,
        role: String,
        roles: Vec<String>,
    },
    #[error(
        "morphism `{morphism}`: writes token `{token}` is not a declared role.field nor a known entity name"
    )]
    WritesUnknownEntity { morphism: String, token: String },
    #[error("morphism `{morphism}`: conserve rule references unknown entity `{entity}`")]
    ConserveUnknownEntity { morphism: String, entity: String },
    #[error("morphism `{morphism}`: depends_on `{dep}` does not name a morphism in this manifest")]
    DependsOnUnknown { morphism: String, dep: String },
    #[error("morphism `{morphism}`: script file `{script}` not found at {resolved}")]
    ScriptMissing {
        morphism: String,
        script: String,
        resolved: String,
    },
    #[error("schema file `{path}` declared in manifest does not exist at {resolved}")]
    SchemaFileMissing { path: String, resolved: String },
    #[error("schema name `{name}` is declared in multiple files: {files:?}")]
    DuplicateSchema { name: String, files: Vec<String> },
    #[error("io reading schema `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self, ManifestError> {
        let text = std::fs::read_to_string(path)?;
        let m: Self = serde_json::from_str(&text)?;
        Ok(m)
    }

    pub fn morphism(&self, name: &str) -> Option<&MorphismSpec> {
        self.morphisms.iter().find(|m| m.name == name)
    }

    /// Schema files this module exposes. Defaults to `["schema.ncl"]`
    /// when the manifest doesn't declare any explicitly. Acepta
    /// también legacy `.k` para no romper módulos no-migrados.
    pub fn effective_schemas(&self) -> Vec<String> {
        if self.schemas.is_empty() {
            vec!["schema.ncl".to_string()]
        } else {
            self.schemas.clone()
        }
    }

    /// Run all semantic checks. Catches author errors that would otherwise
    /// surface as opaque runtime failures — misspelled entity names that
    /// silently make conservation a no-op, role typos in writes that allow
    /// any op through, unresolvable script paths, etc.
    pub fn validate(&self, module_dir: &Path) -> Result<(), ValidationError> {
        // 1. Resolve schemas: read each file, parse schema names, detect
        //    cross-file duplicates. Build the set of known entity names.
        let mut entity_to_files: HashMap<String, Vec<String>> = HashMap::new();
        for s in self.effective_schemas() {
            let resolved = module_dir.join(&s);
            if !resolved.exists() {
                return Err(ValidationError::SchemaFileMissing {
                    path: s.clone(),
                    resolved: resolved.display().to_string(),
                });
            }
            let content = std::fs::read_to_string(&resolved).map_err(|e| {
                ValidationError::Io {
                    path: s.clone(),
                    source: e,
                }
            })?;
            for name in extract_schema_names(&content) {
                entity_to_files.entry(name).or_default().push(s.clone());
            }
        }
        for (name, files) in &entity_to_files {
            if files.len() > 1 {
                return Err(ValidationError::DuplicateSchema {
                    name: name.clone(),
                    files: files.clone(),
                });
            }
        }
        let known_entities: HashSet<&str> =
            entity_to_files.keys().map(String::as_str).collect();

        // 2. Manifest-level: morphism names must be unique.
        let mut seen: HashSet<&str> = HashSet::new();
        for m in &self.morphisms {
            if !seen.insert(m.name.as_str()) {
                return Err(ValidationError::DuplicateMorphism(m.name.clone()));
            }
        }
        let known_morphisms: HashSet<&str> =
            self.morphisms.iter().map(|m| m.name.as_str()).collect();

        // 3. Per-morphism checks.
        for m in &self.morphisms {
            let mut roles: HashSet<&str> = HashSet::new();
            for inp in &m.inputs {
                if !roles.insert(inp.role.as_str()) {
                    return Err(ValidationError::DuplicateRole {
                        morphism: m.name.clone(),
                        role: inp.role.clone(),
                    });
                }
                if !known_entities.contains(inp.entity.as_str()) {
                    return Err(ValidationError::InputUnknownEntity {
                        morphism: m.name.clone(),
                        entity: inp.entity.clone(),
                        known: sorted(&known_entities),
                    });
                }
            }

            for token in &m.writes {
                if let Some((role, _field)) = token.split_once('.') {
                    if !roles.contains(role) {
                        return Err(ValidationError::WritesUnknownRole {
                            morphism: m.name.clone(),
                            token: token.clone(),
                            role: role.to_string(),
                            roles: m.inputs.iter().map(|i| i.role.clone()).collect(),
                        });
                    }
                } else if !known_entities.contains(token.as_str()) {
                    return Err(ValidationError::WritesUnknownEntity {
                        morphism: m.name.clone(),
                        token: token.clone(),
                    });
                }
            }

            for rule in &m.invariants.conserve {
                if !known_entities.contains(rule.entity.as_str()) {
                    return Err(ValidationError::ConserveUnknownEntity {
                        morphism: m.name.clone(),
                        entity: rule.entity.clone(),
                    });
                }
            }

            for dep in &m.depends_on {
                if !known_morphisms.contains(dep.as_str()) {
                    return Err(ValidationError::DependsOnUnknown {
                        morphism: m.name.clone(),
                        dep: dep.clone(),
                    });
                }
            }

            let script_resolved = module_dir.join(&m.script);
            if !script_resolved.exists() {
                return Err(ValidationError::ScriptMissing {
                    morphism: m.name.clone(),
                    script: m.script.clone(),
                    resolved: script_resolved.display().to_string(),
                });
            }
        }

        Ok(())
    }
}

/// Cheap line-scan over a `.k` file to extract every `schema NAME` declared
/// at column 0 (top-level). Tolerates inheritance (`schema X(Y):`) and
/// generic params (`schema X[T]:`); ignores comments and string literals
/// because top-level KCL syntax doesn't admit them ambiguously.
/// Extrae los nombres de entities declarados en un schema Nickel.
///
/// Convención de los `schema.ncl` de Nakui: el archivo evalúa a un
/// record top-level cuyas keys son los entity names (CapitalCase).
/// Las helpers locales (`let positive_int = ...`) no matchean
/// porque no están indentadas con 2 spaces ni empiezan con
/// mayúscula.
///
/// Heurística (no parser completo): líneas con exactamente 2 spaces
/// de indent + identifier CapitalCase + `=`. Robusto para los
/// schemas actuales; si futuras convenciones requieren otro
/// indent, flexibilizar acá.
fn extract_schema_names(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim_start_matches(' ');
        let leading_spaces = line.len() - trimmed.len();
        if leading_spaces != 2 {
            continue;
        }
        let first = match trimmed.chars().next() {
            Some(c) => c,
            None => continue,
        };
        if !first.is_ascii_uppercase() {
            continue;
        }
        let name: String = trimmed
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() {
            continue;
        }
        // Después del identifier debe venir `=` (eventualmente
        // tras whitespace).
        let after = &trimmed[name.len()..];
        if !after.trim_start().starts_with('=') {
            continue;
        }
        out.push(name);
    }
    out
}

fn sorted(set: &HashSet<&str>) -> Vec<String> {
    let mut v: Vec<String> = set.iter().map(|s| s.to_string()).collect();
    v.sort();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_schema_names_handles_nickel_record_top_level() {
        let content = r#"
let positive_int = std.contract.from_predicate (fun n => n > 0) in
let currency_iso = std.contract.from_predicate (fun s => true) in

{
  Caja = {
    id | String,
    saldo | positive_int,
  },

  Movimiento = {
    id | String,
    monto | positive_int,
  } | std.contract.from_predicate (fun r => true),

  Transferencia = {
    id | String,
  },
}
"#;
        let names = extract_schema_names(content);
        assert_eq!(names, vec!["Caja", "Movimiento", "Transferencia"]);
    }

    #[test]
    fn extract_schema_names_skips_let_bindings_and_lowercase() {
        // `let x = ...` no debe aparecer; tampoco lowercase keys
        // (no son entities por convención).
        let content = r#"
let positive_int = ... in
{
  Caja = { id | String },
  helper = "not an entity",
}
"#;
        let names = extract_schema_names(content);
        assert_eq!(names, vec!["Caja"]);
    }
}
