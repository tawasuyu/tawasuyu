#!/usr/bin/env python3
"""Generate Swiss next-lunar-eclipse times + types from a fixed start
date, covering ~5 years of eclipses."""

from __future__ import annotations

import argparse
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

import swisseph as swe


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", required=True)
    parser.add_argument("--n", type=int, default=10, help="Number of eclipses to find")
    parser.add_argument("--start", type=float, default=2_460_310.5,
                        help="Starting JD (UT). Default = 2024-01-01.")
    args = parser.parse_args()

    swe.set_ephe_path("/home/sergio/.local/share/swisseph")

    lunar = []
    jd = args.start
    for i in range(args.n):
        ret_flag, tret = swe.lun_eclipse_when(jd, swe.FLG_SWIEPH, 0)
        kind = "unknown"
        if ret_flag & swe.ECL_TOTAL:
            kind = "total"
        elif ret_flag & swe.ECL_PARTIAL:
            kind = "partial"
        elif ret_flag & swe.ECL_PENUMBRAL:
            kind = "penumbral"
        lunar.append({
            "max_jd_ut": float(tret[0]),
            "kind": kind,
            "flags_hex": f"0x{ret_flag:x}",
        })
        jd = tret[0] + 1.0

    solar = []
    jd = args.start
    for i in range(args.n):
        ret_flag, tret = swe.sol_eclipse_when_glob(jd, swe.FLG_SWIEPH, 0)
        kind = "unknown"
        if ret_flag & swe.ECL_ANNULAR_TOTAL:
            kind = "hybrid"
        elif ret_flag & swe.ECL_TOTAL:
            kind = "total"
        elif ret_flag & swe.ECL_ANNULAR:
            kind = "annular"
        elif ret_flag & swe.ECL_PARTIAL:
            kind = "partial"
        solar.append({
            "max_jd_ut": float(tret[0]),
            "kind": kind,
            "flags_hex": f"0x{ret_flag:x}",
        })
        jd = tret[0] + 1.0

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps({
        "description": (
            f"Swiss Ephemeris {swe.version} next {args.n} lunar + solar "
            f"eclipses from JD {args.start} (UT). Generated "
            f"{datetime.now(timezone.utc).isoformat()}."
        ),
        "start_jd_ut": args.start,
        "eclipses": lunar,           # backwards-compatible field name
        "lunar_eclipses": lunar,
        "solar_eclipses_global": solar,
    }, indent=2))
    print(f"Wrote {len(lunar)} lunar + {len(solar)} solar eclipses to {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
