#!/usr/bin/env python3
"""Chaos games: teams built from the strangest supported mechanics, played in
Showdown and the engine and compared after every input, as check_reference.py
does.

Every Pokémon gets three moves from a pool of the moves that interact the
most (Transform, Skill Swap, Role Play, Entrainment, Reflect Type, Perish
Song, Future Sight, Instruct, Copycat, Encore, Baton Pass, Shed Tail, Ally
Switch, Revival Blessing, Trick...), one attack, an ability from a similar
pool (Illusion, Trace, Imposter, Protean, Mimicry, Magic Bounce, Mummy,
Wandering Spirit, Receiver, Opportunist...) and an item from one (Red Card,
Eject Button, White Herb, Metronome, terrain seeds...). Zoroark and
Zoroark-Hisui with Illusion and Ditto with Imposter come often. Learnsets are
ignored: no move here depends on its user's species. Real teams never look
like this, which is the point: interactions that real play reaches once in a
million games come up every few dozen.

    python scripts/check_chaos.py --games 2000 --seed 1
    python scripts/check_chaos.py --game 1033 --seed 1 --verbose   # one game, logs

Exits 1 if any game diverged; each divergence (with both teams) goes to
--out.
"""

from __future__ import annotations

import argparse
import collections
import multiprocessing
import random
import sys
import traceback
from pathlib import Path

import importlib.util

import ninjask
from ninjask import to_id
from ninjask.refsim import RefSim, seed_from_int

# check_reference.py, beside this script: its play loop and divergence report.
_spec = importlib.util.spec_from_file_location("check_reference", Path(__file__).resolve().parent / "check_reference.py")
assert _spec and _spec.loader
cr = importlib.util.module_from_spec(_spec)
sys.modules["check_reference"] = cr  # worker processes unpickle Divergence by module name
_spec.loader.exec_module(cr)

FORMAT = cr.FORMAT

CHAOS_MOVES = """perishsong futuresight transform skillswap roleplay entrainment simplebeam worryseed gastroacid
instruct copycat sleeptalk encore disable torment taunt imprison batonpass shedtail allyswitch afteryou quash
trick switcheroo fling knockoff thief covet recycle uturn voltswitch partingshot chillyreception revivalblessing
healingwish wish memento destinybond spite substitute protect wideguard quickguard followme ragepowder
helpinghand trickroom magicroom wonderroom gravity tailwind haze clearsmog psychup topsyturvy powertrick
powerswap guardswap speedswap lockon magnetrise ingrain aquaring leechseed curse yawn rest snore uproar outrage
petaldance thrash counter mirrorcoat metalburst comeuppance endeavor finalgambit painsplit superfang lashout
burningjealousy expandingforce risingvoltage grassyglide terrainpulse weatherball soak magicpowder trickortreat
forestscurse reflecttype electrify throatchop tidyup""".split()
CHAOS_ABILITIES = """illusion trace imposter protean libero mimicry magicbounce mirrorarmor mummy wanderingspirit
skilllink parentalbond receiver symbiosis pickpocket magician stickyhold klutz unburden harvest cudchew ripen
intimidate defiant competitive contrary simple unaware moldbreaker prankster gooey cursedbody emergencyexit
regenerator naturalcure pressure frisk anticipation forewarn innardsout aftermath roughskin pickup telepathy
friendguard healer flowerveil sweetveil aromaveil oblivious owntempo queenlymajesty armortail psychicsurge
grassysurge electricsurge drizzle drought snowwarning sandstream cloudnine opportunist stalwart levitate
sheerforce""".split()
CHAOS_ITEMS = """choicescarf redcard ejectbutton metronome whiteherb mentalherb airballoon focussash rockyhelmet
shedshell lightclay terrainextender electricseed grassyseed mistyseed psychicseed sitrusberry lumberry
leppaberry ironball normalgem quickclaw kingsrock bindingband bigroot leftovers lifeorb focusband shellbell
widelens zoomlens brightpowder heatrock damprock smoothrock icyrock""".split()
LOCKED_FLAGS = {"failroleplay", "failskillswap", "noentrain", "notrace", "noreceiver"}
NATURES = ["Adamant", "Modest", "Jolly", "Timid", "Bold", "Careful", "Brave", "Quiet"]


def pools(d, spec) -> dict:
    """The chaos pools cut down to what the engine supports; everything it
    supports (`wide`); attacks; species; each species' Mega Stones."""
    supported = set(d.implemented_moves()) | set(d.generic_moves())
    abilities, items = set(d.supported_abilities()), set(d.supported_items())
    attacks = sorted(
        m for m in d.generic_moves() if spec.moves[m]["category"] != "Status" and spec.moves[m].get("basePower", 0) >= 60
    )
    legal = sorted(spec.team_legal_species & set(d.species_ids()))
    stones: dict[str, list[str]] = {}
    for i in sorted(items):
        for base in (spec.items.get(i, {}).get("megaStone") or {}):
            stones.setdefault(base, []).append(i)
    return {
        "moves": [m for m in CHAOS_MOVES if m in supported],
        "abilities": [a for a in CHAOS_ABILITIES if a in abilities],
        "items": [i for i in CHAOS_ITEMS if i in items],
        "attacks": attacks,
        "legal": legal,
        "all_moves": sorted(m for m in supported if m != "struggle"),
        # An ability no move or ability can pass on (Illusion, Stance Change,
        # Disguise...) only comes on a species that has it.
        "all_abilities": sorted(
            a for a in abilities - {"noability"}
            if not LOCKED_FLAGS <= set(spec.abilities.get(a, {}).get("flags", {}))
        ),
        "all_items": sorted(i for i in items if not spec.items.get(i, {}).get("megaStone")),
        "stones": stones,
    }


def chaos_team(rng: random.Random, d, spec, parts, size: int = 6, mix: str = "chaos") -> str | None:
    """A packed team of `size` chaos sets, or None if the engine refuses it.

    `mix`: "chaos" draws from the chaos pools; "wide" from everything the
    engine supports, Mega Stones included; "mixed" picks one of the two for
    each Pokémon."""
    if mix != "chaos":
        return wide_team(rng, d, spec, parts, size, mix)
    moves, abilities, items, attacks, legal = (parts[k] for k in ("moves", "abilities", "items", "attacks", "legal"))
    sets, used = [], set()
    while len(sets) < size:
        r = rng.random()
        sp = "zoroark" if r < 0.12 else "zoroarkhisui" if r < 0.2 else "ditto" if r < 0.26 else rng.choice(legal)
        base = spec.species[sp]["baseSpecies"] if sp in spec.species else sp
        if base in used:  # Species Clause
            continue
        used.add(base)
        ability = "illusion" if sp.startswith("zoroark") else "imposter" if sp == "ditto" else rng.choice(abilities)
        ms = rng.sample(moves, 3)
        ms.append(rng.choice([m for m in attacks if m not in ms]))
        if sp == "ditto":
            ms = ["transform"]
        item = rng.choice(items + [""])
        nature = rng.choice(NATURES)
        evs = ",".join(str(rng.choice([0, 0, 16, 32])) for _ in range(6))
        name = spec.species[sp]["name"] if sp in spec.species else sp
        sets.append(f"{name}||{item}|{ability}|{','.join(ms)}|{nature}|{evs}||||50|")
    team = "]".join(sets)
    return None if d.team_problems(team) else team


def wide_team(rng: random.Random, d, spec, parts, size: int, mix: str) -> str | None:
    sets, used = [], set()
    while len(sets) < size:
        chaos = mix == "mixed" and rng.random() < 0.5
        r = rng.random()
        sp = "zoroark" if r < 0.06 else "ditto" if r < 0.09 else rng.choice(parts["legal"])
        name = spec.species[sp]["name"] if sp in spec.species else sp
        base = spec.species[sp]["baseSpecies"] if sp in spec.species else sp
        if base in used:
            continue
        used.add(base)
        abilities = parts["abilities"] if chaos else parts["all_abilities"]
        own = [to_id(a) for a in spec.species[sp]["abilities"].values()] if sp in spec.species else []
        own = [a for a in own if a in d.supported_abilities()]
        if sp == "zoroark":
            ability = "illusion"
        elif sp == "ditto":
            ability = "imposter"
        elif own and rng.random() < 0.3:
            ability = rng.choice(own)
        else:
            ability = rng.choice([a for a in abilities if a in parts["all_abilities"] or not chaos])
        pool = parts["moves"] if chaos else parts["all_moves"]
        ms = rng.sample(pool, 3)
        ms.append(rng.choice([m for m in parts["attacks"] if m not in ms]))
        if sp == "ditto":
            ms = ["transform"]
        stones = parts["stones"].get(name, [])
        if stones and rng.random() < 0.5:
            item = rng.choice(stones)
            # No Pokémon that can Mega Evolve can transform first.
            if ability == "imposter":
                ability = rng.choice([a for a in parts["all_abilities"] if a != "imposter"])
            if "transform" in ms:
                ms[ms.index("transform")] = rng.choice([m for m in parts["attacks"] if m not in ms])
        else:
            item = rng.choice((parts["items"] if chaos else parts["all_items"]) + [""])
        nature = rng.choice(NATURES)
        evs = ",".join(str(rng.choice([0, 0, 16, 32])) for _ in range(6))
        sets.append(f"{name}||{item}|{ability}|{','.join(ms)}|{nature}|{evs}||||50|")
    team = "]".join(sets)
    return None if d.team_problems(team) else team


_worker: dict = {}


def _init(verbose: bool = False, mix: str = "chaos") -> None:
    sim = RefSim()
    multiprocessing.util.Finalize(None, sim.close, exitpriority=10)
    d, spec = ninjask.dex(FORMAT), ninjask.load(FORMAT)
    _worker.update(sim=sim, dex=d, spec=spec, parts=pools(d, spec), verbose=verbose, mix=mix)


def teams_for(g: int, seed: int) -> tuple[random.Random, list[str]]:
    rng = random.Random(f"chaos:{seed}:{g}")
    teams: list[str] = []
    while len(teams) < 2:
        t = chaos_team(rng, _worker["dex"], _worker["spec"], _worker["parts"], mix=_worker.get("mix", "chaos"))
        if t:
            teams.append(t)
    return rng, teams


def run_game(job: tuple[int, int]) -> tuple[int, int, str | None, str | None]:
    """Play chaos game g: (g, inputs, divergence kind or None, details)."""
    g, seed = job
    rng, teams = teams_for(g, seed)
    try:
        inputs, _ = cr.play(_worker["sim"], _worker["dex"], seed_from_int(g, salt=f"chaos:{seed}"), teams[0], teams[1], rng, _worker["verbose"])
        return g, inputs, None, None
    except cr.Divergence as e:
        return g, 0, getattr(e, "kind", "other"), f"{e}\nteams:\n{teams[0]}\n{teams[1]}"
    except Exception as e:  # noqa: BLE001 - a crash is a finding too
        kind = f"crash: {type(e).__name__}: {str(e)[:100]}"
        return g, 0, kind, f"{traceback.format_exc()[-2000:]}\nteams:\n{teams[0]}\n{teams[1]}"


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--games", type=int, default=500)
    p.add_argument("--first", type=int, default=0, help="number of the first game")
    p.add_argument("--seed", type=int, default=1, help="seed for teams and choices")
    p.add_argument("--workers", type=int, default=max(1, multiprocessing.cpu_count()))
    p.add_argument("--out", default="chaos-divergences.txt", help="where each divergence is written")
    p.add_argument("--game", type=int, help="play only this game, printing its logs (with --verbose)")
    p.add_argument("--verbose", action="store_true")
    p.add_argument("--mix", choices=["chaos", "wide", "mixed"], default="chaos",
                   help="chaos pools only, everything supported (Mega Stones too), or a mix per Pokémon")
    a = p.parse_args()

    if a.game is not None:
        _init(a.verbose, a.mix)
        g, inputs, kind, detail = run_game((a.game, a.seed))
        print(detail if kind else f"game {g} matched ({inputs} inputs)")
        sys.exit(1 if kind else 0)

    kinds: collections.Counter[str] = collections.Counter()
    first: dict[str, list[int]] = {}
    matched = inputs = 0
    jobs = [(g, a.seed) for g in range(a.first, a.first + a.games)]
    with multiprocessing.Pool(a.workers, _init, (False, a.mix)) as pool, open(a.out, "w", encoding="utf-8") as out:
        for g, n, kind, detail in pool.imap_unordered(run_game, jobs, chunksize=2):
            if kind is None:
                matched += 1
                inputs += n
                continue
            kinds[kind] += 1
            first.setdefault(kind, []).append(g)
            out.write(f"=== game {g} seed {a.seed}: {kind}\n{detail}\n\n")
            out.flush()
    print({"matched": matched, "diverged": sum(kinds.values()), "inputs": inputs})
    for k, n in kinds.most_common():
        print(f"{n:5d}  {k}  games {sorted(first[k])[:6]}")
    sys.exit(1 if kinds else 0)


if __name__ == "__main__":
    main()
