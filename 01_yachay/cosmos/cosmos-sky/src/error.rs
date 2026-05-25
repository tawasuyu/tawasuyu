//! Unified error type for the `eternal-sky` façade. Internally wraps the
//! lower-level error types from the time and ephemeris crates so callers
//! only need a single `Result` alias.

use cosmos_time::TimeError;
use cosmos_validation::oracle::OracleError;
use thiserror::Error;

pub type SkyResult<T> = Result<T, SkyError>;

#[derive(Debug, Error)]
pub enum SkyError {
    /// Calendar input was out of range or otherwise unrepresentable.
    #[error("invalid civil time: {0}")]
    InvalidCivilTime(String),

    /// An ISO 8601 string could not be parsed.
    #[error("invalid ISO 8601 timestamp: {0}")]
    InvalidIso8601(String),

    /// Time-scale conversion failed inside `eternal-time`.
    #[error("time conversion failed: {0}")]
    Time(#[from] TimeError),

    /// Ephemeris backend (SPK kernel or analytical theory) returned an error.
    #[error("ephemeris backend failed: {0}")]
    Ephemeris(#[from] OracleError),

    /// The requested body is not supported by the current backend
    /// (e.g. asking for Chiron without an asteroid kernel that contains it).
    #[error("body {body:?} is not supported by the active backend: {reason}")]
    UnsupportedBody { body: crate::body::Body, reason: &'static str },

    /// An observer-dependent quantity was requested without supplying an
    /// observer (e.g. local sidereal time without a location).
    #[error("this operation requires an Observer but none was supplied")]
    ObserverRequired,

    /// An operation that requires a JPL SPK planetary kernel was
    /// invoked on a session opened with the analytical VSOP2013
    /// backend (or any other non-SPK source).
    #[error("this operation requires an SPK planetary kernel; session was opened without one")]
    SpkRequired,
}
