//! Fixed-size numeric encodings of a player's view and of its choices, for
//! models: no JSON and no strings. The Python classes hand them over as
//! `bytes` that `numpy.frombuffer` reads without copying (little-endian).
//!
//! - [`observation`]: `OBS_LEN` floats from a [`Tracker`]'s view, so only
//!   what that player knows.
//! - [`ids`]: `IDS_LEN` small integers, the categorical part of the same
//!   view (species, item, ability, moves) for embedding tables, `0` meaning
//!   unknown.
//! - [`battle_observation`] and [`battle_ids`]: the same two layouts filled
//!   from a [`Battle`] (full information, both teams) seen from one side:
//!   `BATTLE_OBS_LEN` floats and `BATTLE_IDS_LEN` integers.
//! - [`action_row`]: `ACTION_FIELDS` integers per active slot of a joint
//!   choice, in the order [`Battle::player_choices`] lists them.
//!
//! The player's own side comes first and the foe's second, whichever side
//! the player is, then the field. Each side has six Pokémon slots: in team
//! order for the tracker's view, in the battle's current Pokémon order
//! (Showdown's `side.pokemon`, the projection's `pokemon` list) for the
//! battle's. A side's block is written the same way whichever side looks,
//! so side `s`'s block seen from `s` equals it seen from the foe.

use crate::battle::Battle;
use crate::dex::types::Status;
use crate::state::{SideCond, SlotChoice, RECHARGE};
use crate::tracker::{ItemView, Tracker};

/// Floats per Pokémon in [`observation`]: HP fraction, fainted, seen,
/// active, status one-hot (none, burn, paralysis, poison, toxic, sleep,
/// freeze), the seven boost stages over 6, Mega Evolved.
pub const MON_FLOATS: usize = 4 + 7 + 7 + 1;
/// Floats per side: six Pokémon and their six side conditions (Tailwind,
/// Reflect, Light Screen, Aurora Veil, Wide Guard, Quick Guard).
pub const SIDE_FLOATS: usize = 6 * MON_FLOATS + 6;
/// Floats of the field, after both sides: weather (rain, sun, sand, snow),
/// terrain (grassy, psychic, electric, misty), Trick Room and Gravity.
pub const FIELD_FLOATS: usize = 4 + 4 + 2;
/// Length of [`observation`]: own side, foe side, field.
pub const OBS_LEN: usize = 2 * SIDE_FLOATS + FIELD_FLOATS;
/// Integers per Pokémon in [`ids`]: species, item, ability, four moves.
pub const MON_IDS: usize = 7;
/// Integers per side: six Pokémon.
pub const SIDE_IDS: usize = 6 * MON_IDS;
/// Length of [`ids`]: two sides of six Pokémon.
pub const IDS_LEN: usize = 2 * SIDE_IDS;
/// Length of [`battle_observation`]: [`observation`]'s layout.
pub const BATTLE_OBS_LEN: usize = OBS_LEN;
/// Length of [`battle_ids`]: [`ids`]'s layout and value ranges.
pub const BATTLE_IDS_LEN: usize = IDS_LEN;
/// Integers per active slot in [`action_row`]: kind (0 pass, 1 move,
/// 2 switch, 3 Struggle or recharge), index (the move's slot 1 to 4, or the
/// team position 1 to 6 to switch to), target (foes 1 and 2, allies -1 and
/// -2, 0 for none), Mega Evolution (0 or 1).
pub const ACTION_FIELDS: usize = 4;

/// One Pokémon's [`MON_FLOATS`], as both observations write them.
struct MonFloats<'a> {
    hp: u16,
    maxhp: u16,
    fainted: bool,
    seen: bool,
    active: bool,
    status: Status,
    boosts: &'a [i8; 7],
    mega: bool,
}

fn flag(b: bool) -> f32 {
    b as u8 as f32
}

/// A Pokémon's floats, or zeros for an empty slot.
fn push_mon(m: Option<MonFloats<'_>>, out: &mut Vec<f32>) {
    let Some(m) = m else {
        out.extend(std::iter::repeat_n(0.0, MON_FLOATS));
        return;
    };
    out.push(if m.maxhp > 0 {
        m.hp as f32 / m.maxhp as f32
    } else {
        0.0
    });
    out.push(flag(m.fainted));
    out.push(flag(m.seen));
    out.push(flag(m.active));
    let status = match m.status {
        Status::None | Status::Fnt => 0,
        Status::Brn => 1,
        Status::Par => 2,
        Status::Psn => 3,
        Status::Tox => 4,
        Status::Slp => 5,
        Status::Frz => 6,
    };
    out.extend((0..7).map(|k| flag(k == status)));
    out.extend(m.boosts.iter().map(|&b| b as f32 / 6.0));
    out.push(flag(m.mega));
}

/// The [`FIELD_FLOATS`], weather and terrain as their enum index.
fn push_field(
    weather: Option<usize>,
    terrain: Option<usize>,
    trick_room: bool,
    gravity: bool,
    out: &mut Vec<f32>,
) {
    out.extend((0..4).map(|k| flag(weather == Some(k))));
    out.extend((0..4).map(|k| flag(terrain == Some(k))));
    out.push(flag(trick_room));
    out.push(flag(gravity));
}

/// Append the view's [`OBS_LEN`] floats to `out`.
pub fn observation(t: &Tracker, out: &mut Vec<f32>) {
    let start = out.len();
    for side in [t.me, 1 - t.me] {
        let sv = &t.sides[side];
        for i in 0..6 {
            let m = sv.mons.get(i).map(|m| MonFloats {
                hp: m.hp,
                maxhp: m.maxhp,
                fainted: m.fainted,
                seen: m.seen,
                active: m.slot.is_some(),
                status: m.status,
                boosts: &m.boosts,
                mega: m.mega,
            });
            push_mon(m, out);
        }
        for c in SideCond::ALL {
            out.push(flag(sv.conditions.iter().any(|(k, _)| *k == c)));
        }
    }
    push_field(
        t.field.weather.map(|(w, _)| w as usize),
        t.field.terrain.map(|(x, _)| x as usize),
        t.field.trick_room.is_some(),
        t.field.gravity.is_some(),
        out,
    );
    debug_assert_eq!(out.len() - start, OBS_LEN);
}

/// Append the view's [`IDS_LEN`] categorical values to `out`. Each is a
/// dex index plus one (0: unknown), except the item: 0 unknown, 1 known to
/// hold nothing, else the item's index plus two. The dex sizes bound them
/// (`Dex.vocab()` in Python).
pub fn ids(t: &Tracker, out: &mut Vec<i16>) {
    let start = out.len();
    for side in [t.me, 1 - t.me] {
        let sv = &t.sides[side];
        for i in 0..6 {
            let Some(m) = sv.mons.get(i) else {
                out.extend(std::iter::repeat_n(0, MON_IDS));
                continue;
            };
            out.push(m.species.map_or(0, |x| x as i16 + 1));
            out.push(match m.item {
                ItemView::Unknown => 0,
                ItemView::None => 1,
                ItemView::Known(x) => x as i16 + 2,
            });
            out.push(m.ability.map_or(0, |x| x as i16 + 1));
            for k in 0..4 {
                out.push(m.moves.get(k).map_or(0, |mv| mv.id as i16 + 1));
            }
        }
    }
    debug_assert_eq!(out.len() - start, IDS_LEN);
}

/// Append [`BATTLE_OBS_LEN`] floats to `out`: the battle seen from side
/// `me` (0 or 1), in [`observation`]'s layout.
///
/// The battle knows everything, so the slots for what a view may not know
/// are filled from the truth: the HP fraction is exact on both sides (a
/// foe's is out of 100 in a view), `seen` is set for every Pokémon (all of
/// the battle's were brought), and active means on the field and not
/// fainted (a view clears the slot at a faint).
pub fn battle_observation(b: &Battle, me: usize, out: &mut Vec<f32>) {
    let start = out.len();
    for side in [me, 1 - me] {
        let s = &b.sides[side];
        for i in 0..6 {
            let m = s.mons.get(i).map(|m| MonFloats {
                hp: m.hp,
                maxhp: m.maxhp,
                fainted: m.fainted,
                seen: true,
                active: m.is_active && !m.fainted,
                status: m.status,
                boosts: &m.boosts,
                mega: m.mega_evolved,
            });
            push_mon(m, out);
        }
        for c in SideCond::ALL {
            out.push(flag(s.conds.get(c) > 0));
        }
    }
    let f = &b.field;
    push_field(
        f.weather.map(|w| w as usize),
        f.terrain.map(|x| x as usize),
        f.trick_room > 0,
        f.gravity > 0,
        out,
    );
    debug_assert_eq!(out.len() - start, BATTLE_OBS_LEN);
}

/// Append [`BATTLE_IDS_LEN`] values to `out`: [`ids`]'s layout and value
/// ranges for the battle seen from side `me`. Nothing is unknown, so a
/// Pokémon's species, item and ability are never 0 (the item is 1 when it
/// holds none); 0 is left for an empty Pokémon or move slot.
pub fn battle_ids(b: &Battle, me: usize, out: &mut Vec<i16>) {
    let start = out.len();
    for side in [me, 1 - me] {
        let s = &b.sides[side];
        for i in 0..6 {
            let Some(m) = s.mons.get(i) else {
                out.extend(std::iter::repeat_n(0, MON_IDS));
                continue;
            };
            out.push(m.species as i16 + 1);
            out.push(m.item.map_or(1, |x| x as i16 + 2));
            out.push(m.ability as i16 + 1);
            for k in 0..4 {
                out.push(m.moves.get(k).map_or(0, |mv| mv.id as i16 + 1));
            }
        }
    }
    debug_assert_eq!(out.len() - start, BATTLE_IDS_LEN);
}

/// Append a joint choice's [`ACTION_FIELDS`] values per slot to `out`.
pub fn action_row(choice: &[SlotChoice], out: &mut Vec<i8>) {
    for c in choice {
        match *c {
            SlotChoice::Pass => out.extend([0, 0, 0, 0]),
            SlotChoice::Switch { index } => out.extend([2, index as i8 + 1, 0, 0]),
            SlotChoice::Move { slot, target, mega } if slot >= RECHARGE => {
                out.extend([3, 1, target, mega as i8])
            }
            SlotChoice::Move { slot, target, mega } => {
                out.extend([1, slot as i8 + 1, target, mega as i8])
            }
        }
    }
}
