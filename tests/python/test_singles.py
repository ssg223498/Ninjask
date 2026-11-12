"""Singles (Champions BSS, `gen9championsbssregmc`): one active Pokémon a side,
bring six and pick three, no target locations.

The engine fields as many slots as the format's `gameType` says; these tests
cover what differs from doubles. Those that compare with Showdown skip when
it is not built, and those that need the format export skip without it.
"""

import importlib.util
import json
import random

import pytest

ninjask = pytest.importorskip("ninjask")

from ninjask.choices import joint_choices, team_preview_choices  # noqa: E402
from ninjask.paths import ENGINE_ROOT, FIXTURE_POOL, SHOWDOWN_DIR  # noqa: E402

FORMAT = "gen9championsbssregmc"


def _mon(name):
    return {"ident": f"p1: {name}", "details": f"{name}, L50", "condition": "100/100", "active": False}


def test_singles_team_preview_lets_each_brought_pokemon_lead():
    """In singles the first Pokémon sent leads alone: every brought one can
    be it. (The doubles reading, two leads, never lets the third lead.)"""
    req = {"teamPreview": True, "maxChosenTeamSize": 3, "side": {"pokemon": [_mon(str(i)) for i in range(6)]}}
    plans = team_preview_choices(req, game_type="singles")
    assert len(plans) == 20 * 3 == len(set(plans))
    assert {"team 123", "team 213", "team 312"} <= set(plans)
    assert {p.split()[1][0] for p in plans} == set("123456")
    # The doubles enumeration is unchanged.
    assert len(team_preview_choices(req)) == len(team_preview_choices(req, game_type="doubles")) == 20 * 3


@pytest.fixture(scope="module")
def d():
    try:
        return ninjask.dex(FORMAT)
    except Exception as e:  # no export and no Showdown to make one
        pytest.skip(f"no {FORMAT} export: {e}")


def _teams():
    return FIXTURE_POOL.read_text(encoding="utf-8").splitlines()


def test_engine_plays_singles(d):
    """One active slot a side: the log, the requests and the choices say so."""
    t = _teams()
    b = ninjask.Battle(d, t[0], t[1], seed=3)
    b.set_logging(True)
    assert "|gametype|singles" in b.log("omniscient", 0)
    req = json.loads(b.request("p1"))
    assert req["teamPreview"] and req["maxChosenTeamSize"] == 3
    # Every brought Pokémon can lead, as in the request-side enumeration.
    assert b.legal_choices("p1") == team_preview_choices(req, game_type="singles")
    b.choose("p1", "team 312")
    b.choose("p2", "team 123")
    req = json.loads(b.request("p1"))
    assert len(req["active"]) == 1
    assert len([m for m in req["side"]["pokemon"] if m["active"]]) == 1
    assert b.legal_choices("p1") == joint_choices(req, game_type="singles")
    with pytest.raises(ValueError):
        b.choose("p1", "move 1, move 1")  # one slot only
    rng = random.Random(0)
    while not b.ended and b.turn < 200:
        for side in ("p1", "p2"):
            if b.needs_choice(side):
                choices = b.legal_choices(side)
                assert all("," not in c for c in choices)
                b.choose_index(side, rng.randrange(len(choices)))
    assert b.ended
    assert not any(line.startswith(("|switch|p1b", "|switch|p2b")) for line in b.log("omniscient", 0))


def _check_reference(monkeypatch):
    """scripts/check_reference.py, loaded for the singles format."""
    monkeypatch.setenv("NINJASK_FORMAT", FORMAT)
    spec = importlib.util.spec_from_file_location("check_reference_singles", ENGINE_ROOT / "scripts" / "check_reference.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    assert module.FORMAT == FORMAT and module.GAME_TYPE == "singles"
    return module


@pytest.mark.skipif(not (SHOWDOWN_DIR / "dist").exists(), reason="Pokémon Showdown not built (scripts/setup_showdown.sh)")
def test_singles_reference_games_match_showdown(d, monkeypatch):
    """Reference mode against Showdown, input by input (logs, requests'
    effects, state and every random draw). A move chosen without a target
    is aimed by getRandomTarget, which in singles takes the foe's slot
    without a draw."""
    from ninjask.refsim import RefSim, seed_from_int

    cr = _check_reference(monkeypatch)
    t = _teams()
    with RefSim() as sim:
        for g in range(8):
            rng = random.Random(f"singles:{g}")
            cr.play(sim, d, seed_from_int(g, salt="singles"), t[g % len(t)], t[(g + 1) % len(t)], rng, False)


SHOWDOWN = pytest.mark.skipif(not (SHOWDOWN_DIR / "dist").exists(), reason="Pokémon Showdown not built (scripts/setup_showdown.sh)")

# Garchomp leads team 1 (Choice Scarf; Dragon Claw and Stomping Tantrum aim
# at one target, Earthquake and Rock Slide at all adjacent ones).
CANDIDATES = [
    "move 1", "move 1 1", "move 4 1", "move 1 -1", "move 1 2", "move 1 -2", "move 2 1", "move 3 -1",
    "move 1, move 2", "move 1 1, switch 2", "move 1 mega", "switch 2", "switch 1", "switch 4", "pass",
]


@SHOWDOWN
@pytest.mark.parametrize("choice", CANDIDATES)
def test_singles_choices_are_accepted_as_showdown_accepts_them(d, choice):
    """The same choice strings refused or taken, by each simulator: more
    parts than active slots are refused, and a target, optional in singles,
    must be the foe's slot when given."""
    from ninjask.refsim import Battle, RefSim, seed_from_int

    t = _teams()
    seed = seed_from_int(1, salt="singles-choice")
    with RefSim() as sim:
        sd = Battle(sim, FORMAT, seed, ("Player 1", t[0]), ("Player 2", t[1]))
        sd.start()
        eng = ninjask.Battle.reference(d, t[0], t[1], seed)
        for side, order in (("p1", "team 412"), ("p2", "team 123")):
            sd.choose(side, order)
            eng.choose(side, order)
        obs = sd.choose("p1", choice)
        sd_ok = not obs.errors.get("p1")
        try:
            eng.choose("p1", choice)
            eng_ok = True
        except ValueError:
            eng_ok = False
        sd.close()
    assert eng_ok == sd_ok, (choice, obs.errors.get("p1"))


class _Script:
    """Stands in for check_reference's `random.Random`: the scripted choices
    first, whether or not the request-side enumeration lists them, then
    random ones."""

    def __init__(self, script):
        self.script = list(script)
        self.rng = random.Random(1)

    def choice(self, options):
        return self.script.pop(0) if self.script else self.rng.choice(list(options))


@SHOWDOWN
@pytest.mark.parametrize("seed", range(4))
def test_singles_targeted_choices_play_like_showdown(d, monkeypatch, seed):
    """Moves chosen with the foe's slot as target ("move 1 1") play exactly
    as in Showdown, draw for draw."""
    from ninjask.refsim import RefSim, seed_from_int

    cr = _check_reference(monkeypatch)
    t = _teams()
    # Garchomp (Choice Scarf: Dragon Claw) against Charizard (Ancient Power, then untargeted).
    script = ["team 412", "team 123", "move 1 1", "move 3 1", "move 1 1", "move 3"]
    with RefSim() as sim:
        cr.play(sim, d, seed_from_int(seed, salt="singles-target"), t[0], t[1], _Script(script), False)


GENGAR = "Gengar||Gengarite|CursedBody|ShadowBall,SludgeBomb,Protect,Thunderbolt|Timid|||||50|"
# (format, team 1, team 2 or a function of the fixture teams, inputs). Each
# case has refused choices: a move disabled in the open (Fake Out after the
# first turn), one Imprison hides, a switch Shadow Tag hides.
REFUSALS = {
    "singles, open disable": (FORMAT, 0, 1, [
        ("p1", "team 213"), ("p2", "team 123"), ("p1", "move 1"), ("p2", "move 4"),
        ("p1", "move 3"), ("p1", "move 3"), ("p1", "move 2"), ("p2", "move 4"),
    ]),
    "singles, hidden Imprison": (FORMAT, 0, 0, [
        ("p1", "team 123"), ("p2", "team 612"), ("p1", "move 2"), ("p2", "move 1"),
        ("p1", "move 4"), ("p1", "move 4"), ("p1", "move 2"), ("p2", "move 3"),
    ]),
    "singles, hidden Shadow Tag": (FORMAT, 0, "gengar", [
        ("p1", "team 123"), ("p2", "team 123"), ("p1", "move 2"), ("p2", "move 3 mega"),
        ("p1", "switch 2"), ("p1", "switch 3"), ("p1", "move 1"), ("p2", "move 1"),
    ]),
    "doubles, open disable": ("gen9championsvgc2026regmc", 0, 0, [
        ("p1", "team 2134"), ("p2", "team 6123"), ("p1", "move 1, move 2 1"), ("p2", "move 1, move 4"),
        ("p1", "move 3 1, move 4"), ("p1", "move 3 1, move 4"), ("p1", "move 2 1, move 2 1"), ("p2", "move 2, move 4"),
    ]),
}


@SHOWDOWN
@pytest.mark.parametrize("case", list(REFUSALS))
def test_refused_choices_update_requests_like_showdown(case):
    """After each input, every request Showdown sends equals the engine's
    (without rqid), and the same choices are refused with the same kind of
    refusal: "[Unavailable choice]" when it changed the request
    (`update`, the move's empty `disabledSource`; in singles Imprison's
    `maybeDisabled` stays), plain otherwise."""
    from ninjask.refsim import Battle, RefSim, seed_from_int

    fmt, a, b, script = REFUSALS[case]
    t = _teams()
    t1 = t[a]
    t2 = "]".join([GENGAR] + t[1].split("]")[1:]) if b == "gengar" else t[b]
    d = ninjask.dex(fmt)
    seed = seed_from_int(5, salt="hid")
    with RefSim() as sim:
        sd = Battle(sim, fmt, seed, ("Player 1", t1), ("Player 2", t2))
        sd.start()
        eng = ninjask.Battle.reference(d, t1, t2, seed)
        refused = 0
        for side, choice in script:
            obs = sd.choose(side, choice)
            sd_err = obs.errors.get(side)
            try:
                eng.choose(side, choice)
                eng_err = None
            except ValueError as e:
                eng_err = str(e)
            assert (sd_err is None) == (eng_err is None), (choice, sd_err, eng_err)
            if sd_err:
                refused += 1
                assert sd_err.startswith("[Unavailable choice]") == eng_err.startswith("[Unavailable choice]"), (choice, sd_err, eng_err)
            for s in ("p1", "p2"):
                want = obs.requests.get(s)
                if want is not None:
                    got = json.loads(eng.request(s))
                    got.pop("rqid")
                    assert got == want, (choice, s)
        sd.close()
    assert refused >= 2


# Damage and turn order for any Pokémon of a party ("p1:2": side and index
# into the projection's `sides[i]["pokemon"]`), benched ones included.


def _positions(d, seeds=range(12), turns=4):
    """Singles positions after team preview and a few random turns, each
    with both slots filled and moves to choose."""
    t = _teams()
    out = []
    for g in seeds:
        b = ninjask.Battle(d, t[g % len(t)], t[(g + 3) % len(t)], seed=g)
        b.set_logging(False)
        b.choose("p1", "team 123")
        b.choose("p2", "team 231")
        rng = random.Random(g)
        for _ in range(rng.randrange(turns + 1)):
            if b.ended:
                break
            for side in ("p1", "p2"):
                if b.needs_choice(side):
                    choices = b.legal_choices(side)
                    b.choose_index(side, rng.randrange(len(choices)))
        if b.ended or not (b.needs_choice("p1") and b.needs_choice("p2")):
            continue
        p = json.loads(b.projection())
        if any(json.loads(b.request(s)).get("forceSwitch") for s in ("p1", "p2")):
            continue
        out.append((b, p))
    assert len(out) >= 6
    return out


def _active_index(side):
    return next(i for i, m in enumerate(side["pokemon"]) if m["isActive"] and not m["fainted"])


def test_damage_party_matches_active(d):
    """Pointing at the Pokémon on the field gives exactly `damage` and `order`."""
    compared = 0
    for b, p in _positions(d):
        k, j = _active_index(p["sides"][0]), _active_index(p["sides"][1])
        a_moves = [m["id"] for m in p["sides"][0]["pokemon"][k]["moves"]]
        b_moves = [m["id"] for m in p["sides"][1]["pokemon"][j]["moves"]]
        for mv in a_moves:
            assert b.damage_party(f"p1:{k}", mv, f"p2:{j}") == b.damage("p1a", mv, "p2a")
            compared += 1
        for mv in b_moves:
            assert b.damage_party(f"p2:{j}", mv, f"p1:{k}") == b.damage("p2a", mv, "p1a")
        for pa in a_moves + ["switch", "priority:1"]:
            for pb in b_moves + ["priority:0"]:
                assert b.order_party(f"p1:{k}", pa, f"p2:{j}", pb) == b.order("p1a", pa, "p2a", pb)
                assert b.order_party(f"p2:{j}", pb, f"p1:{k}", pa) == b.order("p2a", pb, "p1a", pa)
    assert compared >= 20


def test_damage_party_bench(d):
    """A benched Pokémon counts as if it were in its side's slot, with its
    own stats and no boosts; the battle is left as it was."""
    seen = 0
    for b, p in _positions(d):
        before = b.projection()
        sides = p["sides"]
        for s, foe in ((0, 1), (1, 0)):
            for x, mon in enumerate(sides[s]["pokemon"]):
                for y, target in enumerate(sides[foe]["pokemon"]):
                    if mon["fainted"] or target["fainted"]:
                        continue
                    for mv in mon["moves"]:
                        c = json.loads(b.damage_party(f"p{s + 1}:{x}", mv["id"], f"p{foe + 1}:{y}"))
                        assert c["targetMaxhp"] == target["maxhp"] and c["targetHp"] == target["hp"]
                        assert 0.0 <= c["ko"] <= c["ko2"] + 1e-12 <= 1.0 + 1e-9
                        if not mon["isActive"] or not target["isActive"]:
                            seen += 1
                    first, tie = b.order_party(f"p{s + 1}:{x}", mon["moves"][0]["id"], f"p{foe + 1}:{y}", "priority:0")
                    assert 0.0 <= first <= 1.0 and 0.0 <= tie <= 1.0
                    first, _ = b.order_party(f"p{s + 1}:{x}", "switch", f"p{foe + 1}:{y}", "priority:0")
                    assert first == 1.0
        assert b.projection() == before, "damage_party changed the battle"
    assert seen >= 100


def test_damage_party_bench_ignores_the_active_pokemons_boosts(d):
    """The active Pokémon's boosts stay with it: raising them changes its own
    damage and order, never a benched Pokémon's, which counts with none. The
    foe on the field keeps its boosts."""
    changed = 0
    for b, p in _positions(d):
        own = p["sides"][0]["pokemon"]
        k, j = _active_index(p["sides"][0]), _active_index(p["sides"][1])
        bench = [i for i, m in enumerate(own) if not m["isActive"] and not m["fainted"]]
        foe_only = json.loads(b.projection())
        foe_only["sides"][1]["pokemon"][j]["boosts"].update({"def": -6, "spd": -6})
        both = json.loads(json.dumps(foe_only))
        both["sides"][0]["pokemon"][k]["boosts"].update({"atk": 6, "spa": 6, "spe": 6})
        foe_only = ninjask.Battle.from_projection(d, json.dumps(foe_only), 0)
        both = ninjask.Battle.from_projection(d, json.dumps(both), 0)
        for i in bench:
            for mv in own[i]["moves"]:
                x, y = f"p1:{i}", f"p2:{j}"
                assert both.damage_party(x, mv["id"], y) == foe_only.damage_party(x, mv["id"], y)
                assert both.order_party(x, mv["id"], y, "priority:0") == foe_only.order_party(x, mv["id"], y, "priority:0")
        for mv in own[k]["moves"]:
            x, y = f"p1:{k}", f"p2:{j}"
            if both.damage_party(x, mv["id"], y) != foe_only.damage_party(x, mv["id"], y):
                changed += 1
    assert changed >= 5


def test_damage_party_refuses_bad_references(d):
    b, p = _positions(d)[0]
    mv = p["sides"][0]["pokemon"][0]["moves"][0]["id"]
    n = len(p["sides"][0]["pokemon"])
    for bad in (f"p1:{n}", "p3:0", "p1a", "p1:", "p1:x", ""):
        with pytest.raises(ValueError):
            b.damage_party(bad, mv, "p2:0")
        with pytest.raises(ValueError):
            b.order_party(bad, "priority:0", "p2:0", "priority:0")
    bench = [i for i, m in enumerate(p["sides"][0]["pokemon"]) if not m["isActive"]]
    # Two different Pokémon of one side can't both be in its one slot.
    with pytest.raises(ValueError):
        b.damage_party(f"p1:{bench[0]}", mv, f"p1:{bench[1]}")


def test_damage_party_bench_matches_the_position_with_it_switched_in(d):
    """A benched Pokémon counts as it would in the position where it had
    switched in with nothing happening on the way in: the projection with
    it and the active Pokémon swapped (Showdown's `switchIn` order)."""
    compared = 0
    for b, p in _positions(d, seeds=range(20)):
        for k, mon in enumerate(p["sides"][0]["pokemon"]):
            if mon["isActive"] or mon["fainted"]:
                continue
            q = json.loads(b.projection())
            mons = q["sides"][0]["pokemon"]
            mons[0], mons[k] = mons[k], mons[0]
            mons[0]["position"], mons[k]["position"] = 0, k
            mons[0]["isActive"], mons[k]["isActive"] = True, False
            q["sides"][0]["active"] = [mons[0]["name"]]
            w = ninjask.Battle.from_projection(d, json.dumps(q), 0)
            for mv in mon["moves"]:
                assert b.damage_party(f"p1:{k}", mv["id"], "p2:0") == w.damage("p1a", mv["id"], "p2a")
                assert b.damage_party("p2:0", mv["id"], f"p1:{k}") == w.damage("p2a", mv["id"], "p1a")
                assert b.order_party(f"p1:{k}", mv["id"], "p2:0", "priority:0") == w.order("p1a", mv["id"], "p2a", "priority:0")
                compared += 1
    assert compared >= 50
