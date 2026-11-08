# Ninjask

[![CI](https://github.com/dcabatin/Ninjask/actions/workflows/ci.yml/badge.svg)](https://github.com/dcabatin/Ninjask/actions/workflows/ci.yml)
[![PyPI](https://img.shields.io/pypi/v/ninjask)](https://pypi.org/project/ninjask/)
[![crates.io](https://img.shields.io/crates/v/ninjask)](https://crates.io/crates/ninjask)

**A Pokémon Champions VGC doubles simulator that matches
[Pokémon Showdown](https://github.com/smogon/pokemon-showdown) exactly, about
200 times faster.** Written in Rust, with Python bindings, for bots,
reinforcement learning, search and large-scale team analysis. Named after
Ninjask, a famously fast Pokémon.

```sh
pip install ninjask
```

## Why Ninjask

If you are building a VGC bot, training an agent with self-play, or asking a
question that takes millions of battles to answer, Showdown is the reference
but it is slow, and a simulator that is fast but subtly wrong teaches your
agent the wrong game. Ninjask is meant to be both:

- **Exact.** Ninjask is a port of Showdown's battle code at a pinned commit.
  In *reference mode* it uses Showdown's random number generator and makes
  the same draws in the same order, so a seeded game produces the same log,
  the same requests and the same state after every input. It matches
  Showdown on 1,000 of 1,000 seeded games of random legal teams and 1,000 of
  1,000 games of 766 real tournament teams from the
  [VGCPastes](https://x.com/VGCPastes) Repository
  ([`pools/`](pools/README.md)), and CI replays 400 against Showdown on every
  change (see [How parity is verified](#how-parity-is-verified)).
- **Fast.** About 6,000 random games (55,000 to 67,000 turns) per second on
  one CPU core with logging off, and 34,000 to 40,000 turns per second with
  the full protocol log on, against Showdown's 150 to 180 turns per second on
  the same machine and teams (`examples/bench.rs --log`,
  `refsim/bench_showdown.js`; a 4-vCPU cloud VM, Xeon at 2.8 GHz).
  A `Battle` is plain data: copying one for search costs microseconds.
- **Complete for Reg M-C.** All 510 moves (and Struggle) and 166 items in the format, and
  221 of its 224 abilities (the other three belong only to Pokémon that are
  not legal in it). All 766 teams of the VGCPastes pool are fully
  supported. Nothing is silently approximated: `Dex.team_problems`
  names anything a team uses that the engine does not simulate.
- **Built for agents.** Besides the battle itself:
  - a **batched training interface** (`ninjask.Batch`): many battles on all
    cores, with each player's observation and legal choices handed over as
    numeric buffers, at about 1,300 random games per second;
  - a per-player **tracker** that builds what one side knows from the
    protocol, and reads Showdown's live logs the same way;
  - **worlds**: battles consistent with one player's view, with the foe's
    hidden sets filled in by your assumptions, for search under hidden
    information;
  - an exact **damage and turn-order calculator** (all 16 rolls, crits,
    KO chances, Speed with Trick Room, Tailwind and priority) that runs the
    battle's own damage code, so it can't drift from play;
  - open team sheets, and Showdown's request and choice formats throughout,
    so a bot written for Showdown carries over with little change.

What it is not: an agent, a search algorithm or a user interface. It is the
simulator underneath them, with the tools to prove it matches the reference.

## Quick start

### Python

```sh
pip install ninjask      # the Reg M-C formats are bundled
```

One wheel per platform serves CPython 3.12 and every later version (the
stable ABI): Linux x86_64 and aarch64 (glibc, and musl for Alpine), macOS
x86_64 and arm64, and Windows x86_64 and arm64. Elsewhere pip builds from
the source distribution, which needs a Rust toolchain.

Play a game between two random policies:

```python
import ninjask
from ninjask.policy import RandomPolicy

team1, team2 = ninjask.verification_teams()[:2]   # 2 of the 4 bundled test teams, Showdown's packed format
result = ninjask.play_engine_game(
    "gen9championsvgc2026regmc", 1, team1, team2, RandomPolicy(1), RandomPolicy(2)
)  # format, seed, teams, policies; winner is "p1", "p2" or None
print(result.winner, result.turns)
```

A policy is any object with `choose(request, log) -> str`: it reads Showdown's
request JSON and answers in Showdown's choice syntax (`"move 1 1, move 2 2"`),
so code written for Showdown bots carries over. To drive a battle yourself:

```python
import json
import ninjask
from ninjask import Battle, dex
from ninjask.policy import RandomPolicy

team1, team2 = ninjask.verification_teams()[:2]
d = dex("gen9championsvgc2026regmc")
assert d.team_problems(team1) == []  # [] when the engine fully supports the team

b = Battle(d, team1, team2, seed=1)
b.set_logging(True)
policy = RandomPolicy(seed=1)
while not b.ended and b.turn < 150:
    for side in ("p1", "p2"):
        if b.needs_choice(side):
            while True:
                text = b.request(side)                           # Showdown's request JSON:
                assert text is not None                          # None only when no choice is due
                request = json.loads(text)
                try:
                    b.choose(side, policy.choose(request, []))   # e.g. "move 1 1, move 2 2"
                    break
                except ValueError:                               # rejected, as Showdown rejects it
                    pass                                         # (a hidden disable); the request now shows it
print(b.winner_side, b.turn)                                    # "p1", "p2" or None; .winner is the player's name
print("\n".join(b.log("p1", 0)[:10]))                           # p1's view of the protocol log
```

`b.copy()` is a cheap, independent copy for search: play a line out without
touching `b`. `b.damage("p1a", move_id, "p2b")` runs the damage calculation
(all 16 rolls, crit chance, KO chances) for the Pokémon in those slots.

`Battle.reference(d, team1, team2, "sodium," + "0" * 64)` runs a battle in
reference mode, seeded like Showdown (a `sodium,<hex>` seed string), so it
reproduces a Showdown game draw for draw.

### Training at scale

For training, `ninjask.Batch` steps many battles at once, in parallel, and
hands over every pending decision as numeric buffers: each player's
observation (only what that player knows) and its legal choices, ready for
`numpy.frombuffer`. One call reports the decisions and one applies a choice
index to each:

```python
import numpy as np
import ninjask

d = ninjask.dex("gen9championsvgc2026regmc")
teams = ninjask.verification_teams()
n = 64
batch = ninjask.Batch(d, n)  # 64 battles, stepped on all cores
batch.reset_many(list(range(n)), [teams[i % 4] for i in range(n)], [teams[(i + 1) % 4] for i in range(n)], list(range(n)))
rng = np.random.default_rng(0)
while True:
    o = batch.observe()  # every pending decision, both sides of every battle
    if o["n"] == 0:
        break
    obs = np.frombuffer(o["obs"], np.float32).reshape(o["n"], ninjask.OBS_LEN)  # what each player sees
    offsets = np.frombuffer(o["offsets"], np.uint32).astype(np.int64)
    counts = offsets[1:] - offsets[:-1]  # legal choices per decision
    picks = (rng.random(o["n"]) * counts).astype(np.int64)  # your policy goes here
    batch.step(picks.tolist())
print([batch.winner(i) for i in range(4)])  # "p1", "p2" or None
```

On one core of a 4-vCPU cloud VM this plays about 700 random games per second
with every decision observed, and about 1,300 on four cores; a loop through
the JSON requests above plays about 45. `Battle.legal_choices(side)`,
`.legal_actions(side)`, `.choose_index(side, i)` and `Tracker.encode()` give the
same interface for a single battle. Choices are listed from the player's
view, so, as in Showdown, one can be refused for something hidden (a foe's
Imprison, a trap): the decision then stays pending, with what the refusal
revealed.

### Rust

```sh
cargo add ninjask
```

```rust
use std::{error::Error, sync::Arc};
use ninjask::{rng::Rng, Battle, Dex};

fn main() -> Result<(), Box<dyn Error>> {
    let dex = Arc::new(Dex::from_json(&std::fs::read_to_string("gen9championsvgc2026regmc.json")?)?);
    let pool = std::fs::read_to_string("teams.txt")?; // one packed team per line
    let teams: Vec<&str> = pool.lines().collect();

    let mut battle = Battle::new(dex, [teams[0], teams[1]], ["Alice", "Bob"], 1)?;
    let mut rng = Rng::new(1);
    while !battle.ended && battle.turn < 150 {
        for side in 0..2 {
            if battle.side_needs_choice(side) {
                let choice = battle.random_choice(side, &mut rng); // or any "move 1 1, switch 3"
                battle.choose(side, &choice)?;
            }
        }
    }
    println!("{:?} won after {} turns", battle.winner_name(), battle.turn);
    Ok(())
}
```

The full program is [`examples/quickstart.rs`](examples/quickstart.rs). The
crate reads a format from its JSON export and does not ship one: the Python
package carries the bundled ones, and a checkout generates any other. To run
the program above, copy the format and the test teams out of the wheel:

```sh
pip install ninjask
D=$(python -c "import ninjask, pathlib; print(pathlib.Path(ninjask.__file__).parent / 'data')")
cp "$D/formats/gen9championsvgc2026regmc.json" .
cp "$D/regmc_teams.txt" teams.txt
```

The [API overview](#api-overview) lists everything else; `cargo doc --open`
(or [docs.rs](https://docs.rs/ninjask)) documents the crate.

## Contributing and ideas

Contributions are welcome, and the most valuable ones are concrete:

- **Found a difference from Showdown?** That is a bug, whatever its size.
  Open a [bug report](https://github.com/dcabatin/Ninjask/issues/new?template=bug.yml)
  with the teams, the seed and the choices that reproduce it;
  `scripts/check_reference.py --verbose` prints both simulators' logs and
  random draws at the first divergence.
- **Have an idea, a use case or a question?** Open an
  [idea](https://github.com/dcabatin/Ninjask/issues/new?template=idea.yml).
  Knowing what you are building helps decide what comes next. Things we
  would like to see: new regulations as they launch, an API that enumerates
  a turn's chance outcomes exactly (for search and endgame solvers), faster
  batched bindings for training, and more languages.
- **Want to write code?** [`CONTRIBUTING.md`](CONTRIBUTING.md) covers the
  setup, the checks a change must pass and how to port a mechanic from
  Showdown.

## Singles: Champions BSS (the `singles-bss` branch)

This branch of the fork `ssg223498/Ninjask` also plays Showdown's
`[Gen 9 Champions] BSS Reg M-C` (`gen9championsbssregmc`): singles, bring
six and pick three, Mega Evolution, the Champions stat points, under Flat
Rules and the VGC timer. The engine fields as many slots per side as the
format's `gameType` says (`FormatInfo.doubles`); everything else is the
same code as for doubles.

```sh
node refsim/export_format.js gen9championsbssregmc > data/derived/format/gen9championsbssregmc.json
# or let ninjask.dex("gen9championsbssregmc") export it on first use
python scripts/check_reference.py --format gen9championsbssregmc --pool teams.txt --games 1000 --workers 4 --requests
```

Verified on 1,000 of 1,000 seeded reference games (36 real singles teams,
random choices) with logs, state, random draws and every request compared,
and by `tests/python/test_singles.py` (choices accepted and refused as
Showdown does, team preview, refused choices and the updated requests).
Doubles keep their logs and draws (see the CHANGELOG).

### Doubles-only assumptions found, and what this branch does

| Where | Assumption | Singles in Showdown | Here |
| --- | --- | --- | --- |
| `battle/choices.rs` `for_each_team_preview_order`, `choices.team_preview_choices` | The first two Pokémon sent lead | One leads | Leads = active slots; `game_type` argument in Python |
| `moves/targeting.rs` `get_random_target` | A random foe, one draw even with a single foe | `foe.active[0]`, no draw (`getRandomTarget`) | No draw in singles (the one divergence in play: every game diverged without it) |
| `battle/choices.rs` `parse_slot_choice` | A target is chosen only with two active | A valid target may be given (`chooseMove`) | Accepted when valid |
| `battle/choices.rs` `parse_choice` | Extra comma parts are ignored | Refused: "You sent choices for 2 Pokémon" | Refused (both game types) |
| `battle/choices.rs` `side_request_data` | `active` = on the field | `position < side.active.length` | As Showdown (both game types) |
| `battle/choices.rs` `request_json` | No `noCancel` | Set when only one side has a request (Cancel Mod) | As Showdown (both game types) |
| `battle/choices.rs` refusals | Imprison's `maybeDisabled` goes after a refusal; all disabled moves shown | `updateDisabledRequest` skips singles: it stays, only the refused move is marked | As Showdown; also `update` and `disabledSource` (both game types) |
| `moves/use_move.rs` `is_last_active`, `move_shown_disabled` | A fainted Pokémon still in its slot is active | `isActive` is cleared by fainting; Imprison skips it | As Showdown (requests in doubles) |
| `moves/status_moves.rs` Ally Switch | The ally is in slot 1 | Fails outside doubles and triples | Fails when one slot (it did already, by finding no active Pokémon there) |
| `scripts/check_reference.py` | The format is Reg M-C | | `--format`, `--requests` |

Already written for any number of slots, and checked unchanged: the
screens' multiplier (`items/mod.rs`, 1/2 in singles), Follow Me, Rage
Powder, After You and Quash failing in singles, redirection only with
more than one active (`RedirectTarget`), the spread modifier (one target
in singles), `isLastActive` for hidden traps and disables, Imposter's
target across, Trick-or-Treat's slot-b glitch, Natural Cure's message
(Showdown's doubles-only cure message is not ported, which is right for
singles).

Not verified for singles: the tracker (`scripts/check_tracker.py`), the
numeric observations and actions of `Batch`/`encode`, worlds, and
outcome enumeration. A player changing a choice before the foe has chosen
(allowed by Cancel Mod) is refused, as in doubles before this branch.

## Scope and parity

- **Format.** Champions doubles under the Reg M-C rules: bring 6, pick 4,
  Mega Evolution, no Terastallization, the Champions stat-point system, and its
  changed status and damage mechanics. Other regulations (M-A, M-B, and later
  ones) are data changes: the rules, species, moves, items and abilities are
  loaded from a JSON export of Showdown's data, not compiled in.
- **Parity.** For the entities it supports, the engine matches one pinned
  Showdown commit (`refsim/SHOWDOWN_COMMIT`) exactly. Run in *reference mode*, it
  uses Showdown's own random number generator and makes the same draws in
  the same order, so a seeded game gives the same log lines and the same state
  after every input. The normal mode uses a faster generator
  (xoshiro256++); its outcome *distributions* are checked against Showdown's.
- **Honest coverage.** Every move, item and ability is *generic* (fully
  described by its data fields), *implemented* (ported from Showdown's code),
  or *unsupported*. The engine never silently simulates an effect it lacks:
  `Dex.team_problems(team)` lists what a team uses that is unsupported, and
  `scripts/report_support.py` reports which entities block the most teams.

### How parity is verified

Showdown at the pinned commit is the specification. Five checks compare the
engine with it; all need Node.js and the Showdown checkout (see
[Working from source](#working-from-source)).

| Check | What it compares |
| --- | --- |
| `scripts/check_reference.py` | Reference mode against Showdown, game by game. Both simulators get the same seed and the same choices. After every input it compares each log line, each side's request and the full state projection, the RNG state included. On a divergence it prints both sequences of RNG draws with their call sites. |
| `scripts/check_chaos.py` | The same comparison on "chaos" teams: every Pokémon carries the mechanics that interact the most (Illusion, Imposter, Transform, Skill Swap, Reflect Type, Perish Song, Future Sight, Instruct, Baton Pass, Revival Blessing...), so interactions that real play reaches once in a million games come up every few dozen. `--mix wide` and `--mix mixed` draw from everything supported. Each bug it has found is replayed in `tests/python/test_chaos.py`. |
| `cargo run --release --example fingerprint` | A hash of the logs, requests and RNG draws of 2000 seeded engine games. It must not change in a refactor that preserves behaviour. |
| `scripts/verify_engine.py` | Outcome distributions. At sampled decision points the Showdown state is imported into the engine and one joint choice is played N times in each simulator. A χ² test of homogeneity per feature (HP, status, boosts, volatiles, fainting, field, the winner) plus an exact-support check, with false-discovery-rate control across all comparisons. `--focus ids` concentrates on named moves, abilities and items. |
| `scripts/check_tracker.py` | The protocol tracker (below) against the true state: a player's view of its own side matches exactly, and everything it believes about the foe is true. It runs on engine games, on Showdown games (`--showdown`) and with open team sheets (`--ots`). |

The reference and tracker checks run on a pool of real-looking teams and on
`--synthetic` teams: random legal teams built over every supported move,
ability and item, so entities no popular team uses are exercised too.

The tests (`cargo test`, `pytest`) cover units of the same machinery: the RNG
against Node's output, the choice syntax, format loading and the reference
harness on a few games.

## Known limitations

- **Three abilities are not simulated:** Gulp Missile, Ice Face and Shields
  Down. They belong only to formes that are not legal in Reg M-C
  (Cramorant-Gulping and -Gorging, Eiscue-Noice, Minior-Meteor);
  `Dex.team_problems` names them if a team uses them.
- **One pinned Showdown.** Parity is with commit `3a766f2` (2026-10-13).
  Changes Showdown makes after it are not followed until the pin moves. The
  tracker reads logs from the live server, which may run a newer version.
- **Wheels** are for CPython 3.12 and later on the platforms listed in the
  quick start; PyPy, free-threaded CPython (3.14t) and other platforms build
  from source and need Rust.
- **The crate ships no format data**: Rust users take it from the wheel (see
  the Rust quick start) or export it from a checkout.
- **Typing.** The compiled classes and the examples above pass
  `mypy --strict`; the helper modules (`verify`, `play`, ...) are not
  checked with `--strict`.

**Stability before 1.0.** A patch release (0.4.x) keeps the public Rust
and Python API and changes behaviour only to match the pinned Showdown more
closely; every such change is in the CHANGELOG. A minor release (0.5) may
change the API or move the Showdown pin, and the CHANGELOG says what to
update. Features marked experimental (today, outcome enumeration) are
outside this promise and may change in any release.

## Working from source

The parity checks, exporting formats other than the bundled ones, and
changing the engine need a checkout and the pinned Showdown.

Requirements: Python 3.12 or later, Rust (stable), Node.js 22 or later, git.

```sh
git clone https://github.com/dcabatin/Ninjask.git
cd Ninjask
python3 -m venv .venv
source .venv/bin/activate

# The package, with its compiled module built from the crate (optimised)
pip install -e '.[test]'

# Pokémon Showdown at the pinned commit, cloned and built into third_party/
./scripts/setup_showdown.sh

# The Reg M-C format, exported from Showdown to data/derived/format/
python -c "import ninjask; ninjask.load('gen9championsvgc2026regmc')"

# Rust and Python tests
cargo test --release
pytest -q
```

`import ninjask` then works from anywhere in that environment. The install is
editable because the checks below drive Showdown through `refsim/` and
`scripts/`, which live in the checkout; after changing Rust code, rebuild with
`pip install -e .` again (or `maturin develop --release`). The Rust tests
that play battles read the format export and skip without it; the rest of
`cargo test --release` needs neither Python nor Showdown. The Python tests
that need Showdown skip themselves when it is not built.

The engine loads a format from a JSON export of Showdown's data,
`data/derived/format/<format>.json`, which `ninjask.dex(format)` and
`ninjask.load(format)` generate from the Showdown checkout on first use. One
exported format is derived rather than Showdown's own:
`gen9championsvgc2026regmcots` (PKMN Arena's open-team-sheets Reg M-C) is the
base format with its sheet rule forced open, built by `refsim/export_format.js`;
its `FormatSpec` reports `open_team_sheets` and the base format it derives from.

### Run the checks

```sh
# Exact parity with Showdown, on 1000 seeded games of synthetic teams
# (4 processes, each with its own Showdown)
python scripts/check_reference.py --synthetic --games 1000 --workers 4

# Outcome distributions, tracker, support report
python scripts/verify_engine.py --cases 100 --n 300
python scripts/check_tracker.py --games 100
python scripts/report_support.py --pool tests/fixtures/regmc_teams.txt

# Throughput (add --log for logging on), Showdown's for comparison, and the fingerprint
cargo run --release --example bench -- data/derived/format/gen9championsvgc2026regmc.json tests/fixtures/regmc_teams.txt 20000
node refsim/bench_showdown.js tests/fixtures/regmc_teams.txt 300
cargo run --release --example fingerprint -- data/derived/format/gen9championsvgc2026regmc.json tests/fixtures/regmc_teams.txt 2000
```

On the bundled fixture teams (`tests/fixtures/regmc_teams.txt`) the fingerprint is
`468d11e0d5f4cf86`. A change that preserves behaviour must leave it unchanged.

The bench plays random legal games on one thread with logging off: about
5,400 to 6,700 games/s over ten runs on one core of a 4-vCPU cloud VM (Xeon
at 2.8 GHz), on the four fixture teams or on the VGCPastes pool
(`pools/vgcpastes-regmc.txt`; about 720,000 instructions per game under
callgrind, 780,000 on the fixture teams). Speed on shared hosts varies by 10% or
more between runs, so compare instruction counts (`valgrind --tool=callgrind`
on the bench, a release build with debug info) rather than games/s across
days.

`check_reference.py` and `check_tracker.py` play on a pool of real teams when
you give one (`--pool pools/vgcpastes-regmc.txt`, or any file with one packed
team per line; or the
`supported-*.txt` pools that `report_support.py` writes to
`data/derived/teams/`), else on the fixture teams, and on random legal teams
with `--synthetic`. `check_reference.py` also takes `--first N` (start at game
N), `--require ids` (player 1's team must use one of those moves, items or
abilities) and `--verbose`; run any script with `--help`.
`report_support.py` reads a pool of packed teams, ranks the moves, items and
abilities that keep the most teams unsupported, and writes the supported ones
as a pool to verify on.

## API overview

**Rust** (`ninjask` crate; `cargo doc --open`)

- `Dex::from_json(text)`: the loaded format. Immutable, shared between
  battles through an `Arc`. Looks up species, moves, items, abilities and the
  type chart, and reports support.
- `Battle::new(dex, [team1, team2], names, seed)`: a battle from two packed
  teams. `Battle::new_with_rng` takes a given `Rng` (reference mode uses Showdown's);
  `Battle::from_sets` takes parsed teams. For many games over one pool,
  prepare each team once (`battle::Team::new(&dex, &sets)`: its Pokémon and
  open team sheet) and build each battle with `Battle::from_teams`, which
  copies them and plays exactly as `from_sets` would;
  `Battle::from_projection` imports a state exported by the reference.
  `Battle` is plain data and `Clone`, so copying it for search is cheap.
- `battle.request_json(side)`, `side_needs_choice(side)`, `choose(side, "move 1 1, switch 3")`
  accept and produce Showdown's request and choice formats.
  `joint_choices`, `sample_joint_choice` and `random_choice` enumerate or draw legal choices.
- `battle.log_lines(viewer, from)`: the protocol log as one viewer sees it
  (the spectator, or either player, hiding what that player has not seen).
- `battle.projection()`: the full state as JSON, in the shape of the reference
  harness's projection.
- `Tracker::new(dex, side)`: builds one player's observation from the protocol
  lines and requests it is fed (`feed`, `feed_request`), or from the engine
  directly (`feed_engine`, for self-play). The same tracker reads Showdown's
  logs, so live play and simulation give the same observations. `to_json()`
  exports the view.
- `world::World::from_view(&tracker, &assumptions) -> Result<Battle, _>`: a
  *world*, the battle one player's view describes. One's own side and the
  field are as the view holds them; the foe's hidden sets (item, ability,
  unrevealed moves, nature, stat points) come from a `SetAssumption` per
  Pokémon, or by default: nothing unrevealed, the sheet's or a neutral nature,
  and a placeholder spread rule (`world::placeholder_points`). Search over
  hidden information can run on worlds. `check_tracker.py --world`
  checks that a world built with the true sets matches the true battle.
- `calc`: exact damage and turn order, read-only. `battle.calc(attacker,
  move, target)` (or `damage_calc` on a copy) gives the 16 rolls of each hit
  without and with a critical hit, the crit chance, the spread reduction,
  the hit-count odds, accuracy, immunity, the expected damage and the
  chances to KO this turn and within two uses (no end-of-turn effects
  between them). It runs `getDamage`'s own two phases (`damage_base`,
  `damage_mods`) for every roll, so it can't drift from play.
  `effective_speed`, `order_odds(a, plan, b, plan)` (switches, priority,
  Quick Claw and the like, Speed, Trick Room, ties) and `speed_order`.
  `cargo run --release --example damage_check -- <dex> <teams> 5000` plays
  random games recording every hit with its predicted distribution: on the
  fixture teams, 0 of 99,176 damages fell outside it, and critical hits, KOs
  and rolls came as often as predicted.
- `rng::Rng`: xoshiro256++ from `Rng::new(seed)`, or Showdown's generator from `Rng::from_showdown_seed`.

**Python, compiled classes** (`ninjask.Dex`, `ninjask.Battle`, `ninjask.Tracker`;
built from `src/python.rs` as `ninjask._core` and re-exported by the package)

- `Dex.load(path)`, `.team_problems(packed)`, `.move_support(id)`,
  `.species_ids()`, `.generic_moves()`, `.implemented_moves()`,
  `.supported_abilities()`, `.supported_items()`, `.vocab()` (embedding-table
  sizes for `Tracker.encode_ids()`).
- `Battle(dex, team1, team2, seed)`, `Battle.reference(...)`,
  `Battle.from_projection(...)`, `.request(side)`, `.needs_choice(side)`,
  `.choose(side, choice)` (raises `ValueError` on an illegal choice, as
  Showdown rejects it), `.log(viewer, start)`, `.log_len()`, `.set_logging(on)`,
  `.projection()`, `.copy()`, `.reseed(seed)`, `.ended`, `.winner` (the
  player's name), `.winner_side` (`"p1"`, `"p2"` or `None`), `.turn`.
- `Battle.legal_choices(side)`: the choices the side's request offers, in
  Showdown syntax and in `ninjask.choices`' order, built without the
  request; `.legal_actions(side)`: the same as int8 rows of
  `.action_width(side)` values (per slot: kind 0 pass, 1 move, 2 switch,
  3 Struggle or recharge; move slot or team position; target; Mega
  Evolution; at team preview, the order sent); `.choose_index(side, i)`.
- `Tracker(dex, side)` with `.feed(lines)`, `.feed_request(json)`,
  `.feed_battle(battle)`, `.view()` and `.world(assumptions=None, seed=0)`
  (a `Battle`; assumptions as JSON, see `world::Assumptions::from_json`).
  `feed_battle` (Rust's `feed_engine`) fills in only the player's own side,
  as its request would; everything else comes from the protocol lines, so in
  self-play feed both: `t.feed(b.log(side, seen)); t.feed_battle(b)`.
  `view()` is JSON text whose `sides` are always in the order p1, p2.
  `.encode()` is the view as `ninjask.OBS_LEN` float32 values (own side
  first), and `.encode_ids()` its species, items, abilities and moves as
  `ninjask.IDS_LEN` int16 values (0 when unknown), as bytes for
  `numpy.frombuffer`; layouts in `src/encode.rs`.
- `Batch(dex, n)`: `n` battles with both players' trackers, stepped in
  parallel with the GIL released. `.reset(i, team1, team2, seed)`,
  `.reset_many(...)`, `.observe()` (every pending decision: who, the
  observation and ids, the legal actions and their offsets),
  `.step(choices)` (one index per decision; all checked before any is
  applied), `.dones()`, `.winner(i)`, `.turn(i)`, `.battle(i)` (a copy).
- `Battle.damage(attacker, move, target)` (slots like "p1a"; JSON),
  `.order(a, plan_a, b, plan_b)` (a plan is a move id, "switch" or
  "priority:N"; returns `(first, tie)`) and `.speeds()`.

**Python, the rest of the package** (`python/ninjask/`)

- `format`: `load(format_id)` returns a `FormatSpec` (species, moves, learnsets,
  legal species) from the Showdown export.
- `__init__`: `dex`, `play_engine_game`, and generators of random supported
  teams (`supported_test_team`).
- `refsim`: a client for the Showdown reference server (`RefSim`, `Battle`).
- `choices`, `policy`, `play`, `verify`, `tracker_check`: legal-choice
  enumeration, the policy interface, game loops for either simulator, and the
  distribution and tracker checks. `choices.joint_choices(request,
  game_type="doubles")` (and `team_preview_choices`) list the legal choices
  from a request's JSON, as Showdown's own bots would;
  `Battle.legal_choices` gives the same list without the JSON. A choice
  from either can still be refused because of something hidden (a trap or
  a disable), so keep the retry loop of the quick start.
- `paths`: where Showdown and generated data live. Defaults are inside the
  checkout (`third_party/`, `data/derived/`); override with `NINJASK_HOME`,
  `NINJASK_SHOWDOWN` and `NINJASK_DATA`. `NINJASK_POOL` sets the default team
  pool of the checks.

## Enumerating chance outcomes (experimental)

**Experimental:** exact, but too slow for most full turns (see the budget
below), and its API may change in any release, including a patch, as a
faster design replaces it.

`Battle::enumerate_outcomes(&choice_p1, &choice_p2, mode)` accepts the existing
structured side choices (`&[SlotChoice]`) and returns
`Result<Vec<Outcome>, EnumerationError>`. Each outcome contains `probability`
(`f64`) and a cloned `battle`. Python uses the existing choice-string syntax:

```python
import ninjask
from ninjask import EnumerationMode

d = ninjask.dex("gen9championsvgc2026regmc")
team1, team2 = ninjask.verification_teams()[:2]
battle = ninjask.Battle(d, team1, team2, seed=1)
battle.choose("p1", "team 1234")
battle.choose("p2", "team 1234")
# p1: Protect and Psychic Terrain; p2: Protect, and Throat Chop at p1's second Pokémon
outcomes = battle.enumerate_outcomes("move 4, move 1", "move 4, move 3 2", EnumerationMode.Precise)
print(len(outcomes), round(sum(o.probability for o in outcomes), 9))  # 21 distinct outcomes, total 1.0
try:
    battle.enumerate_outcomes(battle.legal_choices("p1")[0], battle.legal_choices("p2")[0])
except ValueError as e:  # more chance paths than max_paths: raise it, use DamageEndpoints or sample
    print(e)
```

An expected value over the outcomes is then
`sum(o.probability * evaluate(o.battle) for o in outcomes)`.

Enumeration preserves the input and any already-submitted choices. Supply an
empty choice (`&[]` in Rust, `""` in Python) for a side that does not need one.
Execution stops at the next player decision request or battle end, including
mid-turn pivot/replacement requests; the returned battles preserve the remaining
queue. Team preview is not supported by this API. Invalid choices return an
error (`ValueError` in Python).

`Precise` exhaustively enumerates the integer outcomes of the existing 32-bit
scaling rule, accumulating probabilities in floating point. `DamageEndpoints`
enumerates only the minimum and maximum damage-variance rolls, each with weight
1/2, including confusion damage and each hit of a multi-hit move. All other
chance events remain exhaustive. It is an approximation, not Showdown's
distribution: the endpoint weights are not the probability of rolling either
endpoint, and KO chances can be far off when a roll range straddles a KO (on
"who faints this turn", up to 42 percentage points in one of 28 real states
measured, though exact in most).

Execution replays explicit decision paths on fresh clones and never calls a
PRNG. Equivalent completed states are merged and their weights summed. There
is no sampling, pruning or silent truncation, but the number of paths grows
with every hit (accuracy, critical hit, damage roll), so each call explores at
most `max_paths` of them (`ninjask::DEFAULT_MAX_PATHS`, 20,000, about 0.7 s
at 33 µs a path) and fails with an error (`ValueError`) beyond that; the
`_within` variants in Rust and `max_paths=` in Python set another budget. On
40 mid-game states of real teams with random choices
(`examples/validate_outcomes.rs`), Precise fit the default budget on 14 and
DamageEndpoints on 22: endgames and turns with few attacks fit, others need a
larger budget or sampling. Where it finished, the enumeration was exact: none
of 28,000 sampled resolutions of those turns fell outside it, and none of the
turns' sampled distributions differed from it beyond sampling noise.
Returned battles retain the input RNG snapshot for subsequent ordinary play
(enumeration itself is independent of it), so reseed an outcome
(`Battle.reseed`) before sampling forward from several of them, or they draw
the same numbers.

`battle.state_key()` creates an immutable, hashable `BattleStateKey` containing
full future-relevant simulator state, including pending choices and queued
actions. Keys exclude logs, RNG state and diagnostics, and are scoped to the
same shared Dex allocation (independently loaded dexes stay distinct). Merged
outcomes retain the first representative battle's logs and diagnostics, not a
history of every contributing path. These keys are not player information-set
keys for imperfect-information training. Python `Outcome.battle` returns an
independent copy, so modifying it cannot change the stored outcome.


Compare enumeration modes with `cargo run --release --example bench_outcomes`.
It uses the self-contained enumeration test fixture: no damage, one hit, a
two-hit move, and both players attacking. Setup is excluded, logging is off,
and timings are medians of seven alternating batches. Output includes merged
state counts. These synthetic singles scenarios isolate damage branching;
measure your training positions separately before projecting throughput.

## Repository layout

```
Cargo.toml, pyproject.toml   crate and Python package (built with maturin)
src/                         the engine
  dex/                         format data, team validation, support reporting
  state/                       battle state: field, sides, Pokémon, the move in use
  battle/                      construction, the action queue, choices and requests, the log
  moves/                       move execution: targeting, hits, damage, multi-hit, two-turn moves
  events/                      Showdown's event hooks: modifiers, switch-in, residual, immunity, ...
  abilities/ items/ conditions/  ported handlers; weather, terrain, status, volatiles, side conditions
  kinds/                       the ids of every special-cased move, item and ability
  projection/                  state export and import, in the reference harness's format
  tracker.rs                   per-player observation built from protocol lines
  rng.rs                       xoshiro256++, and Showdown's ChaCha20 generator
  python.rs                    PyO3 bindings
examples/                    bench (throughput), fingerprint (behaviour hash), damage_check
python/ninjask/              the Python package; the compiled module lands here as _core
refsim/                      Node.js driver for the pinned Showdown: server, format export
scripts/                     the checks above, and setup_showdown.sh
tests/python/, tests/fixtures/   pytest files, a small team pool
pools/                       real teams to verify on (VGCPastes), and their sources
```

Each ported handler cites the Showdown function it follows. To add an entity,
port it from the pinned commit, then run `check_reference.py --require <id>`
and `verify_engine.py --focus <id>`: a port is done when they pass.

## Embedding in another extension

A Rust extension of your own whose functions take Ninjask's objects (say, an
observation encoder that reads a `Battle` and a `Tracker`) must be compiled
into the same module as Ninjask's classes: PyO3 only accepts an object of the
class compiled into the module that receives it, so a second copy of `Battle`
would be incompatible. Build one module holding both, and tell the `ninjask`
package to take its classes from it:

1. Depend on the crate with the feature `python` (not `python-module`):
   `ninjask = { git = "https://github.com/dcabatin/Ninjask", features = ["python"] }`,
   and on the same pyo3 as Ninjask (`pyo3 = "0.30"`).
2. In your `#[pymodule]`, call `ninjask::python::register(m)?`, then add your
   own classes. Their methods take `&ninjask::python::PyBattle`,
   `&PyTracker` and `&PyDex`, whose `inner` fields are the Rust objects.
3. Declare your module under the `ninjask.extension` entry-point group in
   your `pyproject.toml`:

   ```toml
   [project.entry-points."ninjask.extension"]
   myproject = "my_module"
   ```

`import ninjask` then re-exports `my_module.Dex`, `.Battle` and `.Tracker`
(`ninjask/_extension.py`), so `ninjask.Battle is my_module.Battle`, and
everything in the package, the checks included, uses them. `ninjask._core` is
used only when no module is declared, so the package's Python files are all
you need besides your own module (for example, `python/` on your project's
path), plus the bundled formats: a checkout's `python/ninjask/data/` is
empty until `scripts/bundle_data.sh` fills it (or copy it from the wheel), and
without it `ninjask.dex()` needs the checkout's Showdown to export a format.

## Documentation

In [`docs/`](docs/):

- [`design.md`](docs/design.md): requirements, architecture, how it is verified,
  and the findings of reference mode (every non-obvious Showdown behaviour the
  engine reproduces).
- [`adding-a-mechanic.md`](docs/adding-a-mechanic.md): how to add a move,
  ability or item, and how to verify it.
- [`CHANGELOG.md`](CHANGELOG.md): what changed in each release.
- [`CONTRIBUTING.md`](CONTRIBUTING.md): how to report a bug, propose an idea or send a change.

## License

MIT; see [LICENSE](LICENSE). The engine ports Pokémon Showdown's logic and
data, which is also MIT-licensed; see [NOTICE](NOTICE).

## Disclaimer

Ninjask is an unofficial fan project. It is not affiliated with, endorsed,
sponsored or approved by Nintendo, Game Freak, Creatures or The Pokémon
Company, nor by Pokémon Showdown. Pokémon and Pokémon character names are
trademarks of their respective owners, used here only to identify the game
mechanics the engine simulates. No official artwork, sprites, sounds or other
game assets are included.
