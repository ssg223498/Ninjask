"""Integration tests against the pinned Pokémon Showdown (skipped if not built)."""


import pytest

from ninjask.policy import RandomPolicy
from ninjask.paths import FIXTURE_POOL, SHOWDOWN_DIR
from ninjask.refsim import Battle, RefSim, seed_from_int
from ninjask.play import play_game

FORMAT = "gen9championsvgc2026regmc"
TEAMS = FIXTURE_POOL.read_text(encoding="utf-8").splitlines()

pytestmark = pytest.mark.skipif(
    not (SHOWDOWN_DIR / "dist").exists(),
    reason="Pokémon Showdown not built (scripts/setup_showdown.sh)",
)


@pytest.fixture(scope="module")
def sim():
    with RefSim() as s:
        yield s


def test_fixture_teams_are_legal(sim):
    for team in TEAMS:
        assert sim.validate(FORMAT, team) is None


def test_random_games_finish_and_replay_exactly(sim):
    for i in range(10):
        r = play_game(sim, FORMAT, seed_from_int(i), TEAMS[0], TEAMS[1], RandomPolicy(i), RandomPolicy(100 + i), ots=bool(i % 2))
        assert r.winner in ("p1", "p2")
        assert r.inputlog[0].startswith(">start")
        replayed = sim.call(cmd="replay", inputlog=r.inputlog)
        assert replayed["ended"]
        assert replayed["turn"] == r.turns
        winner_name = {"p1": "Player 1", "p2": "Player 2"}[r.winner]
        assert replayed["winner"] == winner_name


def test_same_seed_and_choices_give_same_states(sim):
    r = play_game(sim, FORMAT, seed_from_int(7), TEAMS[2], TEAMS[3], RandomPolicy(1), RandomPolicy(2))
    a = sim.call(cmd="replay", inputlog=r.inputlog, states=True)
    b = sim.call(cmd="replay", inputlog=r.inputlog, states=True)
    assert a["states"] == b["states"]
    assert len(a["states"]) == len(r.inputlog) - sum(line.startswith(">version") for line in r.inputlog)


def test_open_sheets_reveal_everything_but_stat_points(sim):
    battle = Battle(sim, FORMAT, seed_from_int(3), ("A", TEAMS[0]), ("B", TEAMS[1]), ots=True)
    obs = battle.start()
    sheets = [line for line in obs.logs["spectator"] if line.startswith("|showteam|")]
    assert len(sheets) == 2
    # Packed sets: NICK|SPECIES|ITEM|ABILITY|MOVES|NATURE|EVS|... with EVS blank.
    first = sheets[0].split("|", 3)[3].split("]")[0].split("|")
    assert first[2] and first[3] and first[4] and first[5]
    assert first[6] == ""
    battle.close()


def test_state_projection_has_hidden_information(sim):
    battle = Battle(sim, FORMAT, seed_from_int(4), ("A", TEAMS[0]), ("B", TEAMS[1]))
    battle.start()
    state = battle.state()
    assert state["prng"].startswith("sodium,")
    mon = state["sides"][0]["pokemon"][0]
    assert {"hp", "maxhp", "item", "ability", "moves", "storedStats", "boosts"} <= set(mon)
    battle.close()


def test_branch_reproduces_live_play_for_each_seed(sim):
    """A copy re-rolled with seed s must end the turn exactly like the live battle reseeded with s."""
    from ninjask.choices import joint_choices

    def new_battle():
        b = Battle(sim, FORMAT, seed_from_int(1), ("A", TEAMS[0]), ("B", TEAMS[1]))
        b.start()
        b.choose("p1", "team 1234")
        return b, b.choose("p2", "team 1234")

    base, obs = new_battle()
    c1 = [c for c in joint_choices(obs.requests["p1"]) if "switch" not in c][1]
    c2 = [c for c in joint_choices(obs.requests["p2"]) if "switch" not in c][0]
    seeds = [seed_from_int(i, "branch") for i in range(60)]
    branched = sim.call(cmd="branch", id=base.id, choices={"p1": c1, "p2": c2}, seeds=seeds)["results"]
    assert all(r["accepted"] for r in branched)
    outcomes = set()
    for seed, result in zip(seeds, branched):
        live, _ = new_battle()
        sim.call(cmd="reseed", id=live.id, seed=seed)
        live.choose("p1", c1)
        live.choose("p2", c2)
        assert live.state() == result["state"]
        outcomes.add(str(result["state"]["sides"]))
        live.close()
    assert len(outcomes) > 1  # different seeds really give different outcomes
    base.close()
