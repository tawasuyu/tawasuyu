use rhai::packages::{
    ArithmeticPackage, BasicArrayPackage, BasicIteratorPackage, BasicMapPackage,
    BasicStringPackage, CorePackage, LogicPackage, Package,
};
use rhai::{Dynamic, Engine, Scope, AST};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;

use crate::delta::FieldOp;

#[derive(Debug, Error)]
pub enum RhaiError {
    #[error("compile error: {0}")]
    Compile(String),
    #[error("runtime error: {0}")]
    Runtime(String),
    #[error("morphism returned non-array")]
    BadDelta,
    #[error("delta op malformed: {0}")]
    BadOp(String),
    #[error("io reading script: {0}")]
    Io(#[from] std::io::Error),
}

pub struct RhaiExecutor {
    engine: Engine,
    /// Compiled-AST cache keyed by absolute script path. Avoids reading +
    /// reparsing on every call (verify_log re-runs every morphism in the
    /// log; without the cache that becomes an O(events × parse) blowup).
    asts: RefCell<HashMap<PathBuf, Arc<AST>>>,
}

impl RhaiExecutor {
    /// Build a deterministic engine. Time, random, IO, debug/print are all
    /// excluded by construction (we register packages by name, not the
    /// StandardPackage bundle which would pull in BasicTimePackage).
    pub fn new_sandboxed() -> Self {
        let mut engine = Engine::new_raw();
        // Deliberately omitted: BasicTimePackage, EvalPackage, DebugPackage.
        CorePackage::new().register_into_engine(&mut engine);
        LogicPackage::new().register_into_engine(&mut engine);
        ArithmeticPackage::new().register_into_engine(&mut engine);
        BasicArrayPackage::new().register_into_engine(&mut engine);
        BasicMapPackage::new().register_into_engine(&mut engine);
        BasicStringPackage::new().register_into_engine(&mut engine);
        BasicIteratorPackage::new().register_into_engine(&mut engine);

        engine.set_max_call_levels(64);
        engine.set_max_expr_depths(64, 32);
        Self {
            engine,
            asts: RefCell::new(HashMap::new()),
        }
    }

    pub fn run(&self, script_path: &Path, input: Value) -> Result<Vec<FieldOp>, RhaiError> {
        let ast = self.ast_for(script_path)?;

        let dyn_input: Dynamic = rhai::serde::to_dynamic(input)
            .map_err(|e| RhaiError::Runtime(format!("input -> dynamic: {}", e)))?;
        let mut scope = Scope::new();
        scope.push_dynamic("input", dyn_input);

        let result: Dynamic = self
            .engine
            .eval_ast_with_scope(&mut scope, &ast)
            .map_err(|e| RhaiError::Runtime(e.to_string()))?;

        let arr = result.into_array().map_err(|_| RhaiError::BadDelta)?;

        let mut ops = Vec::with_capacity(arr.len());
        for item in arr {
            let json: Value = rhai::serde::from_dynamic(&item)
                .map_err(|e| RhaiError::BadOp(format!("dynamic -> json: {}", e)))?;
            let op: FieldOp =
                serde_json::from_value(json).map_err(|e| RhaiError::BadOp(e.to_string()))?;
            ops.push(op);
        }
        Ok(ops)
    }

    /// Returns a cached compiled AST for `script_path`, compiling it on the
    /// first call. Cache hits avoid filesystem IO and parse cost entirely.
    fn ast_for(&self, script_path: &Path) -> Result<Arc<AST>, RhaiError> {
        if let Some(ast) = self.asts.borrow().get(script_path) {
            return Ok(Arc::clone(ast));
        }
        let source = std::fs::read_to_string(script_path)?;
        let compiled = self
            .engine
            .compile(&source)
            .map_err(|e| RhaiError::Compile(e.to_string()))?;
        let arc = Arc::new(compiled);
        self.asts
            .borrow_mut()
            .insert(script_path.to_path_buf(), Arc::clone(&arc));
        Ok(arc)
    }
}
