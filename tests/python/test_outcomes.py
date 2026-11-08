"""Self-contained enumeration tests; no reference simulator is required."""
import json
from pathlib import Path
import pytest
from ninjask import Battle, Dex, EnumerationMode, Outcome, BattleStateKey

@pytest.fixture
def battle():
    dex = Dex.load(str(Path(__file__).parents[1] / "fixtures" / "enumeration.json"))
    b = Battle(dex, "Fast|||battlearmor|Tackle,Splash|Hardy|||||50|",
               "Slow|||battlearmor|Splash|Hardy|||||50|", 1)
    b.choose("p1", "team 1")
    b.choose("p2", "team 1")
    return b

def test_modes_and_immutable_input(battle):
    key = battle.state_key()
    assert isinstance(key, BattleStateKey)
    precise = battle.enumerate_outcomes("move 1", "move 1")
    almost = battle.enumerate_outcomes("move 1", "move 1", EnumerationMode.DamageEndpoints)
    assert len(precise) > 2
    assert len(almost) == 2
    assert all(isinstance(o, Outcome) and isinstance(o.battle, Battle) for o in precise)
    assert sum(o.probability for o in precise) == pytest.approx(1)
    assert [o.probability for o in almost] == [0.5, 0.5]
    assert battle.state_key() == key
    assert {key: "same"}[battle.copy().state_key()] == "same"
    assert all(o.battle.state_key() != key for o in precise)
    copied = precise[0].battle
    copied.choose("p1", "move 2")
    assert copied.state_key() != precise[0].battle.state_key()

def test_pending_choice_and_errors(battle):
    expected = {o.battle.state_key(): o.probability for o in battle.enumerate_outcomes("move 1", "move 1")}
    battle.choose("p1", "move 1")
    key = battle.state_key()
    actual = {o.battle.state_key(): o.probability for o in battle.enumerate_outcomes("", "move 1")}
    assert actual == expected
    assert battle.state_key() == key
    with pytest.raises(ValueError):
        battle.enumerate_outcomes("move 1", "move 1")
    with pytest.raises(ValueError):
        battle.enumerate_outcomes("", "move 99")

def test_deterministic_turn(battle):
    outcomes = battle.enumerate_outcomes("move 2", "move 1")
    assert len(outcomes) == 1
    assert outcomes[0].probability == 1


@pytest.mark.parametrize("mode", [EnumerationMode.Precise, EnumerationMode.DamageEndpoints])
def test_accuracy_and_crit_probabilities(mode, tmp_path):
    spec = json.loads((Path(__file__).parents[1] / "fixtures" / "enumeration.json").read_text(encoding="utf-8"))
    spec["moves"][0]["accuracy"] = 75
    fixture = tmp_path / "accuracy.json"
    fixture.write_text(json.dumps(spec))
    dex = Dex.load(str(fixture))
    b = Battle(dex, "Fast|||noability|Tackle|Hardy|||||50|",
               "Slow|||noability|Splash|Hardy|||||50|", 1)
    b.choose("p1", "team 1")
    b.choose("p2", "team 1")
    projection = json.loads(b.projection())
    projection["sides"][1]["pokemon"][0]["hp"] = 30
    b = Battle.from_projection(dex, json.dumps(projection), 1)
    result = b.enumerate_outcomes("move 1", "move 1", mode)
    ko = sum(o.probability for o in result if o.battle.ended)
    miss = sum(o.probability for o in result
               if json.loads(o.battle.projection())["sides"][1]["pokemon"][0]["hp"] == 30)
    assert ko == pytest.approx(0.75 * (178956971 / 4294967296), abs=1e-12, rel=0)
    assert miss == pytest.approx(0.25, abs=1e-12, rel=0)
    assert sum(o.probability for o in result) == pytest.approx(1, abs=1e-12, rel=0)


def test_path_budget(battle):
    import ninjask
    assert ninjask.DEFAULT_MAX_PATHS > 1
    with pytest.raises(ValueError, match="max_paths"):
        battle.enumerate_outcomes("move 1", "move 1", max_paths=1)
    full = battle.enumerate_outcomes("move 1", "move 1")
    bounded = battle.enumerate_outcomes("move 1", "move 1", EnumerationMode.Precise, max_paths=ninjask.DEFAULT_MAX_PATHS)
    assert sorted(o.probability for o in full) == sorted(o.probability for o in bounded)
