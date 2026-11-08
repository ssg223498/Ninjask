"""How much of a Reg M-C team pool does the engine support?

Reads a team pool (by default <data>/teams/<format>-pastes.txt, one packed team per line), asks the engine which
moves, items and abilities each team uses that it can't simulate, and ranks
the blockers greedily: the entity whose port would make the most teams fully
supported comes first, then the next given the first, and so on. Writes
support-<format>.json, and the fully supported teams (a pool to train on)
to teams/supported-pool.txt, in the data directory (ninjask.paths:
data/derived/).

Usage:
    python scripts/report_support.py --pool teams.txt
"""

from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path

from ninjask import dex
from ninjask.paths import DATA_DIR, TEAMS_DIR


def entity(problem: str) -> str:
    """'move protect (has code: ...)' -> 'move protect'; 'item lifeorb' -> 'item lifeorb'."""
    words = problem.split()
    if words[0] == "mega":
        return " ".join(words[:3])
    return " ".join(words[:2])


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("--format", default="gen9championsvgc2026regmc")
    p.add_argument("--engine-format", default="gen9championsdoublescustomgame")
    p.add_argument("--top", type=int, default=40)
    p.add_argument("--pool", type=Path, help="team pool to read (default: <data>/teams/<format>-pastes.txt)")
    args = p.parse_args()
    pool_path = args.pool or TEAMS_DIR / f"{args.format}-pastes.txt"
    teams = [t for t in pool_path.read_text(encoding="utf-8").splitlines() if t.strip()]
    d = dex(args.engine_format)
    blockers: list[set[str]] = []
    for t in teams:
        try:
            problems = d.team_problems(t)
        except ValueError as e:
            problems = [f"parse {e}"]
        blockers.append({entity(x) for x in problems})
    supported = sum(1 for b in blockers if not b)
    counts = Counter(e for b in blockers for e in b)

    # Greedy: port the entity that completes the most teams next.
    remaining = [set(b) for b in blockers if b]
    order = []
    done = supported
    while remaining and len(order) < args.top:
        gain = Counter()
        for b in remaining:
            for e in b:
                # Teams completed if e (and everything already ported) were supported.
                if len(b) == 1:
                    gain[e] += 1
        if gain:
            best, g = gain.most_common(1)[0]
        else:
            # No single port completes a team: take the most common blocker.
            best, g = Counter(e for b in remaining for e in b).most_common(1)[0][0], 0
        for b in remaining:
            b.discard(best)
        done += sum(1 for b in remaining if not b)
        remaining = [b for b in remaining if b]
        order.append({"entity": best, "teams_using": counts[best], "supported_after": done})

    out = {
        "format": args.format,
        "teams": len(teams),
        "supported": supported,
        "blockers_by_teams": counts.most_common(),
        "greedy_port_order": order,
    }
    # The default pool's supported teams are the main pool (supported-pool.txt); another
    # pool's go to their own file (verification reads both, ninjask.verification_teams).
    suffix = "" if args.pool is None else "-" + pool_path.stem.removeprefix(f"{args.format}-")
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    dest = DATA_DIR / f"support-{args.format}{suffix}.json"
    dest.write_text(json.dumps(out, indent=1), encoding="utf-8")
    name = "supported-pool.txt" if args.pool is None else f"supported-{pool_path.stem}.txt"
    TEAMS_DIR.mkdir(parents=True, exist_ok=True)
    pool = TEAMS_DIR / name
    pool.write_text("".join(t + "\n" for t, b in zip(teams, blockers) if not b), encoding="utf-8")
    print(f"{supported}/{len(teams)} pool teams fully supported")
    print("most common blockers:", ", ".join(f"{e} {n}" for e, n in counts.most_common(25)))
    print("greedy port order (teams supported after each):")
    for row in order:
        print(f"  {row['entity']:32s} used by {row['teams_using']:4d}  -> {row['supported_after']}")
    print(dest)
    print(pool)


if __name__ == "__main__":
    main()
