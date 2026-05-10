use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

use crate::delta::{FieldOp, simulate_on};
use crate::graph::{GraphError, ManifestGraph};
use crate::nickel_validator::{self, NickelError};
use crate::manifest::{ConserveRule, Manifest, ManifestError, MorphismSpec, ValidationError};
use crate::rhai_executor::{RhaiError, RhaiExecutor};
use crate::store::{Store, StoreError};

#[derive(Debug, Error)]
pub enum ExecError {
    #[error("morphism `{0}` not in manifest")]
    UnknownMorphism(String),
    #[error("missing input role `{role}` for morphism `{morphism}`")]
    MissingInput { morphism: String, role: String },
    #[error("duplicate input id {id} bound to roles `{role_a}` and `{role_b}`")]
    DuplicateInputId {
        id: Uuid,
        role_a: String,
        role_b: String,
    },
    #[error("entity `{0}` id `{1}` not found in store")]
    EntityMissing(String, Uuid),
    #[error(
        "capability violation: morphism `{morphism}` produced op on `{token}` not in writes={declared:?}"
    )]
    CapabilityViolation {
        morphism: String,
        token: String,
        declared: Vec<String>,
    },
    #[error(
        "conservation violated: Σ Δ {entity}.{field} where {group_by} = {group:?} = {total} (expected 0)"
    )]
    ConservationViolation {
        entity: String,
        field: String,
        group_by: String,
        group: String,
        total: i128,
    },
    #[error("conservation rule {entity}.{field}: {message}")]
    ConservationMalformed {
        entity: String,
        field: String,
        message: String,
    },
    #[error("schema pre-check failed on `{role}` ({entity}): {source}")]
    SchemaPre {
        role: String,
        entity: String,
        #[source]
        source: NickelError,
    },
    #[error("schema post-check failed on `{role}` ({entity}): {source}")]
    SchemaPost {
        role: String,
        entity: String,
        #[source]
        source: NickelError,
    },
    #[error("schema post-check failed on created {entity} {id}: {source}")]
    SchemaPostCreate {
        entity: String,
        id: Uuid,
        #[source]
        source: NickelError,
    },
    #[error("rhai: {0}")]
    Rhai(#[from] RhaiError),
    #[error("store: {0}")]
    Store(#[from] StoreError),
    #[error("manifest: {0}")]
    Manifest(#[from] ManifestError),
    #[error("manifest validation: {0}")]
    ManifestValidation(#[from] ValidationError),
    #[error("manifest graph: {0}")]
    Graph(#[from] GraphError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub struct Executor {
    pub manifest: Manifest,
    pub graph: ManifestGraph,
    pub module_dir: PathBuf,
    pub schema_path: PathBuf,
    pub rhai: RhaiExecutor,
    /// `true` when `schema_path` is a tempfile bundle created by
    /// `load_module`; Drop removes it. `false` for inline-built executors
    /// that point at a real schema file owned by the caller (tests).
    pub owned_bundle: bool,
    /// Per-morphism `schema_hash`: SHA-256 of (Nickel bundle + manifest
    /// spec + rhai script bytes), computed once at load. The hash es
    /// el determinism contract para evolución de schemas —
    /// `verify_log` lo usa para rechazar logs cuyos entries se
    /// produjeron bajo reglas distintas.
    pub schema_hashes: HashMap<String, [u8; 32]>,
    /// Module-wide bundle hash: SHA-256 de los bytes del bundle Nickel.
    /// Stamped onto every `LogEntry::Seed` via `seed_and_log` so
    /// `verify_log` can flag seeds whose entity schemas have evolved
    /// since they were logged. Coarser than `schema_hashes` (any
    /// schema.k edit moves it, even one that doesn't affect the seeded
    /// entity) but cheap and conservative — false positives over false
    /// negatives, like the morphism hash.
    pub schema_bundle_hash: [u8; 32],
}

impl Drop for Executor {
    fn drop(&mut self) {
        if self.owned_bundle {
            let _ = std::fs::remove_file(&self.schema_path);
        }
    }
}

/// One row of the bound-inputs map. Holds both `role` and `entity` so the
/// capability check can verify a Set's `path.entity` matches the role's
/// declared entity (catches uuid-collision and lazy scripts).
#[derive(Debug, Clone)]
struct InputBinding {
    role: String,
    entity: String,
}

impl Executor {
    pub fn load_module(module_dir: impl Into<PathBuf>) -> Result<Self, ExecError> {
        let module_dir = module_dir.into();
        let manifest = Manifest::load(&module_dir.join("nsmc.json"))?;
        manifest.validate(&module_dir)?;
        let graph = ManifestGraph::build(&manifest)?;
        let schema_path = build_schema_bundle(&module_dir, &manifest.effective_schemas())?;

        let schema_bundle_bytes = std::fs::read(&schema_path)?;
        let schema_bundle_hash = compute_schema_bundle_hash(&schema_bundle_bytes);
        let mut schema_hashes = HashMap::with_capacity(manifest.morphisms.len());
        for spec in &manifest.morphisms {
            let script_path = module_dir.join(&spec.script);
            let hash = compute_morphism_schema_hash(&schema_bundle_bytes, spec, &script_path)?;
            schema_hashes.insert(spec.name.clone(), hash);
        }

        Ok(Self {
            manifest,
            graph,
            module_dir,
            schema_path,
            rhai: RhaiExecutor::new_sandboxed(),
            owned_bundle: true,
            schema_hashes,
            schema_bundle_hash,
        })
    }

    /// Hash for the named morphism in the currently loaded module. `None`
    /// if no such morphism is declared. Used by `verify_log` to enforce
    /// the schema-version contract.
    pub fn schema_hash(&self, morphism: &str) -> Option<[u8; 32]> {
        self.schema_hashes.get(morphism).copied()
    }

    /// Single 32-byte hash representing the entire module's schema:
    /// every morphism's hash, in canonical name order, framed and
    /// chained. Snapshots pin this so a snapshot taken under bundle A
    /// can be detected when later loaded against bundle B.
    pub fn module_schema_hash(&self) -> [u8; 32] {
        let mut entries: Vec<(&String, &[u8; 32])> = self.schema_hashes.iter().collect();
        entries.sort_by_key(|(name, _)| name.as_str().to_owned());
        let mut hasher = Sha256::new();
        hasher.update(b"nakui-module-v1\0");
        for (name, hash) in entries {
            hasher.update((name.len() as u64).to_le_bytes());
            hasher.update(name.as_bytes());
            hasher.update(hash);
        }
        hasher.finalize().into()
    }

    /// Compute the ops for a morphism without mutating the store.
    ///
    /// Pipeline:
    ///   1. Resolve manifest spec; bind caller's role->id to spec inputs.
    ///   2. Reject duplicate ids across roles.
    ///   3. Load every input entity; KCL pre-check each.
    ///   4. Run the Rhai script with `{ states, ids, params }`.
    ///   5. Capability check: every Set targets a tracked id whose entity
    ///      matches the role's declared entity, and produces a `<role>.<field>`
    ///      token in `writes`; Create/Delete produce `<entity>` tokens.
    ///   6. Delta-level invariants (conservation rules).
    ///   7. Per-input KCL post-check (skipped for inputs that the ops Delete).
    ///   8. KCL-validate every Created record against its entity schema.
    ///   9. Pre-apply check: store.apply_dry_run guarantees apply will land.
    ///
    /// On `Ok`, the returned ops are *contractually applicable* — caller can
    /// log first and then apply, knowing apply will succeed barring transient
    /// backend faults.
    pub fn compute<S: Store>(
        &self,
        store: &S,
        morphism_name: &str,
        inputs: &[(&str, Uuid)],
        params: Value,
    ) -> Result<Vec<FieldOp>, ExecError> {
        let spec: &MorphismSpec = self
            .manifest
            .morphism(morphism_name)
            .ok_or_else(|| ExecError::UnknownMorphism(morphism_name.to_string()))?;

        // 1. Bind inputs.
        let inputs_map: BTreeMap<String, Uuid> = inputs
            .iter()
            .map(|(role, id)| (role.to_string(), *id))
            .collect();
        for spec_in in &spec.inputs {
            if !inputs_map.contains_key(&spec_in.role) {
                return Err(ExecError::MissingInput {
                    morphism: morphism_name.to_string(),
                    role: spec_in.role.clone(),
                });
            }
        }

        // 2. Build id -> binding (role + entity), rejecting duplicates.
        let mut id_to_input: HashMap<Uuid, InputBinding> = HashMap::new();
        for spec_in in &spec.inputs {
            let id = inputs_map[&spec_in.role];
            if let Some(other) = id_to_input.get(&id) {
                return Err(ExecError::DuplicateInputId {
                    id,
                    role_a: other.role.clone(),
                    role_b: spec_in.role.clone(),
                });
            }
            id_to_input.insert(
                id,
                InputBinding {
                    role: spec_in.role.clone(),
                    entity: spec_in.entity.clone(),
                },
            );
        }

        // 3. Load + pre-check every input.
        let mut loaded: BTreeMap<String, Value> = BTreeMap::new();
        let mut id_strings: BTreeMap<String, String> = BTreeMap::new();
        for spec_in in &spec.inputs {
            let id = inputs_map[&spec_in.role];
            let state = store
                .load(&spec_in.entity, id)
                .ok_or_else(|| ExecError::EntityMissing(spec_in.entity.clone(), id))?;
            self.validate_entity(&spec_in.entity, &state)
                .map_err(|e| ExecError::SchemaPre {
                    role: spec_in.role.clone(),
                    entity: spec_in.entity.clone(),
                    source: e,
                })?;
            loaded.insert(spec_in.role.clone(), state);
            id_strings.insert(spec_in.role.clone(), id.to_string());
        }

        // 4. Rhai.
        let script_path = self.module_dir.join(&spec.script);
        let input = json!({
            "states": loaded,
            "ids": id_strings,
            "params": params,
        });
        let ops = self.rhai.run(&script_path, input)?;

        // 5. Capability check.
        let declared: HashSet<&str> = spec.writes.iter().map(String::as_str).collect();
        for op in &ops {
            let token = match op {
                // Set y Clear comparten el mismo token shape: ambos
                // mutan un field específico de un record tracked.
                FieldOp::Set { path, .. } | FieldOp::Clear { path } => {
                    match id_to_input.get(&path.id) {
                        Some(binding) if binding.entity == path.entity => {
                            format!("{}.{}", binding.role, path.field)
                        }
                        Some(_) => {
                            return Err(ExecError::CapabilityViolation {
                                morphism: morphism_name.to_string(),
                                token: format!(
                                    "<entity-mismatch>.{}.{}",
                                    path.entity, path.field
                                ),
                                declared: spec.writes.clone(),
                            });
                        }
                        None => {
                            return Err(ExecError::CapabilityViolation {
                                morphism: morphism_name.to_string(),
                                token: format!(
                                    "<untracked id>.{}.{}",
                                    path.entity, path.field
                                ),
                                declared: spec.writes.clone(),
                            });
                        }
                    }
                }
                FieldOp::Create { entity, .. } => entity.clone(),
                FieldOp::Delete { entity, .. } => entity.clone(),
            };
            if !declared.contains(token.as_str()) {
                return Err(ExecError::CapabilityViolation {
                    morphism: morphism_name.to_string(),
                    token,
                    declared: spec.writes.clone(),
                });
            }
        }

        // 6. Conservation invariants.
        for rule in &spec.invariants.conserve {
            check_conservation(rule, &loaded, &id_to_input, &ops)?;
        }

        // 7. Per-input KCL post-check; skip Deleted inputs.
        for spec_in in &spec.inputs {
            let id = inputs_map[&spec_in.role];
            if let Some(new_state) =
                simulate_on(&loaded[&spec_in.role], &spec_in.entity, id, &ops)
            {
                self.validate_entity(&spec_in.entity, &new_state)
                    .map_err(|e| ExecError::SchemaPost {
                        role: spec_in.role.clone(),
                        entity: spec_in.entity.clone(),
                        source: e,
                    })?;
            }
        }

        // 8. Validate every Created record against its entity schema.
        for op in &ops {
            if let FieldOp::Create { entity, id, data } = op {
                self.validate_entity(entity, data)
                    .map_err(|e| ExecError::SchemaPostCreate {
                        entity: entity.clone(),
                        id: *id,
                        source: e,
                    })?;
            }
        }

        // 9. Pre-apply check: structural compatibility with current store state.
        store.apply_dry_run(&ops)?;

        Ok(ops)
    }

    /// compute + apply, for callers that don't need event logging.
    pub fn run<S: Store>(
        &self,
        store: &mut S,
        morphism_name: &str,
        inputs: &[(&str, Uuid)],
        params: Value,
    ) -> Result<Vec<FieldOp>, ExecError> {
        let ops = self.compute(store, morphism_name, inputs, params)?;
        store.apply(&ops)?;
        Ok(ops)
    }

    fn validate_entity(&self, entity: &str, state: &Value) -> Result<(), NickelError> {
        nickel_validator::vet(&self.schema_path, state, entity)
    }
}

/// Module-wide hash of the Nickel bundle bytes. Stamped on
/// `LogEntry::Seed` entries (which don't run through any morphism, so
/// `compute_morphism_schema_hash` doesn't apply). Bumped by any byte
/// change in any schema file the manifest exposes — coarser than a
/// per-entity hash would be, but doesn't require Nickel parsing.
fn compute_schema_bundle_hash(schema_bundle_bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"nakui-bundle-v1\0");
    hasher.update((schema_bundle_bytes.len() as u64).to_le_bytes());
    hasher.update(schema_bundle_bytes);
    hasher.finalize().into()
}

/// Per-morphism schema hash. SHA-256 with length-prefixed framing over
/// three inputs that together determine the morphism's deterministic
/// behaviour: the KCL schema bundle (entity shapes + invariants), the
/// manifest spec (writes, conserve, depends_on, etc.), and a
/// **normalized** form of the Rhai script — comments stripped and
/// whitespace runs collapsed, with string literals preserved exactly.
///
/// The normalization makes the hash invariant to cosmetic edits (a
/// developer adding a `// TODO` doesn't invalidate the log) without
/// missing real behavioural changes. The framing tag is bumped to
/// `nakui-schema-v2` so logs hashed under v1 (raw bytes) cleanly fail
/// SchemaMismatch on upgrade rather than silently divergence.
fn compute_morphism_schema_hash(
    schema_bundle_bytes: &[u8],
    spec: &MorphismSpec,
    script_path: &Path,
) -> std::io::Result<[u8; 32]> {
    let script_bytes = std::fs::read(script_path)?;
    let script_source = std::str::from_utf8(&script_bytes).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("script {} is not valid UTF-8: {}", script_path.display(), e),
        )
    })?;
    let normalized_script = normalize_rhai_source(script_source);
    let spec_json = serde_json::to_vec(spec).expect("MorphismSpec serializes");

    let mut hasher = Sha256::new();
    hasher.update(b"nakui-schema-v2\0");
    hasher.update(b"schema:");
    hasher.update((schema_bundle_bytes.len() as u64).to_le_bytes());
    hasher.update(schema_bundle_bytes);
    hasher.update(b"spec:");
    hasher.update((spec_json.len() as u64).to_le_bytes());
    hasher.update(&spec_json);
    hasher.update(b"script:");
    hasher.update((normalized_script.len() as u64).to_le_bytes());
    hasher.update(normalized_script.as_bytes());
    Ok(hasher.finalize().into())
}

/// Strip line/block comments and collapse whitespace runs in a Rhai
/// source string. Preserves string literals exactly. Used to make the
/// schema hash invariant to cosmetic edits.
///
/// Limitations:
///   - Doesn't handle backtick template literals (Rhai 1.x interp
///     strings). If the modules ever start using them, the normalizer
///     must be extended; until then it's not a concern for the
///     production scripts in `modules/`.
///   - Doesn't handle nested block comments — Rhai itself doesn't
///     either.
pub fn normalize_rhai_source(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    let mut prev_was_space = true; // strip leading whitespace

    while let Some(c) = chars.next() {
        // Line comment: //...\n
        if c == '/' && chars.peek() == Some(&'/') {
            chars.next();
            while let Some(&n) = chars.peek() {
                if n == '\n' {
                    break;
                }
                chars.next();
            }
            continue;
        }
        // Block comment: /* ... */
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut prev = '\0';
            while let Some(n) = chars.next() {
                if prev == '*' && n == '/' {
                    break;
                }
                prev = n;
            }
            continue;
        }
        // String literal: copy verbatim including escape sequences.
        if c == '"' {
            out.push('"');
            while let Some(n) = chars.next() {
                if n == '\\' {
                    out.push('\\');
                    if let Some(esc) = chars.next() {
                        out.push(esc);
                    }
                } else if n == '"' {
                    out.push('"');
                    break;
                } else {
                    out.push(n);
                }
            }
            prev_was_space = false;
            continue;
        }
        // Whitespace run → single space (or nothing if at edge).
        if c.is_whitespace() {
            if !prev_was_space {
                out.push(' ');
                prev_was_space = true;
            }
            continue;
        }
        // Regular character.
        out.push(c);
        prev_was_space = false;
    }

    if out.ends_with(' ') {
        out.pop();
    }
    out
}

/// Construye un bundle Nickel: en lugar de concatenar contenidos
/// (cada `.ncl` es una expresión record completa, no juntable como
/// texto plano), emite un archivo que mergea via `&` los imports.
///
/// El operador `&` de Nickel mergea records: si las keys son
/// distintas (que es lo esperado entre schemas de módulos distintos)
/// el resultado tiene la unión. Si hay colisión, Nickel rebota con
/// un error claro al evaluar — ya cubierto por `manifest::validate`
/// que chequea duplicados antes de llegar acá.
///
/// Verifica que cada path exista para fallar early con I/O error.
/// El path en el `import "..."` queda absoluto (resuelto desde
/// `module_dir`) para que el evaluator lo encuentre desde el
/// tempdir.
fn build_schema_bundle(
    module_dir: &std::path::Path,
    schemas: &[String],
) -> std::io::Result<PathBuf> {
    let mut imports: Vec<String> = Vec::with_capacity(schemas.len());
    for s in schemas {
        let p = module_dir.join(s);
        // Verificar existencia + canonicalize para path absoluto
        // estable (evita que cwd movimiento rompa el bundle).
        let abs = std::fs::canonicalize(&p)?;
        let abs_str = abs.display().to_string();
        let escaped = abs_str.replace('\\', "\\\\").replace('"', "\\\"");
        imports.push(format!("(import \"{escaped}\")"));
    }
    let combined = if imports.is_empty() {
        // Bundle vacío = record vacío. Cualquier validación contra
        // un entity rebota con "field not found" — comportamiento
        // razonable para un módulo sin schemas declarados.
        "{}".to_string()
    } else {
        imports.join(" & ")
    };
    let bundle = std::env::temp_dir().join(format!("nakui_schema_{}.ncl", Uuid::new_v4()));
    std::fs::write(&bundle, combined)?;
    Ok(bundle)
}

fn check_conservation(
    rule: &ConserveRule,
    loaded: &BTreeMap<String, Value>,
    id_to_input: &HashMap<Uuid, InputBinding>,
    ops: &[FieldOp],
) -> Result<(), ExecError> {
    let mut delta_by_group: HashMap<String, i128> = HashMap::new();

    for op in ops {
        if let FieldOp::Set { path, value } = op {
            if path.entity != rule.entity || path.field != rule.field {
                continue;
            }
            let binding = id_to_input
                .get(&path.id)
                .filter(|b| b.entity == path.entity)
                .ok_or_else(|| ExecError::ConservationMalformed {
                    entity: rule.entity.clone(),
                    field: rule.field.clone(),
                    message: format!(
                        "Set on id {} with entity {} cannot be reconciled to a tracked input",
                        path.id, path.entity
                    ),
                })?;
            let old_state = &loaded[&binding.role];
            let old_val =
                old_state
                    .get(&rule.field)
                    .and_then(Value::as_i64)
                    .ok_or_else(|| ExecError::ConservationMalformed {
                        entity: rule.entity.clone(),
                        field: rule.field.clone(),
                        message: format!("old value at role `{}` is not i64", binding.role),
                    })?;
            let new_val =
                value
                    .as_i64()
                    .ok_or_else(|| ExecError::ConservationMalformed {
                        entity: rule.entity.clone(),
                        field: rule.field.clone(),
                        message: format!("Set value at role `{}` is not i64", binding.role),
                    })?;
            let group_key = match &rule.group_by {
                Some(g) => old_state
                    .get(g)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                None => String::new(),
            };
            *delta_by_group.entry(group_key).or_insert(0) +=
                (new_val as i128) - (old_val as i128);
        }
    }

    for (group, total) in &delta_by_group {
        if *total != 0 {
            return Err(ExecError::ConservationViolation {
                entity: rule.entity.clone(),
                field: rule.field.clone(),
                group_by: rule.group_by.clone().unwrap_or_else(|| "(global)".into()),
                group: group.clone(),
                total: *total,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_line_and_block_comments() {
        let src = r#"
// header comment
let x = 1; // trailing
/* block
   spans lines */
let y = 2;
"#;
        let normalized = normalize_rhai_source(src);
        assert_eq!(normalized, "let x = 1; let y = 2;");
    }

    #[test]
    fn normalize_collapses_whitespace_runs() {
        let src = "let     a  =\t\t1;\n\n\n\nlet b   =   2;";
        let normalized = normalize_rhai_source(src);
        assert_eq!(normalized, "let a = 1; let b = 2;");
    }

    #[test]
    fn normalize_preserves_strings_verbatim_including_double_spaces() {
        // The double space, the // inside, and the escape are preserved
        // exactly because they're inside a string literal — semantic
        // content, not cosmetic.
        let src = r#"let s = "hello  // not a comment \"world\"";"#;
        let normalized = normalize_rhai_source(src);
        assert_eq!(normalized, r#"let s = "hello  // not a comment \"world\"";"#);
    }

    #[test]
    fn normalize_is_idempotent() {
        let src = "// a\nlet x  =  1;\n";
        let n1 = normalize_rhai_source(src);
        let n2 = normalize_rhai_source(&n1);
        assert_eq!(n1, n2);
    }

    #[test]
    fn normalize_distinguishes_real_changes() {
        // Adding a new statement is a non-cosmetic change — the
        // normalized output must reflect it.
        let a = "let x = 1;";
        let b = "let x = 1; let y = 2;";
        assert_ne!(normalize_rhai_source(a), normalize_rhai_source(b));

        // Same for changing a literal value.
        let c = "let x = 1;";
        let d = "let x = 2;";
        assert_ne!(normalize_rhai_source(c), normalize_rhai_source(d));
    }

    #[test]
    fn normalize_handles_comment_at_end_without_newline() {
        let src = "let x = 1; // no trailing newline";
        let normalized = normalize_rhai_source(src);
        assert_eq!(normalized, "let x = 1;");
    }

    #[test]
    fn normalize_handles_unterminated_block_comment() {
        // Defensive: if someone writes `/* ...` and forgets to close,
        // we don't infinite-loop or panic. The trailing content is
        // discarded, which is fine — Rhai won't parse this either.
        let src = "let x = 1; /* never ends";
        let normalized = normalize_rhai_source(src);
        assert_eq!(normalized, "let x = 1;");
    }
}
