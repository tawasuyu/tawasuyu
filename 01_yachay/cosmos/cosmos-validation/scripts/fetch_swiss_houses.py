#!/usr/bin/env python3
"""Generate Swiss Ephemeris house-cusp references for a small set of charts.

Output schema is consumed by the Rust `houses-check` bin: each chart has
location, epoch, delta-T, and the Swiss-computed Ascendant, MC, and the
twelve cusps for several house systems.
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

import swisseph as swe


@dataclass
class Chart:
    name: str
    lat_deg: float
    lon_deg: float  # positive east
    jd_tdb: float
    delta_t_seconds: float


def jd_tdb_to_jd_tt(jd_tdb: float) -> float:
    # 2-term Fairhead-Bretagnon, accurate to ~30 µs.
    t = (jd_tdb - 2_451_545.0) / 36_525.0
    g_rad = ((357.5277233 + 35_999.05034 * t) % 360.0) * math.pi / 180.0
    dtdb = 0.001657 * math.sin(g_rad) + 0.000022 * math.sin(2.0 * g_rad)
    return jd_tdb - dtdb / 86_400.0


def jd_tt_to_jd_ut1(jd_tt: float, delta_t_seconds: float) -> float:
    # ΔT = TT − UT1, so UT1 = TT − ΔT.
    return jd_tt - delta_t_seconds / 86_400.0


def swiss_houses(chart: Chart) -> dict:
    swe.set_ephe_path("/home/sergio/.local/share/swisseph")
    jd_tt = jd_tdb_to_jd_tt(chart.jd_tdb)
    jd_ut = jd_tt_to_jd_ut1(jd_tt, chart.delta_t_seconds)

    out: dict[str, object] = {}
    systems = [
        ("whole_sign", b"W"),
        ("equal", b"E"),
        ("placidus", b"P"),
        ("koch", b"K"),
        ("regiomontanus", b"R"),
        ("campanus", b"C"),
        ("porphyry", b"O"),
    ]
    for label, hsys in systems:
        cusps, ascmc = swe.houses(jd_ut, chart.lat_deg, chart.lon_deg, hsys)
        out[f"{label}_cusps_deg"] = list(cusps[:12])
        if "ascendant_deg" not in out:
            out["ascendant_deg"] = float(ascmc[0])
            out["mc_deg"] = float(ascmc[1])
            out["armc_deg"] = float(ascmc[2])
    return out


def charts() -> list[Chart]:
    return [
        # Greenwich at J2000.0 TDB. Delta-T at J2000.0 ≈ 63.83 s.
        Chart("Greenwich @ J2000", 51.4769, 0.0, 2_451_545.0, 63.83),
        # Madrid at the 2023 anchor. Delta-T 2023 ≈ 69.5 s.
        Chart("Madrid @ 2023-02-25", 40.4168, -3.7038, 2_460_000.5, 69.5),
        # New York City at 1968. Delta-T 1968 ≈ 38.3 s.
        Chart("New York @ 1968-05-24", 40.7128, -74.006, 2_440_000.5, 38.3),
        # Sydney at J2000. Southern-hemisphere sanity check.
        Chart("Sydney @ J2000", -33.8688, 151.2093, 2_451_545.0, 63.83),
    ]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", required=True)
    args = parser.parse_args()

    docs = []
    for c in charts():
        print(f"computing houses for {c.name} ...", file=sys.stderr)
        swiss = swiss_houses(c)
        docs.append({
            "name": c.name,
            "lat_deg": c.lat_deg,
            "lon_deg": c.lon_deg,
            "jd_tdb": c.jd_tdb,
            "delta_t_seconds": c.delta_t_seconds,
            "swiss": swiss,
        })

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps({
        "description": (
            f"Swiss Ephemeris {swe.version} house cusps for selected charts. "
            f"Generated {datetime.now(timezone.utc).isoformat()}."
        ),
        "charts": docs,
    }, indent=2))
    print(f"Wrote {len(docs)} charts to {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
