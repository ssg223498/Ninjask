"""Engine tests: support honesty, legal play, and agreement with Showdown."""

import json
import random

import pytest

ninjask = pytest.importorskip("ninjask")

from ninjask.policy import RandomPolicy  # noqa: E402
from ninjask import dex, generic_test_team, play_engine_game, supported_test_team  # noqa: E402
from ninjask.paths import ENGINE_ROOT, FORMAT_DIR, SHOWDOWN_DIR, TEAMS_DIR  # noqa: E402
from ninjask.refsim import Battle, RefSim, seed_from_int  # noqa: E402

FORMAT = "gen9championsdoublescustomgame"

pytestmark = pytest.mark.skipif(
    not (SHOWDOWN_DIR / "dist").exists(),
    reason="Pokémon Showdown not built (scripts/setup_showdown.sh)",
)


def test_support_is_honest():
    d = dex(FORMAT)
    assert d.move_support("earthquake") == "generic"
    assert d.move_support("nightshade") == "generic"  # fixed damage is a modelled field
    assert d.move_support("outrage") == "implemented"  # ported handler (lockedmove)
    assert d.move_support("ragingfury") == "generic"  # the same lockedmove, with no code of its own
    assert d.move_support("uproar") == "implemented"  # self.volatileStatus with code, now ported (uproar)
    assert d.move_support("copycat") == "implemented"  # calls the battle's last move, now ported
    assert d.move_support("sleeptalk") == "implemented"  # calls one of the user's moves, now ported
    assert d.move_support("protect") == "implemented"  # ported handler
    assert d.move_support("substitute") == "implemented"
    assert d.move_support("revivalblessing") == "implemented"
    assert d.move_support("transform") == "implemented"
    assert d.move_support("hyperbeam") == "generic"  # its recharge volatile is modelled
    assert d.team_problems("Garchomp|||megasol|Copycat,Earthquake|Jolly|||||50|") == []  # both supported
    assert d.team_problems("Garchomp||lifeorb|roughskin|Protect,Earthquake|Jolly|||||50|") == []
    # A Mega Stone is checked with the Mega's ability too (Mega Sol is ported now).
    assert d.team_problems("Meganium||meganiumite|overgrow|Protect,Earthquake|Jolly|||||50|") == []
    # Every move, item and ability of Reg M-C is supported, but the abilities
    # of species it bans (Cramorant, Eiscue, Minior), which are still
    # reported as unsupported.
    regmc = "gen9championsvgc2026regmc"
    d = dex(regmc)
    spec = json.loads((FORMAT_DIR / f"{regmc}.json").read_text(encoding="utf-8"))
    unported = {"gulpmissile", "iceface", "shieldsdown"}
    assert [m["id"] for m in spec["moves"] if d.move_support(m["id"]).startswith("unsupported")] == []
    assert set(d.supported_items()) >= {i["id"] for i in spec["items"]}
    assert {a["id"] for a in spec["abilities"]} - set(d.supported_abilities()) == unported
    problems = d.team_problems("Garchomp|||gulpmissile|Protect,Earthquake|Jolly|||||50|")
    assert any("gulpmissile" in p for p in problems)


def test_generated_teams_are_supported_and_legal():
    rng = random.Random(0)
    teams = [generic_test_team(FORMAT, rng) for _ in range(10)]
    with RefSim() as sim:
        for t in teams:
            assert dex(FORMAT).team_problems(t) == []
            assert sim.validate(FORMAT, t) is None


def test_supported_teams_are_supported_and_legal():
    rng = random.Random(3)
    teams = [supported_test_team(FORMAT, rng) for _ in range(10)]
    with RefSim() as sim:
        for t in teams:
            assert dex(FORMAT).team_problems(t) == []
            assert sim.validate(FORMAT, t) is None


def test_supported_team_games_finish_without_rejections():
    rng = random.Random(4)
    teams = [supported_test_team(FORMAT, rng) for _ in range(8)]
    for i in range(50):
        r = play_engine_game(FORMAT, i, teams[i % 8], teams[(i + 3) % 8], RandomPolicy(i), RandomPolicy(100 + i))
        assert r.winner in ("p1", "p2", None)
        # Only what the request hid can be refused (Imprison, Shadow Tag), as
        # in Showdown, and the updated request is then followed.
        assert all(e.startswith("[Unavailable choice]") for _, _, e in r.rejections), r.rejections[:3]
        assert all(c != "default" for _, c, _ in r.rejections), r.rejections[:3]


def test_random_games_finish_without_rejections():
    rng = random.Random(1)
    teams = [generic_test_team(FORMAT, rng) for _ in range(8)]
    for i in range(50):
        r = play_engine_game(FORMAT, i, teams[i % 8], teams[(i + 3) % 8], RandomPolicy(i), RandomPolicy(100 + i))
        assert r.winner in ("p1", "p2")
        assert sum(r.rejected.values()) == 0, r.rejections[:3]


def test_stats_and_requests_match_showdown():
    rng = random.Random(2)
    t1, t2 = generic_test_team(FORMAT, rng), generic_test_team(FORMAT, rng)
    eng = ninjask.Battle(dex(FORMAT), t1, t2, 5)
    eng.choose("p1", "team 1234")
    eng.choose("p2", "team 1234")
    with RefSim() as sim:
        ref = Battle(sim, FORMAT, seed_from_int(5), ("Player 1", t1), ("Player 2", t2))
        ref.start()
        ref.choose("p1", "team 1234")
        obs = ref.choose("p2", "team 1234")
        ref_state = ref.state()
        ref.close()
    eng_state = json.loads(eng.projection())
    for es, rs in zip(eng_state["sides"], ref_state["sides"]):
        for em, rm in zip(es["pokemon"], rs["pokemon"]):
            assert em["species"] == rm["species"]
            assert em["maxhp"] == rm["maxhp"]
            assert em["storedStats"] == rm["storedStats"]
            assert [m["maxpp"] for m in em["moves"]] == [m["maxpp"] for m in rm["moves"]]
            assert em["isActive"] == rm["isActive"]
    eng_req = json.loads(eng.request("p1"))
    ref_req = obs.requests["p1"]
    assert [[m["id"] for m in a["moves"]] for a in eng_req["active"]] == [
        [m["id"] for m in a["moves"]] for a in ref_req["active"]
    ]
    assert [m["target"] for m in eng_req["active"][0]["moves"]] == [m["target"] for m in ref_req["active"][0]["moves"]]


def test_outcome_distributions_match_showdown():
    from ninjask.verify import verify

    report = verify(FORMAT, cases=8, n=150, seed=11)
    assert report["failures"] == [], report["failures"][:2]
    # Every position imports, and the import restores the projection exactly.
    assert [e for e in report["errors"] if "import" in e["error"] or "engine" in e["error"]] == []


# Teams for the multi-turn move tests: p2 only uses Helping Hand.
LOCK_P1 = (
    "Archaludon|||stamina|ElectroShot,Protect,FlashCannon,DracoMeteor|Modest|,,,32,,||||50|]"
    "Charizard|||blaze|HyperBeam,Protect,Flamethrower,HelpingHand|Modest|,,,32,,||||50|]"
    "Pelipper|||drizzle|Protect,HelpingHand,RainDance,BodySlam|Bold|32,,32,,,||||50|]"
    "Venusaur|||overgrow|SolarBeam,Protect,SludgeBomb,HelpingHand|Modest|||||50|"
)
LOCK_P2 = (
    "Snorlax|||thickfat|HelpingHand,BodySlam,Protect,RainDance|Careful|32,,,,32,||||50|]"
    "Blastoise|||torrent|HelpingHand,FlashCannon,Protect,RainDance|Bold|32,,32,,,||||50|]"
    "Venusaur|||overgrow|HelpingHand,SludgeBomb,Protect,SolarBeam|Bold|32,,32,,,||||50|]"
    "Pelipper|||keeneye|HelpingHand,BodySlam,Protect,RainDance|Bold|32,,32,,,||||50|"
)


def _lead(seed: int, order: str = "team 1234") -> "ninjask.Battle":
    b = ninjask.Battle(dex(FORMAT), LOCK_P1, LOCK_P2, seed)
    b.choose("p1", order)
    b.choose("p2", "team 1234")
    return b


def _mon(b, side: str, name: str) -> dict:
    state = json.loads(b.projection())
    s = next(x for x in state["sides"] if x["id"] == side)
    return next(p for p in s["pokemon"] if p["name"] == name)


def test_hyper_beam_forces_a_recharge_turn():
    recharged = 0
    for seed in range(8):
        b = _lead(seed)
        b.choose("p1", "move 2, move 1 1")
        b.choose("p2", "move 1 -2, move 1 -1")
        if "mustrecharge" not in _mon(b, "p1", "Charizard")["volatiles"]:
            continue  # Hyper Beam missed
        recharged += 1
        req = json.loads(b.request("p1"))
        assert req["active"][1] == {"moves": [{"move": "Recharge", "id": "recharge"}], "trapped": True}
        with pytest.raises(ValueError):
            b.choose("p1", "move 3 1, switch 3")
        b.choose("p1", "move 3 1, move 1")
        b.choose("p2", "move 1 -2, move 1 -1")
        assert "mustrecharge" not in _mon(b, "p1", "Charizard")["volatiles"]
        assert len(json.loads(b.request("p1"))["active"][1]["moves"]) == 4
    assert recharged > 0


def test_electro_shot_charges_then_strikes_locked():
    b = _lead(1)
    b.choose("p1", "move 1 1, move 2")
    b.choose("p2", "move 1 -2, move 1 -1")
    arch = _mon(b, "p1", "Archaludon")
    assert {"twoturnmove", "electroshot"} <= set(arch["volatiles"])
    assert arch["boosts"]["spa"] == 1
    req = json.loads(b.request("p1"))
    assert req["active"][0] == {"moves": [{"move": "Electro Shot", "id": "electroshot"}], "trapped": True}
    with pytest.raises(ValueError):
        b.choose("p1", "move 1 2, move 3 1")  # no target may be chosen
    with pytest.raises(ValueError):
        b.choose("p1", "switch 3, move 3 1")
    b.choose("p1", "move 1, move 3 1")
    b.choose("p2", "move 1 -2, move 1 -1")
    arch = _mon(b, "p1", "Archaludon")
    assert not {"twoturnmove", "electroshot"} & set(arch["volatiles"])
    assert arch["moves"][0]["pp"] == arch["moves"][0]["maxpp"] - 1  # PP is spent on the charge turn only
    assert arch["boosts"]["spa"] == 1
    assert arch["lastMove"] == "electroshot"


def test_electro_shot_fires_at_once_in_rain():
    b = _lead(1, "team 3124")  # Pelipper's Drizzle, Archaludon in slot 2
    b.choose("p1", "move 1, move 1 1")
    b.choose("p2", "move 1 -2, move 1 -1")
    arch = _mon(b, "p1", "Archaludon")
    assert "twoturnmove" not in arch["volatiles"]
    assert arch["boosts"]["spa"] == 1
    assert len(json.loads(b.request("p1"))["active"][1]["moves"]) == 4


def test_tracker_view_is_true_in_engine_games():
    """A player's tracker never knows anything false, and knows its own side."""
    from ninjask import tracker_check as ct

    teams = None  # the main pool's real teams
    if not (TEAMS_DIR / "supported-pool.txt").exists():
        # No generated pool yet (scripts/report_support.py writes it): use synthetic teams.
        rng = random.Random(11)
        teams = [supported_test_team(ct.FORMAT, rng, size=6) for _ in range(20)]
    counts = ct.engine_games(25, seed=11, teams=teams)
    problems = {k: v for k, v in counts.items() if k not in ("checks", "mask checks", "hidden legality", "_shown") and not k.startswith("trap_")}
    assert counts["checks"] > 300
    assert problems == {}


def test_tracker_reads_mega_evolution_and_items():
    d = dex(FORMAT)
    t = ninjask.Tracker(d, "p1")
    t.feed([
        "|poke|p2|Salamence, L50, M|",
        "|poke|p2|Incineroar, L50, M|",
        "|switch|p2a: Salamence|Salamence, L50, M|100/100",
        "|switch|p2b: Incineroar|Incineroar, L50, M|100/100",
        "|-ability|p2a: Salamence|Intimidate|boost",
        "|detailschange|p2a: Salamence|Salamence-Mega, L50, M",
        "|-mega|p2a: Salamence|Salamence|Salamencite",
        "|move|p2b: Incineroar|Fake Out|p1a: Garchomp",
        "|-enditem|p2b: Incineroar|Sitrus Berry|[eat]",
        "|-heal|p2b: Incineroar|75/100|[from] item: Sitrus Berry",
        "|switch|p2b: Kingambit|Kingambit, L50, M|100/100",
        "|-damage|p1a: Garchomp|80/100|[from] item: Rocky Helmet|[of] p2b: Kingambit",
    ])
    view = json.loads(t.view())
    sal, inc, gambit = view["sides"][1]["pokemon"]
    assert (sal["species"], sal["ability"], sal["item"], sal["mega"]) == ("salamencemega", "aerilate", "salamencite", True)
    assert inc["hp"] == 75 and [m["id"] for m in inc["moves"]] == ["fakeout"]
    assert inc["item"] == "" and inc["slot"] is None  # the eaten berry stays gone
    assert gambit["item"] == "rockyhelmet"  # Rocky Helmet names its holder in [of]


# ---------------------------------------------------------------------------
# State import: the engine must restore every state it can be in, so that a
# search can re-roll a copy of any position. Each scenario plays scripted turns
# on Showdown to reach a position with the state under test, imports it, checks
# that the engine's projection is the projection it was given, and compares
# 100 re-rolls of the next turn with Showdown's copy.


def _set(species, ability, moves, gender="", item="", nature="Hardy"):
    return f"{species}||{item}|{ability}|{','.join(moves)}|{nature}||{gender}|||50|"


def _team(*mons):
    return "]".join(mons)


IDLE = ("Teatime", "BodySlam", "Protect", "Detect")
BENCH = (_set("Snorlax", "thickfat", IDLE, "M"), _set("Clefable", "magicguard", IDLE, "F"))
# Each scenario: team 1, team 2, the turns played (a choice per side; a side
# left out has no request), and a test that the position holds the state.
RESTORE_SCENARIOS = {
    # Trapping, an attraction and a salted, leeched foe: several residual
    # volatiles on one Pokémon, whose order decides how their ties resolve.
    "volatile stack": (
        _team(
            _set("Garchomp", "roughskin", ("Attract", "Infestation", "Protect", "Teatime"), "F"),
            _set("Umbreon", "synchronize", ("MeanLook", "SaltCure", "LeechSeed", "Teatime"), "M"),
            *BENCH,
        ),
        _team(_set("Snorlax", "thickfat", IDLE, "M"), _set("Hatterene", "magicbounce", IDLE, "F"), *BENCH),
        [
            {"p1": "move 2 1, move 1 1", "p2": "move 1, move 1"},
            {"p1": "move 1 1, move 2 1", "p2": "move 1, move 1"},
            {"p1": "move 4, move 3 1", "p2": "move 1, move 1"},
        ],
        lambda s: {"partiallytrapped", "saltcure", "attract", "trapped", "leechseed"} <= set(s["sides"][1]["pokemon"][0]["volatiles"]),
    ),
    # Octolock, Syrup Bomb, Lock-On, Power Trick, No Retreat, Ingrain, Aqua Ring, Uproar.
    "holds and locks": (
        _team(
            _set("Grapploct", "limber", ("Octolock", "LockOn", "SyrupBomb", "Teatime"), "M"),
            _set("Appletun", "ripen", ("SyrupBomb", "Ingrain", "Protect", "Teatime"), "M"),
            *BENCH,
        ),
        _team(
            _set("Slowbro", "oblivious", ("PowerTrick", "AquaRing", "Protect", "Teatime"), "M"),
            _set("Kommo-o", "overcoat", ("NoRetreat", "Uproar", "Protect", "Teatime"), "M"),
            *BENCH,
        ),
        [
            {"p1": "move 1 1, move 1 2", "p2": "move 1, move 1"},
            {"p1": "move 2 1, move 2", "p2": "move 2, move 2"},
        ],
        lambda s: {"octolock", "powertrick", "lockon", "syrupbomb", "noretreat", "uproar"}
        <= {v for side in s["sides"] for mon in side["pokemon"] if mon["isActive"] for v in mon["volatiles"]},
    ),
    # Slot conditions, Magic Room, and stats traded by Speed Swap and Guard Split.
    "slots and room": (
        _team(
            _set("Gardevoir", "synchronize", ("FutureSight", "Protect", "Teatime", "Wish"), "F"),
            _set("Clefable", "magicguard", ("Wish", "HealingWish", "Protect", "Teatime"), "F"),
            *BENCH,
        ),
        _team(
            _set("Alakazam", "magicguard", ("MagicRoom", "SpeedSwap", "Protect", "Teatime"), "M"),
            _set("Slowking", "oblivious", ("GuardSplit", "Protect", "Teatime", "Wish"), "M"),
            *BENCH,
        ),
        [
            {"p1": "move 1 1, move 4", "p2": "move 1, move 1 1"},
            {"p1": "move 4, move 2", "p2": "move 2 1, move 3"},
            {"p1": "pass, switch 3"},
        ],
        lambda s: {"slot:futuremove", "slot:wish", "slot:healingwish", "field:magicroom"}
        <= {f"slot:{c}" for side in s["sides"] for conds in side["slotConditions"] for c in conds}
        | {f"field:{c}" for c in s["field"]["pseudoWeather"]},
    ),
    # Cud Chew's berry and Belch (a berry eaten), Rage Fist (hits taken).
    "berries and hits": (
        _team(
            _set("Farigiraf", "cudchew", ("Teatime", "Belch", "Protect", "Detect"), "M", item="oranberry"),
            _set("Snorlax", "thickfat", ("RageFist", "Protect", "Detect", "Teatime"), "M"),
            *BENCH,
        ),
        _team(
            _set("Gyarados", "moxie", ("BodySlam", "Protect", "Detect", "Teatime"), "M"),
            _set("Hippowdon", "sandstream", ("BodySlam", "Protect", "Detect", "Teatime"), "M"),
            *BENCH,
        ),
        [
            {"p1": "move 1, move 4", "p2": "move 1 2, move 1 2"},
        ],
        lambda s: s["sides"][0]["pokemon"][0]["abilityState"].get("berry") == "oranberry"
        and s["sides"][0]["pokemon"][0]["ateBerry"]
        and s["sides"][0]["pokemon"][1]["timesAttacked"] >= 2,
    ),
}


@pytest.mark.parametrize("name", sorted(RESTORE_SCENARIOS))
def test_import_restores_state_and_rerolls_like_showdown(name):
    from ninjask.choices import joint_choices
    from ninjask.verify import check_point, significant

    t1, t2, turns, reached = RESTORE_SCENARIOS[name]
    results = []
    reached_seeds = 0
    with RefSim() as sim:
        for seed in range(1, 7):
            battle = Battle(sim, FORMAT, seed_from_int(seed, "restore"), ("A", t1), ("B", t2))
            battle.start()
            battle.choose("p1", "team 1234")
            obs = battle.choose("p2", "team 1234")
            for turn in turns:
                for side, choice in turn.items():
                    obs = battle.choose(side, choice)
                    assert obs.errors[side] is None, (name, side, choice, obs.errors[side])
            if not reached(battle.state()):
                # A move missed: the scenario needs another seed.
                battle.close()
                continue
            reached_seeds += 1
            rng = random.Random(seed)
            choices = {s: rng.choice(joint_choices(r)) for s, r in obs.requests.items() if r and not r.get("wait")}
            result = check_point(sim, FORMAT, battle, seed, seed, choices, 100)
            assert result.error is None, (name, seed, result.error)
            results.append(result)
    assert reached_seeds >= 2, name
    assert significant(results) == []


def test_import_restores_opportunists_boosts_in_order():
    """Opportunist's pending boosts live in an object Showdown never holds at a request, so
    this position is made from a real one: the pending boosts are written into its projection."""
    t1 = _team(
        _set("Snorlax", "opportunist", IDLE, "M"),
        _set("Clefable", "magicguard", IDLE, "F"),
        *BENCH,
    )
    t2 = _team(_set("Snorlax", "thickfat", IDLE, "M"), _set("Hatterene", "magicbounce", IDLE, "F"), *BENCH)
    with RefSim() as sim:
        battle = Battle(sim, FORMAT, seed_from_int(1, "restore"), ("A", t1), ("B", t2))
        battle.start()
        battle.choose("p1", "team 1234")
        battle.choose("p2", "team 1234")
        state = battle.state()
        battle.close()
    state["sides"][0]["pokemon"][0]["abilityState"]["oppBoosts"] = [["spe", 2], ["atk", 1]]
    eng = ninjask.Battle.from_projection(dex(FORMAT), json.dumps(state), 7)
    from ninjask.verify import restore_diff

    assert restore_diff(state, json.loads(eng.projection())) == []
    eng.set_logging(True)
    eng.choose("p1", "move 1, move 1")
    eng.choose("p2", "move 1, move 1")
    log = [line for line in eng.log("omniscient", 0) if "Opportunist" in line or "-boost" in line]
    assert log[:3] == [
        "|-ability|p1a: Snorlax|Opportunist|boost",
        "|-boost|p1a: Snorlax|spe|2",
        "|-boost|p1a: Snorlax|atk|1",
    ], log


class _ScriptedChoices:
    """Stands in for the `random.Random` of `check_reference.play`: the scripted
    choices first (each must be on offer), then random ones."""

    def __init__(self, script):
        self.script = list(script)
        self.rng = random.Random(1)

    def choice(self, options):
        if self.script:
            want = self.script.pop(0)
            assert want in options, (want, options)
            return want
        return self.rng.choice(list(options))


def _check_reference():
    import importlib.util

    spec = importlib.util.spec_from_file_location("check_reference", ENGINE_ROOT / "scripts" / "check_reference.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@pytest.mark.parametrize("seed", range(1, 5))
def test_parental_bond_pollen_puff_heals_a_full_ally_like_showdown(seed, capsys):
    """Parental Bond reaches a Pollen Puff user only by Skill Swap (a Mega Kangaskhan
    cannot learn the move): the first hit tops the ally up, the second finds it full."""
    cr = _check_reference()

    def mon(species, item, ability, moves, nature="Adamant"):
        return f"{species}||{item}|{ability}|{','.join(moves)}|{nature}||||||50|"

    t1 = _team(
        mon("Kangaskhan", "kangaskhanite", "scrappy", ["BodySlam", "Protect", "FakeOut", "Earthquake"]),
        mon("Meowscarada", "", "overgrow", ["PollenPuff", "SkillSwap", "Protect", "FlowerTrick"]),
        mon("Snorlax", "", "thickfat", ["BodySlam", "Protect", "Curse", "Rest"]),
        mon("Meganium", "", "overgrow", ["PollenPuff", "Protect", "GrassKnot", "Synthesis"]),
    )
    t2 = _team(
        mon("Garchomp", "", "roughskin", ["Earthquake", "Protect", "DragonClaw", "Substitute"]),
        mon("Incineroar", "", "intimidate", ["FakeOut", "Protect", "FlareBlitz", "KnockOff"]),
        mon("Snorlax", "", "thickfat", ["BodySlam", "Protect", "Curse", "Rest"]),
        mon("Meganium", "", "overgrow", ["PollenPuff", "Protect", "GrassKnot", "Synthesis"]),
    )
    script = [
        "team 1234", "team 1234",
        "move 1 1 mega, move 2 -1", "move 2, move 4 1",  # Mega, Skill Swap; Knock Off hurts Kangaskhan
        "move 1 1, move 1 -1", "move 4, move 2",  # Pollen Puff on the hurt ally
    ]
    with RefSim() as sim:
        cr.play(sim, dex(cr.FORMAT), seed_from_int(seed, salt="pb"), t1, t2, _ScriptedChoices(script), True)
    log = capsys.readouterr().out.splitlines()
    puff = next(i for i, line in enumerate(log) if line.strip().startswith("|move|p1b: Meowscarada|Pollen Puff|p1a: Kangaskhan"))
    assert [line.strip().split("|")[1] for line in log[puff + 1 : puff + 3]] == ["-heal", "-hitcount"], log[puff : puff + 4]
    assert log[puff + 1].strip().endswith("180/180") and log[puff + 2].strip().endswith("|2")


def test_doubles_endgame_survivor_still_needs_a_target():
    """A doubles side down to one active Pokémon: the engine rejects a
    bare single-target move from the survivor and accepts the targeted
    form — the ground truth `joint_choices(game_type="doubles")` encodes.
    The battle is scripted: Earthquake faints both Pikachu leads, Outrage
    one of the replacements, leaving a lone Pikachu whose Quick Attack
    needs a target (its Fake Out is request-disabled by now: not its
    first turn out)."""
    from ninjask.choices import joint_choices

    pika = _set("Pikachu", "static", ("FakeOut", "QuickAttack", "Protect", "Detect"))
    t2 = _team(pika, pika, pika, pika)
    t1 = _team(
        _set("Garchomp", "roughskin", ("Earthquake", "Outrage", "Protect", "SwordsDance"), "M", nature="Adamant"),
        _set("Snorlax", "thickfat", IDLE, "M"),
        _set("Clefable", "magicguard", IDLE, "F"),
        _set("Pelipper", "drizzle", IDLE, "F"),
    )
    b = ninjask.Battle(dex(FORMAT), t1, t2, 7)
    b.choose("p1", "team 1234")
    b.choose("p2", "team 1234")
    b.choose("p2", "move 2 1, move 2 1")  # Quick Attacks; Earthquake faints both leads
    b.choose("p1", "move 1, move 3")
    b.choose("p2", "switch 3, switch 4")
    b.choose("p1", "move 2, move 3")  # Outrage faints one replacement
    b.choose("p2", "move 2 1, move 2 1")

    req = json.loads(b.request("p2"))
    assert req.get("active") and not req.get("forceSwitch")
    survivor = next(
        s for s, mon in enumerate(req["side"]["pokemon"][:2]) if not mon["condition"].startswith("0 fnt")
    )
    targeted = ", ".join("move 2 1" if s == survivor else "pass" for s in range(2))
    bare = ", ".join("move 2" if s == survivor else "pass" for s in range(2))
    choices = joint_choices(req, game_type="doubles")
    assert targeted in choices
    assert bare not in choices

    b.choose("p1", "move 1, move 3")  # locked Outrage, Protect
    with pytest.raises(ValueError, match="needs a target"):
        b.choose("p2", bare)
    b.choose("p2", targeted)  # Quick Attack with its target: accepted


def test_verification_pool_paths(tmp_path, monkeypatch):
    from ninjask import FIXTURE_POOL, default_pool, verification_teams

    fixture = verification_teams(FIXTURE_POOL)
    assert len(fixture) == 4
    (tmp_path / "supported-a.txt").write_text("\n".join(fixture[:3]) + "\n")
    (tmp_path / "supported-b.txt").write_text("\n".join(fixture[2:]) + "\n")
    (tmp_path / "other.txt").write_text("not a team\n")
    assert verification_teams(tmp_path) == fixture  # name order, without duplicates
    monkeypatch.setenv("NINJASK_POOL", str(tmp_path))
    assert default_pool() == tmp_path


def _packed(species, item, ability, moves, nature="Adamant"):
    return f"{species}||{item}|{ability}|{','.join(moves)}|{nature}||||||50|"


def _play_scripted(seeds, salt, t1, t2, script, capsys):
    """Each seed played against Showdown (`check_reference.play` raises on any
    difference in logs, state or draws); the logs printed, per seed."""
    cr = _check_reference()
    logs = []
    with RefSim() as sim:
        for seed in seeds:
            cr.play(sim, dex(cr.FORMAT), seed_from_int(seed, salt=salt), t1, t2, _ScriptedChoices(script), True)
            logs.append([line.strip() for line in capsys.readouterr().out.splitlines()])
    return logs


def test_curse_chosen_before_mega_evolving_into_a_ghost_bypasses_good_as_gold(capsys):
    """Absol chooses Curse while still Dark (the request aims it at itself)
    and Mega Evolves into Dark/Ghost Absol-Mega-Z the same turn. Showdown's
    queued move keeps its 'self' target, so Champions' onHit curses a
    random foe directly: Good as Gold never sees it, and Gholdengo can be
    the one cursed."""
    idle = ["CalmMind"]
    t1 = _team(
        _packed("Absol", "absolitez", "justified", ["Curse", "CalmMind"]),
        _packed("Pikachu", "", "static", idle),
        _packed("Snorlax", "", "thickfat", idle),
        _packed("Garchomp", "", "roughskin", idle),
    )
    t2 = _team(
        _packed("Gholdengo", "leftovers", "goodasgold", idle),
        _packed("Pikachu", "", "static", idle),
        _packed("Snorlax", "", "thickfat", idle),
        _packed("Garchomp", "", "roughskin", idle),
    )
    script = ["team 1234", "team 1234", "move 1 mega, move 1", "move 1, move 1"]
    logs = _play_scripted(range(8), "curse", t1, t2, script, capsys)
    cursed = [next(line.split("|")[2] for line in log if line.startswith("|-start|") and "|Curse|" in line) for log in logs]
    assert "p2a: Gholdengo" in cursed, cursed
    assert not any("Good as Gold" in line for log in logs for line in log)


def test_metal_burst_and_comeuppance_leave_a_substitute_half_points():
    """1.5 times an odd damage reaches a substitute unrounded (Substitute's
    onTryPrimaryHit takes getDamage as it is), so Showdown leaves it x.5 HP;
    the projection says so, and the reference check compares it."""
    idle = ["CalmMind"]
    t1 = _team(
        _packed("Aggron", "", "sturdy", ["MetalBurst", "CalmMind"]),
        _packed("Kingambit", "", "defiant", ["Comeuppance", "CalmMind"]),
        _packed("Snorlax", "", "thickfat", idle),
        _packed("Garchomp", "", "roughskin", idle),
    )
    t2 = _team(
        _packed("Dragonite", "", "innerfocus", ["Substitute", "RockSlide", "CalmMind"], "Modest"),
        _packed("Gholdengo", "", "goodasgold", idle),
        _packed("Snorlax", "", "thickfat", idle),
        _packed("Garchomp", "", "roughskin", idle),
    )
    script = ["team 1234", "team 1234", "move 2, move 2", "move 1, move 1", "move 1, move 1", "move 2, move 1"]
    d = dex(_check_reference().FORMAT)
    hps = []
    for seed in range(12):
        b = ninjask.Battle(d, t1, t2, seed)
        for side, choice in zip(["p1", "p2"] * 3, script):
            b.choose(side, choice)
        hps.append(json.loads(b.projection())["sides"][1]["pokemon"][0]["volatiles"].get("substitute", {}).get("hp"))
    assert any(isinstance(hp, float) and hp % 1 == 0.5 for hp in hps), hps


def test_metal_burst_into_a_substitute_matches_showdown(capsys):
    """The half points carry on: Metal Burst and Comeuppance into one
    substitute, then Drain Punch breaking what is left (its drain is
    rounded up from the x.5 damage)."""
    idle = ["CalmMind"]
    t1 = _team(
        _packed("Aggron", "", "sturdy", ["MetalBurst", "CalmMind", "DrainPunch"]),
        _packed("Kingambit", "", "defiant", ["Comeuppance", "CalmMind"]),
        _packed("Snorlax", "", "thickfat", idle),
        _packed("Garchomp", "", "roughskin", idle),
    )
    t2 = _team(
        _packed("Dragonite", "", "innerfocus", ["Substitute", "RockSlide", "CalmMind"], "Modest"),
        _packed("Gholdengo", "", "goodasgold", idle),
        _packed("Snorlax", "", "thickfat", idle),
        _packed("Garchomp", "", "roughskin", idle),
    )
    script = ["team 1234", "team 1234", "move 2, move 2", "move 1, move 1", "move 1, move 1", "move 2, move 1", "move 3 1, move 2", "move 3, move 1"]
    logs = _play_scripted(range(12), "burst", t1, t2, script, capsys)
    assert any(any(line.endswith("[from] drain|[of] p2a: Dragonite") for line in log) for log in logs)


ITEM_START_CASES = {
    # setItem's Start runs on a holder that ignores its item: a Klutz Golurk
    # that takes a Metronome by Trick gets the Metronome volatile...
    "trick a metronome onto klutz": (
        _team(
            _packed("Golurk", "", "klutz", ["Trick", "CalmMind"], "Modest"),
            _packed("Pikachu", "", "static", ["CalmMind"], "Timid"),
            _packed("Snorlax", "", "thickfat", ["CalmMind"], "Calm"),
            _packed("Garchomp", "", "roughskin", ["CalmMind"], "Jolly"),
        ),
        _team(
            _packed("Gliscor", "metronome", "hypercutter", ["PoisonJab", "CalmMind"]),
            _packed("Pikachu", "", "static", ["CalmMind"], "Timid"),
            _packed("Snorlax", "", "thickfat", ["CalmMind"], "Calm"),
            _packed("Garchomp", "", "roughskin", ["CalmMind"], "Jolly"),
        ),
        ["team 1234", "team 1234", "move 1 1, move 1", "move 2, move 1", "move 2, move 1", "move 2, move 1"],
    ),
    # ...and one handed a White Herb with a stat lowered uses it up, though
    # the reset itself (onUse) is skipped.
    "trick a white herb onto klutz": (
        _team(
            _packed("Golurk", "", "klutz", ["CalmMind"], "Modest"),
            _packed("Pikachu", "", "static", ["CalmMind"], "Timid"),
            _packed("Snorlax", "", "thickfat", ["CalmMind"], "Calm"),
            _packed("Garchomp", "", "roughskin", ["CalmMind"], "Jolly"),
        ),
        _team(
            _packed("Florges", "whiteherb", "flowerveil", ["Trick", "CalmMind"], "Timid"),
            _packed("Pikachu", "", "static", ["IcyWind", "CalmMind"], "Timid"),
            _packed("Snorlax", "", "thickfat", ["CalmMind"], "Calm"),
            _packed("Garchomp", "", "roughskin", ["CalmMind"], "Jolly"),
        ),
        ["team 1234", "team 1234", "move 1, move 1", "move 2, move 1", "move 1, move 1", "move 1 1, move 2"],
    ),
    # At switch-in the item's handlers go through singleEvent('SwitchIn'),
    # which skips an ignored item: a Klutz Metronome holder gets no volatile,
    # and a White Herb under Magic Room stays put when a Pokémon comes in.
    "klutz metronome switching in; white herb under magic room": (
        _team(
            _packed("Snorlax", "whiteherb", "thickfat", ["CalmMind"], "Calm"),
            _packed("Pikachu", "", "static", ["CalmMind"], "Timid"),
            _packed("Golurk", "metronome", "klutz", ["CalmMind"], "Modest"),
            _packed("Florges", "", "flowerveil", ["CalmMind"], "Calm"),
        ),
        _team(
            _packed("Golurk", "", "noguard", ["MagicRoom", "CalmMind"], "Modest"),
            _packed("Pikachu", "", "static", ["IcyWind", "CalmMind"], "Timid"),
            _packed("Snorlax", "", "thickfat", ["CalmMind"], "Calm"),
            _packed("Garchomp", "", "roughskin", ["CalmMind"], "Jolly"),
        ),
        ["team 1234", "team 1234", "move 1, move 1", "move 1, move 2", "move 1, move 1", "move 2, move 1", "move 1, switch 3", "move 2, move 2"],
    ),
}


@pytest.mark.parametrize("case", sorted(ITEM_START_CASES))
def test_item_start_on_a_holder_that_ignores_its_item_matches_showdown(case, capsys):
    t1, t2, script = ITEM_START_CASES[case]
    _play_scripted(range(4), "items", t1, t2, script, capsys)


@pytest.mark.parametrize("seed", range(3))
def test_reflect_type_under_illusion_keeps_the_disguise_types_like_showdown(seed, capsys):
    """A disguised Zoroark that Reflect Types a foe: Showdown's end-of-turn
    type catch-up looks at `pokemon.illusion || pokemon`, the teammate it
    appears as, so nothing is shown while the disguise holds; once a hit
    breaks it, the Zoroark's real (copied) types are shown at the next one."""
    t1 = _team(
        _packed("Vivillon", "", "compoundeyes", ["CalmMind", "StruggleBug"], "Timid"),
        _packed("Pikachu", "", "static", ["CalmMind"], "Timid"),
        _packed("Snorlax", "", "thickfat", ["CalmMind"], "Calm"),
        _packed("Garchomp", "", "roughskin", ["CalmMind"], "Jolly"),
    )
    t2 = _team(
        _packed("Zoroark-Hisui", "", "illusion", ["ReflectType", "CalmMind"], "Bold"),
        _packed("Pikachu", "", "static", ["CalmMind"], "Timid"),
        _packed("Snorlax", "", "thickfat", ["CalmMind"], "Calm"),
        _packed("Wyrdeer", "", "intimidate", ["CalmMind"], "Calm"),
    )
    script = ["team 1234", "team 1234", "move 1, move 1", "move 1 1, move 1", "move 2, move 1", "move 2, move 1",
              "move 1, move 1", "move 2, move 1"]
    log = _play_scripted([seed], "illusion", t1, t2, script, capsys)[0]
    assert any("|-start|p2a: Wyrdeer|typechange|[from] move: Reflect Type" in line for line in log)
    broken = next(i for i, line in enumerate(log) if line.startswith("|-end|p2a: Zoroark|Illusion"))
    assert not any("typechange|Bug/Flying|[silent]" in line for line in log[:broken])
    assert any(line == "|-start|p2a: Zoroark|typechange|Bug/Flying|[silent]" for line in log[broken:])


@pytest.mark.parametrize("seed", range(3))
def test_regional_forme_keeps_its_species_mega_stone_like_showdown(seed, capsys):
    """Most Mega Stones' onTakeItem goes by `baseSpecies.baseSpecies`, so a
    Slowbro-Galar holding Slowbronite keeps it: Trick fails, and Knock Off
    neither removes it nor gets its 1.5x boost."""
    t1 = _team(
        _packed("Golurk", "zoomlens", "noguard", ["Trick", "KnockOff", "CalmMind"], "Adamant"),
        _packed("Pikachu", "", "static", ["CalmMind"], "Timid"),
        _packed("Snorlax", "", "thickfat", ["CalmMind"], "Calm"),
        _packed("Garchomp", "", "roughskin", ["CalmMind"], "Jolly"),
    )
    t2 = _team(
        _packed("Slowbro-Galar", "slowbronite", "quickdraw", ["CalmMind"], "Bold"),
        _packed("Pikachu", "", "static", ["CalmMind"], "Timid"),
        _packed("Snorlax", "", "thickfat", ["CalmMind"], "Calm"),
        _packed("Garchomp", "", "roughskin", ["CalmMind"], "Jolly"),
    )
    script = ["team 1234", "team 1234", "move 1 1, move 1", "move 1, move 1", "move 2 1, move 1", "move 1, move 1"]
    logs = _play_scripted([seed], "megastone", t1, t2, script, capsys)
    assert any("|-fail|p1a: Golurk" in log for log in logs)
