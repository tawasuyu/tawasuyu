#!/usr/bin/env python3
"""Generate Swiss apparent ecliptic-of-date positions for the curated
fixed-star list, across J2000 / 2023 / 1968 epochs."""

from __future__ import annotations

import argparse
import json
import math
import sys
from datetime import datetime, timezone
from pathlib import Path

import swisseph as swe

NAMES = [
    "Sirius", "Canopus", "Rigil Kentaurus", "Arcturus", "Vega", "Capella",
    "Rigel", "Procyon", "Betelgeuse", "Achernar", "Hadar", "Altair",
    "Acrux", "Aldebaran", "Antares", "Spica", "Pollux", "Fomalhaut",
    "Deneb", "Mimosa", "Regulus", "Adara", "Castor", "Shaula",
    "Bellatrix", "Elnath",
]


def jd_tdb_to_jd_tt(jd_tdb: float) -> float:
    t = (jd_tdb - 2_451_545.0) / 36_525.0
    g_rad = ((357.5277233 + 35_999.05034 * t) % 360.0) * math.pi / 180.0
    dtdb = 0.001657 * math.sin(g_rad) + 0.000022 * math.sin(2.0 * g_rad)
    return jd_tdb - dtdb / 86_400.0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", required=True)
    args = parser.parse_args()

    swe.set_ephe_path("/home/sergio/.local/share/swisseph")

    epochs = [
        (2_440_000.5, "1968-05-24"),
        (2_451_545.0, "J2000.0"),
        (2_460_000.5, "2023-02-25"),
    ]

    out: dict = {"epochs": []}
    for jd_tdb, label in epochs:
        jd_tt = jd_tdb_to_jd_tt(jd_tdb)
        # Swiss `fixstar2` (NOT `fixstar2_ut`) takes JD in TT/ET.
        rows = []
        for name in NAMES:
            xx, used_name, _ret = swe.fixstar2(name, jd_tt, swe.FLG_SWIEPH)
            rows.append({
                "name": name,
                "swiss_name": used_name.split(",")[0].strip(),
                "ecl_lon_deg": float(xx[0]),
                "ecl_lat_deg": float(xx[1]),
            })
        out["epochs"].append({
            "label": label,
            "jd_tdb": jd_tdb,
            "stars": rows,
        })

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps({
        "description": (
            f"Swiss Ephemeris {swe.version} apparent ecliptic-of-date "
            f"positions for 26 named bright stars at three epochs. "
            f"Generated {datetime.now(timezone.utc).isoformat()}."
        ),
        **out,
    }, indent=2))
    print(f"Wrote {sum(len(e['stars']) for e in out['epochs'])} entries "
          f"({len(out['epochs'])} epochs × {len(NAMES)} stars) to {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
