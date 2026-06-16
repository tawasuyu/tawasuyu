//! `sandokan-sysmon-core` — núcleo agnóstico del **modo Sistema** del monitor.
//!
//! Lectura cruda de `/proc` (barrido de procesos + jiffies de CPU/RAM) y envío
//! de señales. Es un `htop` mínimo, sin UI ni Llimphi: cualquier frontend (la
//! UI Llimphi, una CLI) lo consume. Vivía atrapado en `sandokan-monitor-llimphi`
//! (frontend); baja acá por la Regla 2.
//!
//! NO es la observación del plano de control —ésa va por el contrato
//! [`sandokan_core::Engine`] vía `sandokan-monitor-core`—. El modo Sistema
//! observa el SO entero (todos los procesos del kernel, no sólo las unidades
//! que sandokan encarnó): una fuente distinta y sin dueño en el control plane.

pub mod procfs;
