import json

import pytest

from ninjask.format import STATS, load
from ninjask.paths import FIXTURE_POOL, SHOWDOWN_DIR
from ninjask.refsim import Battle, RefSim, seed_from_int

pytestmark = pytest.mark.skipif(
    not (SHOWDOWN_DIR / "dist").exists(),
    reason="Pokémon Showdown not built (scripts/setup_showdown.sh)",
)

FORMAT = "gen9championsvgc2026regmc"
TEAMS = FIXTURE_POOL.read_text(encoding="utf-8").splitlines()


@pytest.fixture(scope="module")
def spec():
    return load(FORMAT)


def test_format_rules_as_data(spec):
    assert spec.game_type == "doubles"
    assert spec.picked_team_size == 4
    assert spec.stat_point_limit == 66
    assert "garchomp" in spec.team_legal_species
    assert "garchompmegaz" in spec.species and spec.species["garchompmegaz"]["battleOnly"]
    assert "earthquake" in spec.learnset("Garchomp")


def test_type_chart(spec):
    assert spec.effectiveness("Ground", ["Flying"]) == 0.0
    assert spec.effectiveness("Ice", ["Dragon", "Ground"]) == 4.0
    assert spec.effectiveness("Fire", ["Water", "Grass"]) == 1.0


def test_regulation_diff_matches_published_additions():
    mb, mc = load("gen9championsvgc2026regmb"), load(FORMAT)
    added = mc.team_legal_species - mb.team_legal_species
    new_megas = {s for s, d in mc.species.items() if d["isMega"]} - {s for s, d in mb.species.items() if d["isMega"]}
    # Victory Road: 29 Pokémon additions and 6 new Mega Evolutions in Reg M-C.
    assert len(added) == 29 and len(new_megas) == 6
    assert not (mb.team_legal_species - mc.team_legal_species)


def test_stat_formula_matches_showdown(spec):
    team = TEAMS[0]
    with RefSim() as sim:
        battle = Battle(sim, FORMAT, seed_from_int(0), ("A", team), ("B", TEAMS[1]))
        battle.start()
        state = battle.state()
        battle.close()
    for packed, mon in zip(team.split("]"), state["sides"][0]["pokemon"]):
        f = packed.split("|")
        species, nature = f[1] or f[0], f[5]
        evs = (f[6].split(",") + [""] * 6)[:6]
        points = {stat: int(x) if x else 0 for stat, x in zip(STATS, evs)}
        assert mon["maxhp"] == spec.stat(species, "hp", points["hp"], nature)
        for stat in STATS[1:]:
            assert mon["storedStats"][stat] == spec.stat(species, stat, points[stat], nature), (species, stat)


def test_export_path_prefers_generated_then_bundled(tmp_path, monkeypatch):
    import ninjask.format as fmt

    cache, bundled = tmp_path / "cache", tmp_path / "bundled"
    cache.mkdir()
    bundled.mkdir()
    monkeypatch.setattr(fmt, "CACHE", cache)
    monkeypatch.setattr(fmt, "BUNDLED", bundled)
    (bundled / "x.json").write_text("{}")
    assert fmt.export_path("x") == bundled / "x.json"  # a wheel's copy
    (cache / "x.json").write_text("{}")
    assert fmt.export_path("x") == cache / "x.json"  # generated data first


def test_load_is_cached_per_export_file(tmp_path, monkeypatch):
    import os

    import ninjask.format as fmt

    monkeypatch.setattr(fmt, "CACHE", tmp_path)
    path = tmp_path / "x.json"
    path.write_text(json.dumps({"format": {"id": "x"}, "species": [], "moves": [], "items": [], "abilities": []}))
    first = fmt.load("x")
    assert fmt.load("x") is first  # parsed once
    path.write_text(json.dumps({"format": {"id": "y"}, "species": [], "moves": [], "items": [], "abilities": []}))
    os.utime(path, ns=(path.stat().st_atime_ns, path.stat().st_mtime_ns + 1_000_000))
    assert fmt.load("x").id == "y"  # a rewritten export is read again
