# Contributing to Ninjask

Thanks for your interest. Ninjask has one rule that shapes everything else:
Pokémon Showdown at the commit in `refsim/SHOWDOWN_COMMIT` is the
specification. A change is correct when the engine does what that commit
does, and the checks below are how we know.

## Ways to help

- **Report a difference from Showdown.** Any mismatch is a bug: a log line,
  a request, a damage roll, a random draw. Open a
  [bug report](https://github.com/dcabatin/Ninjask/issues/new?template=bug.yml)
  with the teams, the seed and the choices that reproduce it.
  `python scripts/check_reference.py --verbose` prints both simulators' logs
  and random draws at the first divergence, which is usually all we need.
- **Share an idea or a use case.** Open an
  [idea](https://github.com/dcabatin/Ninjask/issues/new?template=idea.yml).
  Knowing what you are building (a bot, a training run, a team analysis)
  helps decide what comes next. Some directions we'd welcome:
  - new regulations as they launch: most are data changes, plus any
    mechanics that are new to them;
  - an API that enumerates a turn's chance outcomes exactly, for search and
    endgame solvers;
  - faster batched bindings for training loops;
  - bindings for other languages.
- **Write code.** Small fixes can go straight to a pull request; for anything
  larger, open an issue first so we can agree on the approach.

## Setting up

Follow the README's [Working from source](README.md#working-from-source):
a virtual environment, `pip install -e '.[test]'`, then
`./scripts/setup_showdown.sh` for the pinned Showdown. You need Python 3.12+,
stable Rust, Node.js 22+ and git.

## What a change must pass

CI runs these on every pull request; run them before you open one:

```sh
cargo fmt --check
cargo clippy --release --all-targets -- -D warnings
cargo clippy --release --all-targets --features python-module -- -D warnings
cargo test --release
pytest -q
python scripts/check_reference.py --synthetic --games 200 --workers 4
python scripts/check_reference.py --pool pools/vgcpastes-regmc.txt --games 200 --workers 4
cargo run --release --example fingerprint -- data/derived/format/gen9championsvgc2026regmc.json tests/fixtures/regmc_teams.txt 2000
```

- **A change that should not change behaviour** (a refactor, a speed-up)
  must leave the fingerprint at `468d11e0d5f4cf86`. If it changes, the change
  altered behaviour, and the reference check says where.
- **A change that does change behaviour** must match Showdown: run
  `check_reference.py` on more games (`--games 1000`, with and without
  `--synthetic`), and say in the pull request why the fingerprint moved.
- **Speed claims** come with numbers from `examples/bench.rs`, ideally
  instruction counts from `valgrind --tool=callgrind`, because wall-clock
  speed on shared machines varies by 10% or more.

## Porting a move, ability or item

Follow [`docs/adding-a-mechanic.md`](docs/adding-a-mechanic.md). In short:

- port from the pinned Showdown's code, not from memory, the games or another
  simulator;
- cite the Showdown function each handler follows, in a comment;
- mark the entity supported only when the checks that document lists pass.

## Style

- Match the code around you: its naming, its comment density, its idiom.
- Comments explain why, and point at the Showdown code being followed.
- Keep a pull request to one change. A refactor and a behaviour change go in
  separate pull requests, so the fingerprint check means something.
- Add a line to `CHANGELOG.md` under "Unreleased" for anything a user would
  notice.

## Conduct

Be kind and assume good faith. Harassment or personal attacks aren't
tolerated in issues, pull requests or anywhere else in the project.
