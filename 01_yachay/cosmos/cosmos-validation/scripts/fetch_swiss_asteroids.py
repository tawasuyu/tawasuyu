#!/usr/bin/env python3
"""Generate Swiss apparent ecliptic-of-date positions for the four
main-belt asteroids (Ceres, Pallas, Juno, Vesta) at three epochs.
Chiron is included if `seas_18.se1` covers it (which it should via the
combined Swiss kernel)."""

from __future__ import annotations

import argparse
import json
import math
import sys
from datetime import datetime, timezone
from pathlib import Path

import swisseph as swe

ASTEROIDS = [
    ("Ceres", swe.CERES),
    ("Pallas", swe.PALLAS),
    ("Juno", swe.JUNO),
    ("Vesta", swe.VESTA),
    ("Chiron", swe.CHIRON),
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

    epoch_rows = []
    for jd_tdb, label in epochs:
        jd_tt = jd_tdb_to_jd_tt(jd_tdb)
        rows = []
        for name, sid in ASTEROIDS:
            try:
                xx, _ = swe.calc(jd_tt, sid, swe.FLG_SWIEPH)
                rows.append({
                    "name": name,
                    "naif_id": 2_000_000 + (sid - swe.CERES + 1) if sid >= swe.CERES else None,
                    "swiss_id": sid,
                    "ecl_lon_deg": float(xx[0]),
                    "ecl_lat_deg": float(xx[1]),
                    "dist_au": float(xx[2]),
                })
            except Exception as e:
                rows.append({"name": name, "swiss_id": sid, "error": str(e)})
        epoch_rows.append({
            "label": label,
            "jd_tdb": jd_tdb,
            "asteroids": rows,
        })

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps({
        "description": (
            f"Swiss Ephemeris {swe.version} apparent ecliptic-of-date for "
            f"Ceres / Pallas / Juno / Vesta / Chiron at three epochs. "
            f"Generated {datetime.now(timezone.utc).isoformat()}."
        ),
        "epochs": epoch_rows,
    }, indent=2))
    print(f"Wrote {len(epoch_rows)} epochs × {len(ASTEROIDS)} asteroids to {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
