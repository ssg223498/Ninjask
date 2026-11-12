//! Evaluator features for a search bot: battle-ai's "v2" set (22 values,
//! `FEATURES_V2`), counted straight from the battle without building the
//! projection. Its Python reference (`battle_ai/agent/features_v2.py`, with
//! v1's `_features_p1` for the first eight) reads `projection()` and calls
//! `damage` / `order` (v1's `matchup`) and `damage_party` /
//! `order_party`; this module reads the same fields and runs the same
//! calculations ([`Battle::damage_calc_party`], [`Battle::order_odds_party`]),
//! so the two agree to rounding.
//!
//! Terms, as the reference names them (indices into a side's party, the
//! projection's `sides[i]["pokemon"]`):
//! - usable moves: PP left and not disabled;
//! - `threat(x, y)`: the best `0.6 * ko + 0.4 * max(ko2, ko)` of x's usable
//!   damaging moves against y, with that move; when none scores above 0,
//!   the first usable move (it may be a status move, whose priority then
//!   orders the turn); with no usable move, no move and 0;
//! - `mu(x, y)`: with a = threat(x, y), b = threat(y, x) and f the chance
//!   that x acts first using a's move against b's,
//!   `a*(f + (1-f)*(1-b)) - b*((1-f) + f*(1-a))`; 0 when neither has a
//!   move, and f is 0 or 1 (the one with a move first) when one lacks it;
//! - the active Pokémon: active, not fainted (nor just revived). A side
//!   waiting to fill a fainted Pokémon's slot has none.
//!
//! Each feature is "own - foe" from p1's side (the ones marked so,
//! "foe - own"); p2's values are p1's negated.

use crate::battle::{Battle, MonRef};
use crate::calc::{DamageCalc, Plan};
use crate::dex::types::*;
use crate::dex::{Dex, Move};
use crate::state::Mon;

/// The features' names, in the order of [`Battle::eval_features`].
pub const FEATURES_V2: [&str; 22] = [
    "alive_diff",
    "hp_diff",
    "status_diff",
    "boost_diff",
    "hazard_diff",
    "mega_left_diff",
    "sash_diff",
    "matchup",
    "cover",
    "best_switch",
    "speed_edge",
    "priority_ko",
    "setup_window",
    "recovery",
    "ko_race",
    "burn_physical",
    "para_faster",
    "sleep_turns",
    "toxic_stage",
    "screens_tailwind",
    "choice_locked_bad",
    "substitute",
];

const THREAT_KO: f64 = 0.6;
const THREAT_KO2: f64 = 0.4;

/// What battle-ai's dex export records of a move for these features.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MoveMarks {
    /// A status move whose boosts to its user add up to 1 or more over the
    /// positive ones (Swords Dance; not Flame Charge, a damaging move).
    pub setup: bool,
    /// The `heal` flag on a move targeting its user (Roost; not Milk
    /// Drink, which targets an adjacent ally or its user).
    pub heal: bool,
    /// The dex priority.
    pub priority: i8,
}

/// The boosts a move gives its user, as battle-ai's dex export reads
/// Showdown's data: `boosts` for a move targeting its user, else
/// `self.boosts`, else the single secondary effect's `self.boosts`.
fn self_boosts(m: &Move) -> Option<&MoveBoosts> {
    if m.target == Target::SelfTarget {
        return m.boosts.as_ref();
    }
    let secondary = match m.secondaries.as_slice() {
        [only] => only.self_boosts.as_ref(),
        _ => None,
    };
    m.self_boosts.as_ref().or(secondary)
}

impl Dex {
    /// [`MoveMarks`] of a move.
    pub fn move_marks(&self, id: MoveId) -> MoveMarks {
        let m = self.mv(id);
        let raised: i32 = self_boosts(m)
            .map(|b| b.vals.iter().filter(|&&v| v > 0).map(|&v| v as i32).sum())
            .unwrap_or(0);
        MoveMarks {
            setup: m.category == Category::Status && raised >= 1,
            heal: m.has(MoveFlag::Heal) && m.target == Target::SelfTarget,
            priority: m.priority,
        }
    }
}

/// A side's Pokémon on the field, as the projection's `isActive` (and not
/// fainted).
fn active_index(b: &Battle, s: usize) -> Option<usize> {
    b.sides[s]
        .mons
        .iter()
        .position(|m| on_field(m) && !m.fainted)
}

fn on_field(m: &Mon) -> bool {
    m.is_active && !m.fainted && !m.revived
}

fn usable(m: &Mon) -> impl Iterator<Item = MoveId> + '_ {
    m.moves
        .iter()
        .filter(|ms| ms.pp > 0 && !ms.disabled)
        .map(|ms| ms.id)
}

fn threat_score(c: &DamageCalc) -> f64 {
    THREAT_KO * c.ko + THREAT_KO2 * c.ko2.max(c.ko)
}

/// The battle as `damage_calc_party` / `order_odds_party` see one pair of
/// Pokémon (one of each side): the battle itself for two Pokémon in their
/// slots, else [`Battle::with_in_slot`]'s copy, made once per pair.
struct PairView {
    key: (MonRef, MonRef),
    /// A copy to run `calc` on (it leaves the battle as it found it).
    battle: Battle,
    /// The pair's places in `battle`, in `key`'s order.
    refs: (MonRef, MonRef),
    /// Both in their slots: order odds come from the battle itself.
    in_slots: bool,
}

/// `threat(x, y)`: the score and the move.
type Threat = (f64, Option<MoveId>);

/// One position's calculations, remembered while the features are counted.
struct Ctx<'a> {
    b: &'a Battle,
    views: Vec<PairView>,
    threats: Vec<((MonRef, MonRef), Threat)>,
}

impl<'a> Ctx<'a> {
    fn new(b: &'a Battle) -> Ctx<'a> {
        Ctx {
            b,
            views: Vec::with_capacity(9),
            threats: Vec::with_capacity(18),
        }
    }

    fn mon(&self, r: MonRef) -> &'a Mon {
        &self.b.sides[r.0].mons[r.1]
    }

    /// The view of the pair {x, y} (one on each side) and where x and y are in it.
    fn view(&mut self, x: MonRef, y: MonRef) -> Result<(usize, MonRef, MonRef), String> {
        let (key, swapped) = if x.0 <= y.0 {
            ((x, y), false)
        } else {
            ((y, x), true)
        };
        let i = match self.views.iter().position(|v| v.key == key) {
            Some(i) => i,
            None => {
                let in_slots = self.b.in_active_slot(key.0)? && self.b.in_active_slot(key.1)?;
                let (battle, refs) = if in_slots {
                    let mut copy = self.b.clone();
                    copy.damage_samples = None;
                    (copy, key)
                } else {
                    let (view, r) = self.b.with_in_slot(&[key.0, key.1])?;
                    (view, (r[0], r[1]))
                };
                self.views.push(PairView {
                    key,
                    battle,
                    refs,
                    in_slots,
                });
                self.views.len() - 1
            }
        };
        let (a, b) = self.views[i].refs;
        Ok(if swapped { (i, b, a) } else { (i, a, b) })
    }

    /// `damage_calc_party(x, move, y)`.
    fn damage(&mut self, x: MonRef, mv: MoveId, y: MonRef) -> Result<DamageCalc, String> {
        let (i, rx, ry) = self.view(x, y)?;
        Ok(self.views[i].battle.calc(rx, mv, ry))
    }

    /// `order_odds_party(x, plan_x, y, plan_y).first`.
    fn order_first(&mut self, x: MonRef, px: MoveId, y: MonRef, py: MoveId) -> Result<f64, String> {
        let (i, rx, ry) = self.view(x, y)?;
        let v = &self.views[i];
        let on = if v.in_slots { self.b } else { &v.battle };
        Ok(on.order_odds(rx, Plan::Move(px), ry, Plan::Move(py)).first)
    }

    fn threat(&mut self, x: MonRef, y: MonRef) -> Result<Threat, String> {
        if let Some((_, t)) = self.threats.iter().find(|(k, _)| *k == (x, y)) {
            return Ok(*t);
        }
        let (mut best, mut move_id) = (0.0, None);
        for mv in usable(self.mon(x)) {
            if move_id.is_none() {
                move_id = Some(mv);
            }
            let d = self.damage(x, mv, y)?;
            if !d.damaging {
                continue;
            }
            let t = threat_score(&d);
            if t > best {
                best = t;
                move_id = Some(mv);
            }
        }
        self.threats.push(((x, y), (best, move_id)));
        Ok((best, move_id))
    }

    /// The chance that x acts first, each using its threat move against
    /// the other; `None` when neither has a move.
    fn first(&mut self, x: MonRef, y: MonRef) -> Result<Option<f64>, String> {
        let (_, am) = self.threat(x, y)?;
        let (_, bm) = self.threat(y, x)?;
        Ok(match (am, bm) {
            (None, None) => None,
            (None, Some(_)) => Some(0.0),
            (Some(_), None) => Some(1.0),
            (Some(a), Some(b)) => Some(self.order_first(x, a, y, b)?),
        })
    }

    fn mu(&mut self, x: MonRef, y: MonRef) -> Result<f64, String> {
        let Some(f) = self.first(x, y)? else {
            return Ok(0.0);
        };
        let (a, _) = self.threat(x, y)?;
        let (b, _) = self.threat(y, x)?;
        Ok(a * (f + (1.0 - f) * (1.0 - b)) - b * ((1.0 - f) + f * (1.0 - a)))
    }
}

/// v1's per-side numbers (`_side_numbers`): alive / 3, HP fractions / 3,
/// status weights / 3, the active Pokémon's boosts / 6, hazard weight,
/// Mega Evolution left, intact Focus Sashes / 3.
fn side_numbers(b: &Battle, s: usize) -> [f64; 7] {
    let side = &b.sides[s];
    let (mut alive, mut hp, mut status, mut sash, mut boost) = (0.0, 0.0, 0.0, 0.0, 0.0);
    let (mut mega_ok, mut megaed) = (false, false);
    for m in &side.mons {
        if b.dex.species(m.species).name.contains("-Mega") {
            megaed = true;
        }
        if m.fainted {
            continue;
        }
        alive += 1.0;
        hp += m.hp as f64 / m.maxhp as f64;
        status += match m.status {
            Status::Slp | Status::Frz => 1.0,
            Status::Par | Status::Brn => 0.6,
            Status::Psn | Status::Tox => 0.5,
            _ => 0.0,
        };
        if m.item.is_some_and(|i| b.dex.item(i).id == "focussash") && m.hp == m.maxhp {
            sash += 1.0;
        }
        if !side.mega_used && b.can_mega_evo((s, m.position)).is_some() {
            mega_ok = true;
        }
        if on_field(m) {
            let k = &m.boosts;
            boost += (k[Boost::Atk as usize] as i32
                + k[Boost::Def as usize] as i32
                + k[Boost::Spa as usize] as i32
                + k[Boost::Spd as usize] as i32
                + k[Boost::Spe as usize] as i32) as f64;
        }
    }
    [
        alive / 3.0,
        hp / 3.0,
        status / 3.0,
        boost / 6.0,
        hazard_weight(b, s),
        if mega_ok && !megaed { 1.0 } else { 0.0 },
        sash / 3.0,
    ]
}

/// v1's hazard weight, summed in the projection's order of the side's
/// conditions: Stealth Rock 1, Toxic Spikes 0.5 per 2 layers, Spikes
/// 1 per 3 layers, Sticky Web 0.5.
fn hazard_weight(b: &Battle, s: usize) -> f64 {
    let c = &b.sides[s].conds;
    let extras = &b.side_extras[s];
    let mut total = 0.0;
    if c.stealth_rock {
        total += 1.0;
    }
    if c.toxic_spikes > 0 {
        total += 0.5 * c.toxic_spikes as f64 / 2.0;
    }
    if extras.spikes > 0 {
        total += extras.spikes as f64 / 3.0;
    }
    if extras.sticky_web {
        total += 0.5;
    }
    total
}

impl Battle {
    /// battle-ai's "v2" evaluator features from `side`'s point of view, in
    /// the order of [`FEATURES_V2`] (see the module documentation). The
    /// battle is left as it was. Not for an ended battle (an evaluator
    /// scores those itself).
    pub fn eval_features(&self, side: usize) -> Result<[f64; 22], String> {
        let mut f = self.eval_features_p1()?;
        if side == 1 {
            for x in f.iter_mut() {
                *x = -*x;
            }
        }
        Ok(f)
    }

    fn eval_features_p1(&self) -> Result<[f64; 22], String> {
        let mut ctx = Ctx::new(self);
        let mut out = [0.0; 22];
        let (own, foe) = (side_numbers(self, 0), side_numbers(self, 1));
        out[0] = own[0] - foe[0];
        out[1] = own[1] - foe[1];
        out[2] = foe[2] - own[2];
        out[3] = own[3] - foe[3];
        out[4] = foe[4] - own[4];
        out[5] = own[5] - foe[5];
        out[6] = own[6] - foe[6];
        let act = [active_index(self, 0), active_index(self, 1)];
        out[7] = self.v1_matchup(&mut ctx, act)?;
        out[8] = cover(&mut ctx, 0)? - cover(&mut ctx, 1)?;
        out[9] = best_switch(&mut ctx, 0, act)? - best_switch(&mut ctx, 1, act)?;
        out[10] = match act {
            [Some(a), Some(b)] => ctx.first((0, a), (1, b))?.map_or(0.0, |f| f - 0.5),
            _ => 0.0,
        };
        out[11] = priority_ko(&mut ctx, 0, act)? - priority_ko(&mut ctx, 1, act)?;
        out[12] = setup_window(&mut ctx, 0, act)? - setup_window(&mut ctx, 1, act)?;
        out[13] = recovery(self, 0) - recovery(self, 1);
        out[14] = match act {
            [Some(a), Some(b)] => {
                let nx = hits_to_ko(&mut ctx, (0, a), (1, b))?;
                let ny = hits_to_ko(&mut ctx, (1, b), (0, a))?;
                ((ny - nx) as f64 / 4.0).clamp(-1.0, 1.0)
            }
            _ => 0.0,
        };
        out[15] = burn_physical(self, 1) - burn_physical(self, 0);
        out[16] = para_faster(self, 1, act) - para_faster(self, 0, act);
        out[17] = sleep_turns(self, 1) - sleep_turns(self, 0);
        out[18] = toxic_stage(self, 1) - toxic_stage(self, 0);
        out[19] = screens_tailwind(self, 0) - screens_tailwind(self, 1);
        out[20] = choice_locked_bad(&mut ctx, 1, act)? - choice_locked_bad(&mut ctx, 0, act)?;
        out[21] = substitute(self, 0, act) - substitute(self, 1, act);
        Ok(out)
    }

    /// v1's `matchup`: `mu` of the two Pokémon in the first slots (`damage`
    /// and `order` on "p1a" and "p2a"), 0 when a side has none on the field.
    fn v1_matchup(&self, ctx: &mut Ctx, act: [Option<usize>; 2]) -> Result<f64, String> {
        if act.contains(&None) {
            return Ok(0.0);
        }
        // "p1a": the Pokémon in the slot (Showdown keeps the active one there).
        let slot = |s: usize| self.active(s, 0).unwrap_or((s, act[s].unwrap()));
        ctx.mu(slot(0), slot(1))
    }
}

fn alive(b: &Battle, s: usize) -> impl Iterator<Item = usize> + '_ {
    b.sides[s]
        .mons
        .iter()
        .enumerate()
        .filter(|(_, m)| !m.fainted)
        .map(|(i, _)| i)
}

/// `cover`: over the foe's Pokémon left, the mean of the best `mu` of one's own.
fn cover(ctx: &mut Ctx, s: usize) -> Result<f64, String> {
    let t = 1 - s;
    let xs: Vec<usize> = alive(ctx.b, s).collect();
    let ys: Vec<usize> = alive(ctx.b, t).collect();
    if xs.is_empty() || ys.is_empty() {
        return Ok(0.0);
    }
    let mut total = 0.0;
    for &y in &ys {
        let mut best = f64::NEG_INFINITY;
        for &x in &xs {
            let v = ctx.mu((s, x), (t, y))?;
            if v > best {
                best = v;
            }
        }
        total += best;
    }
    Ok(total / ys.len() as f64)
}

/// `best_switch`: the best `mu` of a benched Pokémon against the foe on the field.
fn best_switch(ctx: &mut Ctx, s: usize, act: [Option<usize>; 2]) -> Result<f64, String> {
    let t = 1 - s;
    let Some(y) = act[t] else { return Ok(0.0) };
    let bench: Vec<usize> = alive(ctx.b, s)
        .filter(|&i| !on_field(&ctx.b.sides[s].mons[i]))
        .collect();
    if bench.is_empty() {
        return Ok(0.0);
    }
    let mut best = f64::NEG_INFINITY;
    for x in bench {
        let v = ctx.mu((s, x), (t, y))?;
        if v > best {
            best = v;
        }
    }
    Ok(best)
}

/// `priority_ko`: the best KO chance of a usable damaging move of priority
/// above 0 (the dex's) on the foe on the field.
fn priority_ko(ctx: &mut Ctx, s: usize, act: [Option<usize>; 2]) -> Result<f64, String> {
    let [Some(_), Some(_)] = act else {
        return Ok(0.0);
    };
    let (a, b) = ((s, act[s].unwrap()), (1 - s, act[1 - s].unwrap()));
    let mut best = 0.0;
    let moves: Vec<MoveId> = usable(ctx.mon(a)).collect();
    for mv in moves {
        if ctx.b.dex.mv(mv).priority <= 0 {
            continue;
        }
        let d = ctx.damage(a, mv, b)?;
        if d.damaging && d.ko > best {
            best = d.ko;
        }
    }
    Ok(best)
}

/// `setup_window`: `1 - threat(foe, own)` when one's Pokémon on the field
/// has a usable setup move, else 0.
fn setup_window(ctx: &mut Ctx, s: usize, act: [Option<usize>; 2]) -> Result<f64, String> {
    let [Some(_), Some(_)] = act else {
        return Ok(0.0);
    };
    let (a, b) = ((s, act[s].unwrap()), (1 - s, act[1 - s].unwrap()));
    let dex = &ctx.b.dex;
    if !usable(ctx.mon(a)).any(|mv| dex.move_marks(mv).setup) {
        return Ok(0.0);
    }
    Ok(1.0 - ctx.threat(b, a)?.0)
}

/// `recovery`: the missing HP fractions of the Pokémon left with a usable
/// healing move, / 3.
fn recovery(b: &Battle, s: usize) -> f64 {
    let mut total = 0.0;
    for i in alive(b, s) {
        let m = &b.sides[s].mons[i];
        if usable(m).any(|mv| b.dex.move_marks(mv).heal) {
            total += 1.0 - m.hp as f64 / m.maxhp as f64;
        }
    }
    total / 3.0
}

/// The uses x needs to KO y: `ceil(y's HP / x's best expected damage)`,
/// at most 5 (5 when it deals none).
fn hits_to_ko(ctx: &mut Ctx, x: MonRef, y: MonRef) -> Result<i64, String> {
    let mut best: f64 = 0.0;
    let moves: Vec<MoveId> = usable(ctx.mon(x)).collect();
    for mv in moves {
        let e = ctx.damage(x, mv, y)?.expected;
        if e > best {
            best = e;
        }
    }
    if best <= 0.0 {
        return Ok(5);
    }
    Ok(((ctx.mon(y).hp as f64 / best).ceil() as i64).min(5))
}

/// `burn_physical`: burned Pokémon left whose Attack is above their Special Attack, / 3.
fn burn_physical(b: &Battle, s: usize) -> f64 {
    alive(b, s)
        .map(|i| &b.sides[s].mons[i])
        .filter(|m| m.status == Status::Brn && m.stats[1] > m.stats[3])
        .count() as f64
        / 3.0
}

/// `para_faster`: paralyzed Pokémon left faster (stored Speed) than the
/// foe on the field, / 3; 0 when the foe has none there.
fn para_faster(b: &Battle, s: usize, act: [Option<usize>; 2]) -> f64 {
    let Some(y) = act[1 - s] else { return 0.0 };
    let foe_spe = b.sides[1 - s].mons[y].stats[5];
    alive(b, s)
        .map(|i| &b.sides[s].mons[i])
        .filter(|m| m.status == Status::Par && m.stats[5] > foe_spe)
        .count() as f64
        / 3.0
}

/// `sleep_turns`: the sleep counters (`statusState.time`) of the asleep
/// Pokémon left, / 9. The projection keeps the counter while the status
/// state is sleep's or freeze's.
fn sleep_turns(b: &Battle, s: usize) -> f64 {
    let mut total: u32 = 0;
    for i in alive(b, s) {
        let m = &b.sides[s].mons[i];
        if m.status == Status::Slp && matches!(m.status_state, Status::Slp | Status::Frz) {
            total += m.status_time as u32;
        }
    }
    total as f64 / 9.0
}

/// `toxic_stage`: the badly poisoned Pokémon left's stages / 16, / 3.
fn toxic_stage(b: &Battle, s: usize) -> f64 {
    let mut total = 0.0;
    for i in alive(b, s) {
        let m = &b.sides[s].mons[i];
        if m.status == Status::Tox {
            let stage = if m.status_state == Status::Tox {
                m.toxic_stage
            } else {
                0
            };
            total += stage as f64 / 16.0;
        }
    }
    total / 3.0
}

/// `screens_tailwind`: the turns left of Reflect, Light Screen, Aurora Veil
/// and Tailwind on the side, / 8.
fn screens_tailwind(b: &Battle, s: usize) -> f64 {
    let c = &b.sides[s].conds;
    (c.reflect as u32 + c.light_screen as u32 + c.aurora_veil as u32 + c.tailwind as u32) as f64
        / 8.0
}

/// `choice_locked_bad`: 1 when one's Pokémon on the field is locked into a
/// move whose `0.6 * ko + 0.4 * max(ko2, ko)` on the foe there is under 0.2.
fn choice_locked_bad(ctx: &mut Ctx, s: usize, act: [Option<usize>; 2]) -> Result<f64, String> {
    let [Some(_), Some(_)] = act else {
        return Ok(0.0);
    };
    let (a, b) = ((s, act[s].unwrap()), (1 - s, act[1 - s].unwrap()));
    let Some(lock) = ctx.mon(a).vol.choice_lock else {
        return Ok(0.0);
    };
    let d = ctx.damage(a, lock, b)?;
    Ok(if threat_score(&d) < 0.2 { 1.0 } else { 0.0 })
}

/// `substitute`: one's Pokémon on the field is behind a substitute.
fn substitute(b: &Battle, s: usize, act: [Option<usize>; 2]) -> f64 {
    match act[s] {
        Some(i) if b.sides[s].mons[i].vol.substitute_halves > 0 => 1.0,
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::dex::team::parse_packed;
    use crate::rng::Rng;
    use crate::state::RequestState;

    fn format_export() -> Option<String> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut dirs = vec![root.join("data/derived"), root.join("../data/derived")];
        if let Ok(d) = std::env::var("NINJASK_DATA") {
            dirs.insert(0, d.into());
        }
        dirs.into_iter()
            .find_map(|d| std::fs::read_to_string(d.join("format/gen9championsbssregmc.json")).ok())
    }

    /// Positions along random singles games, empty slots included.
    fn positions() -> Option<Vec<Battle>> {
        let json = format_export()?;
        let dex = Arc::new(Dex::from_json(&json).unwrap());
        let root = env!("CARGO_MANIFEST_DIR");
        let teams: Vec<_> =
            std::fs::read_to_string(format!("{root}/tests/fixtures/regmc_teams.txt"))
                .unwrap()
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| parse_packed(&dex, l).unwrap())
                .collect();
        let mut rng = Rng::new(5);
        let mut out = Vec::new();
        for g in 0..12 {
            let mut b = Battle::from_sets(
                dex.clone(),
                [&teams[g % teams.len()], &teams[(g * 5 + 2) % teams.len()]],
                ["a", "b"],
                300 + g as u64,
            );
            b.log_enabled = false;
            b.choose_team(0, vec![0, 1, 2]);
            b.choose_team(1, vec![1, 2, 0]);
            for _ in 0..8 {
                if b.ended || b.request == RequestState::Ended {
                    break;
                }
                out.push(b.clone());
                for s in 0..2 {
                    if b.side_needs_choice(s) {
                        let c = b.sample_joint_choice(s, &mut rng);
                        b.choose_structured(s, c);
                    }
                }
            }
        }
        Some(out)
    }

    /// The remembered pair views give exactly `damage_calc_party` and
    /// `order_odds_party`, for every pair, move and order of calls.
    #[test]
    fn pair_views_match_the_party_calculations() {
        let Some(bs) = positions() else { return };
        let mut compared = 0;
        for b in &bs {
            let mut ctx = Ctx::new(b);
            for x in alive(b, 0).collect::<Vec<_>>() {
                for y in alive(b, 1).collect::<Vec<_>>() {
                    for (p, q) in [((0, x), (1, y)), ((1, y), (0, x))] {
                        let moves: Vec<MoveId> = b.mon(p).moves.iter().map(|m| m.id).collect();
                        let other: Vec<MoveId> = b.mon(q).moves.iter().map(|m| m.id).collect();
                        for &mv in &moves {
                            assert_eq!(
                                ctx.damage(p, mv, q).unwrap(),
                                b.damage_calc_party(p, mv, q).unwrap()
                            );
                            for &ov in &other {
                                assert_eq!(
                                    ctx.order_first(p, mv, q, ov).unwrap(),
                                    b.order_odds_party(p, Plan::Move(mv), q, Plan::Move(ov))
                                        .unwrap()
                                        .first
                                );
                            }
                            compared += 1;
                        }
                    }
                }
            }
        }
        assert!(compared > 500, "{compared}");
    }

    #[test]
    fn features_are_antisymmetric_and_leave_the_battle() {
        let Some(bs) = positions() else { return };
        for b in &bs {
            let before = b.projection();
            let (f1, f2) = (b.eval_features(0).unwrap(), b.eval_features(1).unwrap());
            assert!(f1.iter().zip(&f2).all(|(a, c)| *a == -*c && a.is_finite()));
            assert_eq!(b.projection(), before);
        }
    }

    #[test]
    fn move_marks_follow_the_export_rule() {
        let Some(json) = format_export() else { return };
        let dex = Dex::from_json(&json).unwrap();
        let marks = |id: &str| dex.move_marks(dex.move_id(id).unwrap());
        assert!(marks("swordsdance").setup && !marks("swordsdance").heal);
        assert!(!marks("flamecharge").setup);
        assert!(marks("roost").heal && !marks("roost").setup);
        assert_eq!(marks("quickattack").priority, 1);
    }
}
