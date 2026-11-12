//! Worlds: a whole [`Battle`] built from one player's view (a [`Tracker`])
//! plus an assumption for what that player can't see of the foe's sets.
//!
//! Search and the engine-computed features both need a battle to run or
//! measure, but a player knows only its own side exactly. A world fills the
//! rest in:
//!
//! - **one's own side** as the view holds it: species, exact HP and stats,
//!   item, ability, moves with PP, status, boosts, volatiles, types;
//! - **the move history** the view reads from the log: the field's last
//!   move, and per Pokémon where its last move was aimed, this and last
//!   turn's move result, the hits it took since it came in (a foe's damage
//!   as its share of the world's max HP), the Protect stall counter, the
//!   item it used up and whether it ate a berry;
//! - **the field** and every condition the view knows (weather, terrain,
//!   rooms, Gravity, Fairy Lock, side and slot conditions), with the turns
//!   left the view can count;
//! - **the foe's side**: what the view knows (species and forme, HP as the
//!   shown percentage, status, boosts, volatiles, revealed moves, a revealed
//!   item or ability), and for the rest a [`SetAssumption`] per Pokémon:
//!   item, ability, the unrevealed moves, nature and stat points.
//!
//! Revealed information always wins over an assumption. With no assumption
//! a foe gets the default ([`default_assumption`]): no item and no ability
//! unless revealed (a species with a single possible ability, a Mega for
//! instance, has it), only its revealed moves, the sheet's nature under open
//! team sheets or a neutral one, and stat points from a placeholder spread
//! rule ([`placeholder_points`]).
//!
//! What the view can't know is filled with a documented guess: a foe's exact
//! HP is the middle of the range its percentage allows; the turns left of
//! confusion, sleep and a rampage, and which Pokémon set a slot condition,
//! are fixed defaults; a weather or terrain whose length the view can't tell
//! (an extending item) gets the default length unless it already outlasted
//! it. Anything that would have needed the log beyond the view (an exact
//! partial-trap length, a hidden Illusion) is left at its default.

use std::sync::Arc;

use serde_json::{json, Map, Value};

use crate::battle::{build_mon, Battle, MonRef};
use crate::dex::team::{champions_max_pp, champions_stats, MonSet};
use crate::dex::types::*;
use crate::dex::{Dex, Nature};
use crate::state::*;
use crate::tracker::{ItemView, MonView, SideView, Tracker};

/// What is assumed about one foe Pokémon's hidden set. Each field left at
/// its default falls back to [`default_assumption`]'s value; whatever the
/// view has revealed overrides it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SetAssumption {
    /// The item it holds now: `Some(None)` for none, `None` for the default
    /// (no item).
    pub item: Option<Option<ItemId>>,
    /// Its current ability.
    pub ability: Option<AbilityId>,
    /// Its moves: the revealed ones come first, then these, up to four.
    pub moves: Vec<MoveId>,
    /// Its nature, as Showdown names it ("Adamant").
    pub nature: Option<String>,
    /// Stat points (Champions) in Showdown's stat order (HP, Atk, Def, SpA,
    /// SpD, Spe).
    pub points: Option<[u16; 6]>,
}

/// Assumptions for a whole world.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Assumptions {
    /// Per foe Pokémon, in the view's order (team-preview order): `None` is
    /// the default assumption.
    pub foe: Vec<Option<SetAssumption>>,
    /// The foe Pokémon brought to battle, by index in the view. By default:
    /// the ones seen, then the unseen ones in preview order, up to the
    /// team size.
    pub brought: Option<Vec<usize>>,
    /// Seed of the world's own RNG.
    pub seed: u64,
}

impl Assumptions {
    /// Read assumptions from JSON: `{"foe": [null | {"item": "choicescarf"
    /// (or "" for none), "ability": "intimidate", "moves": ["fakeout", ...],
    /// "nature": "Jolly", "evs": {"hp": 2, "atk": 32, ...}}, ...],
    /// "brought": [0, 2, 3, 5], "seed": 1}`, with ids or names. Every key is
    /// optional.
    pub fn from_json(dex: &Dex, v: &Value) -> Result<Assumptions, String> {
        let mut out = Assumptions {
            seed: v["seed"].as_u64().unwrap_or(0),
            ..Default::default()
        };
        if let Some(b) = v["brought"].as_array() {
            out.brought = Some(
                b.iter()
                    .filter_map(|x| x.as_u64())
                    .map(|x| x as usize)
                    .collect(),
            );
        }
        for a in v["foe"].as_array().into_iter().flatten() {
            if a.is_null() {
                out.foe.push(None);
                continue;
            }
            let item = match a["item"].as_str() {
                None => None,
                Some("") => Some(None),
                Some(i) => Some(Some(
                    dex.item_id(i).ok_or_else(|| format!("unknown item {i}"))?,
                )),
            };
            let ability = match a["ability"].as_str() {
                None => None,
                Some(x) => Some(
                    dex.ability_id(x)
                        .ok_or_else(|| format!("unknown ability {x}"))?,
                ),
            };
            let mut moves = Vec::new();
            for m in a["moves"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|m| m.as_str())
            {
                moves.push(dex.move_id(m).ok_or_else(|| format!("unknown move {m}"))?);
            }
            let points = a["evs"].as_object().map(|evs| {
                let mut p = [0u16; 6];
                for st in Stat::ALL {
                    p[st as usize] = evs.get(st.id()).and_then(|x| x.as_u64()).unwrap_or(0) as u16;
                }
                p
            });
            out.foe.push(Some(SetAssumption {
                item,
                ability,
                moves,
                nature: a["nature"]
                    .as_str()
                    .filter(|n| !n.is_empty())
                    .map(str::to_string),
                points,
            }));
        }
        Ok(out)
    }
}

/// A battle built from a player's view; see the module documentation.
pub struct World;

impl World {
    /// Build the battle that `view` describes, with `assumptions` for the
    /// foe's hidden sets. The battle waits for both players' move choices
    /// (a move request), with logging off. Fails only on a view the format
    /// can't hold (a species not in the dex).
    pub fn from_view(view: &Tracker, assumptions: &Assumptions) -> Result<Battle, String> {
        Builder::new(view, assumptions).build()
    }
}

/// The placeholder spread rule (66 points, at most 32 per stat), from the
/// moves and nature of a set:
///
/// - the attacking stat is Attack if the moves include at least as many
///   physical as special damaging moves, else Special Attack;
/// - no damaging move (support): 32 HP, 17 Defense, 17 Special Defense;
/// - a Speed-raising nature: 32 attacking stat, 32 Speed, 2 HP;
/// - a Speed-lowering nature: 32 HP, 32 attacking stat, 1 Defense, 1 Special Defense;
/// - otherwise: 32 HP, 32 attacking stat, 2 Speed.
pub fn placeholder_points(dex: &Dex, moves: &[MoveId], nature: Nature) -> [u16; 6] {
    let (mut physical, mut special) = (0, 0);
    for &m in moves {
        match dex.mv(m).category {
            Category::Physical => physical += 1,
            Category::Special => special += 1,
            Category::Status => {}
        }
    }
    let mut p = [0u16; 6];
    let (hp, atk, def, spa, spd, spe) = (0, 1, 2, 3, 4, 5);
    if physical == 0 && special == 0 {
        p[hp] = 32;
        p[def] = 17;
        p[spd] = 17;
        return p;
    }
    let attack = if physical >= special { atk } else { spa };
    if nature.plus == Some(Stat::Spe) {
        p[attack] = 32;
        p[spe] = 32;
        p[hp] = 2;
    } else if nature.minus == Some(Stat::Spe) {
        p[hp] = 32;
        p[attack] = 32;
        p[def] = 1;
        p[spd] = 1;
    } else {
        p[hp] = 32;
        p[attack] = 32;
        p[spe] = 2;
    }
    p
}

/// The nature assumed when nothing shows it.
pub const NEUTRAL_NATURE: &str = "Serious";

/// The default assumption for foe Pokémon `k` of `view`: what the view
/// knows of it, and otherwise no item, no ability (unless its species can
/// have only one), only its revealed moves, the sheet's nature or a neutral
/// one, and the placeholder spread over its known moves.
pub fn default_assumption(view: &Tracker, k: usize) -> SetAssumption {
    let dex = view.dex();
    let m = &view.sides[1 - view.me].mons[k];
    let moves: Vec<MoveId> = m.moves.iter().map(|mv| mv.id).collect();
    let nature = m.nature.clone().unwrap_or_else(|| NEUTRAL_NATURE.into());
    let points = placeholder_points(dex, &moves, dex.nature(&nature));
    SetAssumption {
        item: Some(match m.item {
            ItemView::Known(i) => Some(i),
            _ => None,
        }),
        ability: m
            .ability
            .or_else(|| species_of(dex, m).and_then(|sp| only_ability(dex, sp))),
        moves,
        nature: Some(nature),
        points: Some(points),
    }
}

/// The species' ability when it can have only one.
fn only_ability(dex: &Dex, sp: SpeciesId) -> Option<AbilityId> {
    let s = dex.species(sp);
    match s.abilities.as_slice() {
        [(_, a)] => dex.ability_id(a),
        _ => None,
    }
}

/// The species a view shows: as last shown, else the preview's (a hidden
/// forme "Urshifu-*" reads as its base species).
fn species_of(dex: &Dex, m: &MonView) -> Option<SpeciesId> {
    m.species.or_else(|| {
        dex.species_id(&m.preview)
            .or_else(|| dex.species_id(m.preview.trim_end_matches("-*")))
    })
}

/// A foe's HP from the percentage shown: the middle of the HP values that
/// show it (Showdown shows `max(1, floor(100 * hp / maxhp))`).
pub fn hp_from_percent(pct: u16, maxhp: u16) -> u16 {
    if pct == 0 {
        return 0;
    }
    if pct >= 100 {
        return maxhp;
    }
    let max = maxhp as u32;
    // The smallest hp with floor(100 hp / max) >= pct, and the largest with < pct + 1.
    let lo = if pct == 1 {
        1
    } else {
        (pct as u32 * max).div_ceil(100)
    };
    let hi = ((pct as u32 + 1) * max)
        .div_ceil(100)
        .saturating_sub(1)
        .max(lo);
    (((lo + hi) / 2).min(max.saturating_sub(1)).max(1)) as u16
}

struct Builder<'a> {
    view: &'a Tracker,
    asm: &'a Assumptions,
    dex: Arc<Dex>,
    /// Per side, view index -> position in the world's side.
    pos: [Vec<Option<usize>>; 2],
}

impl<'a> Builder<'a> {
    fn new(view: &'a Tracker, asm: &'a Assumptions) -> Builder<'a> {
        Builder {
            view,
            asm,
            dex: view.dex().clone(),
            pos: [Vec::new(), Vec::new()],
        }
    }

    /// Turns of a condition of `full` turns that started on `since` still
    /// left (at least 1; one that outlasted `full` had an extender:
    /// `extended` turns).
    fn left(&self, since: u32, full: u8, extended: u8) -> u8 {
        let elapsed = self.view.residuals_since(since) as i32;
        let l = full as i32 - elapsed;
        let l = if l <= 0 { extended as i32 - elapsed } else { l };
        l.max(1) as u8
    }

    fn build(mut self) -> Result<Battle, String> {
        let view = self.view;
        let me = view.me;
        let names = [view.sides[0].name.as_str(), view.sides[1].name.as_str()];
        let mut b = Battle::skeleton(self.dex.clone(), names, self.asm.seed);
        b.log_enabled = false;
        b.turn = view.turn;
        b.request = RequestState::Move;
        b.turn_state.last_move = view.last_move;
        // Which view Pokémon go where on each side.
        let orders = [self.order(0), self.order(1)];
        for (s, order) in orders.iter().enumerate() {
            let mut pos = vec![None; view.sides[s].mons.len()];
            for (p, &k) in order.iter().enumerate() {
                if let Some(k) = k {
                    pos[k] = Some(p);
                }
            }
            self.pos[s] = pos;
        }
        for (s, order) in orders.iter().enumerate() {
            let mut mons = Vec::new();
            for (p, &k) in order.iter().enumerate() {
                let Some(k) = k else { continue };
                mons.push(self.mon(s, k, p, s == me)?);
            }
            let side = &mut b.sides[s];
            side.mega_used = mons.iter().any(|m| m.mega_evolved);
            side.pokemon_left = mons.iter().filter(|m| !m.fainted).count();
            side.total_fainted = mons.iter().filter(|m| m.fainted).count() as u8;
            side.mons = mons;
        }
        self.field(&mut b);
        for s in 0..2 {
            self.side_conditions(&mut b, s);
        }
        // Trapping links (`trapper` on the holder of a `trapped`).
        for s in 0..2 {
            for i in 0..b.sides[s].mons.len() {
                if let Some(h) = b.sides[s].mons[i].vol.trapped {
                    if h.0 < 2 && h.1 < b.sides[h.0].mons.len() {
                        b.sides[h.0].mons[h.1].vol.trapper |= Battle::trap_bit((s, i));
                    }
                }
            }
        }
        // Ability order: the active Pokémon in turn, as if they came in so.
        b.vol_stamp = 8;
        let mut order = 0;
        for s in 0..2 {
            for i in 0..b.sides[s].mons.len() {
                if b.sides[s].mons[i].is_active {
                    order += 1;
                    b.sides[s].mons[i].ability_order = order;
                }
            }
        }
        b.refresh_speeds();
        for r in b.all_active() {
            b.run_trap_pokemon(r, false);
        }
        Ok(b)
    }

    /// The view Pokémon of side `s` in the world's order: position 0 and 1
    /// are the active slots (`None` when a slot has nobody to stand in it).
    fn order(&self, s: usize) -> Vec<Option<usize>> {
        let side = &self.view.sides[s];
        let own = s == self.view.me;
        let mut out: Vec<Option<usize>> = Vec::new();
        if own {
            // Request order.
            let mut ks: Vec<usize> = (0..side.mons.len())
                .filter(|&k| side.mons[k].team_pos.is_some())
                .collect();
            ks.sort_by_key(|&k| side.mons[k].team_pos);
            if ks.is_empty() {
                // Before the first request: the active ones, then the rest.
                ks = (0..side.mons.len()).collect();
                ks.sort_by_key(|&k| side.mons[k].slot.unwrap_or(9));
            }
            return ks.into_iter().map(Some).collect();
        }
        let brought: Vec<usize> = match &self.asm.brought {
            Some(b) => b.iter().copied().filter(|&k| k < side.mons.len()).collect(),
            None => {
                let size = if side.team_size > 0 {
                    side.team_size
                } else {
                    4
                };
                let mut v: Vec<usize> = (0..side.mons.len())
                    .filter(|&k| side.mons[k].seen || side.mons[k].slot.is_some())
                    .collect();
                for k in 0..side.mons.len() {
                    if v.len() >= size {
                        break;
                    }
                    if !v.contains(&k) {
                        v.push(k);
                    }
                }
                v
            }
        };
        let mut rest: Vec<usize> = brought.clone();
        for slot in 0..2 {
            let k = side
                .mons
                .iter()
                .enumerate()
                .position(|(k, m)| m.slot == Some(slot) && brought.contains(&k))
                .or_else(|| {
                    // An empty slot: a fainted Pokémon still stands in it.
                    rest.iter()
                        .copied()
                        .find(|&k| side.mons[k].fainted && side.mons[k].slot.is_none())
                });
            if let Some(k) = k {
                rest.retain(|&x| x != k);
            }
            out.push(k);
        }
        // An empty slot with nobody fainted to stand in it is left to the
        // next Pokémon, inactive.
        out.extend(rest.into_iter().map(Some));
        while out.len() > 2 && out[..2].contains(&None) {
            let i = out.iter().position(|x| x.is_none()).unwrap_or(0);
            if let Some(Some(k)) = out.pop() {
                out[i] = Some(k);
            }
        }
        out.retain(|x| x.is_some());
        out
    }

    /// The view Pokémon at a world reference.
    fn mref(&self, s: usize, k: usize) -> Option<MonRef> {
        self.pos[s].get(k).copied().flatten().map(|p| (s, p))
    }

    /// An active Pokémon of side `s` (slot a first), as a projection id.
    fn some_active(&self, s: usize) -> Option<String> {
        let side = &self.view.sides[s];
        (0..2)
            .find_map(|slot| side.mons.iter().position(|m| m.slot == Some(slot)))
            .and_then(|k| self.mref(s, k))
            .map(crate::projection::mref_id)
    }

    fn mon(&self, s: usize, k: usize, position: usize, own: bool) -> Result<Mon, String> {
        let dex = &self.dex;
        let view = self.view;
        let m = &view.sides[s].mons[k];
        let species = species_of(dex, m).ok_or_else(|| format!("unknown species {}", m.preview))?;
        // The assumption, completed by the default.
        let asm = if own {
            SetAssumption::default()
        } else {
            let given = self.asm.foe.get(k).cloned().flatten().unwrap_or_default();
            let def = default_assumption(view, k);
            SetAssumption {
                item: given.item.or(def.item),
                ability: given.ability.or(def.ability),
                moves: if given.moves.is_empty() {
                    def.moves
                } else {
                    given.moves
                },
                nature: given.nature.or(def.nature),
                points: given.points.or(def.points),
            }
        };
        let item = match m.item {
            ItemView::Known(i) => Some(i),
            ItemView::None => None,
            ItemView::Unknown => asm.item.flatten(),
        };
        let no_ability = dex.ability_id("noability").unwrap_or(0);
        let ability = m.ability.or(asm.ability).unwrap_or(no_ability);
        let mut moves: Vec<MoveId> = m.moves.iter().map(|mv| mv.id).collect();
        for &id in &asm.moves {
            if moves.len() >= 4 {
                break;
            }
            if !moves.contains(&id) {
                moves.push(id);
            }
        }
        let nature = asm.nature.clone().unwrap_or_default();
        let set = MonSet {
            nickname: if m.name.is_empty() {
                dex.species(species).base_species.clone()
            } else {
                m.name.clone()
            },
            species,
            item,
            ability,
            moves: moves.clone(),
            nature: nature.clone(),
            points: asm.points.unwrap_or([0; 6]),
            gender: m.gender.map(|g| g.to_string()).unwrap_or_default(),
            level: if m.level > 0 { m.level } else { 50 },
            shiny: false,
        };
        let mut mon = build_mon(dex, &set, position);
        mon.name = set.nickname.as_str().into();
        if mon.gender == "N" {
            mon.gender.clear();
        }
        // Base ability: the one before Trace or a Mega Evolution, when shown.
        mon.base_ability = m.base_ability.unwrap_or(ability);
        mon.mega_evolved = m.mega || dex.species(species).is_mega;
        if own {
            if let Some(st) = m.stats {
                mon.stats[1..].copy_from_slice(&st);
            }
            mon.maxhp = m.maxhp.max(1);
            mon.stats[0] = mon.maxhp;
            mon.hp = m.hp.min(mon.maxhp);
        } else {
            mon.hp = hp_from_percent(m.hp, mon.maxhp);
        }
        if m.fainted {
            mon.hp = 0;
            mon.fainted = true;
        }
        mon.speed = mon.stats[Stat::Spe as usize] as i32;
        mon.status = if m.fainted { Status::None } else { m.status };
        mon.status_state = match mon.status {
            Status::Slp | Status::Frz | Status::Tox | Status::Psn | Status::Brn | Status::Par => {
                mon.status
            }
            _ => Status::None,
        };
        if mon.status == Status::Slp {
            // Sleep's turns are secret: one more turn asleep at least.
            mon.status_start_time = 3;
            mon.status_time = 2;
        }
        if mon.status == Status::Tox {
            mon.toxic_stage = 1;
        }
        // Moves: one's own PP from the request, a foe's from its uses.
        for slot in mon.moves.iter_mut() {
            if let Some(v) = m.moves.iter().find(|mv| mv.id == slot.id) {
                slot.pp = match v.pp {
                    Some(pp) => pp,
                    None => slot.max_pp.saturating_sub(v.uses.min(255) as u8),
                };
                if let Some(mp) = v.maxpp {
                    slot.max_pp = mp;
                }
                slot.used = v.uses > 0;
            }
        }
        mon.last_move = m.last_move;
        // Kept off the field too: where its last move was aimed, the item it
        // used up, a berry eaten.
        mon.last_move_target_loc = m.last_move_target_loc;
        mon.last_item = m.last_item;
        mon.ate_berry = m.ate_berry;
        mon.is_active = m.slot.is_some() && m.slot == Some(position) || (position < 2 && m.fainted);
        if !mon.is_active || mon.fainted {
            // Off the field: nothing that lasts only while on it.
            return Ok(mon);
        }
        mon.active_turns = 1;
        mon.boosts = m.boosts;
        // Types: a type change, then the added type.
        if let Some(ts) = &m.types {
            mon.types = [
                ts.first().copied().unwrap_or(NO_TYPE),
                ts.get(1).copied().unwrap_or(NO_TYPE),
            ];
        }
        mon.added_type = m.added_type.unwrap_or(NO_TYPE);
        mon.hurt_this_turn = m.hurt_this_turn.then_some(1);
        mon.stats_raised_this_turn = m.stats_raised_this_turn;
        mon.stats_lowered_this_turn = m.stats_lowered_this_turn;
        mon.move_this_turn_result = m.move_this_turn;
        mon.move_last_turn_result = m.move_last_turn;
        mon.times_attacked = m.times_attacked;
        // The hits taken: a foe's damage is a share of its HP.
        mon.attacked_by = m
            .attacked_by
            .iter()
            .map(|a| Attacker {
                side: a.side as u8,
                name: a.name.as_str().into(),
                damage: if own {
                    a.damage as u32
                } else {
                    (a.damage as u32 * mon.maxhp as u32 + 50) / 100
                },
                mv: a.mv,
                this_turn: a.this_turn,
                slot: (a.side as u8, a.slot as u8),
                numbered: true,
            })
            .collect();
        if own {
            mon.illusion = m.shown_as.clone();
        }
        for v in &m.volatiles {
            if let Some(n) = v.strip_prefix("fallen").and_then(|n| n.parse::<u8>().ok()) {
                mon.ability_state.fallen = Some(n);
            }
        }
        if !m.volatiles.is_empty()
            || m.locked.is_some()
            || m.ally_switch.is_some()
            || m.stall.is_some()
        {
            let vols = self.volatiles_json(s, m, &mon);
            let stamp = mon.vol.added;
            mon.vol = crate::projection::projected_volatiles(dex, &vols);
            mon.vol.added = stamp;
        }
        if let Some((_, _)) = m.locked {
            // The request shows one's own lock; a foe's needs its move.
            if mon.vol.locked_move.is_none() {
                mon.vol.locked_move = m.last_move;
            }
        }
        Ok(mon)
    }

    /// The view's volatiles as a projection's `volatiles` object.
    fn volatiles_json(&self, s: usize, m: &MonView, mon: &Mon) -> Value {
        let dex = &self.dex;
        let mut out = Map::new();
        let since = |id: &str| m.volatile_since(id);
        // Taunt, Encore and Disable: the length the view worked out, if any.
        let left = |id: &str, full: u8| {
            let full = m.volatile_length(id).unwrap_or(full);
            since(id).map(|t| self.left(t, full, full)).unwrap_or(full)
        };
        let source = |vol: &str| {
            m.trap_sources
                .iter()
                .find(|(v, _)| v == vol)
                .and_then(|(_, (hs, hk, _))| self.mref(*hs, *hk))
                .map(crate::projection::mref_id)
        };
        let foe_active = self.some_active(1 - s);
        let last = m.last_move.map(|id| dex.mv(id).id.clone());
        for v in &m.volatiles {
            let id = v.as_str();
            let entry = match id {
                "confusion" => json!({"time": 2}),
                "encore" => match &last {
                    Some(mv) => json!({"duration": left(id, 3), "move": mv}),
                    None => continue,
                },
                "disable" => match &last {
                    Some(mv) => json!({"duration": left(id, 4), "move": mv}),
                    None => continue,
                },
                "taunt" => json!({"duration": left(id, 3)}),
                "yawn" => json!({"duration": left(id, 2)}),
                "throatchop" => json!({"duration": left(id, 2)}),
                "healblock" => json!({"duration": left(id, 2)}),
                "magnetrise" => json!({"duration": left(id, 5)}),
                "uproar" => json!({"duration": left(id, 3)}),
                "syrupbomb" => json!({"duration": left(id, 4), "source": foe_active}),
                "partiallytrapped" => {
                    let mut t = json!({
                        "duration": left(id, 5),
                        "boundDivisor": if m.bound_band { 6 } else { 8 },
                    });
                    if let Some(src) = source(id) {
                        t["source"] = json!(src);
                    }
                    t
                }
                "trapped" => match source(id) {
                    Some(src) => json!({"linkedStatus": "trapper", "linked": [src]}),
                    None => json!({"linkedStatus": "trapper", "linked": []}),
                },
                "octolock" => json!({"source": source(id).or(foe_active.clone())}),
                "attract" => json!({"source": foe_active}),
                "leechseed" => json!({"sourceSlot": format!("p{}a", 2 - s)}),
                "substitute" => json!({"hp": (mon.maxhp / 4).max(1)}),
                "mustrecharge" => json!({"duration": 1}),
                "dragoncheer" => {
                    let dragon = dex
                        .type_id("Dragon")
                        .is_some_and(|d| mon.types.contains(&d));
                    json!({"hasDragonType": dragon})
                }
                "twoturnmove" => {
                    // The charging move: its known move with the `charge` flag.
                    let charge = m
                        .moves
                        .iter()
                        .map(|mv| mv.id)
                        .find(|&id| dex.mv(id).has(MoveFlag::Charge));
                    if let Some(c) = charge {
                        let cid = dex.mv(c).id.clone();
                        out.insert(cid, json!({"duration": 2, "targetLoc": 0}));
                        json!({"duration": 2, "move": dex.mv(c).id})
                    } else {
                        continue;
                    }
                }
                "flashfire" | "imprison" | "glaiverush" | "focusenergy" | "charge" | "curse"
                | "aquaring" | "ingrain" | "saltcure" | "smackdown" | "gastroacid" | "torment"
                | "noretreat" | "powertrick" | "minimize" | "endure" | "destinybond" => json!({}),
                _ => {
                    if let Some(n) = id.strip_prefix("perish").and_then(|n| n.parse::<u8>().ok()) {
                        // "perish3" holds from the move to the first residual (duration 4).
                        let fresh =
                            n == 3 && since(id) == Some(self.view.turn) && !self.view.upkeep_done;
                        out.insert(
                            "perishsong".into(),
                            json!({"duration": if fresh { 4 } else { n }}),
                        );
                        continue;
                    }
                    if let Some(n) = id
                        .strip_prefix("stockpile")
                        .and_then(|n| n.parse::<u8>().ok())
                    {
                        out.insert("stockpile".into(), json!({"layers": n, "def": n, "spd": n}));
                        continue;
                    }
                    continue;
                }
            };
            out.insert(id.to_string(), entry);
        }
        if let (Some((uses, _)), Some(mv)) = (m.locked, &last) {
            let left = 3u8.saturating_sub(uses).max(1);
            out.insert(
                "lockedmove".into(),
                json!({"duration": left, "trueDuration": left, "move": mv}),
            );
        }
        if let Some((counter, duration)) = m.stall {
            out.insert(
                "stall".into(),
                json!({"counter": counter, "duration": duration}),
            );
        }
        if let Some((counter, turn)) = m.ally_switch {
            let fresh = turn == self.view.turn && !self.view.upkeep_done;
            out.insert(
                "allyswitch".into(),
                json!({"counter": counter, "duration": if fresh { 2 } else { 1 }}),
            );
        }
        Value::Object(out)
    }

    fn field(&self, b: &mut Battle) {
        let f = &self.view.field;
        let dex = b.dex.clone();
        let dur = |id: &str, default: u8| dex.cond_num(id, "duration").map_or(default, |x| x as u8);
        // An extending item someone on the field holds (or is assumed to) makes 8 turns.
        let held = |b: &Battle, item: &str| {
            b.sides
                .iter()
                .flat_map(|s| s.mons.iter())
                .any(|m| m.item.is_some_and(|i| b.dex.item(i).id == item))
        };
        if let Some((w, since)) = f.weather {
            let full = if held(b, w.rock()) { 8 } else { dur(w.id(), 5) };
            b.field.weather = Some(w);
            b.field.weather_turns = self.left(since, full, 8);
        }
        if let Some((t, since)) = f.terrain {
            let full = if held(b, "terrainextender") {
                8
            } else {
                dur(t.id(), 5)
            };
            b.field.terrain = Some(t);
            b.field.terrain_turns = self.left(since, full, 8);
        }
        if let Some(since) = f.trick_room {
            b.field.trick_room = self.left(since, dur("trickroom", 5), 5);
        }
        if let Some(since) = f.gravity {
            b.field.gravity = self.left(since, dur("gravity", 5), 5);
        }
        if let Some(since) = f.magic_room {
            b.field.magic_room = self.left(since, dur("magicroom", 5), 5);
        }
        if let Some(since) = f.wonder_room {
            b.field.wonder_room = self.left(since, dur("wonderroom", 5), 5);
        }
        if let Some(since) = f.fairy_lock {
            b.field.fairy_lock = self.left(since, dur("fairylock", 2), 2);
        }
    }

    fn side_conditions(&self, b: &mut Battle, s: usize) {
        let sv: &SideView = &self.view.sides[s];
        for &(c, since) in &sv.conditions {
            let full = match c {
                SideCond::Tailwind => b.cond_u8(c.id(), "duration", 4),
                SideCond::WideGuard | SideCond::QuickGuard => b.cond_u8(c.id(), "duration", 1),
                _ => b.cond_u8(c.id(), "duration", 5),
            };
            // Screens last 8 turns with a Light Clay (held, or assumed, on the side).
            let screen = matches!(
                c,
                SideCond::Reflect | SideCond::LightScreen | SideCond::AuroraVeil
            );
            let clay = b.sides[s]
                .mons
                .iter()
                .any(|m| m.item.is_some_and(|i| b.dex.item(i).id == "lightclay"));
            let (full, extended) = match (screen, clay) {
                (true, true) => (8, 8),
                (true, false) => (full, 8),
                _ => (full, full),
            };
            *b.sides[s].conds.get_mut(c) = self.left(since, full, extended);
            b.sides[s].conds.stamp(c);
        }
        let conds = &mut b.sides[s].conds;
        if sv.toxic_spikes > 0 {
            conds.toxic_spikes = sv.toxic_spikes;
            conds.next_added += 1;
            conds.toxic_spikes_added = conds.next_added;
        }
        if sv.stealth_rock {
            conds.stealth_rock = true;
            conds.next_added += 1;
            conds.stealth_rock_added = conds.next_added;
        }
        let extras = &mut b.side_extras[s];
        if sv.spikes > 0 {
            extras.spikes = sv.spikes;
            conds.next_added += 1;
            extras.spikes_added = conds.next_added;
        }
        if sv.sticky_web {
            extras.sticky_web = true;
            conds.next_added += 1;
            extras.sticky_web_added = conds.next_added;
        }
        if let Some(since) = sv.safeguard {
            extras.safeguard = self.left(since, 5, 5);
        }
        // Slot conditions: the Pokémon in the slot stands in for whoever set them.
        for slot in 0..2 {
            let v = &sv.slots[slot];
            let occupant = b.active(s, slot);
            let c = &mut b.slot_conds[s][slot];
            if let Some(turn) = v.wish {
                let (hp, name) = occupant
                    .map(|r| {
                        let m = &b.sides[r.0].mons[r.1];
                        (m.maxhp as f64 / 2.0, m.name.to_string())
                    })
                    .unwrap_or((1.0, String::new()));
                c.wish = Some(WishState {
                    hp,
                    starting_turn: turn,
                    source_slot: slot,
                    wisher: name,
                });
            }
            if v.healing_wish {
                c.healing_wish = Some(slot);
            }
            if let Some(turn) = v.future {
                let source = (0..2)
                    .find_map(|i| {
                        let side = &b.sides[1 - s];
                        (i < side.mons.len() && side.mons[i].is_active).then_some((1 - s, i))
                    })
                    .unwrap_or((1 - s, 0));
                c.future = Some(FutureState {
                    ending_turn: turn.saturating_sub(1) + 2,
                    source,
                });
            }
        }
    }
}

/// Maximum PP of a move under the format's rule.
pub fn max_pp(dex: &Dex, id: MoveId) -> u8 {
    champions_max_pp(dex.mv(id).pp, dex.mv(id).no_pp_boosts)
}

/// A set's stats under the format's rule (Champions stat points).
pub fn set_stats(dex: &Dex, species: SpeciesId, points: &[u16; 6], nature: &str) -> [u16; 6] {
    champions_stats(&dex.species(species).base_stats, points, dex.nature(nature))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hp_from_percent_is_consistent() {
        for maxhp in [1u16, 7, 99, 100, 150, 207, 404] {
            for hp in 1..=maxhp {
                let pct = (100 * hp as u32 / maxhp as u32).max(1) as u16;
                let guess = hp_from_percent(pct, maxhp);
                let back = (100 * guess as u32 / maxhp as u32).max(1) as u16;
                assert_eq!(back, pct, "maxhp {maxhp} hp {hp}");
            }
        }
    }
}
