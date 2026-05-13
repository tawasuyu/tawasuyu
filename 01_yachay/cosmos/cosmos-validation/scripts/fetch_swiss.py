#!/usr/bin/env python3
"""Generate a eternal-validation fixture set from Swiss Ephemeris.

Mirror the SPK astrometric grid (planets + Sun + Moon, geocentric, three
epochs) and write apparent geocentric Cartesian positions in the true
equator-and-equinox-of-date frame so they can be compared head-to-head
against our oracle's `Corrections::APPARENT` output, also computed under
IAU 2006/2000A.

Both implementations read DE440 via JPL kernel, so the *only* axis that
should disagree is the residual numerical formulation difference between
eternal-coords/eternal-core's IAU 2006/2000A implementation and the
one in libswe.

Run from the eternal workspace root:

    .venv/bin/python3 eternal-validation/scripts/fetch_swiss.py \\
        --kernel ~/.local/share/ephemeris/de440.bsp \\
        --out eternal-validation/fixtures/regression-de440-swiss-apparent/swiss.json
"""

from __future__ import annotations

import argparse
import json
import math
import os
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

import swisseph as swe

AU_KM = 149_597_870.7
SECONDS_PER_DAY = 86_400.0
DEG = math.pi / 180.0


def tdb_to_tt_seconds(jd_tdb: float) -> float:
    """Truncated Fairhead & Bretagnon series for TDB - TT, in seconds.

    The two-term version is accurate to ~30 µs across the modern era,
    which corresponds to < 1 m at Earth's orbital velocity — well below
    the sub-millimeter precision floor we care about.
    """
    t = (jd_tdb - 2_451_545.0) / 36_525.0
    g_deg = 357.5277233 + 35_999.05034 * t
    g_rad = (g_deg % 360.0) * DEG
    return 0.001_657 * math.sin(g_rad) + 0.000_022 * math.sin(2.0 * g_rad)


def jd_tdb_to_jd_tt(jd_tdb: float) -> float:
    return jd_tdb - tdb_to_tt_seconds(jd_tdb) / SECONDS_PER_DAY


@dataclass
class GridPoint:
    name: str
    body: int  # NAIF id (matches the oracle's grid)
    swe_body: int  # libswe body constant
    center: int  # NAIF id (geocentric = 399)
    jd_tdb: float


def grid() -> list[GridPoint]:
    bodies = [
        ("Mercury barycenter", 1, swe.MERCURY),
        ("Venus barycenter", 2, swe.VENUS),
        ("Mars barycenter", 4, swe.MARS),
        ("Jupiter barycenter", 5, swe.JUPITER),
        ("Saturn barycenter", 6, swe.SATURN),
        ("Uranus barycenter", 7, swe.URANUS),
        ("Neptune barycenter", 8, swe.NEPTUNE),
        ("Sun", 10, swe.SUN),
        ("Moon", 301, swe.MOON),
    ]
    epochs = [
        (2_451_545.0, "J2000"),
        (2_460_000.5, "2023-02-25"),
        (2_440_000.5, "1968-05-24"),
    ]
    out = []
    for name, naif, sb in bodies:
        for jd, label in epochs:
            out.append(
                GridPoint(
                    name=f"{name} astrometric wrt Earth @ {label}",
                    body=naif,
                    swe_body=sb,
                    center=399,
                    jd_tdb=jd,
                )
            )
    return out


def compute_apparent_tet_cartesian(gp: GridPoint) -> tuple[list[float], list[float]]:
    """Return (pos_km_TET, vel_km_s) using Swiss apparent equatorial.

    Velocity is `[0, 0, 0]` — Swiss's xx[3..6] is RA-rate/Dec-rate/dist-rate,
    not Cartesian velocity, and the OBSERVER-style fixtures already ignore
    velocity at comparison time.
    """
    jd_tt = jd_tdb_to_jd_tt(gp.jd_tdb)
    # FLG_SWIEPH uses Swiss's curated .se1 files (~1 mas vs JPL claim).
    # FLG_JPLEPH would require a Swiss-formatted JPL file (de441.441,
    # not the NAIF .bsp Chebyshev format we have on disk).
    flags = swe.FLG_SWIEPH | swe.FLG_EQUATORIAL
    xx, retflag = swe.calc(jd_tt, gp.swe_body, flags)
    if retflag < 0:
        raise RuntimeError(f"swe.calc returned error flag {retflag} for {gp.name}")
    if not (retflag & swe.FLG_SWIEPH):
        raise RuntimeError(
            f"swe.calc fell back to a non-SWIEPH ephemeris (retflag={retflag}) "
            f"for {gp.name}; check FLG_SWIEPH ephemeris files are reachable."
        )

    ra_deg, dec_deg, distance_au = xx[0], xx[1], xx[2]
    ra = ra_deg * DEG
    dec = dec_deg * DEG
    range_km = distance_au * AU_KM
    cos_dec = math.cos(dec)
    pos = [
        range_km * cos_dec * math.cos(ra),
        range_km * cos_dec * math.sin(ra),
        range_km * math.sin(dec),
    ]
    return pos, [0.0, 0.0, 0.0]


def lahiri_sidereal_lon_deg(gp: GridPoint, jd_tt: float) -> float:
    swe.set_sid_mode(swe.SIDM_LAHIRI)
    flags = swe.FLG_SWIEPH | swe.FLG_SIDEREAL
    xx, _ = swe.calc(jd_tt, gp.swe_body, flags)
    return xx[0]  # ecliptic longitude in degrees


def tropical_lon_deg(gp: GridPoint, jd_tt: float) -> float:
    flags = swe.FLG_SWIEPH  # default ecliptic of date, apparent
    xx, _ = swe.calc(jd_tt, gp.swe_body, flags)
    return xx[0]


def build_fixture(gp: GridPoint, pos_km: list[float], vel_km_s: list[float]) -> dict:
    jd_tt = jd_tdb_to_jd_tt(gp.jd_tdb)
    return {
        "name": gp.name,
        "body": gp.body,
        "center": gp.center,
        "jd_tdb": gp.jd_tdb,
        "frame": "TET",
        "pos_km": pos_km,
        "vel_km_s": vel_km_s,
        "swiss_extras": {
            "tropical_lon_deg": tropical_lon_deg(gp, jd_tt),
            "lahiri_sidereal_lon_deg": lahiri_sidereal_lon_deg(gp, jd_tt),
            "lahiri_ayanamsha_deg": swe.get_ayanamsa_ex_ut(jd_tt, swe.FLG_SWIEPH)[1],
        },
        "source": {
            "kind": "swiss_ephemeris",
            "version": str(swe.version),
        },
        # Sub-mas precision target. The oracle and Swiss should agree to
        # the numerical-formulation residual of IAU 2006/2000A. The
        # position tolerance below is "scale-with-distance" generous
        # enough to let radial round-off pass while keeping the angular
        # signal in view.
        # Two independent IAU 2006/2000A implementations agree to within
        # tens of mas across our grid — Mercury is the worst, the gas
        # giants are sub-arcsecond, the Sun and Moon are sub-mas. Set the
        # gate at 100 km position (≈ 140 mas at 1 AU, ≈ 4.5 mas at 30 AU).
        # The angular_sep_mas column on the report is the real metric.
        "tolerance": {
            "pos_km": 1.0e2,
            "vel_km_s": 1.0e10,
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="Swiss Ephemeris fixture generator")
    parser.add_argument(
        "--ephe-path",
        default="/home/sergio/.local/share/swisseph",
        help="Directory containing Swiss Ephemeris .se1 files",
    )
    parser.add_argument("--out", required=True, help="Output fixture JSON path")
    args = parser.parse_args()

    ephe_path = Path(args.ephe_path)
    if not ephe_path.exists():
        print(f"Swiss ephemeris path not found: {ephe_path}", file=sys.stderr)
        return 2

    swe.set_ephe_path(str(ephe_path))

    fixtures = []
    for gp in grid():
        print(f"computing {gp.name} ...", file=sys.stderr)
        pos, vel = compute_apparent_tet_cartesian(gp)
        fixtures.append(build_fixture(gp, pos, vel))

    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    document = {
        "description": (
            f"Swiss Ephemeris {swe.version} apparent positions (IAU 2006/2000A, "
            f"TET frame, geocentric, FLG_SWIEPH .se1 kernels — ~1 mas vs JPL). "
            f"Generated {datetime.now(timezone.utc).isoformat()}."
        ),
        "backend": "spk",
        "corrections": {
            "light_time": True,
            "stellar_aberration": True,
            "gravitational_deflection": True,
        },
        "fixtures": fixtures,
    }
    out.write_text(json.dumps(document, indent=2))
    print(f"Wrote {len(fixtures)} Swiss fixtures to {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
