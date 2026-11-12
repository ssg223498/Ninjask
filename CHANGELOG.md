# Changelog

## Unreleased

Singles: Champions BSS Reg M-C (`gen9championsbssregmc`, bring 6 pick 3)
plays like Showdown on the `singles-bss` branch, checked on 1,000 of 1,000
reference games with every request compared (`check_reference.py --format
gen9championsbssregmc --requests`). Doubles games keep the same logs and
random numbers (the fingerprint's `logs` and `draws` hashes are
unchanged); their requests change where they now match Showdown's
(`active`, `noCancel`, the updates after a refused choice), so the
fingerprint moved from `fb8b1d107db9f213` to `468d11e0d5f4cf86`. With
`--requests`, doubles match on 1,000 of 1,000 synthetic games and 1,000 of
1,000 games of the VGCPastes pool.

### Added

- `scripts/check_reference.py --format <id>` (or `$NINJASK_FORMAT`) checks any
  exported format, and `--requests` (or `$NINJASK_CHECK_REQUESTS=1`) also
  compares every request Showdown sends with the engine's (without `rqid`,
  which Showdown's server adds, not its simulator).
- `team_preview_choices(request, game_type="singles")`: in singles one
  Pokémon leads, so each brought one can (the doubles reading, two leads,
  never lets the third one lead). `RandomPolicy` passes its game type.
- `tests/python/test_singles.py`.
- `Battle.damage_party` and `Battle.order_party` (Rust:
  `Battle::damage_calc_party`, `order_odds_party`, `with_in_slot`; branch
  `singles-bss-eval`): `damage` and `order` for any Pokémon of a party,
  named "p1:2" (the side and the index into the projection's
  `sides[i]["pokemon"]`). A benched Pokémon counts as if it were in its
  side's first slot, switched in with nothing happening on the way in: the
  battle's field, weather and side conditions, no boosts. Pointing at
  Pokémon on the field gives exactly `damage` and `order`.

### Fixed

- Singles: a move used without a chosen target (resolveAction, a retarget
  after a faint) takes the foe's slot without a random draw, as Showdown's
  `getRandomTarget` does in singles. The engine drew `random(1)`, which
  shifted every later draw.
- Singles: team preview orders (`Battle.legal_choices`, `Batch`) let each
  brought Pokémon lead, not only the first two.
- Singles: a target may be given ("move 1 1") when it is valid, as Showdown
  accepts it; the engine refused any target.
- Choices with more parts than active slots ("move 1, move 2" in singles,
  three parts in doubles) are refused, as Showdown refuses them; the engine
  ignored the extra parts.
- Requests: `active` is Showdown's `position < side.active.length`, so the
  Pokémon in an active slot is active at team preview and after fainting.
- Requests: `noCancel` as Showdown sets it under Cancel Mod (Flat Rules):
  when only one side has a request.
- Requests after a refused choice: `update`, and the refused move disabled
  with an empty `disabledSource`, as Showdown's `chooseMove` update leaves
  them; the first refusal of a disabled move is "[Unavailable choice]" even
  when the request showed it disabled. In singles Imprison's `maybeDisabled`
  stays and only the refused move is marked (`updateDisabledRequest` skips
  singles), and picking it again is refused as unavailable again.
- Requests in doubles with a fainted Pokémon left in its slot: it is not
  the last active one (Showdown's `isLastActive` needs `isActive`, which
  fainting clears), and its moves don't show a foe's Imprison (the
  `DisableMove` pass skips it), as Showdown's requests show them.
- Windows: the package, the checks and the tests read and write text as
  UTF-8 (the format export, team pools, the reference server's pipe), not
  in the locale's code page.
- `refsim/server.js` names the call sites of Showdown's random draws for a
  checkout in any directory and with Windows paths.

## 0.4.0 (2026-11-05)

A native interface for training (`ninjask.Batch`, player-view legal
choices, numeric observations), experimental exact outcome enumeration
contributed by CalebBrody (dcabatin/Ninjask#8), and `research/` on what
trading parity for speed would buy. And the fixes for 70 bugs (one
regression game each in `tests/python/test_chaos.py`) found by fuzzing the
strangest mechanics against Showdown (`scripts/check_chaos.py`):
more than 150,000 such games and 34,000 games of a pool that fields every
legal species, ability, move and item. Their divergence rate fell from
2.5% to about 1 in 4,000, every divergence found was fixed, and parity on
real and random teams stays exact. Every seeded game gives the same logs,
requests and draws as 0.3.0 except where the fixes below apply. The
fingerprint moved from `6e2f85907f97e4e9` to `fb8b1d107db9f213` with the
same logs, requests and random numbers: a status move's AfterMoveSecondary
sort now collects the frozen target's handler, as Showdown does, so a
traced sort records one item where it had none.

Upgrading an editable checkout from before 0.3.0: delete
`python/ninjask/_core.cpython-*.so`, or Python keeps loading it instead of
the new `_core.abi3.so`.

### Added

- **Experimental:** Rust and Python outcome enumeration for search and regret minimization,
  with exhaustive chance probabilities or a damage-endpoint approximation,
  and hashable full battle-state keys.
  (dcabatin/Ninjask#8, by CalebBrody.) Each call explores at most
  `max_paths` chance paths (`DEFAULT_MAX_PATHS`, 20,000, about 0.7 s) and
  raises an error beyond that instead of running for minutes: on 40
  mid-game states of real teams, Precise fit the default budget on 14 and
  DamageEndpoints on 22 (`examples/validate_outcomes.rs`). Where it finished
  it was exact against 28,000 sampled resolutions. The damage-endpoint mode
  is `EnumerationMode.DamageEndpoints` (an approximation). Being
  experimental, the API may change in any release.
- **A native interface for training**, with no JSON and no strings in the
  loop:
  - `Battle.legal_choices(side)` lists the choices the side's request
    offers, in `ninjask.choices`' order, from the player's view: a hidden
    disable or trap never shapes the list, so a listed choice can be
    refused, as in Showdown. `.legal_actions(side)` gives them as int8 rows
    (`action_width(side)` per choice), and `.choose_index(side, i)` submits
    one by position.
  - `Tracker.encode()` and `.encode_ids()` give a player's view as float32
    and int16 bytes for `numpy.frombuffer` (layouts in `src/encode.rs`;
    `ninjask.OBS_LEN`, `IDS_LEN`, `ACTION_FIELDS`); `Dex.vocab()` gives
    the embedding-table sizes.
  - `ninjask.Batch(dex, n)` steps `n` battles together on all cores with
    the GIL released: `observe()` reports every pending decision as
    numeric buffers, and `step()` applies one choice index to each (every
    index is checked before any is applied). In Rust,
    `ninjask::batch::Batch`, `Battle::player_choices` and
    `for_each_player_choice`. About 700 random games/s with every decision
    observed on one core of a 4-vCPU VM, and 1,300 on four; a loop through
    the JSON requests plays about 45.
- `scripts/check_chaos.py`: games between teams made of the strangest
  supported mechanics (Illusion, Imposter, Transform, Reflect Type, Perish
  Song, Future Sight, Instruct, Skill Swap and the like), compared with
  Showdown after every input. It found the fixes under Fixed below.
- `research/`: what trading parity for speed would buy (about 1.2 times
  in the engine, for real changes to play) and how much engine speed
  matters in training. The research-only switches behind it
  (`ninjask::approx`, feature `approx`) are off in every release.

### Fixed

- A Curse chosen before its user becomes a Ghost the same turn (Absol
  Mega Evolving into Absol-Mega-Z) keeps the user as its target, as in
  Showdown, whose queued move keeps the target it was resolved with: its
  onHit then curses a random foe directly, so Good as Gold doesn't stop
  it. The engine aimed it at a foe instead, which Good as Gold blocked.
- Metal Burst and Comeuppance deal 1.5 times an odd damage to a
  substitute unrounded, as in Showdown, which leaves the substitute with
  x.5 HP. Two such hits add up to a whole point a later hit no longer
  has to take, and a drain or recoil from breaking such a substitute is
  rounded from the x.5. The engine rounded the damage down. The
  projection shows the half (`"hp": 30.5`).
- An item handed to a Pokémon that ignores its item (Klutz, Magic Room)
  by Trick, Recycle, Symbiosis or the like still runs its Start, as in
  Showdown: a Metronome adds its volatile, a White Herb on a lowered stat
  is used up (without resetting it), and a Choice item clears an old
  Choice lock. The engine skipped them. At switch-in an ignored item
  still does nothing. Found by the large reference run in 1 of 5,000
  synthetic games (a Klutz Golurk Tricking away a Metronome).
- Found by "chaos" games, whose every Pokémon carries Illusion, Imposter,
  Transform, Reflect Type, Perish Song, Future Sight, Skill Swap and the
  like, and by the coverage pool of just_a_simulator (418 teams that
  between them field everything legal). Every game that diverged now
  matches; `tests/python/test_chaos.py` replays one per bug:
  - Type knowledge as Showdown keeps it (`knownType`): a Pokémon that
    transforms into a foe, or Reflect Types one, hides its new types until
    something attacks it, so Reflect Type on such an ally keeps the old
    apparent type, and the end-of-turn catch-up looks at the teammate a
    disguised Zoroark appears as. Status moves now count as attacks for it
    (`attackedBy`), and the projection carries `knownType`.
  - A fainted Pokémon's queued move no longer sorts the Unaware holders
    when the queue is re-sorted (it isn't active any more), which drew an
    extra random number.
  - Opportunist's onEnd forgets its pending boosts (also on fainting).
  - Revival Blessing with nobody to revive fails after Protean and Libero
    act (its check is onTryHit), not before.
  - Struggle is typeless (`???`) rather than Stellar, so Protean and Libero
    leave it alone and it gets no STAB.
  - Each queued move keeps its own `originalTarget` (Stalwart, Snipe
    Shot): an Instructed move queued in front of the user's own no longer
    replaces the target the user's own move tracks.
  - Destiny Bond doesn't take down the user of a Future Sight that knocks
    out its holder (`futuremove`).
  - A Pokémon brought in by Baton Pass or Shed Tail starts with no
    record of past hits (`timesAttacked`, `attackedBy`), as
    `copyVolatileFrom`'s `clearVolatile` leaves it.
  - The AfterMove event collects no handlers when neither the user nor
    runMove's target is still active (a Copycat that called Memento), so
    White Herb waits for the next move.
  - Iron Ball's effectiveness handler returns at once under Gravity,
    Ingrain or Smack Down, so a grounded Bug/Flying holder resists Ground.
  - A residual ability handler suppressed by Gastro Acid still gets the
    faint check after it (a Perish Song faint shows before `upkeep`).
  - The AfterMove handlers leave out a Pokémon left with no HP (a user
    its own recoil knocked out), as `alliesAndSelf()` and `foes()` do.
  - Volatiles that Showdown adds with `addVolatile` now run the
    TryAddVolatile sort first, which draws when two handlers tie (two
    Sweet Veils): Protect's and Wide Guard's `stall`, Follow Me, Rage
    Powder, Perish Song and Throat Chop's secondary.
  - A status move records `attackedBy` only on the targets it took effect
    on past any substitute (`targetsCopy`).
  - Stalwart doesn't make Counter, Mirror Coat, Metal Burst and Comeuppance
    track their target (`move.target !== 'scripted'`): they still go to
    the last attacker.
  - A ModifyBoost event on a Pokémon that has left the field (a Future
    Sight's user) sorts no Unaware handlers.
  - Air Lock's and Cloud Nine's onEnd runs under Gastro Acid too (End
    isn't suppressed).
  - Healing Wish with nobody to switch in fails after Protean and Libero
    act (onTryHit), like Revival Blessing.
  - Baton Pass and Shed Tail clear the newcomer's last move and move
    results too (`copyVolatileFrom`'s `clearVolatile`).
  - A fainting Illusion holder's onFaint joins the Faint event's sort (it
    ties with an ally's Receiver at equal speed).
  - Cud Chew re-eating a berry under Magic Room (or Klutz) has no effect,
    as Showdown's suppressed `Eat` event leaves it.
  - A charging move called by another (Copycat) remembers the slot of its
    target after redirection (Lightning Rod), as `twoturnmove` does.
  - Clear Body, White Smoke and Full Metal Body stay silent for Octolock's
    drop.
  - A status move whose effect returns NOT_FAIL (Heal Pulse at full HP)
    doesn't count as an attack (`attackedBy`).
  - Event sorts collect a suppressed ability's handler too, as
    `findEventHandlers` does, and skip it when it runs: Gastro Acid, Mold
    Breaker, and a transformed Pokémon's `notransform` ability (a Ditto
    copying Morpeko's Hunger Switch), for Damp, No Guard, Pickpocket,
    Berserk, Unburden, Symbiosis, Sniper and the residual handlers.
  - Ally Switch's own AfterMoveSecondary event goes to the Pokémon that
    used it, in its new slot (it ran an ally's Berserk).
  - Magic Bounce bounces a side move (Spikes) even while its holder is
    mid-Dive: `onAllyTryHitSide` checks the move's first target.
  - Life Orb's recoil follows a Sheer Force move whose user lost Sheer
    Force during it (Wandering Spirit): `suppressingSecondaries()` checks
    the ability, not only the move.
  - Flower Veil (unlike Clear Body) still announces Octolock's drop.
  - Every side move runs TryHitSide's sort over its side's Magic Bounce,
    Sap Sipper and Soundproof holders; Sap Sipper drinks a Grass move
    aimed at its side, and Soundproof announces a sound one.
  - Replacing or suppressing Illusion (Simple Beam, Worry Seed,
    Entrainment, Gastro Acid) breaks the disguise first, as its onEnd does.
  - An ignored White Herb (Magic Room, Klutz) still sorts in AfterMove
    and AfterMega.
  - TryBoost's handlers (the target's own ability and each Flower Veil on
    its side) run in speed order: a faster ally's Flower Veil blocks a
    drop before Mirror Armor reflects it.
  - Life Orb, Expert Belt and resist berries join the ModifyDamage sort
    while ignored; Overcoat defers to a Grass type's own powder immunity.
  - Future Sight on a revived Pokémon not yet sent in calculates its
    damage and fails; a partial trap remembers its user while that user
    is off the field; a Pokémon off the field takes no status (Spicy Spray
    on a Future Sight user).
  - An ally's Pollen Puff rolls Stench's flinch secondary like King's
    Rock's; the ModifySecondaries sort (Shield Dust, Parental Bond) and
    each tryTrap's Immunity sort are drawn, with Magnet Rise, Dig and Dive
    among the Immunity handlers.
  - A regional forme keeps its species' Mega Stone (Slowbro-Galar and
    Slowbronite): most stones' onTakeItem goes by the base species' own
    base species. Found by parity on the 418-team coverage pool of
    just_a_simulator.
  - The `???` type never gets STAB (Struggle from a Pokémon Burn Up left
    `???`); Burn Up from a frozen user that isn't Fire type goes through
    the freeze check instead of thawing it; a terrain set during the
    residuals (Seed Sower hit by Future Sight) keeps its full duration,
    the old terrain's countdown ticking instead; Baton Pass doesn't pass on
    unknown types.
  - Stench's flinch on a move with its own secondary handler (Syrup Bomb,
    Throat Chop...) is applied, not handed to that handler; a damaging
    move aimed at its own user (a locked Thrash with no foe in range)
    checks the user's type immunity.
  - Shell Side Arm compares boosted stats without the ModifyBoost event,
    so Unaware doesn't hide the user's boosts; the residual sort collects
    every Pokémon in a slot, a revived one not yet sent back in included
    (with its ability).
  - Later hits of Population Bomb and Triple Axel: a fractional accuracy
    (after the stage table) takes no ModifyAccuracy modifier (Gravity,
    Compound Eyes...), as Showdown's runEvent modifies only whole numbers;
    Skill Link's removal of the per-hit check is decided once, so losing
    the ability mid-move (Wandering Spirit) changes nothing.
  - A status move's AfterMoveSecondary sort collects every handler
    (Pickpocket, Red Card, Eject Button, freeze), not only Berserk's; Heal
    Bell ignores a fainted ally's Good as Gold; Leftovers taken during the
    residuals (Symbiosis) still heals its old holder that turn.
  - A Pokémon revived in its slot and not yet sent back in counts as
    inactive: its speed collects no handlers (no Unaware sort), and a
    partial trap it set ends. A substitute's damage takes Unaware from
    Showdown's `activeTarget`, the last target of the accuracy step, so on
    a spread move another target's Unaware can ignore the user's stages.
  - A forced eat (Teatime) skips TryEatItem, so Ripen isn't announced;
    Oblivious and Own Tempo on a Pokémon with no HP announce themselves
    but can't remove the volatile (removeVolatile needs HP); a switch-in
    item handler belongs to the item it was collected for, so Symbiosis
    mid-switch-in skips the new holder's and still announces a given-away
    Air Balloon for its old holder.
  - Symbiosis handlers are collected in slot order (`alliesAndSelf`),
    which decides speed ties; Syrup Bomb stays on a holder with no HP
    (removeVolatile needs HP); a one-hit KO move on a semi-invulnerable
    target (No Guard) skips the level and Ice checks and keeps its own
    accuracy.
  - Perish Song's TryHit sees its type after Liquid Voice (Dry Skin,
    Water Absorb); Pickup on a Pokémon with no HP shows its message but
    gives no item; an Instructed move whose first action is a before-turn
    or priority-charge one (Mirror Coat) leaves no original target for the
    Pokémon's own queued move.
  - A fainted target's Pressure takes no extra PP (Future Sight aimed at
    it): an inactive Pokémon's ability is ignored.
  - A Zoroark whose Illusion is suppressed (Gastro Acid passed by Baton
    Pass) is revealed as it faints: onFaint is skipped, End isn't; Fling's
    item line shows even when Symbiosis already gave the item away; the
    drag itself runs DragOut again, so Guard Dog gained mid-move (Wandering
    Spirit) holds the target.
- `check_reference.py` and `check_chaos.py` choose again as often as
  Showdown refuses a choice for something hidden from the player (an
  Imprison-disabled move, then a switch out of a hidden Shadow Tag trap),
  not just once.

### Changed

- Enumerating joint choices and team-preview orders no longer allocates
  per choice; the internal random player is unchanged, and so is the
  fingerprint.

## 0.3.0 (2026-11-04)

Two fixes found by checking the engine on inputs nobody chose (games under
a fresh seed, and teams from public sources); every other seeded game gives
the same logs, requests and draws as 0.2.0, and the fingerprint is still
`6e2f85907f97e4e9`. The checks can now fail, and the wheels cover more of
the Python world.

### Changed

- **Wheels use the stable ABI**: one wheel per platform for CPython 3.12
  and every later version (3.14 included), and new wheels for Linux musl
  (Alpine; x86_64 and aarch64) and Windows on Arm.
- **The checks fail when they find a problem.** `check_reference.py`,
  `check_tracker.py` and `verify_engine.py` exited 0 whatever they found, so
  CI's parity step could not fail; they now exit 1 on a divergence, a
  tracker or world mismatch, or a failed comparison. CI also replays 200
  games of real teams (below).
- `ninjask.__all__` lists the package's public names, so
  `from ninjask import Battle` type-checks under `mypy --strict`.
- pyo3 0.25 → 0.30, which fixes RUSTSEC-2026-0176 and RUSTSEC-2026-0177
  (Ninjask called neither affected API). A crate that embeds Ninjask's
  classes (feature `python`) must build against pyo3 0.30 too.
- `scripts/verify_engine.py` checks Reg M-C by default (it checked the
  Champions doubles custom game); `--format` still picks another.

### Fixed

- Mimicry now changes its holder's type when the terrain ends while the
  holder is at 0 HP with its faint still pending, as Showdown does (the
  `-activate` and `-end typechange` lines were missing). Found by
  `check_reference.py` in 1 of 10,000 synthetic games; the fingerprint is
  unchanged.
- The damage calculation (`Battle::calc`, Python `Battle.damage`) gave Fling
  a base power of 0 (1 or 2 HP a roll) instead of its item's Fling power.
  `damage_check` on 704 teams from public replays found it: 6 of 87,591
  first hits disagreed with play, all Fling. Play itself was right.

### Added

- `pools/vgcpastes-regmc.txt`: 766 Reg M-C tournament teams from the
  [VGCPastes](https://x.com/VGCPastes) Repository, with each team's sheet ID
  and `scripts/build_vgcpastes_pool.py` to rebuild it. The README's
  real-team claims are measured on it (1,000 of 1,000 games; 766 of 766
  supported).
- README: known limitations, the stability promise before 1.0, where the
  Rust quick start's data comes from, legal choices and tracker feeding in
  Python, and speed figures re-measured with their spread and machine.

## 0.2.0 (2026-10-29)

Behaviour is unchanged: every seeded game gives the same logs, requests and
random draws as 0.1.0 (the fingerprint is still `6e2f85907f97e4e9`), except
for the one line noted under Changed.

### Faster

- About 21% fewer instructions per random game on a pool of real Reg M-C
  teams: roughly 7,000 random games per second on one core with logging off,
  and about 250 times Showdown's speed with logging on in both. The turn loop
  allocates far less (short handler and target lists are inline `SmallVec`s,
  a new dependency), event sorts and `eachEvent('Update')` skip work when no
  handler can act or draw, choice generation hoists per-Pokémon checks, and
  the release profile uses fat LTO.
- `ninjask.load()` parses each format export once per process (keyed on the
  file and its modification time), as `dex()` already did. The spec is
  shared, so treat it as read-only.

### Changed

- The open-team-sheets format `gen9championsvgc2026regmcots` reports the
  base format's name, as PKMN Arena and Showdown do, so its `|tier|` line no
  longer ends in "OTS". Its id and forced-open sheets are unchanged.
- LICENSE: the copyright holder's name is corrected to David Cabatingan
  (0.1.0's packages carry a misspelling).

### Added

- `Dex::mega_forme` and `EventHandlers::has_event`. No public type changed.
- `examples/quickstart.rs`, and `--log` for `examples/bench.rs`.
- `refsim/bench_showdown.js`: Showdown's own throughput on the same random
  games, for comparison.
- Documentation: the README reorganised around why to use Ninjask, how, and
  how to contribute; crate-level docs for docs.rs; CONTRIBUTING.md; issue and
  pull request templates.

## 0.1.0

First public release.

- Pokémon Champions VGC doubles (Reg M-C) engine in Rust, matching Pokémon
  Showdown at the pinned commit (`refsim/SHOWDOWN_COMMIT`) log line for log
  line in reference mode, on every supported move, item and ability.
- Python bindings (`ninjask`): `Dex`, `Battle`, `Tracker`, worlds for
  search, the damage and turn-order calculation, and the parity checks.
- The Python package bundles the format exports for Reg M-C (closed and open
  team sheets), Reg M-B and the Champions doubles custom game.
