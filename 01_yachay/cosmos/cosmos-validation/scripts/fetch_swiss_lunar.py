#!/usr/bin/env python3
"""Generate Swiss reference values for the four lunar special points:
SE_MEAN_NODE, SE_TRUE_NODE, SE_MEAN_APOG, SE_OSCU_APOG, across a small
date grid spanning 1900-2100.
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from datetime import datetime, timezone
from pathlib import Path

import swisseph as swe


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
        (2_415_020.5, "1900-01-01"),
        (2_440_000.5, "1968-05-24"),
        (2_451_545.0, "J2000.0"),
        (2_460_000.5, "2023-02-25"),
        (2_488_069.5, "2100-01-01"),
    ]

    docs = []
    for jd_tdb, label in epochs:
        # Swiss `calc` (not calc_ut) takes JD in TT/ET — keep this clean.
        jd_tt = jd_tdb_to_jd_tt(jd_tdb)
        flags = swe.FLG_SWIEPH
        mean_node, _ = swe.calc(jd_tt, swe.MEAN_NODE, flags)
        true_node, _ = swe.calc(jd_tt, swe.TRUE_NODE, flags)
        mean_apog, _ = swe.calc(jd_tt, swe.MEAN_APOG, flags)
        oscu_apog, _ = swe.calc(jd_tt, swe.OSCU_APOG, flags)
        docs.append({
            "label": label,
            "jd_tdb": jd_tdb,
            "jd_tt": jd_tt,
            "mean_node_deg": float(mean_node[0]),
            "true_node_deg": float(true_node[0]),
            "mean_apog_deg": float(mean_apog[0]),
            "oscu_apog_deg": float(oscu_apog[0]),
        })

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps({
        "description": (
            f"Swiss Ephemeris {swe.version} mean and true / osculating "
            f"lunar nodes and Lilith. Generated "
            f"{datetime.now(timezone.utc).isoformat()}."
        ),
        "samples": docs,
    }, indent=2))
    print(f"Wrote {len(docs)} samples to {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
