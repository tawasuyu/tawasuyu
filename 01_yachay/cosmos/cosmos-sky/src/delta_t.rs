//! ΔT (= TT − UT1) lookup.
//!
//! Thin re-export of the table-driven implementation that already lives
//! in `eternal-validation`. Kept as its own module so the façade has a
//! single canonical place to ask the question "what is ΔT at this JD?"
//! without callers learning the internal layout.
//!
//! Modern era (1968–2030): linear interpolation of the IERS observed
//! ΔT table at sub-yearly nodes (sub-second accuracy). Outside that
//! range, Espenak/Meeus polynomial fits take over.

/// ΔT in seconds at the given Julian Date (TDB or TT — they differ by at
/// most ~2 ms which is well below the precision of the underlying table).
pub fn delta_t_seconds(jd: f64) -> f64 {
    cosmos_validation::delta_t::delta_t_seconds(jd)
}
