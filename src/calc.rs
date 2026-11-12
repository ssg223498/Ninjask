//! Exact damage, accuracy, KO chances and turn order, read-only.
//!
//! What an expert computes in their head, from the engine's own code: the
//! damage of a move is `getDamage`'s two phases (`Battle::damage_base`
//! before the random roll, `Battle::damage_mods` after it), run for each
//! of the 16 rolls with and without a critical hit, so the calculation and
//! the battle can't drift apart. Everything the engine models applies:
//! abilities, items, weather and terrain, screens, a pending Helping Hand,
//! boosts, burns, the spread reduction when a spread move has two targets,
//! Parental Bond's second hit, multi-hit moves.
//!
//! The functions take `&mut Battle` for the engine's handlers but leave the
//! battle as they found it: they run with the battle in *probe* mode, where
//! nothing is drawn for an outcome (Fickle Beam takes its usual power), no
//! berry is eaten (its effect still applies), nothing is logged, and the RNG
//! and every state they touch are restored. The `&self` forms work on a copy.
//!
//! The probabilities assume the battle as it stands: the move used now, on
//! this target, with nothing else happening first.
//!
//! - **KO this turn**: the move hits (its accuracy) and its total damage
//!   reaches the target's current HP, critical hits included. A Sturdy or
//!   Focus Sash holder at full HP (or one under Endure) survives the first
//!   hit, a Focus Band holder one time in ten, and an intact Disguise takes
//!   the first hit.
//! - **2HKO**: two uses of the move, each landing with its accuracy,
//!   without anything in between: no end-of-turn damage or healing, no stat
//!   changes, the same field.

use std::borrow::Cow;

use crate::battle::{Battle, MonRef};
use crate::dex::types::*;
use crate::dex::Move;
use crate::kinds::{AbilityKind, ItemKind, MoveKind};
use crate::moves::damage::{modify_damage, DamageMods, CRIT_ODDS};

/// The damage of one hit for each of the 16 random rolls (`roll` 0 is the
/// highest: damage x 100/100, down to x 85/100), without and with a
/// critical hit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HitRolls {
    pub normal: [u32; 16],
    pub crit: [u32; 16],
}

/// A damaging hit recorded in play with its predicted distribution
/// (`Battle::damage_samples`), to test the calculation.
#[derive(Clone, Debug)]
pub struct DamageSample {
    pub attacker: MonRef,
    pub target: MonRef,
    pub move_id: MoveId,
    /// The hit (1 for the first; Parental Bond, multi-hit moves).
    pub hit: u8,
    pub spread: bool,
    pub crit_chance: f64,
    pub rolls: HitRolls,
    /// The target's HP before the hit.
    pub target_hp: u32,
    /// The damage `getDamage` returned, and whether it was a critical hit.
    pub damage: u32,
    pub crit: bool,
    /// For a first hit: the spread flag and first-hit rolls of the full
    /// calculation ([`Battle::calc`]) on the same state, which rebuilds the
    /// move from its id.
    pub full: Option<(bool, HitRolls)>,
    /// For a first hit of a single-hit move: the full calculation's chance
    /// that the hit KOs (its `ko` over its accuracy: Sturdy, Focus Sash and
    /// the like included).
    pub full_ko: Option<f64>,
    /// The target's HP after the hit was applied (Sturdy, Focus Sash...).
    pub hp_after: Option<u32>,
}

/// One move of one Pokémon against one target.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DamageCalc {
    /// A damaging move (not a status move).
    pub damaging: bool,
    /// The target is immune: its type, an ability (Levitate, Volt Absorb,
    /// Soundproof...), a powder move on a Grass type, a Prankster move on a
    /// Dark type, a one-hit KO on a higher level.
    pub immune: bool,
    /// The chance the move hits (1 when it can't miss).
    pub accuracy: f64,
    /// The chance of a critical hit, per hit.
    pub crit_chance: f64,
    /// The spread reduction applies (two targets or more).
    pub spread: bool,
    /// The chance of each number of hits: `hit_counts[i]` for `i + 1` hits.
    pub hit_counts: Vec<f64>,
    /// The rolls of each hit (`hits[i]` for hit `i + 1`; Parental Bond's
    /// second hit and Triple Axel's differ). Fixed damage has 16 equal rolls.
    pub hits: Vec<HitRolls>,
    /// Damage that doesn't depend on the rolls (Night Shade, Super Fang...).
    pub fixed: Option<u32>,
    pub target_hp: u32,
    pub target_maxhp: u32,
    /// Expected total damage when the move hits.
    pub expected: f64,
    /// The chance to KO this turn (accuracy included).
    pub ko: f64,
    /// The chance to KO within two uses (accuracy included).
    pub ko2: f64,
}

impl DamageCalc {
    /// The smallest and largest total damage when the move hits.
    pub fn range(&self) -> (u32, u32) {
        let n = self.hit_counts.len();
        let lo_hits = self.hit_counts.iter().position(|&p| p > 0.0).unwrap_or(0) + 1;
        let lo = self.hits.iter().take(lo_hits).map(|h| h.normal[15]).sum();
        let hi = self
            .hits
            .iter()
            .take(n)
            .map(|h| h.crit[0].max(h.normal[0]))
            .sum();
        (lo, hi)
    }
}

/// What a Pokémon plans to do, for the turn order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// Use this move.
    Move(MoveId),
    /// A damaging move of this priority (to compare speeds).
    Priority(i8),
    Switch,
}

/// The chance that one action goes before another.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OrderOdds {
    /// The first goes first (a speed tie counts half).
    pub first: f64,
    /// The order is a coin flip between equal speeds.
    pub tie: f64,
}

/// How a target can survive the first hit of a move.
#[derive(Clone, Copy, Debug, Default)]
struct Survival {
    /// Sturdy or Focus Sash at full HP, or Endure this turn: the hit leaves 1 HP.
    full_hp: bool,
    /// Focus Band: one time in ten the hit leaves 1 HP.
    band: bool,
    /// An intact Disguise takes the hit.
    disguise: bool,
}

/// Quick Draw's and Quick Claw's chances (in `run_fractional_priority`).
const QUICK_DRAW: (u32, u32) = (3, 10);
const QUICK_CLAW: (u32, u32) = (1, 5);

/// What [`Battle::calc`] saves on entry and restores on exit (see the
/// module documentation): the RNG, the scratch fields a calculation
/// touches, and a Stance Change user's forme.
struct CalcSave {
    rng: crate::rng::Rng,
    flags: (Option<MonRef>, u8, bool, bool, bool, Option<MonRef>),
    user: Option<crate::state::Mon>,
}

/// A move as `use_move` modifies it for one user, ready to calculate
/// against any target: the steps of the preparation that don't look at
/// the target, run once however many targets follow. Shell Side Arm's
/// step does look at its target, so a preparation made without one is
/// only shared for other moves (see [`Battle::calc_all`]).
struct PreparedMove {
    kind: MoveKind,
    /// The modified move, when a step changed it; else the dex's own.
    mv: Option<Move>,
    move_id: MoveId,
    move_type: TypeId,
    type_changed: bool,
}

impl Battle {
    /// The calculation for `attacker`'s move `move_id` against `target`,
    /// on a copy of the battle.
    pub fn damage_calc(&self, attacker: MonRef, move_id: MoveId, target: MonRef) -> DamageCalc {
        let mut copy = self.clone();
        copy.damage_samples = None;
        copy.calc(attacker, move_id, target)
    }

    /// A copy of the battle in which each of `refs` that is not in one of
    /// its side's active slots (a benched Pokémon) is in the side's first
    /// slot, as if it had switched in with nothing happening on the way in
    /// (no entry hazards or switch-in abilities): the Pokémon it displaces
    /// goes to its place on the bench, with what refers to either following
    /// them (Showdown `switchIn` swaps the two in `side.pokemon`), and it
    /// counts with no boosts. The field, the weather and the side
    /// conditions are the battle's own. Returns the copy and where each of
    /// `refs` is in it. A side can have only one Pokémon of `refs` there
    /// when one of them is benched (they can't share the slot); an index
    /// past the party is an error too.
    pub fn with_in_slot(&self, refs: &[MonRef]) -> Result<(Battle, Vec<MonRef>), String> {
        for &r in refs {
            self.in_active_slot(r)?;
        }
        let mut view = self.clone();
        view.damage_samples = None;
        let mut out = refs.to_vec();
        for s in 0..self.sides.len() {
            let n_active = self.sides[s].n_active;
            let Some(&(_, k)) = refs.iter().find(|&&(rs, i)| rs == s && i >= n_active) else {
                continue;
            };
            if refs.iter().any(|&(rs, i)| rs == s && i != k) {
                return Err(format!(
                    "two Pokémon of side {s}, one of them benched: they can't share its slot"
                ));
            }
            let flip = |i: usize| match i {
                0 => k,
                i if i == k => 0,
                i => i,
            };
            view.swap_refs(s, flip);
            let side = &mut view.sides[s];
            side.mons.swap(0, k);
            side.mons[0].position = 0;
            side.mons[k].position = k;
            side.mons[k].is_active = false;
            let m = &mut side.mons[0];
            m.is_active = true;
            m.boosts = [0; 7];
            for r in out.iter_mut().filter(|r| r.0 == s) {
                r.1 = 0;
            }
        }
        Ok((view, out))
    }

    /// [`Battle::damage_calc`] for any Pokémon of the two parties, benched
    /// ones counting as in their side's slot ([`Battle::with_in_slot`]).
    /// For two Pokémon in active slots, exactly `damage_calc`.
    pub fn damage_calc_party(
        &self,
        attacker: MonRef,
        move_id: MoveId,
        target: MonRef,
    ) -> Result<DamageCalc, String> {
        if self.in_active_slot(attacker)? && self.in_active_slot(target)? {
            return Ok(self.damage_calc(attacker, move_id, target));
        }
        let (mut view, r) = self.with_in_slot(&[attacker, target])?;
        Ok(view.calc(r[0], move_id, r[1]))
    }

    /// [`Battle::order_odds`] for any Pokémon of the two parties, benched
    /// ones counting as in their side's slot ([`Battle::with_in_slot`]).
    /// For two Pokémon in active slots, exactly `order_odds`.
    pub fn order_odds_party(
        &self,
        a: MonRef,
        pa: Plan,
        b: MonRef,
        pb: Plan,
    ) -> Result<OrderOdds, String> {
        if self.in_active_slot(a)? && self.in_active_slot(b)? {
            return Ok(self.order_odds(a, pa, b, pb));
        }
        let (view, r) = self.with_in_slot(&[a, b])?;
        Ok(view.order_odds(r[0], pa, r[1], pb))
    }

    /// Whether `r` is in one of its side's active slots (the Pokémon there
    /// may have fainted); an error when there is no such Pokémon.
    pub(crate) fn in_active_slot(&self, (s, i): MonRef) -> Result<bool, String> {
        if s >= self.sides.len() || i >= self.sides[s].mons.len() {
            return Err(format!("no Pokémon {i} on side {s}"));
        }
        Ok(i < self.sides[s].n_active)
    }

    /// Enter probe mode and save what a calculation touches.
    fn calc_begin(&mut self, r: MonRef) -> CalcSave {
        let save = CalcSave {
            rng: self.rng.clone(),
            flags: (
                self.ability_breaker,
                self.move_hit,
                self.log_enabled,
                self.probe,
                self.active_move.pbond,
                self.active_move.ignore_evasion,
            ),
            // Stance Change turns the user into its Blade forme: restored after.
            user: (self.ability_kind(r) == AbilityKind::StanceChange).then(|| self.mon(r).clone()),
        };
        self.probe = true;
        self.log_enabled = false;
        self.ability_breaker = None;
        save
    }

    /// Restore what [`Battle::calc_begin`] saved.
    fn calc_end(&mut self, r: MonRef, save: CalcSave) {
        if let Some(m) = save.user {
            self.sides[r.0].mons[r.1] = m;
        }
        (
            self.ability_breaker,
            self.move_hit,
            self.log_enabled,
            self.probe,
            self.active_move.pbond,
            self.active_move.ignore_evasion,
        ) = save.flags;
        self.rng = save.rng;
    }

    /// The calculation for `attacker`'s move `move_id` against `target`.
    /// The battle is left as it was (see the module documentation).
    pub fn calc(&mut self, attacker: MonRef, move_id: MoveId, target: MonRef) -> DamageCalc {
        let save = self.calc_begin(attacker);
        let prep = self.prepare_move(attacker, move_id, Some(target));
        let out = self.calc_prepared(attacker, &prep, target);
        self.calc_end(attacker, save);
        out
    }

    /// The calculation for `attacker`'s move `move_id` against each of
    /// `targets`, in order: the same results as [`Battle::calc`] per
    /// target, with the target-independent preparation (the modified
    /// move and its type) run once instead of once per target. Shell
    /// Side Arm, whose preparation reads its target, is calculated per
    /// target exactly as `calc` does.
    pub fn calc_all(
        &mut self,
        attacker: MonRef,
        move_id: MoveId,
        targets: &[MonRef],
    ) -> Vec<DamageCalc> {
        if self.dex.move_kind[move_id as usize] == MoveKind::ShellSideArm {
            return targets
                .iter()
                .map(|&t| self.calc(attacker, move_id, t))
                .collect();
        }
        let save = self.calc_begin(attacker);
        let prep = self.prepare_move(attacker, move_id, None);
        let out = targets
            .iter()
            .map(|&t| self.calc_prepared(attacker, &prep, t))
            .collect();
        self.calc_end(attacker, save);
        out
    }

    /// The move as `use_move` modifies it, with the same handlers, and
    /// its type after `ModifyMove`. `ssa_target` is Shell Side Arm's
    /// target; without one that step is skipped (its caller then only
    /// shares the preparation for moves it doesn't apply to).
    fn prepare_move(
        &mut self,
        r: MonRef,
        move_id: MoveId,
        ssa_target: Option<MonRef>,
    ) -> PreparedMove {
        let dex = self.dex.clone();
        let kind = dex.move_kind[move_id as usize];
        let mut mv: Cow<Move> = Cow::Borrowed(dex.mv(move_id));
        if let Some(m) = self.long_reach_on_modify_move(r, &mv) {
            mv = Cow::Owned(m);
        }
        let (move_type, type_changed) = self.move_type(r, &mv, kind);
        if kind == MoveKind::ShellSideArm {
            if let Some(t) = ssa_target {
                if let Some(m) = self.shell_side_arm_on_modify_move(r, Some(t), &mv) {
                    mv = Cow::Owned(m);
                }
            }
        }
        if let Some(m) = self.wonder_room_on_modify_move(&mv) {
            mv = Cow::Owned(m);
        }
        self.run_ability_modify_move(r, &mv);
        if let Some(m) = self.sheer_force_on_modify_move(r, &mv) {
            mv = Cow::Owned(m);
        }
        // Fling's `onPrepareHit` (after `ModifyMove`) takes its power from
        // the held item; only the power, as the rest changes the state.
        if kind == MoveKind::Fling {
            if let Some((_, power)) = self.fling_item(r) {
                let mut m = mv.into_owned();
                m.base_power = power;
                mv = Cow::Owned(m);
            }
        }
        PreparedMove {
            kind,
            mv: match mv {
                Cow::Owned(m) => Some(m),
                Cow::Borrowed(_) => None,
            },
            move_id,
            move_type,
            type_changed,
        }
    }

    fn calc_prepared(&mut self, r: MonRef, prep: &PreparedMove, t: MonRef) -> DamageCalc {
        let dex = self.dex.clone();
        let kind = prep.kind;
        let mut mv: Cow<Move> = match &prep.mv {
            Some(m) => Cow::Borrowed(m),
            None => Cow::Borrowed(dex.mv(prep.move_id)),
        };
        let (move_type, type_changed) = (prep.move_type, prep.type_changed);
        let mut out = DamageCalc {
            target_hp: self.mon(t).hp as u32,
            target_maxhp: self.mon(t).maxhp as u32,
            ..Default::default()
        };
        out.damaging = mv.category != Category::Status;
        out.immune = t != r && self.move_immune(r, t, &mv, move_type);
        out.accuracy = self
            .move_accuracy(r, t, &mv)
            .map_or(1.0, |a| a.min(100) as f64 / 100.0);
        if !out.damaging || out.immune || self.mon(t).hp == 0 {
            return out;
        }
        let target_type = self
            .modify_move_target_type(r, &mv, Some(t))
            .unwrap_or_else(|| self.move_target_type(r, &mv));
        out.spread = matches!(target_type, Target::AllAdjacent | Target::AllAdjacentFoes)
            && self.spread_target_count(r, target_type) > 1;
        if let Some(m) = self.parental_bond_on_prepare_hit(r, &mv, out.spread) {
            mv = Cow::Owned(m);
        }
        out.hit_counts = self.hit_count_odds(r, &mv, kind, out.accuracy);
        let max_hits = out.hit_counts.len();
        if let Some(d) = self.fixed_damage(r, t, &mv, kind) {
            out.fixed = Some(d);
            out.hits = vec![
                HitRolls {
                    normal: [d; 16],
                    crit: [d; 16],
                };
                max_hits
            ];
        } else {
            // Triple Axel's power and Parental Bond's second hit change by hit.
            let per_hit = kind == MoveKind::TripleAxel || self.active_move.pbond;
            for h in 1..=max_hits {
                if h == 1 || per_hit {
                    self.move_hit = h as u8;
                    let (pc, rolls) =
                        self.hit_rolls(r, t, &mv, kind, move_type, type_changed, out.spread);
                    out.crit_chance = pc;
                    out.hits.push(rolls);
                } else {
                    let first = out.hits[0];
                    out.hits.push(first);
                }
            }
        }
        let survival = self.first_hit_survival(t, &mv);
        out.finish(survival);
        out
    }

    /// One hit's damage for each roll, with and without a critical hit, in
    /// the battle as it stands (the hit number, Parental Bond and the like
    /// as set), and the critical-hit chance. The battle is left as it was.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn hit_rolls(
        &mut self,
        r: MonRef,
        t: MonRef,
        mv: &Move,
        kind: MoveKind,
        move_type: TypeId,
        type_changed: bool,
        spread: bool,
    ) -> (f64, HitRolls) {
        let saved_rng = self.rng.clone();
        let saved = (self.probe, self.log_enabled, self.mon(t).ability_state);
        self.probe = true;
        self.log_enabled = false;
        let crit_chance = self.crit_chance(r, t, mv);
        let mut rolls = HitRolls::default();
        let base = self.damage_base(r, t, mv, kind, move_type, type_changed, false);
        let mut mods = self.damage_mods(r, t, mv, kind, move_type, false, spread, 0);
        // A critical hit changes the base damage only when it drops a stage
        // (a lowered attack, a raised defense), and the modifiers only
        // through Sniper and the target's screens: otherwise reuse them.
        let base_crit = if self.crit_drops_stages(r, t, mv) {
            self.damage_base(r, t, mv, kind, move_type, type_changed, true)
        } else {
            base
        };
        let screens = {
            let c = &self.sides[t.0].conds;
            c.reflect > 0 || c.light_screen > 0 || c.aurora_veil > 0
        };
        let mut mods_crit = if screens || self.has_ability(r, AbilityKind::Sniper) {
            self.damage_mods(r, t, mv, kind, move_type, true, spread, 0)
        } else {
            DamageMods { crit: true, ..mods }
        };
        for roll in 0..16 {
            mods.roll = roll as u32;
            mods_crit.roll = roll as u32;
            rolls.normal[roll] = modify_damage(base, &mods);
            rolls.crit[roll] = modify_damage(base_crit, &mods_crit);
        }
        (self.probe, self.log_enabled) = (saved.0, saved.1);
        self.sides[t.0].mons[t.1].ability_state = saved.2;
        self.rng = saved_rng;
        (crit_chance, rolls)
    }

    /// Whether a critical hit ignores a stage `damage_base` would use: the
    /// attacker's lowered attack or the target's raised defense.
    fn crit_drops_stages(&self, r: MonRef, t: MonRef, mv: &Move) -> bool {
        let physical = mv.category == Category::Physical;
        let atk_stat =
            mv.override_offensive_stat
                .unwrap_or(if physical { Stat::Atk } else { Stat::Spa });
        let def_stat =
            mv.override_defensive_stat
                .unwrap_or(if physical { Stat::Def } else { Stat::Spd });
        let a = if mv.offensive_pokemon_is_target { t } else { r };
        self.mon(a).boosts[Boost::of_stat(atk_stat) as usize] < 0
            || (!mv.ignore_defensive && self.mon(t).boosts[Boost::of_stat(def_stat) as usize] > 0)
    }

    /// The chance of a critical hit (`getDamage`'s crit roll).
    pub fn crit_chance(&self, r: MonRef, t: MonRef, mv: &Move) -> f64 {
        if self.crit_prevented(r, t, mv) {
            return 0.0;
        }
        if mv.will_crit {
            return 1.0;
        }
        let ratio = self.run_modify_crit_ratio(r, t, mv.crit_ratio).clamp(0, 4);
        let (n, d) = CRIT_ODDS[ratio as usize];
        n as f64 / d as f64
    }

    /// Whether `t` is immune to `r`'s move, by the steps `use_move` takes
    /// before the accuracy roll: the target's `TryHit` abilities, type
    /// immunity (status moves aside; Scrappy), powder moves on Grass types,
    /// Prankster moves on Dark foes, the move's own immunities, and a
    /// one-hit KO on a higher level or an immune type. Protection, a
    /// semi-invulnerable target and Psychic Terrain are not counted.
    pub fn move_immune(&self, r: MonRef, t: MonRef, mv: &Move, move_type: TypeId) -> bool {
        if self.ability_try_hit_blocks(r, t, mv, move_type) {
            return true;
        }
        let checks_immunity = (mv.category != Category::Status
            || mv.ignore_immunity == Some(false))
            && !self.ignores_type_immunity(r, move_type);
        if checks_immunity && self.type_immunity(t, move_type).is_some() {
            return true;
        }
        let ty = self.types_of(t);
        let immune_to = |effect: &str| {
            ty.iter()
                .any(|&x| self.dex.type_immune_to_effect(x, effect))
        };
        if mv.has(MoveFlag::Powder) && immune_to("powder") {
            return true;
        }
        if self.prankster_boosted(r, mv) && t.0 != r.0 && immune_to("prankster") {
            return true;
        }
        if self.has_try_immunity(mv) && self.run_try_immunity(t, r, mv) {
            return true;
        }
        if let Some(immune_type) = mv.ohko {
            let (lu, lt) = (self.mon(r).level, self.mon(t).level);
            if lu < lt || immune_type.is_some_and(|ty| self.has_type_of(t, ty)) {
                return true;
            }
        }
        false
    }

    /// How many Pokémon a spread move of `target` type from `r` would hit.
    fn spread_target_count(&self, r: MonRef, target: Target) -> usize {
        self.all_active()
            .into_iter()
            .filter(|&x| x != r && self.mon(x).hp > 0)
            .filter(|&x| target == Target::AllAdjacent || x.0 != r.0)
            .count()
    }

    /// The chance of each number of hits (index `i` for `i + 1` hits): 2-5
    /// hits at 35/35/15/15%, Skill Link's 5, a fixed count, and for Triple
    /// Axel and Population Bomb each hit after the first landing with the
    /// move's accuracy.
    fn hit_count_odds(&self, r: MonRef, mv: &Move, kind: MoveKind, accuracy: f64) -> Vec<f64> {
        let mut odds = match self.hit_count(r, mv) {
            None => vec![1.0],
            Some((2, 5)) => vec![0.0, 0.35, 0.35, 0.15, 0.15],
            Some((lo, hi)) => {
                let (lo, hi) = (lo.max(1) as usize, hi.max(lo).max(1) as usize);
                let n = (hi - lo + 1) as f64;
                (1..=hi)
                    .map(|k| if k >= lo { 1.0 / n } else { 0.0 })
                    .collect()
            }
        };
        if self.has_multiaccuracy(r, kind) && accuracy < 1.0 {
            // Each later hit lands with the accuracy, else the move stops.
            let max = odds.len();
            let mut out = vec![0.0; max];
            for (i, &p) in odds.iter().enumerate() {
                let planned = i + 1;
                for (k, slot) in out.iter_mut().enumerate().take(planned) {
                    let hits = k + 1;
                    let reach = accuracy.powi(hits as i32 - 1);
                    let stop = if hits == planned { 1.0 } else { 1.0 - accuracy };
                    *slot += p * reach * stop;
                }
            }
            odds = out;
        }
        odds
    }

    /// How the target can survive the first hit (see [`Survival`]).
    fn first_hit_survival(&self, t: MonRef, mv: &Move) -> Survival {
        let m = self.mon(t);
        let full = m.hp == m.maxhp;
        let item = self.item_kind(t);
        Survival {
            full_hp: (full
                && (self.has_ability(t, AbilityKind::Sturdy) || item == Some(ItemKind::FocusSash)))
                || m.vol.endure,
            band: item == Some(ItemKind::FocusBand),
            disguise: mv.category != Category::Status && self.disguised(t),
        }
    }

    /// The effective Speed of `r`: stages, items, abilities, Tailwind and
    /// paralysis (`getStat('spe')`).
    pub fn effective_speed(&self, r: MonRef) -> u32 {
        self.get_stat(r, Stat::Spe, true, true)
    }

    /// The chance that `a` doing `pa` acts before `b` doing `pb`: switches
    /// before moves, then the priority bracket (Prankster, Gale Wings,
    /// Grassy Glide included) with its fractional part (Quick Claw, Quick
    /// Draw, Stall), then Speed, reversed under Trick Room, a tie being a
    /// coin flip. Speed changes during the turn are not foreseen.
    pub fn order_odds(&self, a: MonRef, pa: Plan, b: MonRef, pb: Plan) -> OrderOdds {
        let (ka, kb) = (self.order_keys(a, pa), self.order_keys(b, pb));
        let (sa, sb) = (self.action_speed(a), self.action_speed(b));
        order_odds_from(&ka, sa, &kb, sb)
    }

    /// The chance each of `refs` acts before each other with a move of
    /// priority 0 ([`Battle::order_odds`] with `Plan::Priority(0)` on
    /// both sides): entry `[i][j]` is the chance `refs[i]` goes before
    /// `refs[j]`. Each Pokémon's order keys and action speed are
    /// computed once, not once per pair.
    pub fn order_matrix(&self, refs: &[MonRef]) -> Vec<Vec<f64>> {
        let keys: Vec<Vec<(i32, f64)>> = refs
            .iter()
            .map(|&r| self.order_keys(r, Plan::Priority(0)))
            .collect();
        let speeds: Vec<i32> = refs.iter().map(|&r| self.action_speed(r)).collect();
        (0..refs.len())
            .map(|i| {
                (0..refs.len())
                    .map(|j| order_odds_from(&keys[i], speeds[i], &keys[j], speeds[j]).first)
                    .collect()
            })
            .collect()
    }

    /// An action's place in the order before Speed, higher first: a switch
    /// above any move, else priority x 10 plus the fractional part, with
    /// the chance of each.
    fn order_keys(&self, r: MonRef, plan: Plan) -> Vec<(i32, f64)> {
        let (priority, status) = match plan {
            Plan::Switch => return vec![(10_000, 1.0)],
            Plan::Priority(p) => (p as i32, false),
            Plan::Move(id) => {
                let mv = self.dex.mv(id);
                (
                    self.move_priority(r, mv) as i32,
                    mv.category == Category::Status,
                )
            }
        };
        self.fractional_priority_odds(r, status)
            .into_iter()
            .map(|(f, p)| (priority * 10 + f, p))
            .collect()
    }

    /// The `FractionalPriority` event's outcomes with their chances (see
    /// `run_fractional_priority`): Stall's -0.1, Quick Draw's 30% for a
    /// damaging move, then Quick Claw's 20% while the value is not above 0.
    pub fn fractional_priority_odds(&self, r: MonRef, status_move: bool) -> Vec<(i32, f64)> {
        let base = self.stall_on_fractional_priority(r);
        let mut odds = vec![(base, 1.0)];
        if self.has_ability(r, AbilityKind::QuickDraw) && !status_move {
            let q = QUICK_DRAW.0 as f64 / QUICK_DRAW.1 as f64;
            odds = vec![(1, q), (base, 1.0 - q)];
        }
        if self.item_kind(r) == Some(ItemKind::QuickClaw) {
            let q = QUICK_CLAW.0 as f64 / QUICK_CLAW.1 as f64;
            let mut next = Vec::new();
            for (f, p) in odds {
                if f <= 0 {
                    next.push((1, p * q));
                    next.push((f, p * (1.0 - q)));
                } else {
                    next.push((f, p));
                }
            }
            odds = next;
        }
        odds
    }

    /// Each active Pokémon's effective Speed and its chance of moving
    /// before each other active Pokémon with a move of the same priority.
    pub fn speed_order(&self) -> Vec<SpeedEntry> {
        let actives = self.all_active();
        actives
            .iter()
            .map(|&a| SpeedEntry {
                mon: a,
                speed: self.effective_speed(a),
                before: actives
                    .iter()
                    .filter(|&&b| b != a)
                    .map(|&b| {
                        let o = self.order_odds(a, Plan::Priority(0), b, Plan::Priority(0));
                        (b, o.first)
                    })
                    .collect(),
            })
            .collect()
    }
}

/// [`Battle::order_odds`] from the two actions' order keys and speeds.
fn order_odds_from(ka: &[(i32, f64)], sa: i32, kb: &[(i32, f64)], sb: i32) -> OrderOdds {
    let mut out = OrderOdds::default();
    for &(x, px) in ka {
        for &(y, py) in kb {
            let p = px * py;
            if x != y {
                if x > y {
                    out.first += p;
                }
            } else if sa != sb {
                if sa > sb {
                    out.first += p;
                }
            } else {
                out.first += p / 2.0;
                out.tie += p;
            }
        }
    }
    out
}

/// An active Pokémon's Speed and turn order ([`Battle::speed_order`]).
#[derive(Clone, Debug, PartialEq)]
pub struct SpeedEntry {
    pub mon: MonRef,
    /// Effective Speed.
    pub speed: u32,
    /// The chance it moves before each other active Pokémon, same priority.
    pub before: Vec<(MonRef, f64)>,
}

/// A damage distribution truncated at a cap: (damage, chance) pairs with
/// distinct damages, a damage at the cap standing for the cap or more.
type Hist = Vec<(u32, f64)>;

/// One hit's distribution: each roll with and without a critical hit.
fn hit_hist(rolls: &HitRolls, crit_chance: f64, cap: u32) -> Hist {
    let mut h: Hist = Vec::with_capacity(32);
    let mut add = |d: u32, p: f64| {
        if p == 0.0 {
            return;
        }
        let d = d.min(cap);
        match h.iter_mut().find(|(x, _)| *x == d) {
            Some(e) => e.1 += p,
            None => h.push((d, p)),
        }
    };
    for i in 0..16 {
        add(rolls.normal[i], (1.0 - crit_chance) / 16.0);
        add(rolls.crit[i], crit_chance / 16.0);
    }
    h
}

/// The distribution of the sum of two, truncated at `cap` (`buf` is scratch).
fn convolve(a: &Hist, b: &Hist, cap: u32, buf: &mut Vec<f64>) -> Hist {
    buf.clear();
    buf.resize(cap as usize + 1, 0.0);
    for &(x, pa) in a {
        for &(y, pb) in b {
            buf[(x + y).min(cap) as usize] += pa * pb;
        }
    }
    buf.iter()
        .enumerate()
        .filter(|(_, &p)| p != 0.0)
        .map(|(d, &p)| (d as u32, p))
        .collect()
}

/// The chance of `cap` or more.
fn tail(h: &Hist, cap: u32) -> f64 {
    h.iter().filter(|(d, _)| *d >= cap).map(|(_, p)| p).sum()
}

impl DamageCalc {
    /// Total damage of one use that hits, truncated at `cap`, with the
    /// first hit's survival applied when `survival` is given.
    fn use_hist(&self, cap: u32, survival: Option<Survival>, buf: &mut Vec<f64>) -> Hist {
        let single = self.hits.len() == 1;
        let mut total: Hist = Vec::new();
        let mut acc: Hist = vec![(0, 1.0)];
        for (i, rolls) in self.hits.iter().enumerate() {
            let mut h = hit_hist(rolls, self.crit_chance, cap);
            if i == 0 {
                if let Some(s) = survival {
                    survive(&mut h, s, cap);
                }
            }
            acc = if i == 0 {
                h
            } else {
                convolve(&acc, &h, cap, buf)
            };
            if single {
                return acc;
            }
            let p = self.hit_counts.get(i).copied().unwrap_or(0.0);
            if p > 0.0 {
                for &(d, q) in &acc {
                    match total.iter_mut().find(|(x, _)| *x == d) {
                        Some(e) => e.1 += p * q,
                        None => total.push((d, p * q)),
                    }
                }
            }
        }
        total
    }

    fn finish(&mut self, survival: Survival) {
        let mean_hit = |r: &HitRolls| -> f64 {
            (0..16)
                .map(|i| {
                    (1.0 - self.crit_chance) * r.normal[i] as f64
                        + self.crit_chance * r.crit[i] as f64
                })
                .sum::<f64>()
                / 16.0
        };
        let mut expected = 0.0;
        let mut reach = 1.0;
        for (i, rolls) in self.hits.iter().enumerate() {
            // Hit i + 1 happens when the move hits at least i + 1 times.
            expected += reach * mean_hit(rolls);
            reach -= self.hit_counts.get(i).copied().unwrap_or(0.0);
        }
        self.expected = expected;
        let hp = self.target_hp;
        if hp == 0 {
            return;
        }
        let a = self.accuracy;
        let (ko1, both) = if self.hits.len() == 1 {
            self.kos_single(survival)
        } else {
            self.kos_general(survival)
        };
        self.ko = (a * ko1).min(1.0);
        self.ko2 = (a * a * both + 2.0 * a * (1.0 - a) * ko1).clamp(self.ko, 1.0);
    }

    /// For one use that hits: the chance it KOs, and that two KO (the
    /// first survivable, the second not). One hit: straight from the 32
    /// outcomes.
    fn kos_single(&self, survival: Survival) -> (f64, f64) {
        let hp = self.target_hp;
        let h = &self.hits[0];
        let pc = self.crit_chance;
        let outcomes: [(u32, f64); 32] = std::array::from_fn(|i| {
            if i < 16 {
                (h.normal[i], (1.0 - pc) / 16.0)
            } else {
                (h.crit[i - 16], pc / 16.0)
            }
        });
        // The chance the target survives a lethal first hit (a disguise
        // takes any first hit).
        let save = if survival.disguise || survival.full_hp {
            1.0
        } else if survival.band {
            0.1
        } else {
            0.0
        };
        // The second hit's outcomes by damage, with the chance of each
        // damage or more.
        // The rolls fall with the roll index: merge the two in rising order.
        let mut sorted = [(0u32, 0.0f64); 32];
        let (mut i, mut j) = (16usize, 32usize);
        for slot in sorted.iter_mut() {
            let take_normal = j == 16 || (i > 0 && outcomes[i - 1].0 <= outcomes[j - 1].0);
            *slot = if take_normal {
                i -= 1;
                outcomes[i]
            } else {
                j -= 1;
                outcomes[j]
            };
        }
        // (Damage past 65535 wraps around, as in Showdown: then sort.)
        if sorted.windows(2).any(|w| w[0].0 > w[1].0) {
            sorted.sort_unstable_by_key(|o| o.0);
        }
        let mut at_least = [0.0; 33];
        for i in (0..32).rev() {
            at_least[i] = at_least[i + 1] + sorted[i].1;
        }
        let reach = |need: u32| at_least[sorted.partition_point(|o| o.0 < need)];
        // Each outcome's `reach` at its own need (the HP its damage
        // leaves), worked out for all of them at once: the needs fall
        // as the damages rise, and each roll list falls with the roll
        // index, so merging the outcomes in falling-damage order
        // sweeps `sorted` once from the bottom instead of a search
        // per outcome.
        let mut reachs = [0.0f64; 32];
        {
            // Outcome indices by damage, highest first.
            let mut order = [0usize; 32];
            let (mut i, mut j) = (0usize, 16usize);
            for slot in order.iter_mut() {
                let take_normal = j == 32 || (i < 16 && outcomes[i].0 >= outcomes[j].0);
                *slot = if take_normal {
                    i += 1;
                    i - 1
                } else {
                    j += 1;
                    j - 1
                };
            }
            if order
                .windows(2)
                .any(|w| outcomes[w[0]].0 < outcomes[w[1]].0)
            {
                // The rolls didn't fall with the index (wrapped
                // damage): answer each outcome on its own.
                for (k, &(x, _)) in outcomes.iter().enumerate() {
                    reachs[k] = reach(hp.saturating_sub(x));
                }
            } else {
                let mut p = 0usize;
                for &k in &order {
                    let need = hp.saturating_sub(outcomes[k].0);
                    while p < 32 && sorted[p].0 < need {
                        p += 1;
                    }
                    reachs[k] = at_least[p];
                }
            }
        }
        let mut ko1 = 0.0;
        let mut both = 0.0;
        for (i, &(x, px)) in outcomes.iter().enumerate() {
            // A lethal first hit is survived with chance `save`: 0
            // damage behind a disguise, else 1 HP left.
            let saved = if x >= hp || survival.disguise {
                px * save
            } else {
                0.0
            };
            let kept = px - saved;
            if x >= hp {
                ko1 += kept;
            }
            both += kept * reachs[i];
            if saved > 0.0 {
                let left = if survival.disguise { hp } else { 1 };
                both += saved * reach(left);
            }
        }
        (ko1, both)
    }

    /// [`DamageCalc::kos_single`] for any number of hits, from the
    /// distributions of the total.
    fn kos_general(&self, survival: Survival) -> (f64, f64) {
        let hp = self.target_hp;
        let mut buf = Vec::new();
        let first = self.use_hist(hp, Some(survival), &mut buf);
        let ko1 = tail(&first, hp);
        // Two uses: the first can be survived (full HP); the second, after a
        // hit, not (Sturdy, a sash or a disguise are spent); after a miss it can.
        let plain = self.use_hist(hp, None, &mut buf);
        let both: f64 = first
            .iter()
            .map(|&(x, pa)| {
                pa * plain
                    .iter()
                    .filter(|&&(y, _)| x + y >= hp)
                    .map(|(_, pb)| pb)
                    .sum::<f64>()
            })
            .sum();
        (ko1, both)
    }
}

/// Apply a way to survive the first hit to its distribution (truncated at
/// the target's HP `hp`).
fn survive(h: &mut Hist, s: Survival, hp: u32) {
    if s.disguise {
        let total: f64 = h.iter().map(|(_, p)| p).sum();
        *h = vec![(0, total)];
        return;
    }
    let lethal = tail(h, hp);
    if lethal == 0.0 || hp == 0 {
        return;
    }
    let saved = if s.full_hp {
        lethal
    } else if s.band {
        lethal * 0.1
    } else {
        return;
    };
    let keep = 1.0 - saved / lethal;
    for e in h.iter_mut() {
        if e.0 >= hp {
            e.1 *= keep;
        }
    }
    match h.iter_mut().find(|(x, _)| *x == hp - 1) {
        Some(e) => e.1 += saved,
        None => h.push((hp - 1, saved)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chi2_tail() {
        // Upper tail of chi-square: 3.84 on 1 df and 25.0 on 15 df are about 0.05.
        assert!((chi2_sf(3.841, 1) - 0.05).abs() < 1e-3);
        assert!((chi2_sf(24.996, 15) - 0.05).abs() < 1e-3);
        assert!((chi2_sf(15.0, 15) - 0.451).abs() < 2e-3);
    }

    #[test]
    fn one_hit_matches_the_general_path() {
        let mut rng = crate::rng::Rng::new(9);
        for case in 0..400 {
            let base = 20 + rng.random(200);
            let mut normal = [0u32; 16];
            for (i, d) in normal.iter_mut().enumerate() {
                *d = base * (100 - i as u32) / 100;
            }
            let crit = normal.map(|d| d * 3 / 2);
            let c = DamageCalc {
                hits: vec![HitRolls { normal, crit }],
                hit_counts: vec![1.0],
                crit_chance: [0.0, 1.0 / 24.0, 0.5][case % 3],
                target_hp: 1 + rng.random(300),
                ..Default::default()
            };
            let s = Survival {
                full_hp: case % 4 == 1,
                band: case % 4 == 2,
                disguise: case % 4 == 3,
            };
            let (a, b) = (c.kos_single(s), c.kos_general(s));
            assert!(
                (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9,
                "{case}: {a:?} vs {b:?}"
            );
        }
    }

    #[test]
    fn histograms_add_up() {
        let r = HitRolls {
            normal: [10; 16],
            crit: [15; 16],
        };
        let h = hit_hist(&r, 0.25, 20);
        let p = |h: &Hist, d: u32| h.iter().find(|x| x.0 == d).map_or(0.0, |x| x.1);
        assert!((p(&h, 10) - 0.75).abs() < 1e-12 && (p(&h, 15) - 0.25).abs() < 1e-12);
        let two = convolve(&h, &h, 20, &mut Vec::new());
        // 10 + 10 = 20 already reaches the cap.
        assert!((tail(&two, 20) - 1.0).abs() < 1e-12);
        let mut s = hit_hist(&r, 0.25, 12);
        survive(
            &mut s,
            Survival {
                full_hp: true,
                ..Default::default()
            },
            12,
        );
        assert!((p(&s, 11) - 0.25).abs() < 1e-12 && tail(&s, 12) == 0.0);
    }
}

/// Play `games` random games between teams drawn from `teams` (seeded by
/// `seed`) with every damaging hit recorded with its predicted distribution
/// ([`DamageSample`]), for testing the calculation against played outcomes.
pub fn sample_games(
    dex: &std::sync::Arc<crate::dex::Dex>,
    teams: &[Vec<crate::dex::team::MonSet>],
    games: usize,
    seed: u64,
) -> Vec<DamageSample> {
    let mut rng = crate::rng::Rng::new(seed);
    let mut out = Vec::new();
    for _ in 0..games {
        let t1 = &teams[rng.random(teams.len() as u32) as usize];
        let t2 = &teams[rng.random(teams.len() as u32) as usize];
        let mut b = Battle::from_sets(dex.clone(), [t1, t2], ["p1", "p2"], rng.next_u64());
        b.log_enabled = false;
        b.damage_samples = Some(Box::default());
        for s in 0..2 {
            let n = b.sides[s].mons.len();
            let mut order: Vec<usize> = (0..n).collect();
            rng.shuffle(&mut order, 0, n);
            b.choose_team(s, order);
        }
        while !b.ended && b.turn < 100 {
            for s in 0..2 {
                if b.request == crate::state::RequestState::Ended || !b.side_needs_choice(s) {
                    continue;
                }
                let c = b.sample_joint_choice(s, &mut rng);
                b.choose_structured(s, c);
            }
        }
        out.extend(b.damage_samples.take().map(|v| *v).unwrap_or_default());
    }
    out
}

/// What [`check_samples`] found.
#[derive(Clone, Debug, Default)]
pub struct CheckReport {
    pub hits: usize,
    /// Damage dealt that is not one of the predicted values.
    pub outside: usize,
    pub crits: usize,
    pub crits_expected: f64,
    pub crit_z: f64,
    /// Hits whose KO chance was strictly between 0 and 1, their KOs, the
    /// KOs predicted, the z-score and a chi-square over ten bins of the
    /// predicted chance (statistic, degrees of freedom, p-value).
    pub ko_uncertain: usize,
    pub kos: usize,
    pub kos_expected: f64,
    pub ko_z: f64,
    pub ko_chi2: (f64, usize, f64),
    /// Hits whose KO was certain either way and went otherwise.
    pub ko_wrong: usize,
    /// Non-critical hits with 16 distinct rolls: the chi-square of the roll
    /// each one took against a uniform draw (statistic, 15, p-value).
    pub roll_hits: usize,
    pub roll_chi2: (f64, usize, f64),
    /// Every hit's damage through its predicted distribution (a randomized
    /// probability integral transform, uniform if the distribution is
    /// right): chi-square over 20 bins (statistic, 19, p-value).
    pub pit_chi2: (f64, usize, f64),
    /// First hits the full calculation was compared on, and agreed on.
    pub full_compared: usize,
    pub full_agree: usize,
    /// Single hits whose KO the full calculation predicted with a chance
    /// strictly between 0 and 1 (after Sturdy, Focus Sash and the like):
    /// their KOs (the target at 0 HP after the hit), the KOs predicted, z.
    pub full_ko_uncertain: usize,
    pub full_kos: usize,
    pub full_kos_expected: f64,
    pub full_ko_z: f64,
    /// Single hits whose KO the full calculation called certain either way
    /// and went otherwise.
    pub full_ko_wrong: usize,
}

impl std::fmt::Display for CheckReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "hits {}, damage outside the predicted values {}",
            self.hits, self.outside
        )?;
        writeln!(
            f,
            "critical hits {} vs {:.1} expected (z {:.2})",
            self.crits, self.crits_expected, self.crit_z
        )?;
        writeln!(
            f,
            "KO-uncertain hits {}: KOs {} vs {:.1} expected (z {:.2}); chi2 {:.2} on {} df, p {:.3}; certain KOs wrong {}",
            self.ko_uncertain, self.kos, self.kos_expected, self.ko_z, self.ko_chi2.0, self.ko_chi2.1, self.ko_chi2.2, self.ko_wrong
        )?;
        writeln!(
            f,
            "rolls of {} hits: chi2 {:.2} on {} df, p {:.3}",
            self.roll_hits, self.roll_chi2.0, self.roll_chi2.1, self.roll_chi2.2
        )?;
        writeln!(
            f,
            "damage through the predicted distribution (PIT), {} hits: chi2 {:.2} on {} df, p {:.3}",
            self.hits, self.pit_chi2.0, self.pit_chi2.1, self.pit_chi2.2
        )?;
        writeln!(
            f,
            "full calculation agrees on {} of {} first hits",
            self.full_agree, self.full_compared
        )?;
        write!(
            f,
            "full calculation's KO chance, {} uncertain single hits: KOs {} vs {:.1} expected (z {:.2}); certain ones wrong {}",
            self.full_ko_uncertain, self.full_kos, self.full_kos_expected, self.full_ko_z, self.full_ko_wrong
        )
    }
}

/// Check recorded hits against their predicted distributions: every damage
/// one of the predicted values; critical hits, KOs and rolls as frequent as
/// predicted (z-scores and chi-square tests).
pub fn check_samples(samples: &[DamageSample]) -> CheckReport {
    let mut r = CheckReport {
        hits: samples.len(),
        ..Default::default()
    };
    let (mut crit_var, mut ko_var, mut full_var) = (0.0, 0.0, 0.0);
    // Ten bins of the KO chance: observed, expected, variance.
    let mut bins = [(0.0f64, 0.0f64, 0.0f64); 10];
    let mut rolls = [0usize; 16];
    let mut pit = [0usize; 20];
    let mut u_rng = crate::rng::Rng::new(0x5eed);
    for s in samples {
        // P(damage < d) and P(damage = d) under the prediction.
        let (mut below, mut at) = (0.0, 0.0);
        for i in 0..16 {
            for (d, p) in [
                (s.rolls.normal[i], (1.0 - s.crit_chance) / 16.0),
                (s.rolls.crit[i], s.crit_chance / 16.0),
            ] {
                if d < s.damage {
                    below += p;
                } else if d == s.damage {
                    at += p;
                }
            }
        }
        let u = below + at * ((u_rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64);
        pit[((u * 20.0) as usize).min(19)] += 1;
        let set = if s.crit {
            &s.rolls.crit
        } else {
            &s.rolls.normal
        };
        if !set.contains(&s.damage) {
            r.outside += 1;
        }
        r.crits += s.crit as usize;
        r.crits_expected += s.crit_chance;
        crit_var += s.crit_chance * (1.0 - s.crit_chance);
        let hp = s.target_hp;
        let p: f64 = (0..16)
            .map(|i| {
                (1.0 - s.crit_chance) * (s.rolls.normal[i] >= hp) as u8 as f64
                    + s.crit_chance * (s.rolls.crit[i] >= hp) as u8 as f64
            })
            .sum::<f64>()
            / 16.0;
        let ko = s.damage >= hp;
        if p <= 1e-12 || p >= 1.0 - 1e-12 {
            if ko != (p > 0.5) {
                r.ko_wrong += 1;
            }
        } else {
            r.ko_uncertain += 1;
            r.kos += ko as usize;
            r.kos_expected += p;
            ko_var += p * (1.0 - p);
            let b = &mut bins[((p * 10.0) as usize).min(9)];
            b.0 += ko as u8 as f64;
            b.1 += p;
            b.2 += p * (1.0 - p);
        }
        let mut distinct = s.rolls.normal.to_vec();
        distinct.dedup();
        if !s.crit && distinct.len() == 16 {
            if let Some(i) = s.rolls.normal.iter().position(|&d| d == s.damage) {
                rolls[i] += 1;
                r.roll_hits += 1;
            }
        }
        if let Some((spread, full)) = s.full {
            if spread == s.spread {
                r.full_compared += 1;
                r.full_agree += (full == s.rolls) as usize;
                if let (Some(p), Some(after)) = (s.full_ko, s.hp_after) {
                    let ko = after == 0;
                    if p <= 1e-12 || p >= 1.0 - 1e-12 {
                        r.full_ko_wrong += (ko != (p > 0.5)) as usize;
                    } else {
                        r.full_ko_uncertain += 1;
                        r.full_kos += ko as usize;
                        r.full_kos_expected += p;
                        full_var += p * (1.0 - p);
                    }
                }
            }
        }
    }
    r.full_ko_z = (r.full_kos as f64 - r.full_kos_expected) / f64::max(full_var, 1e-12).sqrt();
    r.crit_z = (r.crits as f64 - r.crits_expected) / crit_var.max(1e-12).sqrt();
    r.ko_z = (r.kos as f64 - r.kos_expected) / ko_var.max(1e-12).sqrt();
    let mut chi2 = 0.0;
    let mut df = 0;
    for (o, e, v) in bins {
        if v > 0.0 {
            chi2 += (o - e) * (o - e) / v;
            df += 1;
        }
    }
    r.ko_chi2 = (chi2, df, chi2_sf(chi2, df));
    let n = r.roll_hits as f64;
    let chi2: f64 = rolls
        .iter()
        .map(|&o| {
            let e = n / 16.0;
            (o as f64 - e).powi(2) / e.max(1e-12)
        })
        .sum();
    r.roll_chi2 = (chi2, 15, chi2_sf(chi2, 15));
    let e = samples.len() as f64 / 20.0;
    let chi2: f64 = pit
        .iter()
        .map(|&o| (o as f64 - e).powi(2) / e.max(1e-12))
        .sum();
    r.pit_chi2 = (chi2, 19, chi2_sf(chi2, 19));
    r
}

/// The chance that a chi-square variable with `df` degrees of freedom
/// exceeds `x` (the upper regularized incomplete gamma Q(df/2, x/2)).
pub fn chi2_sf(x: f64, df: usize) -> f64 {
    if df == 0 {
        return 1.0;
    }
    let (a, x) = (df as f64 / 2.0, x / 2.0);
    if x <= 0.0 {
        return 1.0;
    }
    let ln_gamma = |z: f64| -> f64 {
        // Lanczos approximation.
        const G: [f64; 9] = [
            0.999_999_999_999_809_9,
            676.520_368_121_885_1,
            -1_259.139_216_722_402_8,
            771.323_428_777_653_1,
            -176.615_029_162_140_6,
            12.507_343_278_686_905,
            -0.138_571_095_265_720_12,
            9.984_369_578_019_572e-6,
            1.505_632_735_149_311_6e-7,
        ];
        let z = z - 1.0;
        let mut s = G[0];
        for (i, &g) in G.iter().enumerate().skip(1) {
            s += g / (z + i as f64);
        }
        let t = z + 7.5;
        0.5 * (2.0 * std::f64::consts::PI).ln() + (z + 0.5) * t.ln() - t + s.ln()
    };
    let lg = ln_gamma(a);
    if x < a + 1.0 {
        // Series for P, then Q = 1 - P.
        let (mut sum, mut term, mut n) = (1.0 / a, 1.0 / a, a);
        for _ in 0..1000 {
            n += 1.0;
            term *= x / n;
            sum += term;
            if term.abs() < sum.abs() * 1e-15 {
                break;
            }
        }
        1.0 - sum * (-x + a * x.ln() - lg).exp()
    } else {
        // Continued fraction for Q (Lentz).
        let tiny = 1e-300;
        let mut b = x + 1.0 - a;
        let mut c = 1.0 / tiny;
        let mut d = 1.0 / b;
        let mut h = d;
        for i in 1..1000 {
            let an = -(i as f64) * (i as f64 - a);
            b += 2.0;
            d = an * d + b;
            if d.abs() < tiny {
                d = tiny;
            }
            c = b + an / c;
            if c.abs() < tiny {
                c = tiny;
            }
            d = 1.0 / d;
            let del = d * c;
            h *= del;
            if (del - 1.0).abs() < 1e-15 {
                break;
            }
        }
        (-x + a * x.ln() - lg).exp() * h
    }
}
