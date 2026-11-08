"""Compare the engine's outcome distributions with Showdown's.

Usage:
    uv run python scripts/verify_engine.py --cases 100 --n 300
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from pathlib import Path

from ninjask.paths import DATA_DIR
from ninjask.verify import verify


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("--format", default="gen9championsvgc2026regmc", help="format id (gen9championsdoublescustomgame plays all six, no team preview)")
    p.add_argument("--cases", type=int, default=50)
    p.add_argument("--n", type=int, default=200, help="outcomes drawn per case from each simulator")
    p.add_argument("--seed", type=int, default=0)
    p.add_argument("--teams", choices=["supported", "pool", "generic"], default="supported")
    p.add_argument("--focus", default="", help="comma-separated move, ability and item ids to concentrate on")
    p.add_argument("--pool", type=Path, help="team file for --teams pool (default: ninjask.verify.SHEETS_POOL)")
    p.add_argument("--out", type=Path, default=DATA_DIR / "verify" / "latest.json")
    args = p.parse_args()
    focus = frozenset(x.strip() for x in args.focus.split(",") if x.strip())
    report = verify(args.format, args.cases, args.n, args.seed, teams=args.teams, focus=focus, pool_file=args.pool)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=1), encoding="utf-8")
    print(f"{report['cases']} cases, {report['comparisons']} comparisons, "
          f"{len(report['failures'])} failures, {len(report['errors'])} errors")
    kinds = Counter(f["feature"].split(".")[-1] for f in report["failures"])
    print("failing feature kinds:", dict(kinds))
    print("test points with each volatile:", report.get("volatile_coverage", {}))
    for e in report["errors"][:10]:
        print("error:", e)
    for f in report["failures"][:12]:
        sd = sorted(f["showdown"].items(), key=lambda kv: -kv[1])[:6]
        en = sorted(f["engine"].items(), key=lambda kv: -kv[1])[:6]
        print(f"case {f['case']} {f['feature']} p={f['p']:.2g} mismatch={f['support_mismatch']} choices={f['choices']}")
        print(f"   showdown {sd}")
        print(f"   engine   {en}")
    print(args.out)
    # Fail the run when a distribution differed. An "error" is a case whose
    # game ended before its test point: skipped, not failed.
    sys.exit(1 if report["failures"] else 0)


if __name__ == "__main__":
    main()
