## What this changes

## Checks

- [ ] `cargo fmt --check`, `cargo clippy` (with and without `--features python-module`), `cargo test --release` and `pytest -q` pass
- [ ] `check_reference.py --synthetic --games 200` matches every game
- [ ] Fingerprint: unchanged (`468d11e0d5f4cf86`), or changed because behaviour changed, and that change matches Showdown (explained above)
- [ ] `CHANGELOG.md` has a line under "Unreleased", if a user would notice
