//! Input for a natal-chart computation: a moment in time and a place,
//! plus a small bag of metadata so the chart can carry its provenance.

use cosmos_sky::{Instant, Observer};

/// How confident the astrologer is in the recorded birth time. Carried
/// forward into the chart metadata so rectification work can mark its
/// best-known time without losing the original.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeCertainty {
    /// The birth time is taken at face value with no asserted uncertainty.
    #[default]
    Exact,
    /// The birth time is approximate; `minutes` is the half-width of the
    /// uncertainty interval (e.g. `30` means ±30 minutes).
    Approximate { minutes: u32 },
    /// The birth time has been adjusted by the astrologer via rectification.
    Rectified,
}

/// Birth (or event) data — everything the chart computer needs to know
/// from the *subject's* side, before the astrologer adds chart-style
/// preferences.
#[derive(Debug, Clone)]
pub struct BirthData {
    pub instant: Instant,
    pub observer: Observer,
    pub name: Option<String>,
    pub time_certainty: TimeCertainty,
    pub note: Option<String>,
}

impl BirthData {
    pub fn new(instant: Instant, observer: Observer) -> Self {
        Self {
            instant,
            observer,
            name: None,
            time_certainty: TimeCertainty::Exact,
            note: None,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_time_certainty(mut self, certainty: TimeCertainty) -> Self {
        self.time_certainty = certainty;
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
}
