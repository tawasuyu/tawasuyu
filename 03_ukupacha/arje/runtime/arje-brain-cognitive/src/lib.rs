//! arje-brain-cognitive — capa estadística del brain.
//!
//! `Observer` con sliding window + marginales + co-ocurrencias + Shannon
//! entropy + información mutua. `crystallize` detecta patrones
//! estadísticamente significativos y los materializa como `Rule`.

pub mod observer;
pub mod crystallize;

pub use observer::Observer;
pub use crystallize::{detect_crystals, Crystal, CrystallizationParams};
