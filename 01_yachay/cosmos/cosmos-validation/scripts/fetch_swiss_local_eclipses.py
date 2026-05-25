#!/usr/bin/env python3
"""Generate Swiss next-local-solar-eclipse times for several observer
locations spanning a 5-year window."""

from __future__ import annotations

import argparse
import json
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

import swisseph as swe


@dataclass
class Site:
    name: str
    lat_deg: float
    lon_deg: float
    elev_m: float


SITES = [
    Site("Madrid", 40.4168, -3.7038, 0.0),
    Site("New York", 40.7128, -74.006, 0.0),
    Site("Sydney", -33.8688, 151.2093, 0.0),
    Site("Tokyo", 35.6762, 139.6503, 0.0),
]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", required=True)
    parser.add_argument("--start", type=float, default=2_460_310.5,
                        help="Search start JD (UT). Default = 2024-01-01.")
    parser.add_argument("--n", type=int, default=4,
                        help="Number of eclipses per site to enumerate.")
    args = parser.parse_args()

    swe.set_ephe_path("/home/sergio/.local/share/swisseph")

    docs = []
    for site in SITES:
        rows = []
        jd = args.start
        for _ in range(args.n):
            try:
                retflag, tret, attr = swe.sol_eclipse_when_loc(
                    jd, [site.lon_deg, site.lat_deg, site.elev_m], swe.FLG_SWIEPH
                )
            except Exception as e:
                rows.append({"error": str(e)})
                break
            kind = "unknown"
            if retflag & swe.ECL_TOTAL:
                kind = "total"
            elif retflag & swe.ECL_ANNULAR:
                kind = "annular"
            elif retflag & swe.ECL_ANNULAR_TOTAL:
                kind = "hybrid"
            elif retflag & swe.ECL_PARTIAL:
                kind = "partial"
            rows.append({
                "max_jd_ut": float(tret[0]),
                "first_contact_jd_ut": float(tret[1]),
                "last_contact_jd_ut": float(tret[4]),
                "kind": kind,
                "magnitude": float(attr[0]),
                "fraction_covered": float(attr[2]),
                "flags_hex": f"0x{retflag:x}",
            })
            jd = tret[0] + 1.0
        docs.append({
            "name": site.name,
            "lat_deg": site.lat_deg,
            "lon_deg": site.lon_deg,
            "elev_m": site.elev_m,
            "eclipses": rows,
        })

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps({
        "description": (
            f"Swiss Ephemeris {swe.version} next {args.n} local solar eclipses "
            f"per site from JD {args.start} (UT). Generated "
            f"{datetime.now(timezone.utc).isoformat()}."
        ),
        "start_jd_ut": args.start,
        "sites": docs,
    }, indent=2))
    total = sum(len(d["eclipses"]) for d in docs)
    print(f"Wrote {len(SITES)} sites × up to {args.n} eclipses ({total} total) to {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
