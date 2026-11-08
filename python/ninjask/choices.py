"""Enumerate legal choices from a Showdown request (doubles and singles).

A request is the JSON a player receives in ``|request|``. Choice strings use
Showdown's syntax: ``move 2 1``, ``move 1 -2 mega``, ``switch 3``, ``pass``,
joined with ``, `` across active slots, and ``team 1234`` at preview.

Targets in doubles: foes are 1 and 2 (their slots), allies are -1 and -2
(our own slots). Moves whose target type needs no choice take no target.
"""

from __future__ import annotations

import itertools
from dataclasses import dataclass

# Target types that require choosing a target location in doubles.
_FOE_OR_ALLY = {"normal", "any"}
_FOE_ONLY = {"adjacentFoe"}
_ALLY_ONLY = {"adjacentAlly"}
_ALLY_OR_SELF = {"adjacentAllyOrSelf"}


@dataclass(frozen=True)
class SlotAction:
    kind: str  # "move", "switch", "pass"
    index: int = 0  # move index (1-based) or team slot (1-based) to switch in
    target: int | None = None
    mega: bool = False

    def __str__(self) -> str:
        if self.kind == "pass":
            return "pass"
        if self.kind == "switch":
            return f"switch {self.index}"
        s = f"move {self.index}"
        if self.target is not None:
            s += f" {self.target}"
        if self.mega:
            s += " mega"
        return s


def _fainted(mon: dict) -> bool:
    return mon["condition"].endswith(" fnt") or mon["condition"].startswith("0")


def move_targets(target_type: str, slot: int, n_active: int, include_ally: bool) -> list[int | None]:
    """Valid target locations for a move used from our ``slot`` (0-based)."""
    if n_active == 1:
        return [None]
    own_loc = -(slot + 1)
    allies = [-(i + 1) for i in range(n_active) if i != slot]
    foes = [i + 1 for i in range(n_active)]
    if target_type in _FOE_OR_ALLY:
        return foes + (allies if include_ally else [])
    if target_type in _FOE_ONLY:
        return foes
    if target_type in _ALLY_ONLY:
        return allies
    if target_type in _ALLY_OR_SELF:
        return [own_loc] + allies
    return [None]


def slot_options(
    request: dict, slot: int, include_ally_targets: bool = False, game_type: str | None = None
) -> list[SlotAction]:
    """Legal actions for one active slot, ignoring interactions between slots.

    ``game_type`` is the battle's Showdown game type (``"doubles"`` or
    ``"singles"``) when the caller knows it. It matters only for target
    enumeration: a request names just the Pokémon still on the field, so a
    doubles battle down to one active Pokémon sends a one-entry ``active``
    list that is indistinguishable from a singles request — yet the engine
    demands a target for a single-target move there and forbids one in
    singles (its validator keys on the slots the battle fields, not on
    who is left standing). Without ``game_type`` the request is read as
    before: one entry means singles.
    """
    if game_type not in (None, "doubles", "singles"):
        raise ValueError(f"unknown game type {game_type!r}")
    side = request["side"]["pokemon"]
    n_active = len(request.get("active", [])) or len(request.get("forceSwitch", []))
    # The slots the battle fields per side, for target enumeration.
    if game_type == "doubles":
        n_slots = max(n_active, 2)
    elif game_type == "singles":
        n_slots = 1
    else:
        n_slots = n_active
    bench = [i + 1 for i, mon in enumerate(side) if not mon.get("active") and not _fainted(mon)]
    force = request.get("forceSwitch")
    if force is not None:
        if not force[slot]:
            return [SlotAction("pass")]
        if side[slot].get("reviving"):
            # Revival Blessing: pick a fainted teammate to revive, even one
            # that fainted in its slot this turn.
            fainted = [i + 1 for i, mon in enumerate(side) if _fainted(mon)]
            return [SlotAction("switch", i) for i in fainted] or [SlotAction("pass")]
        # "pass" is allowed only when there aren't enough replacements;
        # joint_choices enforces that.
        return [SlotAction("switch", i) for i in bench] + [SlotAction("pass")]
    active = request["active"][slot]
    mon = side[slot]
    if _fainted(mon) or mon.get("commanding"):
        return [SlotAction("pass")]
    options: list[SlotAction] = []
    mega_flags = [False, True] if active.get("canMegaEvo") else [False]
    for i, mv in enumerate(active["moves"], start=1):
        if mv.get("disabled"):
            continue
        # Locked moves (charging turn, recharge, rampage) come without a
        # target type and must be chosen without a target.
        if "target" not in mv:
            targets: list[int | None] = [None]
        else:
            targets = move_targets(mv["target"], slot, n_slots, include_ally_targets)
        # A Struggle choice ignores "mega" (Showdown neither Mega Evolves
        # nor checks that it could): "move 1" is the one choice.
        megas = [False] if mv.get("id") == "struggle" else mega_flags
        for t, mega in itertools.product(targets, megas):
            options.append(SlotAction("move", i, t, mega))
    if not active.get("trapped"):
        options.extend(SlotAction("switch", i) for i in bench)
    if not options:
        # Showdown accepts move 1 when nothing else is possible (Struggle).
        options.append(SlotAction("move", 1))
    return options


def joint_choices(
    request: dict, include_ally_targets: bool = False, game_type: str | None = None
) -> list[str]:
    """All legal joint choices for a move or switch request.

    Removes combinations that Showdown rejects: two slots switching to the
    same Pokémon, two Mega Evolutions in one turn, and a forced switch that
    passes while another slot could still switch in.

    ``game_type`` is forwarded to `slot_options` (see there): pass
    ``"doubles"`` whenever the battle is a doubles battle, so an endgame
    request with one active Pokémon still enumerates move targets.
    """
    n = len(request.get("active", [])) or len(request.get("forceSwitch", []))
    per_slot = [slot_options(request, s, include_ally_targets, game_type) for s in range(n)]
    force = request.get("forceSwitch")
    combos = []
    for combo in itertools.product(*per_slot):
        switches = [a.index for a in combo if a.kind == "switch"]
        if len(switches) != len(set(switches)):
            continue
        if sum(a.mega for a in combo) > 1:
            continue
        combos.append((len(switches), combo))
    if force is not None and combos:
        # Showdown requires filling as many forced slots as possible.
        most = max(n for n, _ in combos)
        combos = [(n, c) for n, c in combos if n == most]
    out = [", ".join(str(a) for a in combo) for _, combo in combos]
    return out


def team_preview_choices(request: dict, game_type: str | None = None) -> list[str]:
    """Every ordered pick of the brought Pokémon; the first two lead (the
    first one in singles, ``game_type="singles"``), the rest in team order."""
    if game_type not in (None, "doubles", "singles"):
        raise ValueError(f"unknown game type {game_type!r}")
    size = len(request["side"]["pokemon"])
    bring = request.get("maxChosenTeamSize") or size
    n_leads = 1 if game_type == "singles" else 2
    out = []
    for picked in itertools.combinations(range(1, size + 1), bring):
        for leads in itertools.combinations(picked, min(n_leads, bring)):
            rest = [p for p in picked if p not in leads]
            out.append("team " + "".join(str(p) for p in (*leads, *rest)))
    return out
