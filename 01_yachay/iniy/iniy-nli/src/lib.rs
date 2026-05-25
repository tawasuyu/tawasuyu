//! iniy-nli — Natural Language Inference entre pares de aserciones.
//!
//! Dado (premisa, hipótesis) devuelve probabilidades de entailment / contradiction / neutral.
//! Backend MVP: trait abstracto. Backend real previsto: ONNX-RT + DeBERTa-v3-MNLI.

use anyhow::Result;
use async_trait::async_trait;
use iniy_core::{Asercion, RelacionNli};

#[async_trait]
pub trait MotorNli: Send + Sync {
    async fn evaluar(&self, premisa: &Asercion, hipotesis: &Asercion) -> Result<RelacionNli>;
}

/// Motor neutro: devuelve siempre "neutral 1.0". Útil para cablear el pipeline.
pub struct MotorNeutro;

#[async_trait]
impl MotorNli for MotorNeutro {
    async fn evaluar(&self, _p: &Asercion, _h: &Asercion) -> Result<RelacionNli> {
        Ok(RelacionNli { entailment: 0.0, contradiction: 0.0, neutral: 1.0 })
    }
}
