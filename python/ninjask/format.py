"""A format's rules and legal data, loaded from refsim/export_format.js output."""

from __future__ import annotations

import json
import math
import subprocess
from dataclasses import dataclass, field
from functools import cached_property, lru_cache
from pathlib import Path

from ninjask.ids import to_id
from ninjask.paths import FORMAT_DIR, PACKAGE_DATA, REFSIM_DIR, node_env, require_checkout

CACHE = FORMAT_DIR
BUNDLED = PACKAGE_DATA / "formats"
STATS = ("hp", "atk", "def", "spa", "spd", "spe")

# Showdown's damageTaken codes: 0 neutral, 1 weak (2x), 2 resists (0.5x), 3 immune.
_TAKEN = {0: 1.0, 1: 2.0, 2: 0.5, 3: 0.0}


@dataclass
class FormatSpec:
    raw: dict
    species: dict[str, dict] = field(init=False)
    moves: dict[str, dict] = field(init=False)
    items: dict[str, dict] = field(init=False)
    abilities: dict[str, dict] = field(init=False)

    def __post_init__(self) -> None:
        self.species = {s["id"]: s for s in self.raw["species"]}
        self.moves = {m["id"]: m for m in self.raw["moves"]}
        self.items = {i["id"]: i for i in self.raw["items"]}
        self.abilities = {a["id"]: a for a in self.raw["abilities"]}

    @property
    def id(self) -> str:
        return self.raw["format"]["id"]

    @property
    def game_type(self) -> str:
        return self.raw["format"]["gameType"]

    @property
    def open_team_sheets(self) -> bool:
        """Battles in this format are played with both team sheets shown:
        its rules force them open (`forceopenteamsheets`). The base Reg M-C
        format only offers open sheets (`openteamsheets`), which a battle
        accepts or declines case by case."""
        return any(r["rule"] == "forceopenteamsheets" for r in self.raw["format"]["rules"])

    @property
    def sim_format_id(self) -> str:
        """The format id the reference simulator creates battles and
        validations with. A derived format (see refsim/export_format.js)
        has no battle format of its own in Showdown, so its battles use
        the format it derives from, with the difference (open sheets)
        applied by the battle itself."""
        return self.raw["format"].get("derivedFrom") or self.id

    @property
    def picked_team_size(self) -> int | None:
        return self.raw["format"]["pickedTeamSize"]

    @property
    def stat_point_limit(self) -> int | None:
        return self.raw["format"]["evLimit"]

    @property
    def uses_stat_points(self) -> bool:
        return self.raw["format"]["mod"].startswith("champions")

    @cached_property
    def team_legal_species(self) -> set[str]:
        return {s["id"] for s in self.raw["species"] if not s["battleOnly"]}

    def learnset(self, species: str) -> set[str]:
        return set(self.raw["learnsets"].get(to_id(species), []))

    def effectiveness(self, attack_type: str, defender_types: list[str]) -> float:
        mult = 1.0
        for t in defender_types:
            mult *= _TAKEN[self.raw["typechart"][t].get(attack_type, 0)]
        return mult

    def stat(self, species: str, stat: str, points: int, nature: str) -> int:
        """A stat at level 50 under the Champions formula (Showdown's statModify).

        HP = base + points + 75; others = (base + points + 20) x nature.
        """
        if not self.uses_stat_points:
            raise NotImplementedError("only the Champions stat formula is implemented")
        base = self.species[to_id(species)]["baseStats"][stat]
        if stat == "hp":
            return base + points + 75
        value = base + points + 20
        n = self.raw["natures"].get(to_id(nature), {})
        if n.get("plus") == stat:
            value = math.floor(value * 110 / 100)
        elif n.get("minus") == stat:
            value = math.floor(value * 90 / 100)
        return value


def export_path(format_id: str, refresh: bool = False) -> Path:
    """The format's export: the one in `CACHE`, else the copy bundled with an
    installed wheel, else exported from Showdown into `CACHE` now (or always,
    with `refresh`)."""
    path = CACHE / f"{format_id}.json"
    if not refresh:
        if path.exists():
            return path
        if (bundled := BUNDLED / f"{format_id}.json").exists():
            return bundled
    require_checkout(f"exporting {format_id} from Showdown")
    path.parent.mkdir(parents=True, exist_ok=True)
    data = subprocess.run(
        ["node", str(REFSIM_DIR / "export_format.js"), format_id],
        check=True,
        capture_output=True,
        text=True,
        encoding="utf-8",
        env=node_env(),
    ).stdout
    path.write_text(data, encoding="utf-8")
    return path


def load(format_id: str, refresh: bool = False) -> FormatSpec:
    """The format's rules and data. Loaded once per export file and process:
    agents and game loops call this per game, and an export is ~700 KB of
    JSON. The spec is shared, so treat it as read-only; `refresh` re-exports
    and reloads."""
    if refresh:
        _parse.cache_clear()
    path = export_path(format_id, refresh)
    return _parse(str(path), path.stat().st_mtime_ns)


@lru_cache(maxsize=8)
def _parse(path: str, mtime_ns: int) -> FormatSpec:
    return FormatSpec(json.loads(Path(path).read_text(encoding="utf-8")))
