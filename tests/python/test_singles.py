"""Singles (Champions BSS, `gen9championsbssregmc`): one active Pokémon a side,
bring six and pick three, no target locations.

The engine fields as many slots as the format's `gameType` says; these tests
cover what differs from doubles. Those that compare with Showdown skip when
it is not built, and those that need the format export skip without it.
"""

import importlib.util
import json
import math
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


# `Battle.eval_features(side)`: battle-ai's "v2" evaluator features (22
# values, from `side`'s point of view), read straight from the battle with
# the calculations of `damage_party` and `order_party`.

FEATURES_V2 = (
    "alive_diff", "hp_diff", "status_diff", "boost_diff", "hazard_diff", "mega_left_diff", "sash_diff", "matchup",
    "cover", "best_switch", "speed_edge", "priority_ko",
    "setup_window", "recovery", "ko_race", "burn_physical", "para_faster", "sleep_turns", "toxic_stage",
    "screens_tailwind", "choice_locked_bad", "substitute",
)
F = {n: i for i, n in enumerate(FEATURES_V2)}


def _walk(d, seeds=range(24), turns=10):
    """Every position along random singles games after team preview, the
    empty slot after a faint (and the middle of a turn) included."""
    t = _teams()
    out = []
    for g in seeds:
        b = ninjask.Battle(d, t[g % len(t)], t[(g + 5) % len(t)], seed=100 + g)
        b.set_logging(False)
        b.choose("p1", "team 123")
        b.choose("p2", "team 312")
        rng = random.Random(f"eval:{g}")
        for _ in range(turns):
            if b.ended:
                break
            out.append(b.copy())
            for side in ("p1", "p2"):
                if b.needs_choice(side):
                    b.choose_index(side, rng.randrange(len(b.legal_choices(side))))
    assert len(out) >= 50
    return out


def _active(side):
    return next((i for i, m in enumerate(side["pokemon"]) if m["isActive"] and not m["fainted"]), None)


def _alive(side):
    return [i for i, m in enumerate(side["pokemon"]) if not m["fainted"]]


def _usable(mon):
    return [mv["id"] for mv in mon["moves"] if mv["pp"] > 0 and not mv["disabled"]]


def test_eval_features_are_antisymmetric(d):
    empty = 0
    for b in _walk(d):
        f1, f2 = b.eval_features("p1"), b.eval_features("p2")
        assert len(f1) == 22 and all(isinstance(x, float) and x == x and abs(x) < 10 for x in f1)
        assert f1 == [-x for x in f2]
        p = json.loads(b.projection())
        empty += any(_active(s) is None for s in p["sides"])
    assert empty >= 3, "no position with an empty slot"
    with pytest.raises(ValueError):
        b.eval_features("p3")


def _swap(p):
    q = json.loads(json.dumps(p))
    q["sides"] = [q["sides"][1], q["sides"][0]]
    for i, side in enumerate(q["sides"]):
        side["id"] = f"p{i + 1}"
    return q


def test_eval_features_follow_the_sides(d):
    """With the two sides swapped, p1's features are the original p2's."""
    checked = 0
    for b in _walk(d, seeds=range(12)):
        p = json.loads(b.projection())
        if p["queueLength"]:
            continue
        w = ninjask.Battle.from_projection(d, json.dumps(p), 0)
        sw = ninjask.Battle.from_projection(d, json.dumps(_swap(p)), 0)
        assert sw.eval_features("p1") == pytest.approx(w.eval_features("p2"), abs=1e-12)
        checked += 1
    assert checked >= 30


_STATUS_W = {"slp": 1.0, "frz": 1.0, "par": 0.6, "brn": 0.6, "psn": 0.5, "tox": 0.5}


def _plain_terms(p):
    """The features read off the projection alone, p1's view."""
    def side_terms(s):
        side, foe = p["sides"][s], p["sides"][1 - s]
        alive = [side["pokemon"][i] for i in _alive(side)]
        act = _active(side)
        foe_act = _active(foe)
        c = side["sideConditions"]
        haz = (1.0 if "stealthrock" in c else 0.0) + 0.5 * c.get("toxicspikes", {}).get("layers", 0) / 2 \
            + c.get("spikes", {}).get("layers", 0) / 3 + (0.5 if "stickyweb" in c else 0.0)
        megaed = any("-Mega" in m["species"] for m in side["pokemon"])
        return {
            "alive": len(alive) / 3,
            "hp": sum(m["hp"] / m["maxhp"] for m in alive) / 3,
            "status": sum(_STATUS_W.get(m["status"], 0.0) for m in alive) / 3,
            "boost": sum(side["pokemon"][act]["boosts"][k] for k in ("atk", "def", "spa", "spd", "spe")) / 6
            if act is not None else 0.0,
            "hazard": haz,
            "mega": 1.0 if any(m["canMegaEvo"] for m in alive) and not megaed else 0.0,
            "sash": sum(m["item"] == "focussash" and m["hp"] == m["maxhp"] for m in alive) / 3,
            "burn": sum(m["status"] == "brn" and m["storedStats"]["atk"] > m["storedStats"]["spa"] for m in alive) / 3,
            "para": 0.0 if foe_act is None else sum(
                m["status"] == "par" and m["storedStats"]["spe"] > foe["pokemon"][foe_act]["storedStats"]["spe"]
                for m in alive) / 3,
            "sleep": sum(m["statusState"].get("time", 0) for m in alive if m["status"] == "slp") / 9,
            "toxic": sum(m["statusState"].get("stage", 0) / 16 for m in alive if m["status"] == "tox") / 3,
            "screens": sum(c.get(k, {}).get("duration", 0) for k in ("reflect", "lightscreen", "auroraveil", "tailwind")) / 8,
            "sub": 1.0 if act is not None and "substitute" in side["pokemon"][act]["volatiles"] else 0.0,
        }
    a, b = side_terms(0), side_terms(1)
    return {
        "alive_diff": a["alive"] - b["alive"], "hp_diff": a["hp"] - b["hp"], "status_diff": b["status"] - a["status"],
        "boost_diff": a["boost"] - b["boost"], "hazard_diff": b["hazard"] - a["hazard"],
        "mega_left_diff": a["mega"] - b["mega"], "sash_diff": a["sash"] - b["sash"],
        "burn_physical": b["burn"] - a["burn"], "para_faster": b["para"] - a["para"],
        "sleep_turns": b["sleep"] - a["sleep"], "toxic_stage": b["toxic"] - a["toxic"],
        "screens_tailwind": a["screens"] - b["screens"], "substitute": a["sub"] - b["sub"],
    }


def _edited(p, g):
    """The projection with statuses, side conditions and a substitute put on
    (varying with `g`), so that the plain terms are not all zero."""
    q = json.loads(json.dumps(p))
    for s, side in enumerate(q["sides"]):
        alive = _alive(side)
        if not alive:
            continue
        m = side["pokemon"][alive[(g + s) % len(alive)]]
        st = ("brn", "par", "slp", "tox", "psn")[(g + s) % 5]
        m["status"] = st
        if st == "slp":
            m["statusState"] = {"startTime": 3, "time": 1 + (g + s) % 3}
        elif st == "tox":
            m["statusState"] = {"stage": (g + 2 * s) % 7}
        else:
            m["statusState"] = {}
        screen = ("reflect", "lightscreen", "tailwind", "auroraveil")[(g + s) % 4]
        side["sideConditions"].update({screen: {"duration": 1 + (g + 3 * s) % 5}, "spikes": {"layers": 1 + s}})
        act = _active(side)
        if act is not None and (g + s) % 2:
            side["pokemon"][act]["volatiles"]["substitute"] = {"hp": 20}
    return q


def test_eval_features_plain_terms(d):
    seen = set()
    for g, b in enumerate(_walk(d, seeds=range(10))):
        p = json.loads(b.projection())
        boards = [(b, p)]
        if not p["queueLength"]:
            q = _edited(p, g)
            w = ninjask.Battle.from_projection(d, json.dumps(q), 0)
            q = json.loads(w.projection())  # as imported
            boards.append((w, q))
        for w, q in boards:
            f = w.eval_features("p1")
            for name, v in _plain_terms(q).items():
                assert f[F[name]] == pytest.approx(v, abs=1e-12), name
                if v:
                    seen.add(name)
    assert {"burn_physical", "sleep_turns", "toxic_stage", "screens_tailwind", "substitute", "hazard_diff"} <= seen


class _Ref:
    """battle-ai's threat, mu and the terms built on them, from damage_party and order_party."""

    def __init__(self, b, p, d):
        self.b, self.p, self.d = b, p, d
        self.memo = {}

    def mon(self, r):
        return self.p["sides"][r[0]]["pokemon"][r[1]]

    def dmg(self, x, mv, y):
        return json.loads(self.b.damage_party(f"p{x[0] + 1}:{x[1]}", mv, f"p{y[0] + 1}:{y[1]}"))

    def threat(self, x, y):
        if (x, y) not in self.memo:
            best, move = 0.0, None
            for mv in _usable(self.mon(x)):
                move = move or mv
                c = self.dmg(x, mv, y)
                t = 0.6 * c["ko"] + 0.4 * max(c["ko2"], c["ko"])
                if c["damaging"] and t > best:
                    best, move = t, mv
            self.memo[(x, y)] = (best, move)
        return self.memo[(x, y)]

    def first(self, x, y):
        am, bm = self.threat(x, y)[1], self.threat(y, x)[1]
        if am is None and bm is None:
            return None
        if am is None or bm is None:
            return 0.0 if am is None else 1.0
        return self.b.order_party(f"p{x[0] + 1}:{x[1]}", am, f"p{y[0] + 1}:{y[1]}", bm)[0]

    def mu(self, x, y):
        f = self.first(x, y)
        if f is None:
            return 0.0
        a, b = self.threat(x, y)[0], self.threat(y, x)[0]
        return a * (f + (1 - f) * (1 - b)) - b * ((1 - f) + f * (1 - a))

    def terms(self):
        sides = self.p["sides"]
        act = [_active(sides[0]), _active(sides[1])]
        out = {}

        def cover(s):
            xs, ys = _alive(sides[s]), _alive(sides[1 - s])
            if not xs or not ys:
                return 0.0
            return sum(max(self.mu((s, x), (1 - s, y)) for x in xs) for y in ys) / len(ys)

        def best_switch(s):
            y = act[1 - s]
            bench = [i for i in _alive(sides[s]) if not sides[s]["pokemon"][i]["isActive"]]
            return 0.0 if y is None or not bench else max(self.mu((s, x), (1 - s, y)) for x in bench)

        def priority_ko(s):
            if None in act:
                return 0.0
            best = 0.0
            for mv in _usable(self.mon((s, act[s]))):
                if self.d.eval_move_marks(mv)[2] > 0:
                    c = self.dmg((s, act[s]), mv, (1 - s, act[1 - s]))
                    if c["damaging"]:
                        best = max(best, c["ko"])
            return best

        def setup_window(s):
            if None in act or not any(self.d.eval_move_marks(mv)[0] for mv in _usable(self.mon((s, act[s])))):
                return 0.0
            return 1.0 - self.threat((1 - s, act[1 - s]), (s, act[s]))[0]

        def recovery(s):
            ms = [sides[s]["pokemon"][i] for i in _alive(sides[s])]
            return sum(1 - m["hp"] / m["maxhp"] for m in ms
                       if any(self.d.eval_move_marks(mv)[1] for mv in _usable(m))) / 3

        def hits(x, y):
            best = max([self.dmg(x, mv, y)["expected"] for mv in _usable(self.mon(x))], default=0.0)
            return 5 if best <= 0 else min(5, math.ceil(self.mon(y)["hp"] / best))

        def locked_bad(s):
            if None in act:
                return 0.0
            lock = self.mon((s, act[s]))["volatiles"].get("choicelock")
            if not lock:
                return 0.0
            c = self.dmg((s, act[s]), lock["move"], (1 - s, act[1 - s]))
            return 1.0 if 0.6 * c["ko"] + 0.4 * max(c["ko2"], c["ko"]) < 0.2 else 0.0

        if None in act:
            out["matchup"] = out["speed_edge"] = out["ko_race"] = 0.0
        else:
            a, b = (0, act[0]), (1, act[1])
            out["matchup"] = self.mu(a, b)
            f = self.first(a, b)
            out["speed_edge"] = 0.0 if f is None else f - 0.5
            out["ko_race"] = min(1.0, max(-1.0, (hits(b, a) - hits(a, b)) / 4))
        out["cover"] = cover(0) - cover(1)
        out["best_switch"] = best_switch(0) - best_switch(1)
        out["priority_ko"] = priority_ko(0) - priority_ko(1)
        out["setup_window"] = setup_window(0) - setup_window(1)
        out["recovery"] = recovery(0) - recovery(1)
        out["choice_locked_bad"] = locked_bad(1) - locked_bad(0)
        return out


def test_eval_features_calculated_terms(d):
    """matchup and the terms on threats, turn order and damage agree with
    the same terms built from damage_party and order_party."""
    nonzero = set()
    for b in _walk(d, seeds=range(14)):
        p = json.loads(b.projection())
        f = b.eval_features("p1")
        for name, v in _Ref(b, p, d).terms().items():
            assert f[F[name]] == pytest.approx(v, abs=1e-12), name
            if v:
                nonzero.add(name)
    assert {"matchup", "cover", "best_switch", "speed_edge", "ko_race"} <= nonzero


def test_eval_features_leave_the_battle_as_it_was(d):
    for b in _walk(d, seeds=range(6)):
        before, key = b.projection(), b.state_key()
        b.eval_features("p1")
        assert b.projection() == before and b.state_key() == key


def test_eval_move_marks(d):
    """(setup, heal, priority): a status move raising its user's stats by 1
    or more in all, a move healing its user (the heal flag, target self),
    and the dex priority."""
    assert d.eval_move_marks("swordsdance") == (True, False, 0)
    assert d.eval_move_marks("roost") == (False, True, 0)
    assert d.eval_move_marks("flamecharge") == (False, False, 0)  # a damaging move
    assert d.eval_move_marks("quickattack") == (False, False, 1)
    assert d.eval_move_marks("bodyslam") == (False, False, 0)
    if "milkdrink" in d.move_ids():  # the heal flag, but target adjacentAllyOrSelf
        assert d.eval_move_marks("milkdrink")[1] is False
    with pytest.raises(ValueError):
        d.eval_move_marks("notamove")


# The move history a world rebuilds from one side's log and requests: the
# field's last move (Copycat), and for that side's Pokémon the last target
# location, this and last turn's move result (Stomping Tantrum), the hits
# taken (Rage Fist, Revenge), the stall counter (Protect), the item used up
# (Recycle) and a berry eaten (Belch).
HISTORY = ("lastMoveTargetLoc", "moveThisTurnResult", "moveLastTurnResult", "timesAttacked", "attackedBy",
           "lastItem", "ateBerry")


def _history(p, side):
    s = next(x for x in p["sides"] if x["id"] == side)
    return {
        "lastMove": p["lastMove"],
        **{
            m["name"]: {**{k: m[k] for k in HISTORY}, "stall": m["volatiles"].get("stall")}
            for m in s["pokemon"] if not m["fainted"]
        },
    }


def test_world_rebuilds_the_move_history(d):
    from ninjask.world_check import true_assumptions

    t = _teams()
    checked, seen, bad = 0, set(), []
    for g in range(16):
        b = ninjask.Battle(d, t[g % len(t)], t[(g + 5) % len(t)], seed=g)
        b.set_logging(True)
        tracker, read = ninjask.Tracker(d, "p1"), 0
        rng = random.Random(g)
        while not b.ended and b.turn < 40:
            for side in ("p1", "p2"):
                if not b.needs_choice(side):
                    continue
                if side == "p1":
                    lines = b.log("p1", read)
                    read += len(lines)
                    tracker.feed(lines)
                    tracker.feed_request(b.request("p1"))
                    truth = json.loads(b.projection())
                    if b.turn > 0:
                        world = json.loads(tracker.world(true_assumptions(json.loads(tracker.view()), truth), 0)
                                           .projection())
                        want, got = _history(truth, "p1"), _history(world, "p1")
                        if want != got:
                            bad.append((g, b.turn, want, got))
                        checked += 1
                        seen |= {k for m in want.values() if isinstance(m, dict) for k, v in m.items()
                                 if v not in (None, 0, [], False)}
                choices = b.legal_choices(side)
                try:
                    b.choose_index(side, rng.randrange(len(choices)))
                except ValueError:  # a move the request hides as disabled: the request now says so
                    pass
    assert checked > 200
    assert not bad, bad[:3]
    # Every item took a value other than its default somewhere.
    assert seen >= {"lastMoveTargetLoc", "moveLastTurnResult", "timesAttacked", "attackedBy", "stall"}, seen


def test_tracker_reads_protect_and_hits(d):
    t = ninjask.Tracker(d, "p1")
    t.feed([
        "|gametype|singles",
        "|switch|p1a: Garchomp|Garchomp, L50, M|183/183",
        "|switch|p2a: Kangaskhan|Kangaskhan, L50, F|100/100",
        "|turn|1",
        "|move|p1a: Garchomp|Protect|p1a: Garchomp",
        "|-singleturn|p1a: Garchomp|Protect",
        "|move|p2a: Kangaskhan|Body Slam|p1a: Garchomp",
        "|-activate|p1a: Garchomp|move: Protect",
        "|",
        "|upkeep",
        "|turn|2",
        "|move|p1a: Garchomp|Protect|p1a: Garchomp",
        "|-singleturn|p1a: Garchomp|Protect",
        "|move|p2a: Kangaskhan|Double-Edge|p1a: Garchomp",
        "|-activate|p1a: Garchomp|move: Protect",
        "|",
        "|upkeep",
        "|turn|3",
    ])
    g = json.loads(t.view())["sides"][0]["pokemon"][0]
    assert g["stall"] == [9, 1] and g["moveLastTurnResult"] is True
    assert json.loads(t.view())["lastMove"] == "doubleedge"
    t.feed([
        "|move|p2a: Kangaskhan|Double-Edge|p1a: Garchomp",
        "|-damage|p1a: Garchomp|120/183",
        "|-damage|p1a: Garchomp|100/183",
        "|-hitcount|p1a: Garchomp|2",
        "|move|p1a: Garchomp|Protect|p1a: Garchomp",
        "|-fail|p1a: Garchomp",
        "|",
        "|upkeep",
        "|turn|4",
    ])
    g = json.loads(t.view())["sides"][0]["pokemon"][0]
    # Protect used last fails without ending the counter (no roll); it ran out at the residual.
    assert g["stall"] is None and g["moveLastTurnResult"] is False
    assert g["timesAttacked"] == 2
    assert g["attackedBy"] == [{"side": 1, "name": "Kangaskhan", "slot": 0, "move": "doubleedge", "damage": 20,
                                "thisTurn": False}]
    assert g["lastMoveTargetLoc"] == -1
