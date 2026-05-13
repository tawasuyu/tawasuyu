#!/usr/bin/env python3
"""Generate Swiss Ephemeris topocentric reference for the Moon and Sun.

For each chart we already use in `fetch_swiss_houses.py`, output the
geocentric and topocentric apparent ecliptic longitude / latitude /
distance for the Moon and the Sun. The diurnal-parallax delta on the
Moon is ~1°; on the Sun it's ~9″ at horizon. Both are easy and
informative validation targets.
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
    lon_deg: float
    elev_m: float
    jd_tdb: float
    delta_t_seconds: float


def jd_tdb_to_jd_tt(jd_tdb: float) -> float:
    t = (jd_tdb - 2_451_545.0) / 36_525.0
    g_rad = ((357.5277233 + 35_999.05034 * t) % 360.0) * math.pi / 180.0
    dtdb = 0.001657 * math.sin(g_rad) + 0.000022 * math.sin(2.0 * g_rad)
    return jd_tdb - dtdb / 86_400.0


def charts() -> list[Chart]:
    return [
        Chart("Greenwich @ J2000", 51.4769, 0.0, 0.0, 2_451_545.0, 63.83),
        Chart("Madrid @ 2023-02-25", 40.4168, -3.7038, 0.0, 2_460_000.5, 69.5),
        Chart("New York @ 1968-05-24", 40.7128, -74.006, 0.0, 2_440_000.5, 38.3),
        Chart("Sydney @ J2000", -33.8688, 151.2093, 0.0, 2_451_545.0, 63.83),
    ]


def compute(chart: Chart) -> dict:
    swe.set_ephe_path("/home/sergio/.local/share/swisseph")
    jd_tt = jd_tdb_to_jd_tt(chart.jd_tdb)
    jd_ut = jd_tt - chart.delta_t_seconds / 86_400.0

    swe.set_topo(chart.lon_deg, chart.lat_deg, chart.elev_m)

    out: dict = {}
    for body, key in [(swe.MOON, "moon"), (swe.SUN, "sun")]:
        # Geocentric apparent ecliptic.
        geo, _ = swe.calc_ut(jd_ut, body, swe.FLG_SWIEPH)
        # Topocentric apparent ecliptic.
        topo, _ = swe.calc_ut(jd_ut, body, swe.FLG_SWIEPH | swe.FLG_TOPOCTR)
        out[f"{key}_geo_lon_deg"] = float(geo[0])
        out[f"{key}_geo_lat_deg"] = float(geo[1])
        out[f"{key}_geo_dist_au"] = float(geo[2])
        out[f"{key}_topo_lon_deg"] = float(topo[0])
        out[f"{key}_topo_lat_deg"] = float(topo[1])
        out[f"{key}_topo_dist_au"] = float(topo[2])
    return out


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", required=True)
    args = parser.parse_args()

    docs = []
    for c in charts():
        print(f"computing topocentric for {c.name} ...", file=sys.stderr)
        swiss = compute(c)
        docs.append({
            "name": c.name,
            "lat_deg": c.lat_deg,
            "lon_deg": c.lon_deg,
            "elev_m": c.elev_m,
            "jd_tdb": c.jd_tdb,
            "delta_t_seconds": c.delta_t_seconds,
            "swiss": swiss,
        })

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps({
        "description": (
            f"Swiss Ephemeris {swe.version} geocentric vs topocentric "
            f"apparent ecliptic for Moon + Sun. Generated "
            f"{datetime.now(timezone.utc).isoformat()}."
        ),
        "charts": docs,
    }, indent=2))
    print(f"Wrote {len(docs)} charts to {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
