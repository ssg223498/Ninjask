"""Type stubs for the compiled classes (src/python.rs). The `ninjask` package
re-exports them; see `ninjask._extension` for where they come from."""

from typing import TypedDict, final

OBS_LEN: int
"""Floats in `Tracker.encode()` (layout: `ninjask::encode::observation`)."""
IDS_LEN: int
"""Values in `Tracker.encode_ids()` (layout: `ninjask::encode::ids`)."""
ACTION_FIELDS: int
"""Values per active slot in an action row (kind, index, target, Mega Evolution)."""
ACTION_WIDTH: int
"""Values per row of `Batch.observe()["actions"]`."""
DEFAULT_MAX_PATHS: int
"""Chance paths `Battle.enumerate_outcomes` explores unless given `max_paths`."""

@final
class EnumerationMode:
    """How `Battle.enumerate_outcomes` treats damage rolls; other chance events are exhaustive."""

    Precise: EnumerationMode
    """All 16 damage rolls."""
    DamageEndpoints: EnumerationMode
    """An approximation: each damage roll replaced by its lowest and highest
    value, half each (KO chances can be badly off when a roll range straddles a KO)."""

@final
class BattleStateKey:
    """The full simulator state, hashable (not a player's information set)."""

    def __hash__(self) -> int: ...
    def __eq__(self, other: object, /) -> bool: ...

@final
class Outcome:
    """One distinct result of `Battle.enumerate_outcomes`."""

    @property
    def probability(self) -> float: ...
    @property
    def battle(self) -> Battle:
        """An independent copy of the resulting battle."""

@final
class Dex:
    """A loaded format: species, moves, items, abilities and support levels."""

    @staticmethod
    def load(path: str) -> Dex:
        """Load an export written by refsim/export_format.js."""
    @property
    def format_id(self) -> str: ...
    def team_problems(self, packed: str) -> list[str]:
        """Reasons a packed team is not fully simulated (empty when it is)."""
    def move_support(self, move_id: str) -> str:
        """Support level of a move by id: "generic", "implemented" or "unsupported (...)"."""
    def species_ids(self) -> list[str]: ...
    def move_ids(self) -> list[str]: ...
    def generic_moves(self) -> list[str]: ...
    def implemented_moves(self) -> list[str]: ...
    def supported_abilities(self) -> list[str]: ...
    def supported_items(self) -> list[str]: ...
    def eval_move_marks(self, move_id: str) -> tuple[bool, bool, int]:
        """`(setup, heal, priority)` as `Battle.eval_features` reads a move."""
    def vocab(self) -> dict[str, int]:
        """Embedding-table sizes for `Tracker.encode_ids()`: "species", "items", "abilities", "moves"."""

@final
class Battle:
    """A battle between two packed teams. Sides are "p1" and "p2"; slots are
    named like "p1a"."""

    def __new__(cls, dex: Dex, team1: str, team2: str, seed: int, names: tuple[str, str] | None = None) -> Battle: ...
    @staticmethod
    def reference(dex: Dex, team1: str, team2: str, seed: str, names: tuple[str, str] | None = None) -> Battle:
        """Reference mode: Showdown's RNG, seeded like Showdown's ("sodium,<hex>")."""
    @staticmethod
    def from_projection(dex: Dex, projection: str, seed: int, names: tuple[str, str] | None = None) -> Battle:
        """Continue from a Showdown state projection (JSON text) with a new seed."""
    def set_rng_trace(self, on: bool) -> None: ...
    def rng_trace(self) -> list[str]:
        """The RNG draws recorded since the last call."""
    @property
    def rng_seed(self) -> str | None:
        """The RNG state as Showdown prints it (reference mode), else None."""
    def request(self, side: str) -> str | None:
        """The side's current request as Showdown JSON text, or None when the
        side has no request (it is not its turn to choose; see `needs_choice`)."""
    def needs_choice(self, side: str) -> bool: ...
    def choose(self, side: str, choice: str) -> None:
        """Submit a choice; raises ValueError with Showdown's message if invalid."""
    def legal_choices(self, side: str) -> list[str]:
        """The choices the side's request offers, in Showdown syntax: what
        `ninjask.choices` lists from the request (ally targets included), in
        its order. Only what the player can see shapes the list, so a choice
        can still be refused for something hidden. Empty with nothing to choose."""
    def legal_actions(self, side: str) -> bytes:
        """`legal_choices` as int8 rows of `action_width(side)` values
        (`numpy.frombuffer(..., numpy.int8)`): per slot kind (0 pass, 1 move,
        2 switch, 3 Struggle or recharge), index, target, Mega Evolution; at
        team preview, the team positions sent."""
    def action_width(self, side: str) -> int:
        """Values per row of `legal_actions(side)` (0 with nothing to choose)."""
    def choose_index(self, side: str, index: int) -> None:
        """Submit `legal_choices(side)[index]`; raises ValueError as `choose` does."""
    def enumerate_outcomes(
        self, choice_p1: str, choice_p2: str, mode: EnumerationMode = ..., max_paths: int = ...
    ) -> list[Outcome]:
        """Experimental: every distinct result of the turn (to the next request
        or the end), with its probability, without drawing a random number. Raises ValueError
        on a bad choice or when the turn has more than `max_paths` chance paths
        (default `DEFAULT_MAX_PATHS`). An empty string for a side with nothing to choose."""
    def state_key(self) -> BattleStateKey: ...
    def log(self, viewer: str, start: int) -> list[str]:
        """The protocol log from line `start`, as `viewer` sees it ("p1", "p2" or the spectator)."""
    def log_len(self) -> int: ...
    def set_logging(self, on: bool) -> None: ...
    def show_team_sheets(self) -> None:
        """Open team sheets: log both teams' sets, as Showdown does at team preview."""
    def projection(self) -> str:
        """The full state as JSON, in the shape of the reference harness's projection."""
    def copy(self) -> Battle: ...
    def reseed(self, seed: int) -> None: ...
    def damage(self, attacker: str, move_id: str, target: str) -> str:
        """The damage calculation for a move from slot `attacker` against slot `target`, as JSON."""
    def order(self, a: str, plan_a: str, b: str, plan_b: str) -> tuple[float, float]:
        """`(first, tie)`: the chance that slot `a` acts before slot `b`, and that a speed tie decides it."""
    def damage_party(self, attacker: str, move_id: str, target: str) -> str:
        """`damage` for any Pokémon of a party, named "p1:2" (side and index into the projection's
        `sides[i]["pokemon"]`); a benched one counts as in its side's slot, with no boosts."""
    def order_party(self, a: str, plan_a: str, b: str, plan_b: str) -> tuple[float, float]:
        """`order` for any Pokémon of a party, named as in `damage_party`."""
    def eval_features(self, side: str) -> list[float]:
        """battle-ai's "v2" evaluator features (22 floats, Rust `ninjask::features::FEATURES_V2`) from
        `side`'s point of view, with the calculations of `damage_party` / `order_party`; p2's are p1's negated."""
    def speeds(self) -> str:
        """Each active Pokémon's effective Speed, `{"p1a": speed, ...}`, as JSON."""
    @property
    def ended(self) -> bool: ...
    @property
    def winner(self) -> str | None:
        """The winning player's name (as Showdown's `battle.winner`), or None.

        `winner_side` gives the side, "p1" or "p2", as `play_engine_game` does."""
    @property
    def winner_side(self) -> str | None:
        """The winning side ("p1" or "p2"), or None for a tie or a battle still running."""
    @property
    def turn(self) -> int: ...

@final
class Tracker:
    """What one player knows, built from its protocol lines and requests."""

    def __new__(cls, dex: Dex, side: str) -> Tracker: ...
    def feed(self, lines: list[str]) -> None:
        """Feed protocol lines from this player's stream."""
    def copy(self) -> Tracker: ...
    def feed_request(self, request: str) -> None:
        """Feed this player's request (JSON)."""
    def feed_battle(self, battle: Battle) -> None:
        """Feed this player's own side straight from an engine battle (self-play)."""
    def view(self) -> str:
        """The view as JSON."""
    def encode(self) -> bytes:
        """The view as `OBS_LEN` float32 values (`numpy.frombuffer(..., numpy.float32)`)."""
    def encode_ids(self) -> bytes:
        """The view's species, items, abilities and moves as `IDS_LEN` int16 values, 0 when unknown."""
    def world(self, assumptions: str | None = None, seed: int = 0) -> Battle:
        """The battle this view describes, with `assumptions` (JSON) for the foe's hidden sets."""

class Observation(TypedDict):
    """`Batch.observe()`: every pending decision, as bytes for `numpy.frombuffer`."""

    n: int
    """The number of decisions."""
    who: bytes
    """uint32 (n, 2): the battle and side (0 is p1) deciding."""
    preview: bytes
    """uint8 (n,): 1 for a team preview decision."""
    obs: bytes
    """float32 (n, OBS_LEN): each player's `Tracker.encode()`."""
    ids: bytes
    """int16 (n, IDS_LEN): each player's `Tracker.encode_ids()`."""
    actions: bytes
    """int8 (rows, ACTION_WIDTH): every decision's legal choices in turn."""
    offsets: bytes
    """uint32 (n + 1,): decision k's choices are rows offsets[k]:offsets[k + 1]."""

@final
class Batch:
    """Many battles stepped together, in parallel, with both players'
    trackers kept in Rust (`ninjask::batch`)."""

    def __new__(cls, dex: Dex, n: int) -> Batch: ...
    def __len__(self) -> int: ...
    @property
    def turn_limit(self) -> int:
        """The turn after which a battle stops as a draw (default 150)."""
    @turn_limit.setter
    def turn_limit(self, turns: int) -> None: ...
    def reset(self, i: int, team1: str, team2: str, seed: int) -> None:
        """Start a battle in slot `i`."""
    def reset_many(self, slots: list[int], teams1: list[str], teams2: list[str], seeds: list[int]) -> None:
        """Start several battles, built in parallel."""
    def observe(self) -> Observation:
        """Every pending decision of every running battle (see `Observation`)."""
    def step(self, choices: list[int]) -> bytes:
        """One choice index per decision of the last `observe`; uint8 per
        decision: bit 0 refused (still pending), bit 1 the battle ended."""
    def dones(self) -> bytes:
        """uint8 per slot: 1 when its battle is over."""
    def done(self, i: int) -> bool: ...
    def winner(self, i: int) -> str | None:
        """The winning side, "p1" or "p2", or None."""
    def turn(self, i: int) -> int: ...
    def battle(self, i: int) -> Battle:
        """A copy of battle `i`."""
