//! Using a move: Showdown's `runMove`, `beforeMove` and `useMove`, and the
//! checks on which moves a Pokémon may choose.

use super::damage;
use super::hit::TryHit;
use crate::battle::log::LogEntry;
use crate::battle::Small;
use crate::battle::{ActiveList, Battle, MonRef};
use crate::dex::events::Event;
use crate::dex::types::*;
use crate::dex::Move;
use crate::kinds::AbilityKind;
use crate::kinds::{EffectRef, MoveKind};
use crate::state::*;

impl Battle {
    /// Whether a move slot can't be used now: no PP, disabled (Imprison
    /// included), or (Champions) Fake Out after the Pokémon's first action
    /// since switching in. Showdown `getMoves()`.
    pub fn move_disabled(&self, r: MonRef, idx: usize) -> bool {
        self.move_disabled_openly(r, idx) || self.move_imprisoned(r, idx)
    }

    /// Whether the request shows a move slot as disabled (Showdown
    /// `getMoves(lockedMove, isLastActive)`). Imprison disables moves
    /// 'hidden', so as not to reveal the foe's moves: the request for the
    /// side's last active Pokémon shows them usable, while the others'
    /// show them disabled. A fainted Pokémon's moves are as the last
    /// `DisableMove` pass left them, which skips its foes' Imprison
    /// (`store_disabled_moves`).
    pub fn move_shown_disabled(&self, r: MonRef, idx: usize) -> bool {
        self.move_disabled_openly(r, idx)
            || (self.move_imprisoned(r, idx) && !self.mon(r).fainted && !self.is_last_active(r))
    }

    /// Showdown `isLastActive`: active (`isActive`, which fainting clears),
    /// with no unfainted Pokémon in a later position on its side.
    pub fn is_last_active(&self, (s, slot): MonRef) -> bool {
        let side = &self.sides[s];
        side.mons[slot].is_active
            && !side.mons[slot].fainted
            && (slot + 1..side.n_active).all(|i| side.mons.get(i).is_none_or(|m| m.fainted))
    }

    /// Imprison (onFoeDisableMove): a move an imprisoning foe knows.
    pub(crate) fn move_imprisoned(&self, r: MonRef, idx: usize) -> bool {
        // Rarely anyone imprisons: look at the foes' slots before the foes.
        if !self.foe_side_imprisons(r.0) {
            return false;
        }
        let id = self.mon(r).moves[idx].id;
        self.foes_of(r)
            .into_iter()
            .any(|f| self.mon(f).vol.imprison && self.mon(f).moves.iter().any(|x| x.id == id))
    }

    /// Whether a Pokémon in the active slots of the side facing side `s`
    /// has Imprison up.
    #[inline]
    pub(crate) fn foe_side_imprisons(&self, s: usize) -> bool {
        let foe_side = &self.sides[1 - s];
        let n = foe_side.n_active.min(foe_side.mons.len());
        let mut any = false;
        for m in &foe_side.mons[..n] {
            any |= m.vol.imprison;
        }
        any
    }

    /// Every reason a move slot can't be used but Imprison.
    pub(crate) fn move_disabled_openly(&self, r: MonRef, idx: usize) -> bool {
        self.mon(r).moves[idx].pp == 0 || self.move_disabled_by_effect(r, idx)
    }

    /// The end-of-turn `DisableMove` pass: Showdown's `moveSlot.disabled`
    /// (true or 'hidden'), which stays as set until the next pass on the
    /// Pokémon, benched or not. The engine works out disabled moves when
    /// asked; this stored copy is for the state projection.
    /// A fainted Pokémon is not active, so only its moves' own handlers run
    /// (Fake Out's), not its foes' Imprison.
    pub(crate) fn store_disabled_moves(&mut self, r: MonRef) {
        let active = !self.mon(r).fainted;
        for i in 0..self.mon(r).moves.len() {
            let disabled =
                self.move_disabled_by_effect(r, i) || (active && self.move_imprisoned(r, i));
            self.mon_mut(r).moves[i].disabled = disabled;
        }
    }

    /// The `DisableMove` handlers but Imprison's: the Choice lock, Throat
    /// Chop, Taunt, Heal Block, Encore, Disable and (Champions) Fake Out's own.
    #[inline(always)]
    fn move_disabled_by_effect(&self, r: MonRef, idx: usize) -> bool {
        let m = self.mon(r);
        let ms = &m.moves[idx];
        let v = &m.vol;
        // Usually no volatile disables anything and Gravity is off: only
        // the move's own conditions remain.
        if v.choice_lock.is_none()
            && v.throat_chop == 0
            && v.taunt == 0
            && v.heal_block == 0
            && v.encore.is_none()
            && v.disable.is_none()
            && !v.torment
            && self.field.gravity == 0
        {
            return self.move_disabled_by_itself(m, ms);
        }
        self.move_disabled_by_volatile(r, idx)
    }

    /// `move_disabled_by_effect` when a disabling volatile or Gravity is up.
    #[inline(never)]
    fn move_disabled_by_volatile(&self, r: MonRef, idx: usize) -> bool {
        let m = self.mon(r);
        let ms = &m.moves[idx];
        if let Some(lock) = m.vol.choice_lock {
            let choice = self.item_kind(r).map(|k| k.is_choice()).unwrap_or(false);
            if choice && ms.id != lock && m.moves.iter().any(|x| x.id == lock) {
                return true;
            }
        }
        let mv = self.dex.mv(ms.id);
        if (m.vol.throat_chop > 0 && mv.has(MoveFlag::Sound))
            || (m.vol.taunt > 0 && mv.category == Category::Status)
        {
            return true;
        }
        if (m.vol.heal_block > 0 && mv.has(MoveFlag::Heal)) || self.gravity_on_disable_move(mv) {
            return true;
        }
        // Encore: every move but the encored one (if the Pokémon still has it).
        if let Some(e) = m.vol.encore {
            if ms.id != e && m.moves.iter().any(|x| x.id == e) {
                return true;
            }
        }
        if m.vol.disable == Some(ms.id) || self.torment_on_disable_move(r, ms.id) {
            return true;
        }
        self.move_disabled_by_itself(m, ms)
    }

    /// The move's own `DisableMove` conditions: `cantusetwice` (Gigaton
    /// Hammer, Blood Moon) right after itself, and Fake Out or First
    /// Impression after the Pokémon's first action.
    #[inline(always)]
    fn move_disabled_by_itself(&self, m: &Mon, ms: &MoveSlot) -> bool {
        (m.last_move == Some(ms.id) && self.dex.mv(ms.id).has(MoveFlag::CantUseTwice))
            || (m.active_move_actions > 0
                && matches!(
                    self.dex.move_kind[ms.id as usize],
                    MoveKind::FakeOut | MoveKind::FirstImpression
                ))
    }

    /// Showdown `runMove`: before-move checks, PP, then `useMove`. `slot` is
    /// a move slot, `STRUGGLE` or `RECHARGE`. Records `moveThisTurnResult`.
    /// `self_target`: the queued action's move targets the user
    /// (`ActionKind::Move::self_target`).
    pub(crate) fn run_move(
        &mut self,
        r: MonRef,
        slot: usize,
        target_loc: i8,
        id: MoveId,
        self_target: bool,
    ) {
        self.mon_mut(r).active_move_actions += 1;
        let move_id = match slot {
            RECHARGE => None,
            STRUGGLE => self.dex.ids.moves.struggle,
            EXTERNAL => Some(id),
            _ => Some(self.mon(r).moves[slot].id),
        };
        // runMove finds the target first: a spread move's random foe is drawn
        // here, before the move is even tried. Struggle (randomNormal) and
        // `recharge` (no target type) get a random foe too.
        let tracks =
            move_id.is_some_and(|id| self.dex.move_kind[id as usize] == MoveKind::SnipeShot);
        let original = self.take_original_target(r);
        let target = match (
            move_id,
            self.target_ignoring_redirection(r, tracks, original),
        ) {
            (_, Some(t)) => Some(t),
            (Some(id), None) => {
                let (tt, smart) = (self.dex.mv(id).target, self.dex.mv(id).smart_target);
                let tt = if self_target { Target::SelfTarget } else { tt };
                let future = self.dex.mv(id).has(MoveFlag::FutureMove);
                self.get_target(r, tt, target_loc, smart, future)
            }
            (None, None) => self.get_random_target(r, Target::RandomNormal),
        };
        // setActiveMove.
        self.active_move.cur_move = move_id;
        self.active_move.cur_user = Some(r);
        if let Err(result) = self.before_move(r, move_id) {
            self.active_move.cur_move = None;
            self.active_move.cur_user = None;
            self.run_move_aborted(r, move_id);
            // MoveAborted: a two-turn move's charge is lost.
            if self.mon(r).vol.two_turn.is_some() {
                self.end_two_turn(r);
            }
            self.mon_mut(r).move_this_turn_result = result;
            return;
        }
        let Some(move_id) = move_id else {
            return;
        };
        // beforeMoveCallback: Focus Punch after a hit.
        if self.focus_punch_before_move_callback(r, move_id) {
            self.active_move.cur_move = None;
            self.active_move.cur_user = None;
            self.mon_mut(r).move_this_turn_result = Some(false);
            return;
        }
        // Destiny Bond's onBeforeMove (priority -1): any other move removes it.
        if self.dex.move_kind[move_id as usize] != MoveKind::DestinyBond {
            self.mon_mut(r).vol.destiny_bond = false;
        }
        // LockMove: the strike of a two-turn move costs no PP; Struggle has none.
        let locked = self.locked_move(r).is_some();
        if !locked && slot != STRUGGLE {
            // deductPP marks the slot used even when it has no PP left.
            self.mon_mut(r).moves[slot].used = true;
            if self.mon(r).moves[slot].pp == 0 {
                let ident = self.ident(r);
                let name = self.dex.mv(move_id).name.clone();
                log_line!(self, "|cant|{ident}|nopp|{name}");
                self.active_move.cur_move = None;
                self.active_move.cur_user = None;
                self.mon_mut(r).move_this_turn_result = Some(false);
                return;
            }
            self.mon_mut(r).moves[slot].pp -= 1;
        }
        // moveUsed
        let m = self.mon_mut(r);
        m.last_move = Some(move_id);
        m.last_move_target_loc = target_loc;
        m.moved_this_turn = true;
        let result = self.use_move(r, move_id, target, target_loc, locked, self_target);
        let moved = self.relocated_mon(r);
        let target = target.map(|t| if t == r { moved } else { t });
        let r = moved;
        // A called move (Copycat, Sleep Talk) is the active move from here
        // on, and its result is the Pokémon's.
        let (move_id, result) = self.called_move_result(r, move_id, result);
        self.mon_mut(r).move_this_turn_result = result;
        self.run_after_move(r, move_id, target);
        self.faint_messages();
        self.check_win();
    }

    /// `BeforeMove` handlers in priority order: recharge (11), sleep/freeze
    /// (10), flinch (8), Throat Chop (6), Taunt (5), confusion (3), Attract
    /// (2), paralysis (1), Choice lock. `Err` holds the move result when the move doesn't
    /// happen: `Some(false)` a failure, `None` (recharging) not one.
    fn before_move(&mut self, r: MonRef, move_id: Option<MoveId>) -> Result<(), Option<bool>> {
        let ident = self.ident(r);
        // runEvent sorts every handler before any runs, so ties draw first,
        // highest priority first: Throat Chop and Heal Block (6), then the
        // foes' Imprisons (4) at equal speed.
        use crate::abilities::{compare_mod, ModHandler};
        let mut six: Small<bool> = Small::new(); // true: Heal Block
        if self.mon(r).vol.throat_chop > 0 {
            six.push(false);
        }
        if self.mon(r).vol.heal_block > 0 {
            six.push(true);
        }
        if six.len() > 1 {
            let speed = self.mon(r).speed;
            let mut hs: Small<(bool, ModHandler)> = six
                .iter()
                .map(|&k| (k, ModHandler::new(6, speed, 2)))
                .collect();
            self.speed_sort_by(&mut hs, |a, b| compare_mod(&a.1, &b.1));
            six = hs.into_iter().map(|(k, _)| k).collect();
        }
        let mut imprisoners: Small<(MonRef, ModHandler)> = self
            .foes_of(r)
            .into_iter()
            .filter(|&f| self.mon(f).vol.imprison)
            .map(|f| (f, ModHandler::new(4, self.mon(f).speed, 2)))
            .collect();
        if imprisoners.len() > 1 {
            self.speed_sort_by(&mut imprisoners, |a, b| compare_mod(&a.1, &b.1));
        }
        // Chilly Reception's onBeforeMove (priority 100): `-prepare` before
        // any other check, with Glaive Rush's drawback ending before the
        // user's next move at the same priority.
        let chilly =
            move_id.is_some_and(|id| self.dex.move_kind[id as usize] == MoveKind::ChillyReception);
        if self.mon(r).vol.chilly_reception && chilly {
            if self.mon(r).vol.glaive_rush {
                // Both handlers tie on priority, speed and subOrder.
                let speed = self.mon(r).speed;
                let mut hs = [
                    ModHandler::new(100, speed, 2),
                    ModHandler::new(100, speed, 2),
                ];
                self.speed_sort_by(&mut hs, compare_mod);
            }
            let ri = self.ident(r);
            log_line!(self, "|-prepare|{ri}|Chilly Reception|[premajor]");
        }
        self.mon_mut(r).vol.glaive_rush = false;
        if self.mon(r).vol.must_recharge > 0 {
            log_line!(self, "|cant|{ident}|recharge");
            self.mon_mut(r).vol.must_recharge = 0;
            return Err(None);
        }
        let defrost = move_id.is_some_and(|id| self.defrosts(r, id));
        match self.mon(r).status {
            Status::Slp => {
                let dec = self.early_bird_sleep_decrement(r);
                let m = self.mon_mut(r);
                m.status_time = m.status_time.saturating_sub(dec);
                if m.status_time == 0 {
                    self.cure_status(r);
                } else {
                    log_line!(self, "|cant|{ident}|slp");
                    // sleepUsable: the move goes on (Snore).
                    if !move_id.is_some_and(|id| self.sleep_usable(id)) {
                        return Err(Some(false));
                    }
                }
            }
            // Champions: thaw when the counter runs out or with chance 1/4;
            // moves that thaw the user skip the check (and thaw in `use_move`).
            Status::Frz if !defrost => {
                let m = self.mon_mut(r);
                m.status_time = m.status_time.saturating_sub(1);
                let thaw = m.status_time == 0 || self.rng.chance(1, 4);
                if thaw {
                    self.cure_status(r);
                } else {
                    log_line!(self, "|cant|{ident}|frz");
                    return Err(Some(false));
                }
            }
            _ => {}
        }
        if self.mon(r).vol.flinch {
            log_line!(self, "|cant|{ident}|flinch");
            self.steadfast_on_flinch(r);
            return Err(Some(false));
        }
        if let Some(id) = move_id {
            let dex = self.dex.clone();
            let mv = dex.mv(id);
            // Disable (7); Champions lets moves that can't be used twice through.
            if self.mon(r).vol.disable == Some(id) && !mv.has(MoveFlag::CantUseTwice) {
                let name = mv.name.clone();
                log_line!(self, "|cant|{ident}|Disable|{name}");
                return Err(Some(false));
            }
            // Throat Chop and Heal Block (both priority 6), in the drawn order.
            for heal_block in six {
                if !heal_block && mv.has(MoveFlag::Sound) {
                    log_line!(self, "|cant|{ident}|move: Throat Chop");
                    return Err(Some(false));
                }
                if heal_block && mv.has(MoveFlag::Heal) {
                    let name = mv.name.clone();
                    log_line!(self, "|cant|{ident}|move: Heal Block|{name}");
                    return Err(Some(false));
                }
            }
            if self.gravity_on_before_move(r, mv) {
                return Err(Some(false));
            }
            if self.mon(r).vol.taunt > 0 && mv.category == Category::Status {
                let name = mv.name.clone();
                log_line!(self, "|cant|{ident}|move: Taunt|{name}");
                return Err(Some(false));
            }
            // Imprison (onFoeBeforeMove, 4): a foe that imprisoned this move.
            let imprisoned = imprisoners
                .iter()
                .any(|&(f, _)| self.mon(f).moves.iter().any(|x| x.id == id));
            if imprisoned {
                let name = self.dex.mv(id).name.clone();
                log_line!(self, "|cant|{ident}|move: Imprison|{name}");
                return Err(Some(false));
            }
        }
        if self.mon(r).vol.confusion > 0 {
            let m = self.mon_mut(r);
            m.vol.confusion -= 1;
            if m.vol.confusion == 0 {
                log_line!(self, "|-end|{ident}|confusion");
            } else {
                log_line!(self, "|-activate|{ident}|confusion");
                if self.rng.chance(33, 100) {
                    let dmg = self.confusion_damage(r);
                    self.apply_move_damage(r, dmg, "|[from] confusion");
                    return Err(Some(false));
                }
            }
        }
        // Attract (2).
        if self.attract_on_before_move(r) {
            return Err(Some(false));
        }
        if self.mon(r).status == Status::Par && self.rng.chance(1, 8) {
            log_line!(self, "|cant|{ident}|par");
            return Err(Some(false));
        }
        // choicelock: a locked Pokémon can't use another move but Struggle.
        if let (Some(lock), Some(id)) = (self.mon(r).vol.choice_lock, move_id) {
            if !self.held_item_kind(r).is_some_and(|k| k.is_choice()) {
                self.mon_mut(r).vol.choice_lock = None;
            } else if id != lock
                && self.dex.move_kind[id as usize] != MoveKind::Struggle
                && !self.ignoring_item(r)
            {
                let name = self.dex.mv(id).name.clone();
                let line = self.log.len();
                log_line!(self, "|move|{ident}|{name}");
                self.attr_move_line(line, "[still]");
                log_line!(self, "|-fail|{ident}");
                return Err(Some(false));
            }
        }
        Ok(())
    }

    /// Showdown `getConfusionDamage`: a 40 BP typeless physical self-hit.
    fn confusion_damage(&mut self, r: MonRef) -> u32 {
        let (atk_boost, def_boost) = self.confusion_stages(r);
        let m = self.mon(r);
        let atk = damage::apply_boost(m.stats[Stat::Atk as usize] as u32, atk_boost);
        let def = damage::apply_boost(
            m.stats[self.wonder_room_stat(Stat::Def) as usize] as u32,
            def_boost,
        );
        let base = damage::base_damage(m.level as u32, 40, atk, def) + 2;
        let roll = self.rng.random_damage(16);
        (base * (100 - roll) / 100).max(1)
    }

    /// The move's type after `ModifyType`: Weather Ball, then abilities
    /// (-ate abilities, Liquid Voice). Also returns whether an -ate ability
    /// changed it.
    pub(crate) fn move_type(&self, r: MonRef, mv: &Move, kind: MoveKind) -> (TypeId, bool) {
        let mut t = mv.type_;
        if kind == MoveKind::TerrainPulse && self.is_grounded(r) {
            if let Some(terrain) = self.field.terrain {
                let name = match terrain {
                    Terrain::Electric => "Electric",
                    Terrain::Grassy => "Grass",
                    Terrain::Misty => "Fairy",
                    Terrain::Psychic => "Psychic",
                };
                t = self.dex.type_id(name).unwrap_or(t);
            }
        }
        // Struggle's onModifyMove: type `???`, which nothing resists or is
        // immune to; Protean and Libero skip it.
        if kind == MoveKind::Struggle {
            return (self.dex.ids.types.typeless.unwrap_or(t), false);
        }
        if kind == MoveKind::WeatherBall {
            if let Some(w) = self.move_weather() {
                let name = match w {
                    Weather::Rain => "Water",
                    Weather::Sun => "Fire",
                    Weather::Sand => "Rock",
                    Weather::Snow => "Ice",
                };
                t = self.dex.type_id(name).unwrap_or(mv.type_);
            }
        }
        let t = self.move_on_modify_type(r, kind, t);
        let (t, changed) = self.ability_move_type(r, mv, t);
        (self.run_modify_type(r, t), changed)
    }

    /// Showdown `queue.willAct()`: is any move or switch still queued?
    fn will_act(&self) -> bool {
        self.queue
            .iter()
            .any(|a| matches!(a.kind, ActionKind::Move { .. } | ActionKind::Switch { .. }))
    }

    /// Whether the target's queued move (`queue.willMove`) is a status move;
    /// `None` when it has no move queued. Struggle counts as physical.
    fn queued_move_is_status(&self, t: MonRef) -> Option<bool> {
        if self.mon(t).fainted {
            return None;
        }
        self.queue.iter().find_map(|a| match a.kind {
            ActionKind::Move {
                side,
                mon,
                slot,
                id,
                ..
            } if (side, mon) == t => {
                // Struggle counts as physical; Recharge as a status move.
                if slot == STRUGGLE {
                    Some(false)
                } else if slot == RECHARGE {
                    Some(true)
                } else if slot == EXTERNAL {
                    Some(self.dex.mv(id).category == Category::Status)
                } else {
                    Some(self.dex.mv(self.mon(t).moves[slot].id).category == Category::Status)
                }
            }
            _ => None,
        })
    }

    /// `-fail` for the user, and the move line gets `[still]`: Showdown's
    /// failed TryHit, a failed status, or effects that all returned false.
    pub(crate) fn fail_move(&mut self, r: MonRef) {
        let ri = self.ident(r);
        log_line!(self, "|-fail|{ri}");
        if let Some(line) = self.last_move_line {
            self.attr_move_line(line, "[still]");
        }
    }

    /// Append an attribute such as `[still]` to the move's log line.
    /// Showdown `attrLastMove`: `[still]` also blanks the target ("if no
    /// animation plays, the target should never be known").
    pub(crate) fn attr_move_line(&mut self, index: usize, attr: &str) {
        if let Some(LogEntry::Plain(line)) = self.log.get_mut(index) {
            if attr == "[still]" {
                // `parts[4] = ''` also adds the field to a line without a
                // target (JS arrays grow on assignment).
                let mut parts: Vec<&str> = line.split('|').collect();
                if parts.len() < 5 {
                    parts.resize(5, "");
                }
                parts[4] = "";
                *line = parts.join("|");
            }
            line.push('|');
            line.push_str(attr);
        }
    }

    /// Showdown `retargetLastMove`: the move line names a new target.
    pub(crate) fn retarget_move_line(&mut self, index: usize, target: MonRef) {
        let ident = self.ident(target);
        if let Some(LogEntry::Plain(line)) = self.log.get_mut(index) {
            let mut parts: Vec<String> = line.split('|').map(str::to_string).collect();
            if parts.len() > 4 {
                parts[4] = ident;
            }
            *line = parts.join("|");
        }
    }

    /// Showdown `useMove` / `useMoveInner` / `trySpreadMoveHit` / `tryMoveHit`.
    /// `locked`: the second turn of a two-turn move (no Pressure). Returns
    /// the move's result for `moveThisTurnResult`: `Some(true)` it did
    /// something, `Some(false)` it failed, `None` it did neither (blocked by
    /// Protect, or charging).
    pub(crate) fn use_move(
        &mut self,
        r: MonRef,
        move_id: MoveId,
        target: Option<MonRef>,
        target_loc: i8,
        locked: bool,
        self_target: bool,
    ) -> Option<bool> {
        // A move Magic Bounce sent back (`move.hasBounced`): no PP, no Choice
        // lock, and it can't be bounced again.
        let bounced = self.bounce.take().is_some();
        // setActiveMove; a move another one called (Copycat, Sleep Talk).
        self.active_move.cur_move = Some(move_id);
        self.active_move.cur_user = Some(r);
        let called_by = self.caller_of(r, move_id);
        self.ability_breaker = None;
        self.active_move.bypass.clear();
        self.active_move.hit_targets = None;
        self.active_move.fling = None;
        let dex = self.dex.clone();
        let mv = dex.mv(move_id);
        let kind = dex.move_kind[move_id as usize];
        // The queued action's move, as resolveAction left it: a Curse aimed
        // at its user (`self_target`) keeps that target type.
        let self_aimed;
        let mv = if self_target {
            self_aimed = crate::dex::Move {
                target: Target::SelfTarget,
                ..mv.clone()
            };
            &self_aimed
        } else {
            mv
        };
        // Beat Up's ModifyMove; a Round moved up by another Round (the
        // action's sourceEffect, replaced by a locked move's).
        let beat_up;
        let mv = if kind == MoveKind::BeatUp {
            beat_up = self.beat_up_on_modify_move(r, mv);
            &beat_up
        } else {
            mv
        };
        let long_reach;
        let mv = match self.long_reach_on_modify_move(r, mv) {
            Some(m) => {
                long_reach = m;
                &long_reach
            }
            None => mv,
        };
        self.active_move.round_doubled =
            std::mem::take(&mut self.turn_state.round_source) && !locked;
        // useMoveInner: ModifyTarget (Comeuppance aims at its last attacker),
        // then moves on the user or its allies aim at the user.
        let target = self.comeuppance_on_modify_target(r, kind).or(target);
        let mut target = if matches!(mv.target, Target::SelfTarget | Target::Allies) {
            Some(r)
        } else {
            target
        };
        // ModifyType, then ModifyMove: a Choice item locks the move in; frozen
        // users thaw when using a move that thaws the user (`frz.onModifyMove`).
        let (move_type, type_changed) = self.move_type(r, mv, kind);
        // The move's own ModifyMove: Shell Side Arm picks its category.
        let shell;
        let mv = match kind {
            MoveKind::ShellSideArm => match self.shell_side_arm_on_modify_move(r, target, mv) {
                Some(m) => {
                    shell = m;
                    &shell
                }
                None => mv,
            },
            _ => mv,
        };
        // Wonder Room's ModifyMove: Body Press swaps the boosts it uses.
        let wonder;
        let mv = match self.wonder_room_on_modify_move(mv) {
            Some(m) => {
                wonder = m;
                &wonder
            }
            None => mv,
        };
        self.active_move.used_type = Some((move_id, move_type));
        if kind == MoveKind::Struggle {
            let ident = self.ident(r);
            log_line!(self, "|-activate|{ident}|move: Struggle");
        }
        self.run_ability_modify_move(r, mv);
        if self.item_kind(r).map(|k| k.is_choice()).unwrap_or(false)
            && self.mon(r).vol.choice_lock.is_none()
            && !bounced
        {
            self.mon_mut(r).vol.choice_lock = Some(move_id);
        }
        if self.mon(r).status == Status::Frz && self.defrosts(r, move_id) {
            let ident = self.ident(r);
            log_line!(self, "|-curestatus|{ident}|frz|[from] move: {}", mv.name);
            let m = self.mon_mut(r);
            m.status = Status::None;
            m.status_state = Status::None;
            m.status_time = 0;
        }
        // ModifyMove changed the target type (Expanding Force in Psychic
        // Terrain): Showdown draws a new random target after the move's own
        // handler and again after the event, both compared with the base target.
        let target_type = self
            .modify_move_target_type(r, mv, target)
            .unwrap_or_else(|| self.move_target_type(r, mv));
        if target_type != mv.target {
            self.get_random_target(r, target_type);
        }
        // runEvent('ModifyMove'): Throat Chop's and Heal Block's handlers (they
        // stop called sound or healing moves) and the Gravity and Wonder Room
        // pseudo-weathers (speed 0) all sort; equal ones draw.
        let v = self.mon(r).vol;
        {
            use crate::abilities::{compare_mod, ModHandler};
            let speed = self.mon(r).speed;
            let mut hs: Small<ModHandler> = Small::new();
            if v.throat_chop > 0 {
                hs.push(ModHandler::new(0, speed, 2));
            }
            if v.heal_block > 0 {
                hs.push(ModHandler::new(0, speed, 2));
            }
            if self.field.gravity > 0 {
                hs.push(ModHandler::new(0, 0, 5));
            }
            if self.field.wonder_room > 0 {
                hs.push(ModHandler::new(
                    0,
                    0,
                    if self.field.wonder_room_armed { 5 } else { 2 },
                ));
                self.field.wonder_room_armed = true;
            }
            if hs.len() > 1 {
                self.speed_sort_by(&mut hs, compare_mod);
            }
        }
        // Their onModifyMove stops a sound or healing move the Pokémon was
        // made to use (Magic Bounce); useMoveInner then draws a random
        // target for the move that is no longer there.
        let stopped = if v.throat_chop > 0 && mv.has(MoveFlag::Sound) {
            Some("|cant|{}|move: Throat Chop".to_string())
        } else if v.heal_block > 0 && mv.has(MoveFlag::Heal) {
            Some(format!("|cant|{{}}|move: Heal Block|{}", mv.name))
        } else {
            None
        };
        if let Some(line) = stopped {
            let ident = self.ident(r);
            if self.log_enabled {
                self.add(line.replacen("{}", &ident, 1));
            }
            self.get_random_target(r, Target::Normal);
            return Some(false);
        }
        // Gravity's onModifyMove stops a called `gravity` move the same way.
        if called_by.is_some() && self.gravity_on_before_move(r, mv) {
            self.get_random_target(r, Target::Normal);
            return Some(false);
        }
        if target_type != mv.target {
            target = self.get_random_target(r, target_type);
        }
        // Throat Chop's and Heal Block's onModifyMove stop a sound or healing
        // move another effect calls (Magic Bounce). The move is then
        // `false`, whose target differs from the base one, so
        // useMoveInner draws a random foe before giving up.
        if (v.throat_chop > 0 && mv.has(MoveFlag::Sound))
            || (v.heal_block > 0 && mv.has(MoveFlag::Heal))
        {
            let ident = self.ident(r);
            if v.throat_chop > 0 && mv.has(MoveFlag::Sound) {
                log_line!(self, "|cant|{ident}|move: Throat Chop");
            } else {
                log_line!(self, "|cant|{ident}|move: Heal Block|{}", mv.name);
            }
            self.get_random_target(r, Target::RandomNormal);
            return Some(false);
        }
        // Sheer Force's ModifyMove: a copy without the secondary effects.
        let sheer;
        let mv = match self.sheer_force_on_modify_move(r, mv) {
            Some(m) => {
                sheer = m;
                &sheer
            }
            None => mv,
        };
        // Stench's ModifyMove (-1): a copy with the 10% flinch secondary.
        let stench;
        let mv = if let Some(m) = self.stench_on_modify_move(r, mv) {
            stench = m;
            &stench
        } else {
            mv
        };
        let ident = self.ident(r);
        // Showdown prints the target with a template string: none is "null".
        let target_ident = target
            .map(|t| self.ident(t))
            .unwrap_or_else(|| "null".into());
        let line = self.log.len();
        self.last_move_line = Some(line);
        if self.log_enabled {
            let from: std::borrow::Cow<str> = if locked {
                "|[from] lockedmove".into()
            } else if bounced {
                "|[from] ability: Magic Bounce".into()
            } else if let Some(source) = called_by {
                format!("|[from] move: {}", self.dex.mv(source).name).into()
            } else if self.active_move.round_doubled {
                "|[from] move: Round".into()
            } else {
                "".into()
            };
            log_line!(self, "|move|{ident}|{}|{target_ident}{from}", mv.name);
        }
        if target.is_none() {
            self.attr_move_line(line, "[notarget]");
            log_line!(self, "|-fail|{ident}");
            return Some(false);
        }
        self.smart_target = mv.smart_target;
        // Stalwart's onModifyMove: `move.tracksTarget = move.target !==
        // 'scripted'` (Counter and Mirror Coat still go where they're sent).
        self.active_move.tracks_target = kind == MoveKind::SnipeShot
            || (self.stalwart_on_modify_move(r) && mv.target != Target::Scripted);
        let future = kind == MoveKind::FutureSight;
        let (targets, retarget) = self.get_move_targets(r, target_type, target, move_type, future);
        if let Some(t) = retarget {
            self.retarget_move_line(line, t);
        }
        let field_move = matches!(
            target_type,
            Target::All | Target::AllySide | Target::FoeSide | Target::AllyTeam
        );
        // DeductPP: Pressure (not for the strike of a two-turn move).
        if !locked && !bounced {
            let extra_pp = self.pressure_pp(r, mv, &targets);
            // A called move's extra PP comes off the caller (`callerMoveForPressure`).
            let pp_id = called_by.unwrap_or(move_id);
            if extra_pp > 0 {
                if let Some(ms) = self.mon_mut(r).moves.iter_mut().find(|ms| ms.id == pp_id) {
                    ms.pp = ms.pp.saturating_sub(extra_pp);
                }
            }
        }
        // With no target left, TryMove still sees the chosen one.
        let last = targets.last().copied().or(target).unwrap_or(r);
        // The move line attributes go to (Showdown `lastMoveLine`); an
        // instant charge move's `-anim` line takes over.
        let mut line = line;
        // The move's own TryMove: two-turn moves charge (or strike at once in
        // their weather).
        if let MoveKind::Charge(ck) = kind {
            if self.mon(r).vol.charging == Some(move_id) {
                let v = &mut self.mon_mut(r).vol;
                v.charging = None;
                v.charging_target_loc = 0;
                v.charging_duration = 0;
            } else {
                log_line!(self, "|-prepare|{ident}|{}", mv.name);
                if ck.spa_boost() {
                    let mut b = [0i8; 7];
                    b[Boost::Spa as usize] = 1;
                    let effect = self.move_ref(mv);
                    self.boost(r, &b, Some(r), effect, false, false);
                }
                if ck.instant_in().is_some() && self.charge_move_weather(r, ck) == ck.instant_in() {
                    self.attr_move_line(line, "[still]");
                    let ti = self.ident(last);
                    line = self.log.len();
                    self.last_move_line = Some(line);
                    log_line!(self, "|-anim|{ident}|{}|{ti}", mv.name);
                } else {
                    // ChargeMove (Power Herb is not modelled), then twoturnmove,
                    // whose start adds the move's own volatile.
                    // Both addVolatile calls (twoturnmove, then the move's own id,
                    // a condition or not) run the TryAddVolatile sort.
                    self.try_add_volatile_sort(r);
                    self.try_add_volatile_sort(r);
                    self.stamp_volatile(r, ResidualVol::TwoTurnMove);
                    if ck.volatile_duration() > 0 {
                        self.stamp_volatile(r, ResidualVol::Charging);
                    }
                    // twoturnmove's onStart: a move called by another (Copycat,
                    // Sleep Talk) aims at the slot of its target as TryMove saw
                    // it, after redirection (Lightning Rod), or of a random foe
                    // if that one has fainted.
                    let mut loc = target_loc;
                    if called_by.is_some() && mv.target != Target::SelfTarget {
                        let mut d = last;
                        if self.mon(d).fainted {
                            let foes: Small<MonRef> = (0..self.sides[1 - r.0].n_active)
                                .filter_map(|sl| self.active(1 - r.0, sl))
                                .collect();
                            if !foes.is_empty() {
                                d = self.rng.sample(&foes);
                            }
                        }
                        loc = self.loc_of(r, d);
                    }
                    let v = &mut self.mon_mut(r).vol;
                    v.two_turn = Some(move_id);
                    v.two_turn_duration = 2;
                    v.charging = Some(move_id);
                    v.charging_target_loc = loc;
                    v.charging_duration = ck.volatile_duration();
                    self.attr_move_line(line, "[still]");
                    // twoturnmove's onStart runs PrepareHit (Protean).
                    self.run_prepare_hit(r, mv, move_type, bounced);
                    return None;
                }
            }
        }
        // The move's own TryMove (`singleEvent`): Double Shock needs an
        // Electric-type user, Burn Up a Fire-type one; Pollen Puff can't
        // heal an ally under Heal Block (`cant`, a failure).
        match kind {
            MoveKind::DoubleShock | MoveKind::BurnUp => {
                let (ty, name) = if kind == MoveKind::DoubleShock {
                    ("Electric", "Double Shock")
                } else {
                    ("Fire", "Burn Up")
                };
                if !self.dex.type_id(ty).is_some_and(|e| self.has_type_of(r, e)) {
                    log_line!(self, "|-fail|{ident}|move: {name}");
                    self.attr_move_line(line, "[still]");
                    return None;
                }
            }
            MoveKind::PollenPuff
                if last.0 == r.0 && last != r && self.mon(r).vol.heal_block > 0 =>
            {
                self.attr_move_line(line, "[still]");
                let ri = self.ident(r);
                log_line!(self, "|cant|{ri}|move: Heal Block|{}", mv.name);
                return Some(false);
            }
            _ => {}
        }
        // runEvent('TryMove'): Armor Tail, Queenly Majesty, Damp.
        if self.run_try_move(r, last, mv, line) {
            return Some(false);
        }
        self.self_destruct(r, kind);
        if targets.is_empty() && !field_move {
            self.attr_move_line(line, "[notarget]");
            log_line!(self, "|-fail|{ident}");
            return Some(false);
        }

        // Try / PrepareHit: move-level failure checks.
        if let Some(result) = self.move_on_try(r, kind, targets.first().copied()) {
            return result;
        }
        let mut fling_move: Option<Move> = None;
        let ok = match kind {
            MoveKind::Fling => match self.fling_on_prepare_hit(r, mv) {
                Some(m) => {
                    fling_move = Some(m);
                    true
                }
                None => false,
            },
            MoveKind::ShellSideArm => {
                self.shell_side_arm_on_prepare_hit(r, targets.first().copied(), mv, line);
                true
            }
            MoveKind::FakeOut | MoveKind::FirstImpression => self.mon(r).active_move_actions <= 1,
            // Sucker Punch also fails against a target that must recharge,
            // even when Encore changed its queued action.
            MoveKind::SuckerPunch => {
                self.queued_move_is_status(targets[0]) == Some(false)
                    && self.mon(targets[0]).vol.must_recharge == 0
            }
            MoveKind::Protect(_) => self.will_act() && self.stall_move(r),
            MoveKind::Endure => self.will_act() && self.stall_move(r),
            MoveKind::AllySwitch
            | MoveKind::DestinyBond
            | MoveKind::NoRetreat
            | MoveKind::StuffCheeks
            | MoveKind::SleepTalk => self.move_on_prepare_hit(r, kind),
            MoveKind::WideGuard | MoveKind::QuickGuard => self.will_act(),
            MoveKind::FollowMe | MoveKind::RagePowder => self.sides[r.0].n_active > 1,
            MoveKind::AuroraVeil => self.effective_weather() == Some(Weather::Snow),
            MoveKind::SteelRoller => self.field.terrain.is_some(),
            MoveKind::ClangorousSoul => {
                let m = self.mon(r);
                m.hp as u32 * 100 > m.maxhp as u32 * 33 && m.maxhp != 1
            }
            MoveKind::Round => {
                self.round_on_try();
                true
            }
            MoveKind::AuraWheel if self.aura_wheel_on_try(r, line) => {
                self.run_move_fail(r, last, mv, kind);
                return Some(false);
            }
            // The move's own onTry: Belch needs a berry eaten; Poltergeist a
            // held item on its target.
            MoveKind::Belch => self.mon(r).ate_berry,
            MoveKind::Poltergeist => targets.first().is_some_and(|&t| self.mon(t).item.is_some()),
            // Counter and Mirror Coat need their volatile to have seen a hit.
            MoveKind::Counter => self.mon(r).vol.counter.is_some_and(|c| c.slot.is_some()),
            MoveKind::MirrorCoat => self
                .mon(r)
                .vol
                .mirror_coat
                .is_some_and(|c| c.slot.is_some()),
            // The target must be about to use a priority attack (`queue.willMove`).
            // `getActionSpeed` stores the modified priority (Grassy Glide, Gale
            // Wings) in the action's move, without the fractional part.
            MoveKind::UpperHand => targets.first().is_some_and(|&t| {
                !self.mon(t).fainted
                    && self.queue.iter().any(|a| match a.kind {
                        ActionKind::Move {
                            side, mon, slot, ..
                        } if (side, mon) == t => {
                            a.priority > 0
                                && slot < crate::state::EXTERNAL
                                && self.dex.mv(self.mon(t).moves[slot].id).category
                                    != Category::Status
                        }
                        _ => false,
                    })
            }),
            // Needs a hit from a foe this turn.
            MoveKind::Comeuppance | MoveKind::MetalBurst => {
                self.last_damaged_by(r).is_some_and(|a| a.this_turn)
            }
            // Needs two moves, itself among them, and every other one used.
            MoveKind::LastResort => {
                let id = self.dex.ids.moves.last_resort;
                let slots = &self.mon(r).moves;
                slots.len() >= 2
                    && slots.iter().any(|s| Some(s.id) == id)
                    && slots.iter().all(|s| Some(s.id) == id || s.used)
            }
            _ => true,
        };
        if !ok {
            log_line!(self, "|-fail|{ident}");
            self.attr_move_line(line, "[still]");
            self.run_move_fail(r, last, mv, kind);
            return Some(false);
        }
        self.run_prepare_hit(r, mv, move_type, bounced);
        // Healing Wish's onTryHit (after PrepareHit, so Protean and Libero
        // change the type first): a Pokémon to switch in.
        if kind == MoveKind::HealingWish && !self.can_switch_side(r.0) {
            self.fail_move(r);
            return Some(false);
        }
        // Revival Blessing's onTryHit (likewise): a fainted teammate to revive.
        if kind == MoveKind::RevivalBlessing && !self.sides[r.0].mons.iter().any(|m| m.fainted) {
            log_line!(self, "|-fail|{ident}");
            self.attr_move_line(line, "[still]");
            self.run_move_fail(r, last, mv, kind);
            return Some(false);
        }
        // Fling's PrepareHit: the thrown item's power and secondary effect.
        let flung;
        let mv = match fling_move {
            Some(m) => {
                flung = m;
                &flung
            }
            None => mv,
        };
        // Parental Bond's PrepareHit: the move now hits twice.
        let parental;
        let mv =
            match self.parental_bond_on_prepare_hit(r, mv, targets.len() > 1 && !mv.smart_target) {
                Some(m) => {
                    parental = m;
                    &parental
                }
                None => mv,
            };

        // Moves that target a side or the whole field (`tryMoveHit`).
        if matches!(
            mv.target,
            Target::All | Target::AllySide | Target::FoeSide | Target::AllyTeam
        ) {
            // TryHitSide on the move's first target (the first active
            // Pokémon of the side it aims at): the `onAllyTryHitSide`
            // handlers of that side, sorted by plain Speed stat
            // (`getStat('spe', true, true)`). Magic Bounce sends a reflectable
            // foe-side move back (stopping the event); Sap Sipper drinks a
            // Grass move aimed at its own side; Soundproof announces a sound
            // move. A suppressed handler still sorts.
            if matches!(
                mv.target,
                Target::FoeSide | Target::AllySide | Target::AllyTeam
            ) {
                use crate::abilities::{compare_mod, ModHandler};
                #[derive(Clone, Copy)]
                enum A {
                    MagicBounce,
                    SapSipper,
                    Soundproof,
                }
                let side = if mv.target == Target::FoeSide {
                    1 - r.0
                } else {
                    r.0
                };
                let mut holders: Small<(MonRef, A, ModHandler)> = (0..self.sides[side].n_active)
                    .filter_map(|sl| self.active(side, sl))
                    .filter(|&h| self.mon(h).hp > 0 && !self.mon(h).fainted)
                    .filter_map(|h| {
                        let k = match self.dex.ability(self.mon(h).ability).id.as_str() {
                            "magicbounce" => A::MagicBounce,
                            "sapsipper" => A::SapSipper,
                            "soundproof" => A::Soundproof,
                            _ => return None,
                        };
                        Some((
                            h,
                            k,
                            ModHandler::new(0, self.mon(h).stats[Stat::Spe as usize] as i32, 7),
                        ))
                    })
                    .collect();
                if holders.len() > 1 {
                    self.speed_sort_by(&mut holders, |a, b| compare_mod(&a.2, &b.2));
                }
                // `target.isSemiInvulnerable()` reads the event's target, not
                // the holder: a holder mid-Dive still bounces.
                let first = (0..self.sides[side].n_active).find_map(|sl| self.active(side, sl));
                let hidden = first.is_some_and(|f| self.semi_invulnerable(f));
                let works = |b: &Battle, h: MonRef| {
                    !b.ability_suppressed(h) && !b.ignoring_ability(h) && !b.mon(h).revived
                };
                for &(h, k, _) in holders.iter() {
                    if !works(self, h) {
                        continue;
                    }
                    match k {
                        A::MagicBounce
                            if !bounced
                                && side != r.0
                                && mv.has(MoveFlag::Reflectable)
                                && !hidden =>
                        {
                            self.bounce_move(r, h, move_id);
                            return Some(false);
                        }
                        A::SapSipper
                            if h != r
                                && side == r.0
                                && move_type == self.dex.type_id("Grass").unwrap_or(NO_TYPE) =>
                        {
                            let mut b = [0i8; 7];
                            b[Boost::Atk as usize] = 1;
                            let a = self.mon(h).ability;
                            self.boost(h, &b, None, EffectRef::Ability(a), false, false);
                        }
                        A::Soundproof if mv.has(MoveFlag::Sound) => {
                            let hi = self.ident(h);
                            log_line!(self, "|-immune|{hi}|[from] ability: Soundproof");
                        }
                        _ => {}
                    }
                }
            }
            let applied = match kind {
                MoveKind::SetWeather(w) => self.set_weather(w, r, None),
                MoveKind::SetTerrain(t) => self.set_terrain(t, r, None),
                MoveKind::TrickRoom => {
                    self.use_trick_room(r);
                    true
                }
                MoveKind::Tailwind => self.add_side_condition(r.0, SideCond::Tailwind, r),
                MoveKind::Reflect => self.add_side_condition(r.0, SideCond::Reflect, r),
                MoveKind::LightScreen => self.add_side_condition(r.0, SideCond::LightScreen, r),
                MoveKind::AuroraVeil => self.add_side_condition(r.0, SideCond::AuroraVeil, r),
                MoveKind::WideGuard | MoveKind::QuickGuard => {
                    let c = if kind == MoveKind::WideGuard {
                        SideCond::WideGuard
                    } else {
                        SideCond::QuickGuard
                    };
                    // onHitSide gives the user the stall counter and counts as
                    // success, even when an ally has set the guard up already.
                    self.add_side_condition(r.0, c, r);
                    self.add_stall(r);
                    true
                }
                MoveKind::Haze => {
                    self.add("|-clearallboost");
                    for p in self.all_active() {
                        self.mon_mut(p).boosts = [0; 7];
                    }
                    true
                }
                MoveKind::PerishSong => self.perish_song(r, mv, move_type),
                MoveKind::StealthRock => self.add_stealth_rock(1 - r.0),
                MoveKind::Gravity => self.add_gravity(),
                MoveKind::FairyLock => self.add_fairy_lock(),
                MoveKind::MagneticFlux => self.magnetic_flux_on_hit_side(r, mv),
                k if crate::moves::field_moves::field_move_kind(k) => self.run_field_move(r, k),
                // Other field moves are reported unsupported (dex/support.rs).
                _ => true,
            };
            if std::mem::take(&mut self.active_move.field_not_fail) {
                return None;
            }
            if !applied {
                log_line!(self, "|-fail|{ident}");
                self.attr_move_line(line, "[still]");
            }
            return Some(applied);
        }

        // Dragon Darts with two targets (`move.smartTarget`): no spread
        // modifier; it turns off at the first target that drops out, whose
        // miss goes unannounced, and the other then takes both hits.
        let mut smart = self.smart_target && targets.len() > 1;
        let spread = targets.len() > 1 && !smart;
        let mut live = targets;
        // Whether a target dropped out with a failure (`false`), rather than
        // being blocked without one (`NOT_FAIL`: Protect, Wide Guard).
        let mut failed = false;

        // 0. Invulnerability (Helping Hand is exempt): the move misses.
        if kind != MoveKind::HelpingHand && !self.skips_invulnerability(r, kind) {
            let mut next = ActiveList::new();
            for &t in &live {
                if self.invulnerable(r, t, &mv.id) {
                    if smart {
                        smart = false;
                    } else {
                        if !spread {
                            self.attr_move_line(line, "[miss]");
                        }
                        let ti = self.ident(t);
                        log_line!(self, "|-miss|{ident}|{ti}");
                    }
                    failed = true;
                } else {
                    next.push(t);
                }
            }
            live = next;
        }
        smart &= !failed;

        // 1. TryHit: Psychic Terrain (priority 4), Protect (3), then abilities.
        // Flash Fire's onTryHit sets the move's accuracy to true, and the
        // move object is shared: the other targets skip the accuracy roll.
        let mut always_hits = false;
        self.smart_target = smart;
        for stage in [4, 3] {
            let mut next = ActiveList::new();
            for &t in &live {
                match if t == r {
                    TryHit::Hit
                } else {
                    self.try_hit(r, t, mv, stage)
                } {
                    TryHit::Hit => next.push(t),
                    TryHit::Blocked => {}
                    TryHit::Failed => failed = true,
                }
            }
            live = next;
        }
        smart = self.smart_target;
        // Magic Bounce (TryHit priority 1): the holder uses a reflectable
        // move back on the user, and the move stops there.
        if !bounced && mv.has(MoveFlag::Reflectable) {
            let mut next = ActiveList::new();
            for &t in &live {
                if t != r
                    && self.has_ability(t, crate::kinds::AbilityKind::MagicBounce)
                    && !self.semi_invulnerable(t)
                {
                    self.bounce_move(r, t, move_id);
                    failed = true;
                } else {
                    next.push(t);
                }
            }
            live = next;
            // Later attributes go to the line written last (Showdown `attrLastMove`).
            line = self.last_move_line.unwrap_or(line);
        }
        let mut next = ActiveList::new();
        for &t in &live {
            if t != r && self.ability_try_hit(r, t, mv, move_type) {
                failed = true;
                always_hits |= matches!(
                    self.ability_kind(t),
                    crate::kinds::AbilityKind::Absorb {
                        effect: crate::kinds::AbsorbEffect::FlashFire,
                        ..
                    }
                );
            } else {
                next.push(t);
            }
        }
        live = next;
        smart &= !failed;
        // 2. Type immunity (status moves ignore it unless `ignoreImmunity: false`;
        //    Scrappy ignores it for Normal and Fighting moves).
        let checks_immunity = (mv.category != Category::Status
            || mv.ignore_immunity == Some(false))
            && !self.ignores_type_immunity(r, move_type);
        if checks_immunity {
            let mut next = ActiveList::new();
            for &t in &live {
                // A smart-targeting move's immunity goes unannounced. The
                // user isn't exempt: a locked Thrash aimed back at its Ghost
                // user (no foe left in range) has no effect.
                if !self.run_immunity(t, move_type, !smart) {
                    failed = true;
                    continue;
                }
                next.push(t);
            }
            live = next;
            smart &= !failed;
        }
        // 3. Move-specific immunities: Grass types to powder moves; Dark types
        //    to foes' Prankster-boosted moves.
        let prankster = self.prankster_boosted(r, mv);
        if mv.has(MoveFlag::Powder) || prankster || self.has_try_immunity(mv) {
            let mut next = ActiveList::new();
            for &t in &live {
                let ty = self.types_of(t);
                let immune_to = |b: &Battle, effect: &str| {
                    ty.iter().any(|&x| b.dex.type_immune_to_effect(x, effect))
                };
                let powder = mv.has(MoveFlag::Powder) && t != r && immune_to(self, "powder");
                let dark = prankster && t.0 != r.0 && immune_to(self, "prankster");
                if powder || self.run_try_immunity(t, r, mv) || dark {
                    let ti = self.ident(t);
                    log_line!(self, "|-immune|{ti}");
                    failed = true;
                    continue;
                }
                next.push(t);
            }
            live = next;
        }
        // 4. Accuracy.
        let mut next = ActiveList::new();
        for &t in &live {
            self.active_move.active_target = Some(t);
            // A one-hit KO can't hit a higher level or (Sheer Cold) an Ice
            // type, unless the target is semi-invulnerable (No Guard reaches
            // it), where Showdown skips the check.
            if let Some(immune_type) = mv.ohko.filter(|_| !self.semi_invulnerable(t)) {
                let (lu, lt) = (self.mon(r).level, self.mon(t).level);
                if lu < lt || immune_type.is_some_and(|ty| self.has_type_of(t, ty)) {
                    let ti = self.ident(t);
                    log_line!(self, "|-immune|{ti}|[ohko]");
                    failed = true;
                    continue;
                }
            }
            // ModifyAccuracy runs for any move but a one-hit KO, sure ones too.
            if mv.ohko.is_none() {
                self.event_sort(Event::ModifyAccuracy, t, Some(r), &[]);
            }
            if always_hits || self.accuracy_hits(r, t, mv) {
                next.push(t);
            } else {
                if smart {
                    smart = false;
                } else {
                    if !spread {
                        self.attr_move_line(line, "[miss]");
                    }
                    let ti = self.ident(t);
                    log_line!(self, "|-miss|{ident}|{ti}");
                }
                failed = true;
            }
        }
        live = next;
        smart &= !failed;
        if live.is_empty() {
            if spread {
                self.attr_move_line(line, "[spread] ");
            }
            self.run_move_fail(r, last, mv, kind);
            return if failed { Some(false) } else { None };
        }
        // 5. Break protection (Feint, Phantom Force): the target's Protect and
        //    its side's guards.
        if mv.breaks_protect {
            for &t in &live {
                let mut broke = self.mon_mut(t).vol.protect.take().is_some();
                let conds = &mut self.sides[t.0].conds;
                for c in [SideCond::WideGuard, SideCond::QuickGuard] {
                    if conds.get(c) > 0 {
                        *conds.get_mut(c) = 0;
                        broke = true;
                    }
                }
                if broke {
                    let ti = self.ident(t);
                    if kind == MoveKind::Feint {
                        log_line!(self, "|-activate|{ti}|move: Feint");
                    } else {
                        log_line!(self, "|-activate|{ti}|move: {}|[broken]", mv.name);
                    }
                    let v = &mut self.mon_mut(t).vol;
                    v.stall_counter = 0;
                    v.stall_duration = 0;
                }
            }
        }

        let mut total_damage = 0;
        let mut hit = true;
        if mv.category == Category::Status || self.ally_puff_on_try_hit(r, &live, kind) {
            // The hit loop keeps every target if the move did something to
            // one (`[spread]` lists them all), and none otherwise.
            let hit_list = self.status_move_hit(r, &live, mv, kind);
            // Parental Bond on an ally's Pollen Puff: a damaging move to
            // Showdown's hit loop, so the heal happens twice and the loop
            // announces `-hitcount` when the second hit was attempted.
            if self.active_move.pbond
                && !hit_list.is_empty()
                && self.ally_puff_on_try_hit(r, &live, kind)
            {
                self.move_hit = 1;
                let mut landed = 1;
                if !live.iter().all(|&t| self.mon(t).hp == 0) && self.mon(r).hp > 0 {
                    self.run_update();
                    self.move_hit = 2;
                    if !self.status_move_hit(r, &live, mv, kind).is_empty() {
                        landed = 2;
                    }
                }
                if self.move_hit == 2 {
                    let first = self.ident(live[0]);
                    log_line!(self, "|-hitcount|{first}|{landed}");
                }
            }
            hit = !hit_list.is_empty();
            if !hit {
                live.clear();
            }
            // The hit loop's gotAttacked on every target it kept (no damage
            // number for a status move).
            if hit {
                for t in std::mem::take(&mut self.active_move.status_reached) {
                    if t != r {
                        self.got_attacked_by_status(t, r, mv.idx);
                    }
                }
            }
            // Status moves go through the same hit loop (Champions
            // `hitStepMoveHitLoop`): eachEvent('Update') after the hit, and
            // again after the loop, when the move did something.
            if hit {
                self.run_update();
                // The hit loop's faint messages (Healing Wish's, Memento's user).
                let user_down = self.mon(r).hp == 0;
                self.faint_messages_check(user_down);
                self.run_update();
                // afterMoveSecondaryEvent on the targets a substitute didn't take.
                // Targets are Pokémon: after Ally Switch they follow theirs.
                let after: Small<MonRef> = self
                    .after_move_secondary_targets(&hit_list)
                    .into_iter()
                    .map(|t| self.follow_relocation(t))
                    .collect();
                if self.ally_puff_on_try_hit(r, &live, kind) {
                    // A damaging move to Showdown (Pollen Puff healing an
                    // ally): Red Card, Eject Button and the rest react.
                    self.begin_damaging_move(mv, false);
                    self.run_after_move_secondary(r, &after, mv);
                } else {
                    self.run_after_move_secondary_status(r, &after, mv);
                }
                // EmergencyExit at the loop's end: with one target, the
                // move's total damage (0 here) on top of the HP its last
                // hit this turn left (Substitute's cost can cross half).
                if let [t] = live[..] {
                    if self.mon(t).hp > 0 {
                        let before = self.mon(t).hurt_this_turn.unwrap_or(0);
                        self.emergency_exit(t, before);
                    }
                }
            }
        } else {
            total_damage =
                self.damaging_move_hit(r, &live, mv, kind, move_type, type_changed, spread, smart);
        }
        // The `[spread]` attribute, with logging on (it edits the move line).
        if spread && self.log_enabled {
            let slots: Vec<String> = live
                .iter()
                .map(|&t| format!("{}{}", self.sides[t.0].id(), ["a", "b"][t.1]))
                .collect();
            self.attr_move_line(line, &format!("[spread] {}", slots.join(",")));
        }
        if !hit {
            return Some(false);
        }
        if let Some(b) = mv.self_boost_after {
            let effect = self.move_ref(mv);
            self.boost_listed(r, &b, Some(r), effect, false, true);
        }
        // AfterMoveSecondarySelf, unless Sheer Force dropped the secondaries
        // (`suppressingSecondaries()`: the user must still have Sheer Force,
        // which Wandering Spirit or Mummy may have taken by now).
        if !(mv.sheer_force && self.has_ability(r, crate::kinds::AbilityKind::SheerForce)) {
            // The event's target: the last of the move's targets, as chosen
            // (useMoveInner's `target`), hit or not.
            let last_target = last;
            let hp_before = self.mon(r).hp;
            self.run_after_move_secondary_self(r, last_target, kind);
            if !self.magician_on_after_move_secondary_self(r, &live, mv) {
                self.after_move_secondary_self(r, last_target, mv, total_damage);
            }
            // EmergencyExit for a user that Life Orb took to half HP or less.
            if r != last_target && mv.category != Category::Status {
                self.emergency_exit(r, hp_before);
            }
        }
        Some(true)
    }

    pub(super) fn move_ref(&self, mv: &Move) -> EffectRef {
        EffectRef::Move(mv.idx)
    }

    /// The slot of the Pokémon's queued move action (`queue.willMove`).
    pub(crate) fn queued_move_slot(&self, t: MonRef) -> Option<usize> {
        if self.mon(t).fainted {
            return None;
        }
        self.queue.iter().find_map(|a| match a.kind {
            ActionKind::Move {
                side, mon, slot, ..
            } if (side, mon) == t => Some(slot),
            _ => None,
        })
    }

    /// `runEvent('MoveAborted')`, before `twoturnmove`'s handler runs: the
    /// volatiles' handlers (`twoturnmove`, `destinybond`, `charge`) tie and
    /// the sort draws. Destiny Bond ends; Charge ends for an Electric move
    /// (its base type: ModifyType hasn't run).
    pub(crate) fn run_move_aborted(&mut self, r: MonRef, move_id: Option<MoveId>) {
        use crate::abilities::{compare_mod, ModHandler};
        let v = self.mon(r).vol;
        let n = [v.two_turn.is_some(), v.destiny_bond, v.charge]
            .iter()
            .filter(|&&x| x)
            .count();
        if n > 1 {
            let speed = self.mon(r).speed;
            let mut hs: Small<ModHandler> = smallvec::smallvec![ModHandler::new(0, speed, 2); n];
            self.speed_sort_by(&mut hs, compare_mod);
        }
        self.mon_mut(r).vol.destiny_bond = false;
        if v.charge {
            let mv = move_id.map(|m| self.dex.mv(m));
            if mv.is_some_and(|mv| {
                self.dex.type_names[mv.type_ as usize] == "Electric" && mv.id != "charge"
            }) {
                self.end_charge(r);
            }
        }
    }

    /// The move's target type at choice time (the request, `resolveAction`):
    /// Champions makes a non-Ghost's Curse target the user.
    pub(crate) fn move_target_at_choice(&self, r: MonRef, mv: &Move) -> Target {
        if self.dex.move_kind[mv.idx as usize] == MoveKind::Curse && !self.is_ghost(r) {
            Target::SelfTarget
        } else {
            mv.target
        }
    }

    /// The move's own `ModifyMove` changing its target type: Curse aims at
    /// the user unless a Ghost uses it, and a Ghost's at a random foe when
    /// it had no target or aimed at an ally.
    pub(crate) fn modify_move_target_type(
        &self,
        r: MonRef,
        mv: &Move,
        target: Option<MonRef>,
    ) -> Option<Target> {
        if self.dex.move_kind[mv.idx as usize] != MoveKind::Curse {
            return None;
        }
        if !self.is_ghost(r) {
            Some(Target::SelfTarget)
        } else if target.is_none_or(|t| t != r && t.0 == r.0) {
            Some(Target::RandomNormal)
        } else {
            Some(mv.target)
        }
    }

    /// Stall's `onFractionalPriority` (-0.1, in tenths), the first handler.
    pub(crate) fn stall_on_fractional_priority(&self, r: MonRef) -> i32 {
        if self.has_ability(r, AbilityKind::Stall) {
            -1
        } else {
            0
        }
    }

    /// `runEvent('PrepareHit')`: Protean and Libero give the user its move's
    /// type, once per switch-in, unless the move was bounced or calls
    /// another move.
    pub(crate) fn run_prepare_hit(
        &mut self,
        r: MonRef,
        mv: &Move,
        move_type: TypeId,
        bounced: bool,
    ) {
        if !self.has_ability(r, AbilityKind::Protean) {
            return;
        }
        let libero = self.ability_id(r) == "libero";
        let st = &self.mon(r).ability_state;
        if (if libero { st.libero } else { st.protean }).is_some() {
            return;
        }
        if bounced || mv.has(MoveFlag::FutureMove) || mv.fields.iter().any(|f| f == "callsMove") {
            return;
        }
        if move_type == NO_TYPE || self.dex.type_names[move_type as usize] == "???" {
            return;
        }
        if self.types_of(r) == [move_type, NO_TYPE, NO_TYPE] {
            return;
        }
        self.set_type(r, [move_type, NO_TYPE]);
        let st = &mut self.mon_mut(r).ability_state;
        if libero {
            st.libero = Some(true);
        } else {
            st.protean = Some(true);
        }
        let (ri, ty) = (
            self.ident(r),
            self.dex.type_names[move_type as usize].clone(),
        );
        let name = self.ability_name(r);
        log_line!(self, "|-start|{ri}|typechange|{ty}|[from] ability: {name}");
    }

    /// `runEvent('ModifyMove')` for the abilities on the user: Stance
    /// Change (priority 1) changes forme; Mold Breaker makes the move ignore
    /// abilities.
    /// frz's onBeforeMove and onModifyMove: a defrost move thaws its user,
    /// except Burn Up from a user that isn't Fire type (which goes through
    /// the freeze check).
    fn defrosts(&self, r: MonRef, id: MoveId) -> bool {
        let mv = self.dex.mv(id);
        mv.has(MoveFlag::Defrost)
            && !(self.dex.move_kind[id as usize] == MoveKind::BurnUp
                && !self
                    .dex
                    .ids
                    .types
                    .fire
                    .is_some_and(|f| self.has_type_of(r, f)))
    }

    pub(crate) fn run_ability_modify_move(&mut self, r: MonRef, mv: &Move) {
        let kind = self.ability_kind(r);
        self.active_move.ignore_evasion = (kind == AbilityKind::KeenEye).then_some(r);
        match kind {
            AbilityKind::StanceChange => self.stance_change(r, mv),
            AbilityKind::MoldBreaker => self.ability_breaker = Some(r),
            _ => {}
        }
    }

    /// Sheer Force's `onModifyMove`: a copy of a move with secondary
    /// effects, without them (and so with its power boost).
    pub(crate) fn sheer_force_on_modify_move(&self, r: MonRef, mv: &Move) -> Option<Move> {
        (self.has_ability(r, crate::kinds::AbilityKind::SheerForce)
            && !mv.secondaries.is_empty()
            && !mv.has_sheer_force_boost)
            .then(|| Move {
                secondaries: Vec::new(),
                self_boosts: None,
                self_volatile: None,
                sheer_force: true,
                ..mv.clone()
            })
    }

    /// Stench's `onModifyMove` (priority -1): a damaging move without a
    /// flinch secondary gets a 10% one. A delayed hit never ran ModifyMove.
    pub(crate) fn stench_on_modify_move(&self, r: MonRef, mv: &Move) -> Option<Move> {
        if !self.has_ability(r, AbilityKind::Stench)
            || mv.category == Category::Status
            || mv.has(MoveFlag::FutureMove)
            || mv
                .secondaries
                .iter()
                .any(|s| s.volatile.as_deref() == Some("flinch"))
        {
            return None;
        }
        let mut m = mv.clone();
        m.secondaries.push(crate::dex::Secondary {
            chance: Some(10),
            volatile: Some("flinch".into()),
            ..Default::default()
        });
        Some(m)
    }

    /// Stalwart's `onModifyMove`: the move tracks its target (no redirection).
    pub(crate) fn stalwart_on_modify_move(&self, r: MonRef) -> bool {
        self.has_ability(r, AbilityKind::Stalwart)
    }

    /// `getTarget` for a `tracksTarget` move (Snipe Shot) or a Stalwart
    /// user: the original target while it is still active, wherever it now
    /// stands.
    pub(crate) fn target_ignoring_redirection(
        &self,
        r: MonRef,
        tracks: bool,
        original: Option<SideName>,
    ) -> Option<MonRef> {
        if !tracks && !self.has_ability(r, AbilityKind::Stalwart) {
            return None;
        }
        let (s, name) = original.as_ref()?;
        let i = self.sides[*s].mons.iter().position(|m| &m.name == name)?;
        let m = &self.sides[*s].mons[i];
        (m.is_active && !m.fainted && !m.revived).then_some((*s, i))
    }

    /// The flinch volatile's `runEvent('Flinch')`: Steadfast raises Speed.
    pub(crate) fn steadfast_on_flinch(&mut self, r: MonRef) {
        if self.has_ability(r, AbilityKind::Steadfast) {
            let mut b = [0i8; 7];
            b[Boost::Spe as usize] = 1;
            let a = self.mon(r).ability;
            self.boost(r, &b, None, EffectRef::Ability(a), false, false);
        }
    }
}
