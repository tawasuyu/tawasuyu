# Changelog

All notable changes to `cosmos-validation` are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

The crate is currently a development harness for the wider `eternal-*`
workspace; it is not published to crates.io and the public API is
considered unstable. Everything below is tracked against the workspace
version `0.1.1-alpha.2`.

### Added

#### Phase 6 — Astronomy façade + symbolic astrology layer

These changes did not modify `cosmos-validation` itself, but they
build directly on its `Oracle` and lunar / asteroid / eclipses / houses
modules. They are listed here so the validation harness's downstream
consumers are documented in one place.

- **`cosmos-sky`** — ergonomic public façade. `Instant` (civil UTC
  with on-demand TT/TDB/UT1/JD), `Observer`, `EphemerisSession`,
  `ApparentPosition` (ecliptic + equatorial + horizon), `Body` enum
  spanning 22 luminaries/planets/nodes/Lilith/asteroids, `find_root`
  generic root-finder over time.
- **`Oracle::spk()`** getter added so higher-level crates can route
  the lunar-node / Lilith / asteroid paths through the same memory-
  mapped kernel without opening a second handle.
- **SPK Type 21 parsing** — `load_segments` now accepts both Type 2
  Chebyshev and Type 21 (Extended Modified Difference Arrays)
  segments. Metadata (NUMREC, MAXDIM) is read from the segment
  trailer; each record's TL, G, REFPOS, REFVEL, DT, KQMAX1, and KQ
  fields are fully parsed. The Newhall (1989) MDA *interpolation*
  step itself is not yet implemented — `compute_state` on a Type 21
  segment returns `SpkError::UnsupportedType(21)` after a successful
  parse rather than silently dropping the body.
- **`cosmos-astrology`** — symbolic layer on top of `cosmos-sky`.
  Adds `NatalChart::compute`, 7 house systems, 8 ayanamshas, 12-kind
  aspect engine with applying/separating, planetary returns,
  secondary/tertiary/minor progressions, true and Naibod solar arc,
  the classical primary-direction trilogy (Placidus mundane,
  Regiomontanus, Campanus) with Ptolemy/Naibod keys and aspect
  branches, transits (snapshot + next-exact), planetary stations,
  synastry, midpoint composite charts, Arabic Parts (Lots),
  Hellenistic profections, lunar phases, and eclipses-on-natal.

#### Phase 1 — SPK reader validation

- **Validation harness scaffold**: `cosmos-validation` crate with
  `Oracle`, `Fixture`, `FixtureSet`, `Tolerance`, JPL Horizons fetcher
  (feature `fetch`), and the regression-test integration.
  ([`6964ce4`](../../commits/6964ce4))
- **VSOP2013 + ELP/MPP02 oracle backend** with per-body realistic
  tolerances and a curated 30-fixture grid.
  ([`d9dddc1`](../../commits/d9dddc1))
- **Earth / Moon split fixtures** wrt EMB for the SPK backend
  (NAIF 399 / 301 wrt 3).
  ([`3bc2469`](../../commits/3bc2469))

#### Phase 2 — IAU correction stack (LT + S + LD + NPB)

- **Light-time correction** with SSB-centred iteration:
  `Oracle::corrected_state` and `Corrections` declaration on
  `FixtureSet`. Sub-millimetre vs Horizons VEC_CORR='LT'.
  ([`c7b7285`](../../commits/c7b7285))
- **Stellar aberration** via `eternal_coords::aberration::apply_aberration`
  (IAU 2000A relativistic formulation). Sub-milliarcsec angular vs
  Horizons VEC_CORR='LT+S'.
  ([`c7b585f`](../../commits/c7b585f))
- **Horizons OBSERVER (spherical) fetcher** for astrometric J2000 RA/Dec.
  TDB → TT conversion via `cosmos-time`'s Fairhead-Bretagnon series.
  Sub-microarcsec match for the LT-only pipeline.
  ([`9b0eb3d`](../../commits/9b0eb3d))
- **Full apparent IAU 2006/2000A pipeline** (`Frame::TrueEquatorEquinoxOfDate`,
  `Corrections::APPARENT`). Adds gravitational light deflection by the Sun
  and `npb_matrix_iau2006a` rotation. Documented ~50 mas systematic gap
  vs Horizons IAU 76/80/94.
  ([`773ec6b`](../../commits/773ec6b))
- **Swiss Ephemeris cross-validation**. `scripts/fetch_swiss.py` produces
  an independent reference; confirms Swiss vs Horizons exhibits the same
  ~50 mas IAU-version offset. Direct oracle-vs-Swiss residual: sub-mas on
  gas giants, 1–40 mas on inner planets.
  ([`7b05bd1`](../../commits/7b05bd1))

#### Phase 3 — astrological pipeline

- **Lahiri sidereal pipeline**: `tet_equatorial_to_ecliptic_of_date`,
  `lahiri_ayanamsha`, `lahiri_sidereal_longitude`. Sub-arcsec at the
  J2000 anchor, ±8″ at ±100 years.
  ([`a982f5f`](../../commits/a982f5f))
- **Ascendant + Midheaven + Whole-Sign + Equal house cusps**. Closed-form
  Meeus 14.4/14.5 with east-of-MC disambiguation. Sub-arcsec MC + Asc
  vs Swiss.
  ([`9319101`](../../commits/9319101))
- **True obliquity** (mean + Δε) in houses + ecliptic conversion.
  Tightens Asc/MC from ~8″ to sub-arcsec.
  ([`336a755`](../../commits/336a755))
- **Additional ayanamshas**: `Ayanamsha` enum covering Lahiri,
  Fagan-Bradley, DeLuce, Raman, Ushashashi, Krishnamurti, Djwhal Khul,
  Yukteshwar. J2000 anchors match Swiss to ten decimals.
  ([`336a755`](../../commits/336a755))
- **Mean lunar nodes + Lilith**: `mean_lunar_node`, `mean_lunar_perigee`,
  `mean_lilith`. ([`336a755`](../../commits/336a755))
- **Placidus house cusps** (port of Swiss `swehouse.c`). Sub-mas match.
  ([`48b0164`](../../commits/48b0164))
- **Koch / Regiomontanus / Campanus / Porphyry house cusps**. All sub-mas.
  ([`02de6e7`](../../commits/02de6e7))
- **Topocentric position pipeline**: `Observer`, WGS-84 →
  ITRS → TET via `R3(GAST)`. Validates Moon + Sun against Swiss
  `SEFLG_TOPOCTR` to < 0.5″ on a 1° parallax effect.
  ([`812e03c`](../../commits/812e03c))
- **True (osculating) lunar node + Lilith** from SPK Moon state.
  Sub-millarcsec node vs Swiss `SE_TRUE_NODE`; sub-arcsec Lilith vs
  `SE_OSCU_APOG`. ([`6cbeaee`](../../commits/6cbeaee))
- **Tighten Mean lunar node + Lilith** to sub-arcsec by adding nutation
  in longitude Δψ (mean reported in true-ecliptic-of-date frame) and
  projecting the apogee onto the ecliptic via the lunar inclination.
  ([`5cc2a4d`](../../commits/5cc2a4d))
- **Fixed-star catalog**: 26 named bright stars from Swiss `sefstars.txt`
  with proper-motion projection, BCRS→GCRS parallax shift, LD + S + NPB.
  Sub-arcsec longitude across 1968 / J2000 / 2023.
  ([`d792ad0`](../../commits/d792ad0))
- **Topocentric alt / az** (modern N=0°/E=90°) + **rise / set / transit**
  finder (coarse-scan + bisection). Sub-arcsec alt/az; rise/set ±100-200 s.
  ([`71ba167`](../../commits/71ba167))
- **Lunar eclipse detector + finder**. Earth-shadow geometry classifying
  None / Penumbral / Partial / Total. Type matches Swiss 10/10; time of
  maximum ±25–40 s.
  ([`169ab00`](../../commits/169ab00))
- **Global solar eclipse detector + finder**. Sun-Moon shadow-cone
  perpendicular distance to Earth's centre. Type matches Swiss 10/10;
  time ±25–40 s.
  ([`20a8a8d`](../../commits/20a8a8d))
- **Asteroid coverage** (Ceres / Pallas / Juno / Vesta) via `sb441-n16.bsp`
  + DE440 chain. Sub-arcsec longitude vs Swiss.
  ([`926b5ce`](../../commits/926b5ce))
- **Local (per-observer) solar eclipses**: topocentric Sun/Moon
  separation + Sun-above-horizon gate. Time sub-second to ±100 s; magnitude
  ±0.001 vs Swiss when central-visible.
  ([`208c781`](../../commits/208c781))

#### Phase 5 — polish

- **IERS ΔT table** with 1968–2030 1-year nodes; captures the post-2020
  Earth-rotation speed-up that broke Espenak's monotonic polynomial.
  ([`3122611`](../../commits/3122611))
- **Light-time correction in eclipse geometry**. Global solar-eclipse
  time-of-maximum residual collapses from ±25–40 s to **±0–4 s** vs
  Swiss `sol_eclipse_when_glob`.
  ([`3122611`](../../commits/3122611))

#### Documentation

- **PRECISION.md** — comprehensive feature × precision inventory.
- **CHANGELOG.md** — this file.
- **README.md** — restructured around features, validation methodology
  and reproduction commands.
- Inline doc-comments on every public function.
- 10 reproducible `scripts/fetch_swiss_*.py` reference fixture generators
  and 11 inspection CLIs under `src/bin/`.

### Known limitations carried forward

- **SPK Type 21** not supported by `cosmos-ephemeris`. Blocks Chiron,
  Pholus, Eris, Sedna and other centaurs / TNOs distributed by JPL
  Horizons as per-body SPK kernels.
- **Lunar eclipse ±30–44 s** residual from parabolic γ-min refinement.
  Brent or golden-section closes it.
- **Local solar eclipses with sunrise/sunset transitions** skipped. Needs
  windowed "max-while-Sun-above-horizon" search.
- **Rise/set ±100–200 s** from flat −34′ horizon convention.
- **Polar motion** omitted in topocentric (sub-mas effect).

See [PRECISION.md](./PRECISION.md) for the full precision table and the
[Suggested next work](./PRECISION.md#suggested-next-work) list.
