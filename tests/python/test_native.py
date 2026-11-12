"""The native interface: player-view legal choices, numeric actions and observations, and Batch.

These need a format export (bundled with the package, or exported in a checkout), not Showdown.
"""

import json
import random

import pytest

ninjask = pytest.importorskip("ninjask")
np = pytest.importorskip("numpy")

from ninjask.choices import joint_choices, team_preview_choices  # noqa: E402

FORMAT = "gen9championsvgc2026regmc"


@pytest.fixture(scope="module")
def d():
    try:
        return ninjask.dex(FORMAT)
    except Exception as e:  # no export and no Showdown to make one
        pytest.skip(f"no {FORMAT} export: {e}")


@pytest.fixture(scope="module")
def teams(d):
    rng = random.Random(1)
    synthetic = [ninjask.supported_test_team(FORMAT, rng) for _ in range(12)]
    # Teams that make the engine know what the player doesn't: a foe's Imprison, hidden traps.
    focus = ninjask.focus_teams(FORMAT, ("imprison", "shadowtag", "meanlook"), [], synthetic=8, seed=2)
    return ninjask.verification_teams() + synthetic + focus


def request_choices(b, side):
    req = json.loads(b.request(side))
    if req.get("teamPreview"):
        return team_preview_choices(req)
    return joint_choices(req, include_ally_targets=True, game_type="doubles")


def test_legal_choices_are_the_requests(d, teams):
    """At every decision, the engine's list equals what ninjask.choices lists from the request JSON."""
    rng = random.Random(3)
    decisions = refused = 0
    for g in range(40):
        b = ninjask.Battle(d, teams[g % len(teams)], teams[(5 * g + 1) % len(teams)], seed=g)
        while not b.ended and b.turn < 60:
            for side in ("p1", "p2"):
                if not b.needs_choice(side):
                    continue
                for _ in range(30):
                    choices = b.legal_choices(side)
                    assert choices == request_choices(b, side)
                    decisions += 1
                    try:
                        b.choose_index(side, rng.randrange(len(choices)))
                        break
                    except ValueError:
                        refused += 1  # something hidden; the request now shows it
    assert decisions > 500


def test_legal_actions_layout(d, teams):
    b = ninjask.Battle(d, teams[0], teams[1], seed=1)
    # Team preview: one row per order, the team positions sent.
    rows = np.frombuffer(b.legal_actions("p1"), np.int8).reshape(-1, b.action_width("p1"))
    choices = b.legal_choices("p1")
    assert len(rows) == len(choices) and b.action_width("p1") == 4
    assert choices[0] == "team " + "".join(str(x) for x in rows[0])
    b.choose("p1", "team 1234")
    b.choose("p2", "team 1234")
    width = b.action_width("p1")
    assert width == 2 * ninjask.ACTION_FIELDS
    rows = np.frombuffer(b.legal_actions("p1"), np.int8).reshape(-1, width)
    choices = b.legal_choices("p1")
    assert len(rows) == len(choices)
    kinds = {1: "move", 2: "switch", 0: "pass", 3: "move"}
    for row, text in zip(rows, choices):
        for slot, part in enumerate(text.split(", ")):
            kind, index, target, mega = row[slot * 4 : slot * 4 + 4]
            words = part.split()
            assert words[0] == kinds[int(kind)]
            assert int(words[1]) == index or kind == 3
            if kind == 1:
                assert (int(words[2]) if len(words) > 2 and words[2] != "mega" else 0) == target
                assert ("mega" in words) == bool(mega)


def test_choose_index(d, teams):
    b = ninjask.Battle(d, teams[0], teams[1], seed=2)
    with pytest.raises(ValueError):
        b.choose_index("p1", 10_000)
    b.choose_index("p1", 0)
    assert not b.needs_choice("p1") and b.needs_choice("p2")
    assert b.legal_choices("p1") == [] and b.action_width("p1") == 0


def test_encodings(d, teams):
    b = ninjask.Battle(d, teams[2], teams[3], seed=3)
    b.choose("p1", "team 1234")
    b.choose("p2", "team 1234")
    t = ninjask.Tracker(d, "p1")
    t.feed(b.log("p1", 0))
    t.feed_battle(b)
    obs = np.frombuffer(t.encode(), np.float32)
    ids = np.frombuffer(t.encode_ids(), np.int16).reshape(2, 6, 7)
    assert obs.shape == (ninjask.OBS_LEN,) and ids.size == ninjask.IDS_LEN
    vocab = d.vocab()
    assert ids[..., 0].max() < vocab["species"] and ids[..., 3:].max() < vocab["moves"]
    # Our own team is known; the foe's moves, items and abilities are not before they show.
    assert (ids[0, :4, 3] > 0).all()
    assert (ids[1, :, 3:] == 0).all() and (ids[1, :, 1] == 0).all()
    assert (ids[1, :, 0] > 0).sum() == 6  # team preview shows the foe's species


def test_battle_encodings(d, teams):
    """Battle.encode(side) and encode_ids(side): the tracker's layout, filled with everything known."""
    b = ninjask.Battle(d, teams[2], teams[3], seed=3)
    b.choose("p1", "team 1234")
    b.choose("p2", "team 1234")
    t = ninjask.Tracker(d, "p1")
    t.feed(b.log("p1", 0))
    t.feed_battle(b)
    assert (ninjask.BATTLE_OBS_LEN, ninjask.BATTLE_IDS_LEN) == (ninjask.OBS_LEN, ninjask.IDS_LEN)
    mon, side = 19, 6 * 19 + 6  # floats per Pokémon and per side (ninjask::encode)
    obs = {s: np.frombuffer(b.encode(s), np.float32) for s in ("p1", "p2")}
    ids = {s: np.frombuffer(b.encode_ids(s), np.int16).reshape(2, 6, 7) for s in ("p1", "p2")}
    assert all(o.shape == (ninjask.BATTLE_OBS_LEN,) for o in obs.values())
    # A side's block is the same seen from either side; the field is shared.
    assert (obs["p1"][:side] == obs["p2"][side : 2 * side]).all()
    assert (obs["p1"][side : 2 * side] == obs["p2"][:side]).all()
    assert (obs["p1"][2 * side :] == obs["p2"][2 * side :]).all()
    assert (ids["p1"][0] == ids["p2"][1]).all() and (ids["p1"][1] == ids["p2"][0]).all()
    # Four brought per side: species, item (1 for none) and ability known; empty slots are 0.
    vocab = d.vocab()
    for s in (0, 1):
        assert (ids["p1"][s, :4, :4] > 0).all() and (ids["p1"][s, 4:] == 0).all()
    assert ids["p1"][..., 0].max() < vocab["species"] and ids["p1"][..., 1].max() < vocab["items"]
    assert ids["p1"][..., 2].max() < vocab["abilities"] and ids["p1"][..., 3:].max() < vocab["moves"]
    # The player's own four brought are fully known to its tracker, in the same order at the start:
    # the same ids, and the same floats but `seen` (the battle sets it for every Pokémon brought;
    # the tracker, for those that have been in). The tracker also lists the two left out (species
    # only), which the battle doesn't have.
    view_ids = np.frombuffer(t.encode_ids(), np.int16).reshape(2, 6, 7)
    assert (view_ids[0, :4] == ids["p1"][0, :4]).all()
    view = np.frombuffer(t.encode(), np.float32)[: 6 * mon].reshape(6, mon)[:4]
    mine = obs["p1"][: 6 * mon].reshape(6, mon)
    assert (np.delete(view, 2, axis=1) == np.delete(mine[:4], 2, axis=1)).all()
    assert (mine[:4, 2] == 1).all() and (mine[4:] == 0).all()
    with pytest.raises(ValueError):
        b.encode("p3")


def test_batch_matches_single_battles(d, teams):
    """A Batch plays exactly as each battle alone, with the same choices."""
    n = 8
    setups = [(teams[(2 * i) % len(teams)], teams[(2 * i + 1) % len(teams)], 50 + i) for i in range(n)]
    pick = lambda i, s, k, count: (31 * i + 17 * s + 7 * k) % count

    batch = ninjask.Batch(d, n)
    batch.reset_many(list(range(n)), [a for a, _, _ in setups], [b for _, b, _ in setups], [s for _, _, s in setups])
    k = [[0, 0] for _ in range(n)]
    seen_b = [[] for _ in range(n)]
    while True:
        o = batch.observe()
        if o["n"] == 0:
            break
        who = np.frombuffer(o["who"], np.uint32).reshape(-1, 2)
        off = np.frombuffer(o["offsets"], np.uint32)
        obs = np.frombuffer(o["obs"], np.float32).reshape(-1, ninjask.OBS_LEN)
        choices = []
        for j, (i, s) in enumerate(who):
            seen_b[i].append(obs[j].tobytes())
            choices.append(pick(int(i), int(s), k[i][s], int(off[j + 1] - off[j])))
            k[i][s] += 1
        batch.step(choices)
    assert all(batch.done(i) for i in range(n))
    assert np.frombuffer(batch.dones(), np.uint8).all()

    for i, (t1, t2, seed) in enumerate(setups):
        b = ninjask.Battle(d, t1, t2, seed=seed)
        trs = [ninjask.Tracker(d, "p1"), ninjask.Tracker(d, "p2")]
        seen, kk, obs_seen = [0, 0], [0, 0], []
        while not b.ended and b.turn < batch.turn_limit:
            picks = []
            for s, side in enumerate(("p1", "p2")):
                if not b.needs_choice(side):
                    continue
                lines = b.log(side, seen[s])
                seen[s] += len(lines)
                trs[s].feed(lines)
                trs[s].feed_battle(b)
                obs_seen.append(trs[s].encode())
                picks.append((side, pick(i, s, kk[s], len(b.legal_choices(side)))))
                kk[s] += 1
            for side, c in picks:
                try:
                    b.choose_index(side, c)
                except ValueError:
                    pass
        assert obs_seen == seen_b[i]
        assert (b.winner_side, b.turn) == (batch.winner(i), batch.turn(i))


def test_batch_errors(d, teams):
    batch = ninjask.Batch(d, 2)
    with pytest.raises(ValueError):
        batch.reset(5, teams[0], teams[1], 1)
    with pytest.raises(ValueError):
        batch.reset(0, "not a team", teams[1], 1)
    batch.reset(0, teams[0], teams[1], 1)
    o = batch.observe()
    assert o["n"] == 2  # both sides at team preview; slot 1 was never started
    with pytest.raises(ValueError):
        batch.step([0])  # one choice for two decisions
    with pytest.raises(ValueError):
        batch.step([0, 10_000])
    # A failed step changes nothing: both decisions are still pending.
    again = batch.observe()
    assert again["n"] == 2 and again["who"] == o["who"]
    assert batch.step([0, 0]) == bytes([0, 0])
