"""Does the engine's reference mode match Showdown exactly?

Plays seeded games in Showdown and in the engine running Showdown's RNG
(`Battle.reference`), feeds both the same choices, and after every input
compares the log lines each wrote since the previous input and the state
projections (refsim `project()`, the engine's `projection()`), which include
the RNG state. A game matches when nothing differs to its end. For each game
that doesn't, the first divergence is printed: that input, both logs, both
RNG states and the fields of the projections that differ.

    uv run python scripts/check_reference.py --games 100
    uv run python scripts/check_reference.py --games 10000 --workers 4 --show 0
    uv run python scripts/check_reference.py --games 1 --first 7 --verbose
    uv run python scripts/check_reference.py --games 500 --workers 4 --require perishsong,haze
    uv run python scripts/check_reference.py --games 2000 --workers 4 --synthetic
    uv run python scripts/check_reference.py --format gen9championsbssregmc --pool teams.txt --games 1000

`--format` (or `$NINJASK_FORMAT`) picks the format; the default is Reg M-C
doubles. `--requests` (or `$NINJASK_CHECK_REQUESTS=1`) also compares every
request Showdown sends with the engine's (without `rqid`, which Showdown's
server adds, not its simulator). Worker processes read both from the
environment.
"""

from __future__ import annotations

import argparse
import difflib
import json
import multiprocessing
import multiprocessing.util
import os
import random
import re
import sys
from collections import Counter
from pathlib import Path

import ninjask

from ninjask.choices import joint_choices, team_preview_choices
from ninjask import dex as engine_dex
from ninjask import load as load_format
from ninjask import focus_teams, supported_test_team, verification_teams
from ninjask.refsim import Battle, RefSim, seed_from_int

FORMAT = os.environ.get("NINJASK_FORMAT") or "gen9championsvgc2026regmc"
GAME_TYPE = load_format(FORMAT).game_type
REQUESTS = os.environ.get("NINJASK_CHECK_REQUESTS") == "1"
TURN_LIMIT = 150

# Lines that carry no battle state: timestamps, format banners, interface, hints.
SKIP = ("|t:|", "|gen|", "|tier|", "|rule|", "|j|", "|uhtml", "|inactive", "|gametype|", "|player|", "|-hint|")


def norm(lines: list[str]) -> list[str]:
    return [line for line in lines if line not in ("|", "") and not line.startswith(SKIP)]


class Divergence(Exception):
    def __init__(self, message: str, kind: str = "other") -> None:
        super().__init__(message)
        self.kind = kind

    def __reduce__(self):  # keep `kind` across worker processes
        return (Divergence, (str(self), self.kind))


def shape(text: str) -> str:
    """A log difference with Pokémon and HP left out, to group like cases."""
    text = re.sub(r"p[12][ab]?: [^|]+", "pX", text)
    return re.sub(r"\b\d+/\d+\b", "hp", text)


def state_diff(a, b, path: str = "") -> list[str]:
    """Paths where two projections differ, with both values."""
    if isinstance(a, dict) and isinstance(b, dict):
        out = []
        for k in sorted(set(a) | set(b)):
            if k not in a or k not in b:
                out.append(f"{path}.{k}: showdown {json.dumps(a.get(k, '<none>'))}, engine {json.dumps(b.get(k, '<none>'))}")
            else:
                out += state_diff(a[k], b[k], f"{path}.{k}")
        return out
    if isinstance(a, list) and isinstance(b, list) and len(a) == len(b):
        return [d for i, (x, y) in enumerate(zip(a, b)) for d in state_diff(x, y, f"{path}[{i}]")]
    return [] if a == b else [f"{path}: showdown {json.dumps(a)}, engine {json.dumps(b)}"]


def site(draw: str) -> str:
    """A draw's call site, condensed: "random(2) eachEvent<runAction:2360"."""
    call, _, frames = draw.partition(" @ ")
    call = call.split(" = ")[0]
    if frames.startswith("src/"):
        return f"{call} {frames}"
    keep = [f for f in frames.split(" < ") if not f.startswith(("Battle.speedSort", "Battle.sample", "Battle.random ", "Battle.randomChance", "battle.speedSort", "queue.sort"))]
    keep = [f.split(" (")[0].split(".")[-1] + ":" + f.rsplit(":", 1)[-1].rstrip(")") for f in keep[:2]]
    return f"{call} {'<'.join(keep)}"


def draw_diff(sd: list[str], eng: list[str]) -> str:
    """The two sequences of draws side by side, up to a little past the first difference."""
    key = lambda d: d.split(" @ ")[0]  # noqa: E731
    first = next((i for i, (a, b) in enumerate(zip(sd, eng)) if key(a) != key(b)), min(len(sd), len(eng)))
    lines = [f"draws: showdown {len(sd)}, engine {len(eng)}; first difference at {first}"]
    for i in range(max(0, first - 3), min(max(len(sd), len(eng)), first + 4)):
        a = sd[i] if i < len(sd) else "-"
        b = eng[i] if i < len(eng) else "-"
        mark = ">>" if i == first else "  "
        lines.append(f"{mark} {i:3d} SD  {a}\n       ENG {b}")
    return "\n".join(lines)


def play(sim: RefSim, d, seed: str, t1: str, t2: str, rng: random.Random, verbose: bool) -> tuple[int, int]:
    """Play one game; returns (inputs, turn). Raises Divergence."""
    sd = Battle(sim, FORMAT, seed, ("Player 1", t1), ("Player 2", t2))
    eng = ninjask.Battle.reference(d, t1, t2, seed)
    eng.set_rng_trace(True)
    read = 0
    inputs = 0
    try:
        obs = sd.start(omniscient=True, trace=True)
        requests = dict(obs.requests)

        def check(where: str, o) -> None:
            nonlocal read
            eng_lines = eng.log("omniscient", read)
            read += len(eng_lines)
            a, b = norm(o.logs["omniscient"]), norm(eng_lines)
            sd_state, eng_state = sd.state(), json.loads(eng.projection())
            sd_seed, eng_seed = sd_state["prng"], eng.rng_seed
            sd_draws = o.rng_trace or []
            eng_draws = [d for d in eng.rng_trace() if not d.startswith("#")]
            if verbose:
                print(f"--- {where}: {len(a)} lines, {len(sd_draws)} draws, rng {'same' if sd_seed == eng_seed else 'DIFFERENT'}")
                for line in a:
                    print(f"    {line}")
            if a != b or sd_seed != eng_seed:
                diff = "\n".join(difflib.unified_diff(a, b, "showdown", "engine", lineterm="", n=2))
                sd_draws = [x for x in sd_draws if not x.startswith("#")]
                draws = draw_diff(sd_draws, eng_draws)
                key = lambda d: d.split(" @ ")[0]  # noqa: E731
                first = next((i for i, (x, y) in enumerate(zip(sd_draws, eng_draws)) if key(x) != key(y)), None)
                if sd_seed == eng_seed:
                    kind = "log only: " + shape(next((f"{x} | {y}" for x, y in zip(a, b) if x != y), "length"))[:120]
                elif first is None:
                    n = min(len(sd_draws), len(eng_draws))
                    extra = sd_draws[n:] or eng_draws[n:]
                    kind = ("showdown draws more: " if len(sd_draws) > n else "engine draws more: ") + (site(extra[0]) if extra else "")
                else:
                    kind = f"{site(sd_draws[first])}  vs  {site(eng_draws[first])}"
                raise Divergence(f"{where}\n{diff}\nrng showdown {sd_seed}\nrng engine   {eng_seed}\n{draws}", kind)
            if REQUESTS:
                for s in ("p1", "p2"):
                    want = o.requests.get(s)
                    if want is None:
                        continue
                    text = eng.request(s)
                    got = json.loads(text) if text else None
                    if got is not None:
                        got.pop("rqid", None)
                    if got != want:
                        paths = state_diff(want, got) if isinstance(got, dict) else [f": showdown {json.dumps(want)[:200]}, engine {got}"]
                        kind = "request: " + re.sub(r"\[\d+\]", "[]", paths[0].split(":")[0]) if paths else "request"
                        raise Divergence(f"{where}\n{s} requests differ:\n  " + "\n  ".join(paths[:20]), kind)
            fields = state_diff(sd_state, eng_state)
            if fields:
                kind = "state: " + re.sub(r"\[\d+\]", "[]", fields[0].split(":")[0])
                raise Divergence(f"{where}\nstate projections differ:\n  " + "\n  ".join(fields[:20]), kind)

        check("start", obs)
        while not obs.ended and obs.turn < TURN_LIMIT:
            acted = False
            for side in ("p1", "p2"):
                req = requests.get(side)
                if not req or req.get("wait"):
                    continue
                choices = team_preview_choices(req, GAME_TYPE) if req.get("teamPreview") else joint_choices(req, include_ally_targets=True, game_type=GAME_TYPE)
                choice = rng.choice(choices)
                obs = sd.choose(side, choice, omniscient=True)
                # Something hidden from the player (Imprison's 'hidden' disable, a
                # trap by an ability the request only calls maybeTrapped):
                # the request offered the choice, Showdown refuses it and asks
                # again with it marked. A player chooses again, as often as
                # that happens; the engine never sees the refused choices.
                for _ in range(8):
                    if not (obs.errors.get(side) and "[Unavailable choice]" in obs.errors[side] and obs.requests.get(side)):
                        break
                    choice = rng.choice(joint_choices(obs.requests[side], include_ally_targets=True, game_type=GAME_TYPE))
                    obs = sd.choose(side, choice, omniscient=True)
                if obs.errors.get(side):
                    raise Divergence(f"turn {obs.turn} {side} {choice!r}: Showdown rejected it: {obs.errors[side]}")
                requests[side] = None
                for s in ("p1", "p2"):
                    if obs.requests.get(s):
                        requests[s] = obs.requests[s]
                try:
                    eng.choose(side, choice)
                except ValueError as e:
                    raise Divergence(f"turn {obs.turn} {side} {choice!r}: the engine rejected it: {e}") from None
                inputs += 1
                check(f"turn {obs.turn} after {side} {choice!r}", obs)
                acted = True
                if obs.ended:
                    break
            if not acted:
                raise Divergence("no side can act")
        if obs.ended != eng.ended:
            raise Divergence(f"ended: showdown {obs.ended}, engine {eng.ended}")
        return inputs, obs.turn
    finally:
        sd.close()


# Per worker process: one reference simulator, the dex and the team pool.
_worker: dict = {}


def _init_worker(verbose: bool, require: tuple[str, ...] = (), synthetic: bool = False, pool: Path | None = None) -> None:
    sim = RefSim()
    multiprocessing.util.Finalize(None, sim.close, exitpriority=10)
    teams = verification_teams(pool)
    if synthetic:
        # Random legal teams over every supported move, ability and item, which
        # reach what no pool team uses.
        rng = random.Random("synthetic")
        teams = [supported_test_team(FORMAT, rng) for _ in range(1000)]
    # With --require, player 1's team always uses one of the given moves,
    # items or abilities: pool teams that do, and legal teams built around them.
    focus = focus_teams(FORMAT, require, teams)
    _worker.update(sim=sim, dex=engine_dex(FORMAT), teams=teams, focus=focus, verbose=verbose)


def run_game(job: tuple[int, int]) -> tuple[int, int, Divergence | None]:
    """Play game g under seed `seed`: (g, inputs, divergence or None)."""
    g, seed = job
    rng = random.Random(f"{seed}:{g}")
    teams = _worker["teams"]
    t1, t2 = rng.choice(_worker["focus"]), rng.choice(teams)
    try:
        inputs, _ = play(_worker["sim"], _worker["dex"], seed_from_int(g, salt=f"reference:{seed}"), t1, t2, rng, _worker["verbose"])
        return g, inputs, None
    except Divergence as e:
        return g, 0, e


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("--games", type=int, default=20)
    p.add_argument("--first", type=int, default=0, help="index of the first game")
    p.add_argument("--seed", type=int, default=0, help="seed for team and choice draws")
    p.add_argument("--show", type=int, default=5, help="divergences to print in full")
    p.add_argument("--workers", type=int, default=1, help="processes, each with its own Showdown")
    p.add_argument("--require", default="", help="comma-separated move, item or ability ids player 1's team must use one of")
    p.add_argument("--synthetic", action="store_true", help="random legal teams over every supported entity instead of the pools")
    p.add_argument("--pool", type=Path, help="team pool: a file or a directory of supported-*.txt (default: ninjask.default_pool())")
    p.add_argument("--format", help="format id (default: $NINJASK_FORMAT, else gen9championsvgc2026regmc)")
    p.add_argument("--requests", action="store_true", help="also compare every request (default: $NINJASK_CHECK_REQUESTS)")
    p.add_argument("--verbose", action="store_true")
    args = p.parse_args()
    # Workers import this module afresh (spawn): hand the options over in the environment.
    global FORMAT, GAME_TYPE, REQUESTS
    if args.requests:
        os.environ["NINJASK_CHECK_REQUESTS"] = "1"
        REQUESTS = True
    if args.format:
        os.environ["NINJASK_FORMAT"] = FORMAT = args.format
        GAME_TYPE = load_format(FORMAT).game_type
    require = tuple(x.strip() for x in args.require.split(",") if x.strip())
    jobs = [(g, args.seed) for g in range(args.first, args.first + args.games)]
    if args.workers > 1:
        with multiprocessing.Pool(args.workers, _init_worker, (args.verbose, require, args.synthetic, args.pool)) as pool:
            results = sorted(pool.imap_unordered(run_game, jobs, chunksize=4))
    else:
        _init_worker(args.verbose, require, args.synthetic, args.pool)
        results = [run_game(job) for job in jobs]
    counts: Counter = Counter()
    kinds: Counter = Counter()
    games_of: dict[str, list[int]] = {}
    shown = 0
    for g, inputs, e in results:
        if e is None:
            counts["matched"] += 1
            counts["inputs"] += inputs
            continue
        counts["diverged"] += 1
        kinds[e.kind] += 1
        games_of.setdefault(e.kind, []).append(g)
        counts["at start" if str(e).split("\n")[0] == "start" else "in play"] += 1
        if shown < args.show:
            shown += 1
            print(f"=== game {g} diverged: {e}\n")
    print(dict(counts))
    for k, n in kinds.most_common():
        print(f"{n:4d}  {k}  (games {games_of[k][:5]})")
    # Fail the run (and CI) when any game diverged.
    sys.exit(1 if counts["diverged"] else 0)


if __name__ == "__main__":
    main()
