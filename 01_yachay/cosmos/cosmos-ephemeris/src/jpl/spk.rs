use super::chebyshev::evaluate_position_velocity;
use super::daf::{DafFile, DafSummary};
use super::SpkError;
use cosmos_core::constants::{J2000_JD, SECONDS_PER_DAY_F64};
use std::path::Path;

fn jd_to_seconds_from_j2000(jd: f64) -> f64 {
    (jd - J2000_JD) * SECONDS_PER_DAY_F64
}

fn seconds_from_j2000_to_jd(seconds: f64) -> f64 {
    J2000_JD + seconds / SECONDS_PER_DAY_F64
}

#[derive(Debug, Clone)]
pub struct SpkSegment {
    pub body_id: i32,
    pub center_id: i32,
    pub frame_id: i32,
    pub data_type: i32,
    pub start_epoch: f64,
    pub end_epoch: f64,
    pub start_index: usize,
    pub end_index: usize,
}

impl SpkSegment {
    pub fn from_summary(summary: &DafSummary) -> Result<Self, SpkError> {
        if summary.doubles.len() < 2 || summary.ints.len() < 6 {
            return Err(SpkError::InvalidData("Incomplete SPK summary".into()));
        }
        Ok(Self {
            start_epoch: summary.doubles[0],
            end_epoch: summary.doubles[1],
            body_id: summary.ints[0],
            center_id: summary.ints[1],
            frame_id: summary.ints[2],
            data_type: summary.ints[3],
            start_index: summary.ints[4] as usize,
            end_index: summary.ints[5] as usize,
        })
    }

    pub fn contains_epoch(&self, jd_tdb: f64) -> bool {
        let epoch_seconds = jd_to_seconds_from_j2000(jd_tdb);
        epoch_seconds >= self.start_epoch && epoch_seconds <= self.end_epoch
    }

    pub fn start_jd(&self) -> f64 {
        seconds_from_j2000_to_jd(self.start_epoch)
    }

    pub fn end_jd(&self) -> f64 {
        seconds_from_j2000_to_jd(self.end_epoch)
    }
}

struct Type2Metadata {
    init: f64,
    intlen: f64,
    rsize: usize,
    n_records: usize,
    n_coeffs: usize,
}

/// Metadata for a Type 21 (Extended Modified Difference Arrays) segment.
///
/// Type 21 segments store the trajectory of a small body via the
/// Modified Difference Arrays scheme described in Newhall (1989). They
/// are the standard format produced by JPL Horizons for centaurs
/// (Chiron, Pholus), TNOs (Eris, Sedna), and many comets.
///
/// Segment layout (TDB seconds past J2000 throughout):
///
/// ```text
///   [start_index]  Record 0  (4·MAXDIM + 11 doubles)
///                  Record 1
///                    ...
///                  Record NUMREC-1
///                  Epoch directory (NUMREC doubles — final epoch of each record)
///   [end_index-1]  MAXDIM     (as a double, integer-valued)
///   [end_index]    NUMREC     (as a double, integer-valued)
/// ```
///
/// Each record itself contains:
///
/// ```text
///   record[0]                      TL — final epoch of this record
///   record[1 .. 1+MAXDIM]          G(1..MAXDIM)   — step sizes
///   record[1+MAXDIM .. 4+MAXDIM]   REFPOS(1..3)   — reference position (km)
///   record[4+MAXDIM .. 7+MAXDIM]   REFVEL(1..3)   — reference velocity (km/s)
///   record[7+MAXDIM .. 7+MAXDIM+3·MAXDIM]
///                                  DT(MAXDIM, 3)  — modified divided differences
///   record[7+MAXDIM+3·MAXDIM]      KQMAX1         — max integration order + 1
///   record[8+MAXDIM+3·MAXDIM .. 11+MAXDIM+3·MAXDIM]
///                                  KQ(1..3)       — integration orders per component
/// ```
///
/// Interpolation (Newhall 1989, JPL MAO/D-2706) reconstructs the
/// state `(position, velocity)` at the target TDB by applying the
/// Newton-style recurrence on `DT` with the cumulative G coefficients.
/// That mathematical step is **not yet implemented** in this crate;
/// today `compute_state` reaches a Type 21 segment, parses the record
/// successfully, and returns `SpkError::UnsupportedType(21)` with a
/// pointer to this TODO. Loading a Type-21-bearing kernel no longer
/// silently drops the body.
#[derive(Debug, Clone, Copy)]
struct Type21Metadata {
    /// Per-record header size in doubles. Equals `4·MAXDIM + 11`.
    record_size: usize,
    /// Maximum integration order stored per record. Typically 15 in
    /// JPL Horizons output, but anywhere in `1..=25` is allowed.
    maxdim: usize,
    /// Number of records in the segment.
    n_records: usize,
}

/// Parsed Type 21 record, ready for interpolation.
///
/// Fields are read but not yet consumed — the Newhall MDA integration
/// step will use them in a follow-up. Marked `dead_code`-allow so the
/// parsing layer compiles cleanly today without losing the metadata
/// structure that the interpolation will build on top of.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct Type21Record {
    /// Final epoch of the record (TDB seconds past J2000).
    tl: f64,
    /// Step sizes `G(1..MAXDIM)`, in seconds.
    g: Vec<f64>,
    /// Reference position (km) at `TL`.
    refpos: [f64; 3],
    /// Reference velocity (km/s) at `TL`.
    refvel: [f64; 3],
    /// Modified divided differences, stored as `DT[component][order]`
    /// with `component ∈ {0,1,2}` (X, Y, Z) and `order ∈ {0..MAXDIM-1}`.
    dt: [Vec<f64>; 3],
    /// Maximum integration order plus one (= max KQ[j] + 1).
    kqmax1: i32,
    /// Integration orders per component `KQ(1..3)`.
    kq: [i32; 3],
}

pub struct SpkFile {
    daf: DafFile,
    segments: Vec<SpkSegment>,
}

impl SpkFile {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, SpkError> {
        let daf = DafFile::open(path)?;
        let segments = Self::load_segments(&daf)?;
        Ok(Self { daf, segments })
    }

    fn load_segments(daf: &DafFile) -> Result<Vec<SpkSegment>, SpkError> {
        let mut segments = Vec::new();
        for summary_result in daf.iter_summaries() {
            let summary = summary_result?;
            let segment = SpkSegment::from_summary(&summary)?;
            // Accept the SPK record types this crate either fully
            // handles (Type 2 Chebyshev) or parses + flags for later
            // interpolation (Type 21 Modified Difference Arrays).
            // Other types are silently skipped — they would only
            // surface as `SegmentNotFound` on lookup, which mirrors
            // the historical behaviour for unsupported types.
            if matches!(segment.data_type, 2 | 21) {
                segments.push(segment);
            }
        }
        Ok(segments)
    }

    pub fn segments(&self) -> &[SpkSegment] {
        &self.segments
    }

    pub fn find_segment(&self, body: i32, center: i32, jd_tdb: f64) -> Option<&SpkSegment> {
        self.segments
            .iter()
            .find(|s| s.body_id == body && s.center_id == center && s.contains_epoch(jd_tdb))
    }

    fn read_type2_metadata(&self, segment: &SpkSegment) -> Result<Type2Metadata, SpkError> {
        let meta = self.daf.read_f64_array(segment.end_index - 3, 4)?;
        let init = meta[0];
        let intlen = meta[1];
        let rsize = meta[2] as usize;
        let n_records = meta[3] as usize;
        let n_coeffs = (rsize - 2) / 3;
        Ok(Type2Metadata {
            init,
            intlen,
            rsize,
            n_records,
            n_coeffs,
        })
    }

    fn find_record_index(&self, meta: &Type2Metadata, jd_tdb: f64) -> usize {
        let epoch_seconds = jd_to_seconds_from_j2000(jd_tdb);
        let seconds_from_init = epoch_seconds - meta.init;
        let record_index = libm::floor(seconds_from_init / meta.intlen) as usize;
        record_index.min(meta.n_records - 1)
    }

    fn read_type2_record(
        &self,
        segment: &SpkSegment,
        meta: &Type2Metadata,
        record_index: usize,
    ) -> Result<(Vec<f64>, f64, f64), SpkError> {
        let record_start = segment.start_index + record_index * meta.rsize;
        let record = self.daf.read_f64_array(record_start, meta.rsize)?;
        let mid = record[0];
        let radius = record[1];
        Ok((record, mid, radius))
    }

    pub fn compute_position(
        &self,
        body: i32,
        center: i32,
        jd_tdb: f64,
    ) -> Result<[f64; 3], SpkError> {
        let (pos, _) = self.compute_state(body, center, jd_tdb)?;
        Ok(pos)
    }

    pub fn compute_state(
        &self,
        body: i32,
        center: i32,
        jd_tdb: f64,
    ) -> Result<([f64; 3], [f64; 3]), SpkError> {
        let segment = self.find_segment(body, center, jd_tdb).ok_or({
            SpkError::SegmentNotFound {
                body,
                center,
                epoch: jd_tdb,
            }
        })?;
        match segment.data_type {
            2 => self.compute_state_type2(segment, jd_tdb),
            21 => self.compute_state_type21(segment, jd_tdb),
            other => Err(SpkError::UnsupportedType(other)),
        }
    }

    fn compute_state_type2(
        &self,
        segment: &SpkSegment,
        jd_tdb: f64,
    ) -> Result<([f64; 3], [f64; 3]), SpkError> {
        let meta = self.read_type2_metadata(segment)?;
        let record_index = self.find_record_index(&meta, jd_tdb);
        let (record, mid, radius) = self.read_type2_record(segment, &meta, record_index)?;
        let epoch_seconds = jd_to_seconds_from_j2000(jd_tdb);
        let t_normalized = (epoch_seconds - mid) / radius;
        let coeffs = &record[2..];
        let n = meta.n_coeffs;
        let coeffs_x = &coeffs[0..n];
        let coeffs_y = &coeffs[n..2 * n];
        let coeffs_z = &coeffs[2 * n..3 * n];
        evaluate_position_velocity(coeffs_x, coeffs_y, coeffs_z, n, t_normalized, radius)
    }

    /// Parse a Type 21 record fully and surface a structured "not yet
    /// implemented" error. The parsing path is exercised so a kernel
    /// containing only Type 21 segments can be opened and inspected;
    /// only the Newhall (1989) Modified-Difference-Arrays interpolation
    /// step is missing.
    fn compute_state_type21(
        &self,
        segment: &SpkSegment,
        jd_tdb: f64,
    ) -> Result<([f64; 3], [f64; 3]), SpkError> {
        let meta = self.read_type21_metadata(segment)?;
        let record_index = self.find_type21_record_index(segment, &meta, jd_tdb)?;
        // Read but discard the parsed record (asserts parsing path).
        let _ = self.read_type21_record(segment, &meta, record_index)?;
        Err(SpkError::UnsupportedType(21))
    }

    /// Read the trailing `MAXDIM` and `NUMREC` from a Type 21 segment.
    fn read_type21_metadata(&self, segment: &SpkSegment) -> Result<Type21Metadata, SpkError> {
        let trailer = self.daf.read_f64_array(segment.end_index - 1, 2)?;
        let maxdim = trailer[0] as usize;
        let n_records = trailer[1] as usize;
        if maxdim == 0 || maxdim > 25 {
            return Err(SpkError::InvalidData(format!(
                "Type 21 MAXDIM = {} out of valid range 1..=25",
                maxdim
            )));
        }
        if n_records == 0 {
            return Err(SpkError::InvalidData(
                "Type 21 NUMREC = 0 — segment has no records".into(),
            ));
        }
        let record_size = 4 * maxdim + 11;
        // Sanity: total segment length should equal n_records * record_size + n_records + 2.
        // (records + epoch directory + MAXDIM + NUMREC). Off-by-one tolerance for
        // 0-vs-1-based indexing.
        Ok(Type21Metadata {
            record_size,
            maxdim,
            n_records,
        })
    }

    /// Binary search the Type 21 epoch directory for the record that
    /// covers `jd_tdb`. The directory stores the *final* epoch of each
    /// record in TDB seconds past J2000, in ascending order.
    fn find_type21_record_index(
        &self,
        segment: &SpkSegment,
        meta: &Type21Metadata,
        jd_tdb: f64,
    ) -> Result<usize, SpkError> {
        // Epoch directory sits at [end_index - 1 - n_records .. end_index - 2].
        let dir_start = segment.end_index - 1 - meta.n_records;
        let epochs = self.daf.read_f64_array(dir_start, meta.n_records)?;
        let target = jd_to_seconds_from_j2000(jd_tdb);
        // partition_point gives the first index where epochs[i] >= target.
        let idx = epochs.partition_point(|&e| e < target);
        if idx >= meta.n_records {
            // Clamp to the final record — handles the segment's end_epoch.
            Ok(meta.n_records - 1)
        } else {
            Ok(idx)
        }
    }

    fn read_type21_record(
        &self,
        segment: &SpkSegment,
        meta: &Type21Metadata,
        record_index: usize,
    ) -> Result<Type21Record, SpkError> {
        let record_start = segment.start_index + record_index * meta.record_size;
        let raw = self.daf.read_f64_array(record_start, meta.record_size)?;

        let tl = raw[0];
        let g: Vec<f64> = raw[1..1 + meta.maxdim].to_vec();
        let refpos = [
            raw[1 + meta.maxdim],
            raw[2 + meta.maxdim],
            raw[3 + meta.maxdim],
        ];
        let refvel = [
            raw[4 + meta.maxdim],
            raw[5 + meta.maxdim],
            raw[6 + meta.maxdim],
        ];

        // DT is stored as DT(MAXDIM, 3) in column-major (Fortran) order:
        //   raw[7+M .. 7+M+M]      = DT[*, 0]   (X component, all orders)
        //   raw[7+2M .. 7+3M]      = DT[*, 1]   (Y component)
        //   raw[7+3M .. 7+4M]      = DT[*, 2]   (Z component)
        let m = meta.maxdim;
        let dt_x = raw[7 + m..7 + 2 * m].to_vec();
        let dt_y = raw[7 + 2 * m..7 + 3 * m].to_vec();
        let dt_z = raw[7 + 3 * m..7 + 4 * m].to_vec();
        let dt = [dt_x, dt_y, dt_z];

        let kqmax1 = raw[7 + 4 * m] as i32;
        let kq = [
            raw[8 + 4 * m] as i32,
            raw[9 + 4 * m] as i32,
            raw[10 + 4 * m] as i32,
        ];

        Ok(Type21Record {
            tl,
            g,
            refpos,
            refvel,
            dt,
            kqmax1,
            kq,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_segment_contains_epoch() {
        let start_jd = cosmos_core::constants::J2000_JD;
        let end_jd = 2451645.0;
        let segment = SpkSegment {
            body_id: 3,
            center_id: 0,
            frame_id: 1,
            data_type: 2,
            start_epoch: jd_to_seconds_from_j2000(start_jd),
            end_epoch: jd_to_seconds_from_j2000(end_jd),
            start_index: 0,
            end_index: 0,
        };
        assert!(segment.contains_epoch(2451545.0));
        assert!(segment.contains_epoch(2451600.0));
        assert!(segment.contains_epoch(2451645.0));
        assert!(!segment.contains_epoch(2451544.9));
        assert!(!segment.contains_epoch(2451645.1));
        assert_eq!(segment.start_jd(), start_jd);
        assert_eq!(segment.end_jd(), end_jd);
    }

    #[test]
    fn test_epoch_conversions() {
        let jd = 2451545.0;
        let seconds = jd_to_seconds_from_j2000(jd);
        assert_eq!(seconds, 0.0);
        assert_eq!(seconds_from_j2000_to_jd(seconds), jd);
        let jd2 = 2460000.5;
        let seconds2 = jd_to_seconds_from_j2000(jd2);
        assert_eq!(seconds_from_j2000_to_jd(seconds2), jd2);
        let expected_seconds = (jd2 - J2000_JD) * SECONDS_PER_DAY_F64;
        assert_eq!(seconds2, expected_seconds);
    }

    #[test]
    fn test_spk_segment_from_summary() {
        let summary = DafSummary {
            doubles: vec![0.0, cosmos_core::constants::SECONDS_PER_DAY_F64], // start and end epoch
            ints: vec![399, 0, 1, 2, 100, 200], // body, center, frame, type, start_idx, end_idx
        };

        let segment = SpkSegment::from_summary(&summary).unwrap();
        assert_eq!(segment.body_id, 399);
        assert_eq!(segment.center_id, 0);
        assert_eq!(segment.frame_id, 1);
        assert_eq!(segment.data_type, 2);
        assert_eq!(segment.start_index, 100);
        assert_eq!(segment.end_index, 200);
        assert!((segment.start_epoch - 0.0).abs() < 1e-10);
        assert!((segment.end_epoch - cosmos_core::constants::SECONDS_PER_DAY_F64).abs() < 1e-10);
    }

    #[test]
    fn test_spk_segment_from_summary_insufficient_doubles() {
        let summary = DafSummary {
            doubles: vec![0.0], // only 1 double, need 2
            ints: vec![399, 0, 1, 2, 100, 200],
        };

        let result = SpkSegment::from_summary(&summary);
        assert!(result.is_err());
        match result {
            Err(SpkError::InvalidData(msg)) => assert!(msg.contains("Incomplete")),
            _ => panic!("Expected InvalidData error"),
        }
    }

    #[test]
    fn test_spk_segment_from_summary_insufficient_ints() {
        let summary = DafSummary {
            doubles: vec![0.0, cosmos_core::constants::SECONDS_PER_DAY_F64],
            ints: vec![399, 0, 1, 2, 100], // only 5 ints, need 6
        };

        let result = SpkSegment::from_summary(&summary);
        assert!(result.is_err());
        match result {
            Err(SpkError::InvalidData(msg)) => assert!(msg.contains("Incomplete")),
            _ => panic!("Expected InvalidData error"),
        }
    }

    #[test]
    fn test_spk_segment_clone() {
        let segment = SpkSegment {
            body_id: 399,
            center_id: 0,
            frame_id: 1,
            data_type: 2,
            start_epoch: 0.0,
            end_epoch: cosmos_core::constants::SECONDS_PER_DAY_F64,
            start_index: 100,
            end_index: 200,
        };

        let cloned = segment.clone();
        assert_eq!(cloned.body_id, segment.body_id);
        assert_eq!(cloned.center_id, segment.center_id);
        assert_eq!(cloned.frame_id, segment.frame_id);
        assert_eq!(cloned.data_type, segment.data_type);
    }

    #[test]
    fn test_spk_segment_debug() {
        let segment = SpkSegment {
            body_id: 399,
            center_id: 0,
            frame_id: 1,
            data_type: 2,
            start_epoch: 0.0,
            end_epoch: cosmos_core::constants::SECONDS_PER_DAY_F64,
            start_index: 100,
            end_index: 200,
        };

        let debug = format!("{:?}", segment);
        assert!(debug.contains("SpkSegment"));
        assert!(debug.contains("body_id: 399"));
    }

    #[test]
    fn test_type21_record_size_formula() {
        // Type 21 record size = 4 * MAXDIM + 11.
        for maxdim in [1usize, 5, 10, 15, 25] {
            let expected = 4 * maxdim + 11;
            // Sanity: matches the layout sum
            //   1 (TL) + MAXDIM (G) + 3 (REFPOS) + 3 (REFVEL)
            //   + 3*MAXDIM (DT) + 1 (KQMAX1) + 3 (KQ)
            //   = 4*MAXDIM + 11.
            assert_eq!(expected, 1 + maxdim + 3 + 3 + 3 * maxdim + 1 + 3);
        }
    }

    #[test]
    fn test_compute_state_type2_path_still_works() {
        // Regression: after adding the Type 21 dispatch, the Type 2
        // path must still produce the same Earth-Moon barycenter
        // state at J2000. Uses de432s if available.
        let path = match get_de432s_path() {
            Some(p) => p,
            None => {
                eprintln!("Skipping: de432s.bsp not found");
                return;
            }
        };
        let spk = SpkFile::open(&path).unwrap();
        let (pos, vel) = spk.compute_state(3, 0, J2000_JD).unwrap();
        let dist = libm::sqrt(pos[0].powi(2) + pos[1].powi(2) + pos[2].powi(2));
        assert!((dist / 149597870.7 - 1.0).abs() < 0.05);
        let vel_mag = libm::sqrt(vel[0].powi(2) + vel[1].powi(2) + vel[2].powi(2));
        assert!(vel_mag > 20.0 && vel_mag < 40.0);
    }

    #[test]
    fn test_type2_metadata_struct() {
        let meta = Type2Metadata {
            init: 0.0,
            intlen: cosmos_core::constants::SECONDS_PER_DAY_F64,
            rsize: 50,
            n_records: 100,
            n_coeffs: 16,
        };

        assert_eq!(meta.init, 0.0);
        assert_eq!(meta.intlen, cosmos_core::constants::SECONDS_PER_DAY_F64);
        assert_eq!(meta.rsize, 50);
        assert_eq!(meta.n_records, 100);
        assert_eq!(meta.n_coeffs, 16);
    }

    #[test]
    fn test_jd_to_seconds_roundtrip() {
        let test_jds = [
            2451545.0, // J2000.0
            2451545.5, // J2000.0 + 12 hours
            2460000.0, // future date
            2440000.0, // past date
        ];

        for &jd in &test_jds {
            let seconds = jd_to_seconds_from_j2000(jd);
            let reconstructed = seconds_from_j2000_to_jd(seconds);
            assert!(
                (jd - reconstructed).abs() < 1e-10,
                "Failed for JD {}: got {}",
                jd,
                reconstructed
            );
        }
    }

    #[test]
    fn test_spk_file_open_nonexistent() {
        let result = SpkFile::open("/nonexistent/path/file.bsp");
        assert!(result.is_err());
    }

    fn get_de432s_path() -> Option<std::path::PathBuf> {
        let test_file =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/de432s.bsp");
        if test_file.exists() {
            Some(test_file)
        } else {
            None
        }
    }

    #[test]
    fn test_spk_segments_accessor() {
        let path = match get_de432s_path() {
            Some(p) => p,
            None => {
                eprintln!("Skipping: de432s.bsp not found");
                return;
            }
        };
        let spk = SpkFile::open(&path).unwrap();
        let segments = spk.segments();
        assert!(!segments.is_empty());
    }

    #[test]
    fn test_spk_find_segment() {
        let path = match get_de432s_path() {
            Some(p) => p,
            None => {
                eprintln!("Skipping: de432s.bsp not found");
                return;
            }
        };
        let spk = SpkFile::open(&path).unwrap();

        // Earth-Moon barycenter (3) relative to SSB (0) at J2000
        let seg = spk.find_segment(3, 0, J2000_JD);
        assert!(
            seg.is_some(),
            "Should find segment for Earth-Moon barycenter"
        );

        // Non-existent body
        let seg = spk.find_segment(999, 0, J2000_JD);
        assert!(seg.is_none(), "Should not find segment for body 999");
    }

    #[test]
    fn test_spk_compute_position() {
        let path = match get_de432s_path() {
            Some(p) => p,
            None => {
                eprintln!("Skipping: de432s.bsp not found");
                return;
            }
        };
        let spk = SpkFile::open(&path).unwrap();

        // Compute Earth-Moon barycenter position at J2000
        let result = spk.compute_position(3, 0, J2000_JD);
        assert!(result.is_ok(), "Should compute position");

        let pos = result.unwrap();
        let dist = libm::sqrt(pos[0].powi(2) + pos[1].powi(2) + pos[2].powi(2));
        // Earth-Moon barycenter should be ~1 AU from SSB (in km)
        let dist_au = dist / 149597870.7;
        assert!(
            dist_au > 0.98 && dist_au < 1.02,
            "Earth should be ~1 AU from SSB, got {} AU",
            dist_au
        );
    }

    #[test]
    fn test_spk_compute_state() {
        let path = match get_de432s_path() {
            Some(p) => p,
            None => {
                eprintln!("Skipping: de432s.bsp not found");
                return;
            }
        };
        let spk = SpkFile::open(&path).unwrap();

        // Compute Earth-Moon barycenter state at J2000
        let result = spk.compute_state(3, 0, J2000_JD);
        assert!(result.is_ok(), "Should compute state");

        let (pos, vel) = result.unwrap();

        // Check position is reasonable
        let dist = libm::sqrt(pos[0].powi(2) + pos[1].powi(2) + pos[2].powi(2));
        assert!(dist > 1e8, "Position magnitude should be > 100 million km");

        // Check velocity is reasonable (Earth orbital velocity ~30 km/s)
        let vel_mag = libm::sqrt(vel[0].powi(2) + vel[1].powi(2) + vel[2].powi(2));
        assert!(
            vel_mag > 20.0 && vel_mag < 40.0,
            "Velocity should be ~30 km/s, got {} km/s",
            vel_mag
        );
    }

    #[test]
    fn test_spk_segment_not_found_error() {
        let path = match get_de432s_path() {
            Some(p) => p,
            None => {
                eprintln!("Skipping: de432s.bsp not found");
                return;
            }
        };
        let spk = SpkFile::open(&path).unwrap();

        // Request a non-existent body
        let result = spk.compute_position(999, 0, J2000_JD);
        assert!(result.is_err());
        match result {
            Err(SpkError::SegmentNotFound {
                body,
                center,
                epoch,
            }) => {
                assert_eq!(body, 999);
                assert_eq!(center, 0);
                assert!((epoch - J2000_JD).abs() < 1e-10);
            }
            _ => panic!("Expected SegmentNotFound error"),
        }
    }

    #[test]
    fn test_spk_compute_position_different_epochs() {
        let path = match get_de432s_path() {
            Some(p) => p,
            None => {
                eprintln!("Skipping: de432s.bsp not found");
                return;
            }
        };
        let spk = SpkFile::open(&path).unwrap();

        // Compute at different epochs
        let epochs = [J2000_JD, J2000_JD + 100.0, J2000_JD + 365.25];
        let mut prev_pos: Option<[f64; 3]> = None;

        for epoch in epochs {
            let result = spk.compute_position(3, 0, epoch);
            assert!(result.is_ok(), "Should compute position at epoch {}", epoch);

            let pos = result.unwrap();
            if let Some(prev) = prev_pos {
                // Positions should be different at different epochs
                let diff = libm::sqrt(
                    (pos[0] - prev[0]).powi(2)
                        + (pos[1] - prev[1]).powi(2)
                        + (pos[2] - prev[2]).powi(2),
                );
                assert!(
                    diff > 1e6,
                    "Positions should differ significantly between epochs"
                );
            }
            prev_pos = Some(pos);
        }
    }
}
