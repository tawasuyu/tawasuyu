//! Unified error type.

use cosmos_sky::SkyError;
use thiserror::Error;

pub type AstrologyResult<T> = Result<T, AstrologyError>;

#[derive(Debug, Error)]
pub enum AstrologyError {
    /// An underlying astronomy or time conversion failed.
    #[error("sky-layer error: {0}")]
    Sky(#[from] SkyError),

    /// A house system could not be computed at the given location
    /// (typical: Placidus / Koch inside the polar circle).
    #[error("house system unavailable here: {0}")]
    HouseSystemUnavailable(&'static str),

    /// Something requested a body that the session was not configured
    /// to compute (e.g. an asteroid without an asteroid kernel attached).
    #[error("body could not be computed: {0}")]
    BodyUnavailable(String),
}
