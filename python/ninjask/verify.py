"""Check the engine's outcome distributions against Showdown's.

For each test case:

1. play a reference-simulator game with random policies for a few turns and
   stop at a decision point;
2. pick a random legal joint choice;
3. draw N outcomes of that turn from Showdown (`branch`) and N from the
   engine (imported from the same Showdown state, reseeded each time);
4. compare per-feature outcome distributions: each Pokémon's HP, status,
   fainted flag, boosts, volatile effects, and the battle's turn, end and
   winner. A chi-square test of homogeneity gives a p-value per feature; a
   feature also fails outright when one simulator produces an outcome the
   other never does at a frequency that rules out chance.

p-values are corrected with Benjamini–Hochberg across all comparisons.
"""

from __future__ import annotations

import json
import random
from collections import Counter
from dataclasses import dataclass, field
from pathlib import Path

import ninjask
import numpy as np
from scipy.stats import chi2_contingency

from ninjask import dex as engine_dex
from ninjask import load as load_format
from ninjask import TEAMS_DIR, generic_test_team, supported_test_team
from ninjask.choices import joint_choices
from ninjask.policy import RandomPolicy
from ninjask.refsim import Battle, RefSim, seed_from_int


def features(state: dict) -> dict[str, str]:
    """Flatten a projection into comparable categorical features."""
    out: dict[str, str] = {
        "turn": str(state["turn"]),
        "ended": str(state["ended"]),
        "winner": str(state["winner"]),
    }
    for side in state["sides"]:
        sid = side["id"]
        out[f"{sid}.pokemonLeft"] = str(side["pokemonLeft"])
        for mon in side["pokemon"]:
            key = f"{sid}.{mon['name']}"
            out[f"{key}.hp"] = str(mon["hp"])
            out[f"{key}.status"] = str(mon["status"])
            out[f"{key}.fainted"] = str(mon["fainted"])
            out[f"{key}.active"] = str(mon["isActive"])
            boosts = {k: v for k, v in mon["boosts"].items() if v}
            out[f"{key}.boosts"] = json.dumps(boosts, sort_keys=True)
            out[f"{key}.volatiles"] = ",".join(volatile_key(k, v) for k, v in sorted(mon["volatiles"].items()))
            out[f"{key}.species"] = mon["species"]
            out[f"{key}.moveActions"] = str(mon.get("activeMoveActions"))
            out[f"{key}.item"] = str(mon.get("item"))
            out[f"{key}.ability"] = str(mon.get("ability"))
            out[f"{key}.lastMove"] = str(mon.get("lastMove"))
            out[f"{key}.moveResult"] = f"{mon.get('moveThisTurnResult')}/{mon.get('moveLastTurnResult')}"
        conds = {k: v.get("duration", v.get("layers")) for k, v in side["sideConditions"].items()}
        out[f"{sid}.sideConditions"] = json.dumps(conds, sort_keys=True)
    f = state["field"]
    out["field.weather"] = f"{f['weather']}:{f['weatherState'].get('duration')}" if f["weather"] else "none"
    out["field.terrain"] = f"{f['terrain']}:{f['terrainState'].get('duration')}" if f["terrain"] else "none"
    pseudo = {k: v.get("duration") for k, v in f["pseudoWeather"].items()}
    out["field.pseudoWeather"] = json.dumps(pseudo, sort_keys=True)
    return out


# Numeric state compared for each volatile (counters and remaining turns).
VOLATILE_DETAIL = {
    "stall": ("counter", "duration"),
    "confusion": ("time",),
    "choicelock": ("move",),
    "mustrecharge": ("duration",),
    "twoturnmove": ("move", "duration"),
    "encore": ("move", "duration"),
    "disable": ("move", "duration"),
    "yawn": ("duration",),
}


def volatile_key(name: str, state: dict) -> str:
    fields = VOLATILE_DETAIL.get(name, ())
    if not fields:
        return name
    return f"{name}(" + ",".join(str(state.get(k)) for k in fields) + ")"


@dataclass
class Comparison:
    case: int
    feature: str
    p_value: float
    support_mismatch: bool
    showdown: dict[str, int]
    engine: dict[str, int]


@dataclass
class CaseResult:
    case: int
    seed: int
    turn: int
    choices: dict[str, str]
    comparisons: list[Comparison] = field(default_factory=list)
    error: str | None = None
    state: dict | None = None
    # Volatiles on active Pokémon at the test point (coverage).
    volatiles: list[str] = field(default_factory=list)
    showdown_logs: list = field(default_factory=list)
    engine_logs: list = field(default_factory=list)


def compare_feature(a: list[str], b: list[str]) -> tuple[float, bool]:
    ca, cb = Counter(a), Counter(b)
    cats = sorted(set(ca) | set(cb))
    if len(cats) <= 1:
        return 1.0, False
    n_a, n_b = len(a), len(b)
    # Support mismatch: a value one side never produced although, at the
    # pooled frequency of both samples, missing it that often has probability
    # below 1e-7 (a threshold that allows for hundreds of thousands of
    # category checks per run).
    mismatch = False
    for c in cats:
        pooled = (ca[c] + cb[c]) / (n_a + n_b)
        if ca[c] == 0 and (1 - pooled) ** n_a < 1e-7:
            mismatch = True
        if cb[c] == 0 and (1 - pooled) ** n_b < 1e-7:
            mismatch = True
    # Pool rare categories so expected counts are reasonable.
    total = Counter(a) + Counter(b)
    keep = [c for c in cats if total[c] >= 10]
    table = [[ca[c] for c in keep], [cb[c] for c in keep]]
    rest_a = n_a - sum(table[0])
    rest_b = n_b - sum(table[1])
    if rest_a + rest_b > 0:
        table[0].append(rest_a)
        table[1].append(rest_b)
    table = [row for row in zip(*table) if sum(row) > 0]
    if len(table) <= 1:
        return 1.0, mismatch
    _, p, _, _ = chi2_contingency(np.array(table).T)
    return float(p), mismatch


# Items whose trigger depends on HP: a test point is "ripe" for them when the
# holder's HP fraction is in this range.
HP_TRIGGER = {"sitrusberry": (0.5, 0.85), "oranberry": (0.5, 0.85), "focussash": (1.0, 1.0)}


def ripe(state: dict, focus: frozenset[str]) -> bool:
    """Whether a focus move, ability or item is on the field and could act this turn."""
    for side in state["sides"]:
        for mon in side["pokemon"]:
            if not mon["isActive"] or mon["fainted"]:
                continue
            frac = mon["hp"] / mon["maxhp"]
            item = mon.get("item")
            if item in focus:
                lo, hi = HP_TRIGGER.get(item, (0.0, 1.0))
                if (lo < frac <= hi) or (lo == hi == frac):
                    return True
            if mon.get("ability") in focus:
                return True
            if any(m["id"] in focus for m in mon["moves"]):
                return True
    return False


def preview_choice(request: dict, rng: random.Random) -> str:
    """A random team order for a team-preview request."""
    n = len(request["side"]["pokemon"])
    order = rng.sample(range(1, n + 1), n)
    return "team " + "".join(str(i) for i in order[: request.get("maxChosenTeamSize") or n])


def run_case(
    sim: RefSim,
    format_id: str,
    case: int,
    teams: list[str],
    n: int,
    rng: random.Random,
    logs: bool = False,
    preview_share: float = 0.15,
    focus: frozenset[str] = frozenset(),
) -> CaseResult:
    """One comparison. With `focus`, the test point is delayed (up to 12
    decisions) until a focus move, ability or item could act."""
    seed = rng.randrange(1 << 30)
    t1, t2 = rng.sample(teams, 2)
    battle = Battle(sim, format_id, seed_from_int(seed, "verify"), ("Player 1", t1), ("Player 2", t2))
    obs = battle.start()
    game_type = load_format(format_id).game_type
    policies = {
        "p1": RandomPolicy(seed, game_type),
        "p2": RandomPolicy(seed + 1, game_type),
    }
    requests = dict(obs.requests)
    # Decisions to play before testing; 0 tests team preview and battle start
    # (lead switch-ins and their abilities).
    stop_after = 0 if rng.random() < preview_share else rng.randint(1, 10)
    decisions = 0
    choices: dict[str, str] = {}
    while True:
        if obs.ended:
            battle.close()
            return CaseResult(case, seed, obs.turn, {}, error="ended before test point")
        pending = {s: r for s, r in requests.items() if r and not r.get("wait") and not r.get("teamPreview")}
        preview = {s: r for s, r in requests.items() if r and r.get("teamPreview")}
        if preview and stop_after == 0 and len(preview) == 2:
            choices = {s: preview_choice(r, rng) for s, r in preview.items()}
            break
        if preview:
            for s, r in preview.items():
                obs = battle.choose(s, policies[s].choose(r, []))
                requests[s] = None
                for s2 in ("p1", "p2"):
                    if obs.requests[s2]:
                        requests[s2] = obs.requests[s2]
            continue
        if decisions >= stop_after and pending:
            # A mid-turn switch request (U-turn, Eject Button) has the rest of
            # the turn queued, which a state import can't carry: play on.
            current = battle.state()
            mid_turn = current.get("queueLength", 0) > 0
            if not mid_turn and (not focus or decisions >= stop_after + 12 or ripe(current, focus)):
                choices = {
                    s: rng.choice(joint_choices(r, game_type=game_type)) for s, r in pending.items()
                }
                break
        for s, r in pending.items():
            for _ in range(30):
                obs = battle.choose(s, policies[s].choose(r, []))
                if obs.errors[s] is None:
                    break
                if obs.requests[s]:
                    r = obs.requests[s]
            requests[s] = None
            for s2 in ("p1", "p2"):
                if obs.requests[s2]:
                    requests[s2] = obs.requests[s2]
            if obs.ended:
                break
        decisions += 1
    return check_point(sim, format_id, battle, case, seed, choices, n, logs)


def check_point(
    sim: RefSim,
    format_id: str,
    battle: Battle,
    case: int,
    seed: int,
    choices: dict[str, str],
    n: int,
    logs: bool = False,
) -> CaseResult:
    """Import the battle's current state into the engine and compare it: the
    import must restore the projection exactly, and `n` re-rolls of the turn
    under `choices` must give the outcomes Showdown's copy gives. Closes `battle`."""
    edex = engine_dex(format_id)
    state = battle.state()
    result = CaseResult(case, seed, state["turn"], choices, state=state)
    result.volatiles = sorted(
        {v for side in state["sides"] for mon in side["pokemon"] if mon["isActive"] for v in mon["volatiles"]}
        | {f"side:{c}" for side in state["sides"] for c in side["sideConditions"]}
        | {f"slot:{c}" for side in state["sides"] for conds in side["slotConditions"] for c in conds}
        | {f"field:{c}" for c in state["field"]["pseudoWeather"]}
        | {f"abilityState:{k}" for side in state["sides"] for mon in side["pokemon"] for k in mon["abilityState"]}
    )
    seeds = [seed_from_int(seed * 1000 + i, "branch") for i in range(n)]
    try:
        ps = sim.call(cmd="branch", id=battle.id, choices=choices, seeds=seeds, logs=logs)["results"]
    finally:
        battle.close()
    if not all(r["accepted"] for r in ps):
        result.error = f"Showdown rejected the choice: {ps[0]['errors']}"
        return result
    ps_feats = [features(r["state"]) for r in ps]
    if logs:
        result.showdown_logs = [r["spectator"] for r in ps]
    eng_feats = []
    state_text = json.dumps(state)
    for i in range(n):
        try:
            eb = ninjask.Battle.from_projection(edex, state_text, seed * 1000 + i)
            eb.set_logging(logs)
            if i == 0:
                # The import must restore every field the projection holds.
                diff = restore_diff(state, json.loads(eb.projection()))
                if diff:
                    result.error = "import differs from the projection: " + "; ".join(diff[:6])
                    return result
            for s, c in choices.items():
                eb.choose(s, c)
            if logs:
                result.engine_logs.append(eb.log("omniscient", 0))
        except ValueError as e:
            result.error = f"engine rejected or failed: {e}"
            return result
        eng_feats.append(features(json.loads(eb.projection())))
    keys = sorted(set(ps_feats[0]) | set(eng_feats[0]))
    for k in keys:
        a = [f.get(k, "?") for f in ps_feats]
        b = [f.get(k, "?") for f in eng_feats]
        p, mismatch = compare_feature(a, b)
        result.comparisons.append(Comparison(case, k, p, mismatch, dict(Counter(a)), dict(Counter(b))))
    return result


def restore_diff(a, b, path: str = "") -> list[str]:
    """Paths where two projections differ (the PRNG state aside; at team
    preview Showdown's queue holds its battle-start action, which the engine
    queues itself when the teams are chosen)."""
    if isinstance(a, dict) and isinstance(b, dict):
        out: list[str] = []
        for k in sorted(set(a) | set(b)):
            if path == "" and (k == "prng" or (k == "queueLength" and a.get("requestState") == "teampreview")):
                continue
            if k not in a or k not in b:
                out.append(f"{path}.{k}: showdown {json.dumps(a.get(k, '<none>'))}, engine {json.dumps(b.get(k, '<none>'))}")
            else:
                out += restore_diff(a[k], b[k], f"{path}.{k}")
        return out
    if isinstance(a, list) and isinstance(b, list) and len(a) == len(b):
        return [d for i, (x, y) in enumerate(zip(a, b)) for d in restore_diff(x, y, f"{path}[{i}]")]
    return [] if a == b else [f"{path}: showdown {json.dumps(a)}, engine {json.dumps(b)}"]


def benjamini_hochberg(pvals: list[float], alpha: float = 0.05) -> list[bool]:
    m = len(pvals)
    order = sorted(range(m), key=lambda i: pvals[i])
    rejected = [False] * m
    k_max = 0
    for rank, i in enumerate(order, start=1):
        if pvals[i] <= alpha * rank / m:
            k_max = rank
    for rank, i in enumerate(order, start=1):
        if rank <= k_max:
            rejected[i] = True
    return rejected


def significant(results: list[CaseResult]) -> list[Comparison]:
    """The comparisons that fail after correcting the p-values across all of them."""
    comps = [cmp for r in results for cmp in r.comparisons]
    flagged = benjamini_hochberg([cmp.p_value for cmp in comps])
    return [cmp for cmp, f in zip(comps, flagged) if f or cmp.support_mismatch]


# The default for `--teams pool`: a pool of Reg M-C team sheets with placeholder spreads (not shipped; pass --pool).
SHEETS_POOL = TEAMS_DIR / "gen9championsvgc2026regmc-sheets-placeholder.txt"


def supported_pool_teams(format_id: str, pool: Path = SHEETS_POOL) -> list[str]:
    """The teams of a pool file (one packed team per line) the engine fully supports."""
    d = engine_dex(format_id)
    return [t for t in pool.read_text(encoding="utf-8").splitlines() if t.strip() and not d.team_problems(t)]


def verify(
    format_id: str,
    cases: int,
    n: int,
    seed: int = 0,
    team_pool: int = 40,
    teams: str = "supported",
    focus: frozenset[str] = frozenset(),
    pool_file: Path | None = None,
) -> dict:
    """Run `cases` comparisons. `teams` picks the test teams: "supported"
    (generated from everything the engine supports), "pool" (real Reg M-C
    teams the engine fully supports) or "generic" (no abilities, items or
    ported moves). `focus` concentrates supported teams on these move,
    ability and item ids. `pool_file` is the team file for "pool" (default
    `SHEETS_POOL`)."""
    rng = random.Random(seed)
    if teams == "supported":
        pool = [supported_test_team(format_id, rng, focus=focus) for _ in range(team_pool)]
    elif teams == "pool":
        pool = supported_pool_teams(format_id, pool_file or SHEETS_POOL)
        if not pool:
            raise ValueError("no pool team is fully supported yet")
    else:
        pool = [generic_test_team(format_id, rng) for _ in range(team_pool)]
    results: list[CaseResult] = []
    with RefSim() as sim:
        for c in range(cases):
            results.append(run_case(sim, format_id, c, pool, n, rng, focus=focus))
    comps = [cmp for r in results for cmp in r.comparisons]
    failures = significant(results)
    coverage = Counter(v for r in results if not r.error for v in r.volatiles)
    return {
        "format": format_id,
        "cases": cases,
        "outcomes_per_case": n,
        "volatile_coverage": dict(sorted(coverage.items())),
        "errors": [{"case": r.case, "seed": r.seed, "error": r.error} for r in results if r.error],
        "comparisons": len(comps),
        "failures": [
            {
                "case": f.case,
                "feature": f.feature,
                "p": f.p_value,
                "support_mismatch": f.support_mismatch,
                "showdown": f.showdown,
                "engine": f.engine,
                "choices": results[f.case].choices,
                "seed": results[f.case].seed,
                "state": results[f.case].state,
            }
            for f in failures
        ],
    }
