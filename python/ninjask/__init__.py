"""Ninjask: a fast Pokémon Champions VGC doubles simulator, checked against Pokémon Showdown.

The compiled classes, `Dex`, `Battle` and `Tracker`, are re-exported here
(see `ninjask._extension` for where they come from). Around them: format
data (`format`), test teams, game loops (`play_engine_game`, `play`), the
Showdown reference client (`refsim`) and the parity checks (`verify`,
`tracker_check`, `world_check`)."""

from __future__ import annotations

import json
import os
import random
from dataclasses import dataclass, field
from functools import lru_cache
from pathlib import Path
from typing import TYPE_CHECKING

from ninjask._extension import module as _compiled

if TYPE_CHECKING:
    from ninjask._core import (
        ACTION_FIELDS,
        ACTION_WIDTH,
        BATTLE_IDS_LEN,
        BATTLE_OBS_LEN,
        DEFAULT_MAX_PATHS,
        IDS_LEN,
        OBS_LEN,
        Batch,
        Battle,
        BattleStateKey,
        Dex,
        EnumerationMode,
        Outcome,
        Tracker,
    )
else:
    Dex = _compiled.Dex
    Battle = _compiled.Battle
    Tracker = _compiled.Tracker
    Batch = _compiled.Batch
    # Layouts of Tracker.encode(), encode_ids(), Battle.legal_actions() and
    # Batch.observe() (ninjask::encode).
    OBS_LEN = _compiled.OBS_LEN
    IDS_LEN = _compiled.IDS_LEN
    ACTION_FIELDS = _compiled.ACTION_FIELDS
    ACTION_WIDTH = _compiled.ACTION_WIDTH
    # Layouts of Battle.encode() and encode_ids(). None from a build that
    # predates them, so that such a build still imports the package.
    BATTLE_OBS_LEN = getattr(_compiled, "BATTLE_OBS_LEN", None)
    BATTLE_IDS_LEN = getattr(_compiled, "BATTLE_IDS_LEN", None)
    DEFAULT_MAX_PATHS = _compiled.DEFAULT_MAX_PATHS
    Outcome = _compiled.Outcome
    EnumerationMode = _compiled.EnumerationMode
    BattleStateKey = _compiled.BattleStateKey

from ninjask.format import export_path, load  # noqa: E402
from ninjask.ids import to_id  # noqa: E402
from ninjask.paths import FIXTURE_POOL, TEAMS_DIR  # noqa: E402
from ninjask.policy import Policy  # noqa: E402

__all__ = [
    "ACTION_FIELDS",
    "ACTION_WIDTH",
    "BATTLE_IDS_LEN",
    "BATTLE_OBS_LEN",
    "Batch",
    "Battle",
    "BattleStateKey",
    "DEFAULT_MAX_PATHS",
    "EnumerationMode",
    "Dex",
    "EngineGameResult",
    "FIXTURE_POOL",
    "IDS_LEN",
    "MAX_RETRIES",
    "OBS_LEN",
    "Outcome",
    "Policy",
    "TEAMS_DIR",
    "TURN_LIMIT",
    "Tracker",
    "default_pool",
    "dex",
    "export_path",
    "focus_teams",
    "generic_test_team",
    "load",
    "play_engine_game",
    "supported_test_team",
    "to_id",
    "verification_teams",
]

TURN_LIMIT = 150
MAX_RETRIES = 25


@lru_cache(maxsize=8)
def dex(format_id: str) -> Dex:
    return Dex.load(str(export_path(format_id)))  # exports the format if missing


@dataclass
class EngineGameResult:
    winner: str | None  # "p1", "p2" or None
    turns: int
    rejected: dict[str, int] = field(default_factory=lambda: {"p1": 0, "p2": 0})
    rejections: list[tuple[str, str, str]] = field(default_factory=list)


def play_engine_game(
    format_id: str,
    seed: int,
    team1: str,
    team2: str,
    policy1: Policy,
    policy2: Policy,
    log: bool = False,
    ots: bool = False,
) -> EngineGameResult:
    """Play one game in the engine with policies that read Showdown requests.

    Sheets are open when `ots` is set or the format forces them open
    (`FormatSpec.open_team_sheets`), so selecting an open-sheets format
    is enough."""
    b = Battle(dex(format_id), team1, team2, seed)
    b.set_logging(log)
    if ots or load(format_id).open_team_sheets:
        b.show_team_sheets()
    policies = {"p1": policy1, "p2": policy2}
    seen = {"p1": 0, "p2": 0}
    rejected = {"p1": 0, "p2": 0}
    rejections: list[tuple[str, str, str]] = []
    while not b.ended and b.turn < TURN_LIMIT:
        acted = False
        for side in ("p1", "p2"):
            if not b.needs_choice(side):
                continue
            req = json.loads(b.request(side))
            new_log = b.log(side, seen[side]) if log else []
            seen[side] = b.log_len()
            for attempt in range(MAX_RETRIES + 1):
                # A retry gets no lines: the policy has seen them.
                choice = policies[side].choose(req, new_log if attempt == 0 else []) if attempt < MAX_RETRIES else "default"
                try:
                    b.choose(side, choice)
                    break
                except ValueError as e:
                    rejected[side] += 1
                    rejections.append((side, choice, str(e)))
                    # An "[Unavailable choice]" updates the request with what
                    # it revealed (a hidden disable or trap).
                    req = json.loads(b.request(side))
            acted = True
            if b.ended:
                break
        if not acted:
            raise RuntimeError("no side can act but the battle has not ended")
    winner = None
    if b.ended and b.winner:
        winner = "p1" if b.winner == "Player 1" else "p2"
    return EngineGameResult(winner, b.turn, rejected, rejections)


def generic_test_team(
    format_id: str, rng: random.Random, size: int = 4, moves_per_mon: int = 4, species_pool: list[str] | None = None
) -> str:
    """A packed team the engine fully supports: No Ability, no item, generic moves.

    Species come from Reg M-C's legal list unless given; moves are drawn from
    each species' learnset restricted to moves the engine treats as generic.
    """
    d = dex(format_id)
    generic = set(d.generic_moves())
    regmc = load("gen9championsvgc2026regmc")
    known = set(d.species_ids())
    pool = species_pool or sorted(regmc.team_legal_species & known)
    natures = ["Adamant", "Modest", "Jolly", "Timid", "Bold", "Careful", "Hardy", "Brave", "Quiet"]
    sets = []
    chosen: set[str] = set()
    while len(sets) < size:
        sp = rng.choice(pool)
        # Species Clause: one of each base species (formes share a name).
        base = to_id(regmc.species[sp]["baseSpecies"]) if sp in regmc.species else sp
        if base in chosen:
            continue
        learn = sorted(set(regmc.learnset(sp)) & generic)
        damaging = [m for m in learn if regmc.moves[m]["category"] != "Status"]
        if len(damaging) < 2:
            continue
        moves = rng.sample(damaging, min(moves_per_mon, len(damaging)))
        chosen.add(base)
        name = regmc.species[sp]["name"]
        points = [0] * 6
        for stat in rng.sample(range(6), 3):
            points[stat] = rng.choice([2, 16, 32])
        while sum(points) > 66:
            i = max(range(6), key=lambda k: points[k])
            points[i] -= 2
        evs = ",".join(str(p) if p else "" for p in points)
        moves_str = ",".join(regmc.moves[m]["name"].replace(" ", "").replace("-", "") for m in moves)
        sets.append(f"{name}|||noability|{moves_str}|{rng.choice(natures)}|{evs}||||50|")
    return "]".join(sets)


def supported_test_team(
    format_id: str,
    rng: random.Random,
    size: int = 4,
    species_pool: list[str] | None = None,
    focus: frozenset[str] = frozenset(),
) -> str:
    """A packed team using only mechanics the engine supports, chosen to exercise them.

    Like `generic_test_team`, but each Pokémon may also get ported moves
    (Protect, Fake Out, Tailwind, weather...), a generic status move, a
    supported ability from its own ability list, and a supported item (a
    Mega Stone for its species, or any other item the engine ports). Sets the engine
    would reject (for instance a Mega whose ability isn't ported) are redrawn.

    `focus` lists move, ability and item ids to concentrate on: they are
    picked whenever a Pokémon can have them, so verification exercises them.
    """
    d = dex(format_id)
    generic = set(d.generic_moves())
    implemented = set(d.implemented_moves())
    abilities = set(d.supported_abilities()) - {"noability"}
    items = set(d.supported_items())
    regmc = load("gen9championsvgc2026regmc")
    known = set(d.species_ids())
    pool = species_pool or sorted(regmc.team_legal_species & known)
    held = sorted(i for i in items if not regmc.items.get(i, {}).get("megaStone"))
    natures = ["Adamant", "Modest", "Jolly", "Timid", "Bold", "Careful", "Hardy", "Brave", "Quiet"]
    sets: list[str] = []
    chosen: set[str] = set()
    while len(sets) < size:
        sp = rng.choice(pool)
        # Species Clause: one of each base species (formes share a name).
        base = to_id(regmc.species[sp]["baseSpecies"]) if sp in regmc.species else sp
        if base in chosen:
            continue
        learn = set(regmc.learnset(sp))
        damaging = sorted(m for m in learn & generic if regmc.moves[m]["category"] != "Status")
        usable = sorted(learn & (generic | implemented))
        # A species with few attacks (Ditto knows only Transform) comes only
        # when it carries a focus move or ability.
        own_abilities = {to_id(a) for a in regmc.species[sp]["abilities"].values()}
        if len(damaging) < 2 and not (usable and (learn & focus or own_abilities & focus)):
            continue
        status = sorted(m for m in learn & generic if regmc.moves[m]["category"] == "Status")
        special = sorted(learn & implemented)
        focus_moves = sorted(learn & focus & (implemented | generic))
        moves = rng.sample(focus_moves, min(len(focus_moves), 2))
        special = [m for m in special if m not in moves]
        n_special = max(0, rng.choice([0, 1, 1, 2, 2]) - len(moves))
        moves += rng.sample(special, min(len(special), n_special))
        if status and rng.random() < 0.4:
            moves.append(rng.choice(status))
        damaging = [m for m in damaging if m not in moves]
        moves += rng.sample(damaging, min(4 - len(moves), len(damaging)))
        if not moves:
            moves.append(rng.choice(usable))
        name = regmc.species[sp]["name"]
        options = [to_id(a) for a in regmc.species[sp]["abilities"].values() if to_id(a) in abilities]
        focus_abilities = [a for a in options if a in focus]
        if focus_abilities:
            ability = rng.choice(focus_abilities)
        else:
            ability = rng.choice(options) if options and rng.random() < 0.8 else "noability"
        stones = sorted(
            i for i, it in regmc.items.items() if i in items and name in (it.get("megaStone") or {})
        )
        focus_items = [i for i in held + stones if i in focus]
        roll = rng.random()
        if focus_items and roll < 0.9:
            item = rng.choice(focus_items)
        elif stones and roll < 0.5:
            item = rng.choice(stones)
        elif held and roll < 0.85:
            item = rng.choice(held)
        else:
            item = ""
        points = [0] * 6
        for stat in rng.sample(range(6), 3):
            points[stat] = rng.choice([2, 16, 32])
        while sum(points) > 66:
            i = max(range(6), key=lambda k: points[k])
            points[i] -= 2
        evs = ",".join(str(p) if p else "" for p in points)
        moves_str = ",".join(regmc.moves[m]["name"].replace(" ", "").replace("-", "") for m in moves)
        packed = f"{name}||{item}|{ability}|{moves_str}|{rng.choice(natures)}|{evs}||||50|"
        if d.team_problems(packed):
            continue
        chosen.add(base)
        sets.append(packed)
    return "]".join(sets)


def default_pool() -> Path:
    """`$NINJASK_POOL`, else the `supported-*.txt` pools in `TEAMS_DIR` if
    there are any (`scripts/report_support.py` writes each pool's
    engine-supported teams there), else the engine's fixture pool."""
    if env := os.environ.get("NINJASK_POOL"):
        return Path(env)
    if any(TEAMS_DIR.glob("supported-*.txt")):
        return TEAMS_DIR
    return FIXTURE_POOL


def verification_teams(pool: Path | str | None = None) -> list[str]:
    """The packed teams of a pool, without duplicates: a file with one team per
    line, or a directory whose `supported-*.txt` files are read in name order.
    The default is `default_pool()`."""
    path = Path(pool) if pool else default_pool()
    files = sorted(path.glob("supported-*.txt")) if path.is_dir() else [path]
    seen: set[str] = set()
    out: list[str] = []
    for f in files:
        for t in f.read_text(encoding="utf-8").splitlines():
            if t and t not in seen:
                seen.add(t)
                out.append(t)
    return out


def _key(s: str) -> str:
    return "".join(c for c in s.lower() if c.isalnum())


def focus_teams(
    format_id: str, require: tuple[str, ...], teams: list[str], synthetic: int = 200, seed: int = 0
) -> list[str]:
    """Teams that use one of the `require` ids (moves, abilities or items).

    An entry may join ids with `+` (`sleeptalk+rest`: one Pokémon has both, as
    Sleep Talk is only useful with a way to fall asleep) and such groups with
    `&` (`skillswap+pollenpuff&kangaskhanite`: the team also has a Pokémon
    with the second group).

    Pool teams that name one, plus, for each id, up to `synthetic` legal teams
    built around it (`supported_test_team`): one Pokémon that can have it
    (learns the move, has the ability, or any Pokémon for an item; a Mega
    Stone's own species), then three others. Most of the legal set appears on
    no real team, so this is how verification reaches it. Raises if the
    engine can't build any team with an id.
    """
    if not require:
        return teams
    entries = [[tuple(g.split("+")) for g in r.split("&")] for r in require]
    out = [t for t in teams if any(all(i in _key(t) for g in e for i in g) for e in entries)]
    spec = load(format_id)
    d = dex(format_id)
    ported = set(d.implemented_moves()) | set(d.generic_moves()) | set(d.supported_abilities()) | set(d.supported_items())
    unported = [i for e in entries for g in e for i in g if i not in ported]
    if unported:
        raise SystemExit(f"not supported by the engine yet: {', '.join(unported)}")
    rng = random.Random(f"focus:{seed}:{','.join(require)}")
    legal = sorted(spec.team_legal_species & set(d.species_ids()))

    def can_have(species: str, r: str) -> bool:
        item = spec.items.get(r, {})
        stone = item.get("megaStone") or {}
        if r in spec.moves:
            return r in spec.learnset(species)
        if stone:
            return spec.species[species]["name"] in stone
        if item:
            return True
        return r in {_key(a) for a in spec.species[species]["abilities"].values()}

    def has_all(team_set: str, g: tuple[str, ...]) -> bool:
        fields = team_set.split("|")
        own = {_key(m) for m in fields[4].split(",")} | {_key(fields[2]), _key(fields[3])}
        return all(i in own for i in g)

    for e in entries:
        capable = [[s for s in legal if all(can_have(s, r) for r in g)] for g in e]
        made = 0
        for _ in range(synthetic * 5):
            if made >= synthetic or not all(capable):
                break
            ids = frozenset(i for g in e for i in g)
            members = [supported_test_team(format_id, rng, size=1, species_pool=c, focus=frozenset(g)) for c, g in zip(capable, e)]
            if not all(has_all(m, g) for m, g in zip(members, e)):
                continue
            rest = supported_test_team(format_id, rng, size=4 - len(members), focus=ids)
            team = "]".join(members) + "]" + rest
            bases = [_key(spec.species[_key(s.split("|")[0])]["baseSpecies"]) for s in team.split("]")]
            if len(set(bases)) < len(bases):
                continue
            out.append(team)
            made += 1
    if not out:
        raise SystemExit(f"no team can use any of {require} (not ported, or no legal user)")
    return out
