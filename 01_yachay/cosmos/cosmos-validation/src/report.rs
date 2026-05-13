//! Error metrics and formatting.

use crate::fixture::{Fixture, Tolerance};
use crate::oracle::StateKmS;

#[derive(Debug, Clone, Copy)]
pub struct ErrorReport {
    /// Magnitude of the position-difference vector, km.
    pub pos_err_km: f64,
    /// Magnitude of the velocity-difference vector, km/s.
    pub vel_err_km_s: f64,
    /// Angular separation between observed and reference position vectors,
    /// measured from the centre body, in milli-arcseconds.
    pub angular_sep_mas: f64,
    /// Reference distance, used to contextualise the error.
    pub ref_distance_km: f64,
}

impl ErrorReport {
    pub fn compute(fixture: &Fixture, observed: &StateKmS) -> Self {
        let dx = observed.pos_km[0] - fixture.pos_km[0];
        let dy = observed.pos_km[1] - fixture.pos_km[1];
        let dz = observed.pos_km[2] - fixture.pos_km[2];
        let pos_err_km = libm::sqrt(dx * dx + dy * dy + dz * dz);

        let dvx = observed.vel_km_s[0] - fixture.vel_km_s[0];
        let dvy = observed.vel_km_s[1] - fixture.vel_km_s[1];
        let dvz = observed.vel_km_s[2] - fixture.vel_km_s[2];
        let vel_err_km_s = libm::sqrt(dvx * dvx + dvy * dvy + dvz * dvz);

        let ref_distance_km = libm::sqrt(
            fixture.pos_km[0] * fixture.pos_km[0]
                + fixture.pos_km[1] * fixture.pos_km[1]
                + fixture.pos_km[2] * fixture.pos_km[2],
        );

        // Angular separation ~ atan(|Δr| / |r|) when error is small.
        // Use the cross-product magnitude for stability.
        let cx = fixture.pos_km[1] * observed.pos_km[2] - fixture.pos_km[2] * observed.pos_km[1];
        let cy = fixture.pos_km[2] * observed.pos_km[0] - fixture.pos_km[0] * observed.pos_km[2];
        let cz = fixture.pos_km[0] * observed.pos_km[1] - fixture.pos_km[1] * observed.pos_km[0];
        let cross_mag = libm::sqrt(cx * cx + cy * cy + cz * cz);
        let obs_mag = libm::sqrt(
            observed.pos_km[0] * observed.pos_km[0]
                + observed.pos_km[1] * observed.pos_km[1]
                + observed.pos_km[2] * observed.pos_km[2],
        );
        let dot = fixture.pos_km[0] * observed.pos_km[0]
            + fixture.pos_km[1] * observed.pos_km[1]
            + fixture.pos_km[2] * observed.pos_km[2];
        let denom = ref_distance_km * obs_mag;
        let angle_rad = if denom > 0.0 {
            libm::atan2(cross_mag, dot)
        } else {
            0.0
        };
        // 1 rad = 206_264_806.247 mas.
        let angular_sep_mas = angle_rad * 206_264_806.247;

        Self {
            pos_err_km,
            vel_err_km_s,
            angular_sep_mas,
            ref_distance_km,
        }
    }

    pub fn within(&self, tol: &Tolerance) -> bool {
        self.pos_err_km <= tol.pos_km && self.vel_err_km_s <= tol.vel_km_s
    }
}

/// Formatted table for the precision-report CLI.
pub struct ReportTable<'a> {
    rows: Vec<(&'a Fixture, ErrorReport, bool)>,
}

impl<'a> ReportTable<'a> {
    pub fn new() -> Self {
        Self { rows: Vec::new() }
    }

    pub fn push(&mut self, fixture: &'a Fixture, report: ErrorReport) {
        let pass = report.within(&fixture.tolerance);
        self.rows.push((fixture, report, pass));
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "{:<40}  {:>14}  {:>14}  {:>14}  {:>6}\n",
            "fixture", "pos_err_km", "vel_err_km/s", "ang_sep_mas", "pass"
        ));
        out.push_str(&"-".repeat(94));
        out.push('\n');
        for (fx, rep, pass) in &self.rows {
            out.push_str(&format!(
                "{:<40}  {:>14.3e}  {:>14.3e}  {:>14.3e}  {:>6}\n",
                truncate(&fx.name, 40),
                rep.pos_err_km,
                rep.vel_err_km_s,
                rep.angular_sep_mas,
                if *pass { "OK" } else { "FAIL" },
            ));
        }
        out
    }

    pub fn all_pass(&self) -> bool {
        self.rows.iter().all(|(_, _, p)| *p)
    }

    pub fn rows(&self) -> &[(&'a Fixture, ErrorReport, bool)] {
        &self.rows
    }
}

impl<'a> Default for ReportTable<'a> {
    fn default() -> Self {
        Self::new()
    }
}

fn truncate(s: &str, n: usize) -> &str {
    if s.len() <= n {
        s
    } else {
        &s[..n]
    }
}
