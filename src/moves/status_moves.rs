//! The effects of status moves, Showdown's `runMoveEffects` for them.

use super::hit::{Did, TryHit};
use crate::battle::Small;
use crate::battle::{Battle, MonRef};
use crate::dex::events::Event;
use crate::dex::types::*;
use crate::dex::Move;
use crate::events::immunity::StatusHandler;
use crate::kinds::EffectRef;
use crate::kinds::MoveKind;
use crate::state::*;

/// Status moves on the target handled in this file.
pub(crate) fn status_move_kind(k: MoveKind) -> bool {
    matches!(
        k,
        MoveKind::WeatherHeal
            | MoveKind::Rest
            | MoveKind::Wish
            | MoveKind::HealingWish
            | MoveKind::Swallow
            | MoveKind::Defog
            | MoveKind::CorrosiveGas
            | MoveKind::TidyUp
    )
}

impl Battle {
    pub(super) fn status_move_hit(
        &mut self,
        r: MonRef,
        targets: &[MonRef],
        mv: &Move,
        kind: MoveKind,
    ) -> Small<MonRef> {
        self.active_move.not_fail.clear();
        // Whether the move did something to any target (`runMoveEffects`).
        let mut any = false;
        let mut plain_fail = false;
        let mut hit = Small::new();
        let mut blocked: Small<MonRef> = Small::new();
        // spreadMoveHit's checks run for every target before any effect.
        let mut reached: Small<MonRef> = Small::new();
        for &t in targets {
            // The move's own TryHit comes first: Yawn fails on a Pokémon
            // with a status.
            if kind == MoveKind::Yawn && self.mon(t).status != Status::None {
                self.fail_move(r);
                continue;
            }
            if self.move_on_try_hit(r, t, kind) {
                continue;
            }
            // TryPrimaryHit: a substitute blocks the move (its damage
            // calculation for a status move comes back empty). The result is
            // null, not false: the hit loop keeps the target, and the move
            // counts as a success, though it did nothing.
            if self.hits_substitute(r, t, mv) {
                self.fail_move(r);
                hit.push(t);
                blocked.push(t);
                continue;
            }
            reached.push(t);
        }
        for t in reached {
            let ti = self.ident(t);
            let ok = match kind {
                MoveKind::Substitute => {
                    // onTryHit: one substitute at a time, and it costs a
                    // quarter of max HP (NOT_FAIL: no `[still]`).
                    let m = self.mon(t);
                    if m.vol.substitute_halves > 0 {
                        log_line!(self, "|-fail|{ti}|move: Substitute");
                        false
                    } else if m.hp as u32 * 4 <= m.maxhp as u32 || m.maxhp == 1 {
                        log_line!(self, "|-fail|{ti}|move: Substitute|[weak]");
                        false
                    } else {
                        // TryAddVolatile, the volatile starts, then onHit pays the cost.
                        let quarter = m.maxhp / 4;
                        self.try_add_volatile_sort(t);
                        self.mon_mut(t).vol.substitute_halves = quarter * 2;
                        log_line!(self, "|-start|{ti}|Substitute");
                        self.end_partial_trap_silently(t);
                        self.direct_damage(t, quarter as u32, "");
                        true
                    }
                }
                MoveKind::Protect(p) => {
                    // The protect volatile (TryAddVolatile, then its start),
                    // then onHit adds or restarts 'stall'.
                    self.try_add_volatile_sort(t);
                    self.mon_mut(t).vol.protect = Some(p);
                    self.stamp_volatile(t, ResidualVol::Protect);
                    log_line!(self, "|-singleturn|{ti}|{}", p.start_name());
                    self.add_stall(t);
                    true
                }
                MoveKind::HelpingHand => {
                    // onTryHit: fails if the ally has already moved (and didn't just switch in).
                    if self.mon(t).active_turns > 0 && !self.has_queued_move(t) {
                        self.fail_move(r);
                        false
                    } else {
                        if self.mon(t).vol.helping_hand == 0 {
                            // addVolatile's TryAddVolatile (Flower Veil, Sweet
                            // Veil ...) sorts its handlers; onRestart doesn't.
                            self.try_add_volatile_sort(t);
                            self.stamp_volatile(t, ResidualVol::HelpingHand);
                        }
                        self.mon_mut(t).vol.helping_hand += 1;
                        let ri = self.ident(r);
                        log_line!(self, "|-singleturn|{ti}|Helping Hand|[of] {ri}");
                        true
                    }
                }
                // addVolatile fails on a volatile already there (an instructed
                // second use in the turn).
                MoveKind::FollowMe | MoveKind::RagePowder
                    if self.mon(t).vol.follow_me && kind == MoveKind::FollowMe
                        || self.mon(t).vol.rage_powder && kind == MoveKind::RagePowder =>
                {
                    self.fail_move(r);
                    false
                }
                MoveKind::FollowMe => {
                    // addVolatile: TryAddVolatile sorts first.
                    self.try_add_volatile_sort(t);
                    self.mon_mut(t).vol.follow_me = true;
                    self.stamp_volatile(t, ResidualVol::FollowMe);
                    log_line!(self, "|-singleturn|{ti}|move: Follow Me");
                    true
                }
                MoveKind::RagePowder => {
                    self.try_add_volatile_sort(t);
                    self.mon_mut(t).vol.rage_powder = true;
                    self.stamp_volatile(t, ResidualVol::RagePowder);
                    log_line!(self, "|-singleturn|{ti}|move: Rage Powder");
                    true
                }
                MoveKind::PartingShot => {
                    let mut b = [0i8; 7];
                    b[Boost::Atk as usize] = -1;
                    b[Boost::Spa as usize] = -1;
                    let effect = self.move_ref(mv);
                    let worked = self.boost(t, &b, Some(r), effect, false, false);
                    if worked || self.dex.ability(self.mon(t).ability).id == "mirrorarmor" {
                        self.set_self_switch(r, mv.idx);
                    }
                    // onHit returns nothing: the move counts as doing something.
                    true
                }
                MoveKind::Taunt => {
                    if self.mon(t).vol.taunt > 0 {
                        self.fail_move(r);
                        false
                    } else if self.run_try_add_volatile(t, "taunt", true) {
                        false
                    } else {
                        // One more turn if the target has already moved this turn.
                        let extra = self.mon(t).active_turns > 0 && !self.has_queued_move(t);
                        self.mon_mut(t).vol.taunt = 3 + extra as u8;
                        self.stamp_volatile(t, ResidualVol::Taunt);
                        log_line!(self, "|-start|{ti}|move: Taunt");
                        true
                    }
                }
                MoveKind::Encore => self.encore(r, t),
                MoveKind::Trick => self.trick(r, t, mv),
                MoveKind::SkillSwap => self.skill_swap(r, t),
                // onHit returns transformInto's result: false fails the move.
                MoveKind::Transform => {
                    let ok = self.transform_into(r, t, false);
                    if !ok {
                        self.fail_move(r);
                    }
                    ok
                }
                MoveKind::Soak => {
                    // onHit: fails on a pure Water type; otherwise setType.
                    let water = self.dex.ids.types.water.unwrap_or(0);
                    if self.types_of(t) == [water, NO_TYPE, NO_TYPE] {
                        log_line!(self, "|-fail|{ti}");
                        false
                    } else {
                        self.set_type(t, [water, NO_TYPE]);
                        log_line!(self, "|-start|{ti}|typechange|Water");
                        true
                    }
                }
                MoveKind::Yawn => self.add_yawn(r, t),
                MoveKind::RevivalBlessing => {
                    let slot = self.mon(r).position;
                    self.sides[r.0].revival[slot] = true;
                    if self.mon(r).hp > 0 {
                        let m = self.mon_mut(r);
                        m.switch_flag = true;
                        m.switch_from_move = Some(mv.idx);
                    }
                    true
                }
                k if crate::moves::status_moves::status_move_kind(k) => {
                    self.status_move_on_hit(r, t, mv, k)
                }
                k if Self::handles_status_move(k) => self.status_move_effects(r, t, mv, k),
                MoveKind::ClangorousSoul => {
                    // onTryHit: raise every stat (null if none can rise), then
                    // onHit: lose a third of max HP.
                    let mut b = [0i8; 7];
                    for st in [Boost::Atk, Boost::Def, Boost::Spa, Boost::Spd, Boost::Spe] {
                        b[st as usize] = 1;
                    }
                    let effect = self.move_ref(mv);
                    if self.boost(r, &b, Some(r), effect, false, false) {
                        let amount = (self.mon(r).maxhp as u32 * 33 / 100).max(1);
                        self.direct_damage(r, amount, "");
                        true
                    } else {
                        false
                    }
                }
                _ => {
                    let did = self.primary_effects(r, t, mv, true);
                    plain_fail |= did == Did::Fail;
                    matches!(did, Did::Done | Did::Nothing)
                }
            };
            any |= ok;
            if ok {
                // The Hit event (Focus Punch loses focus to an ally's Pollen Puff).
                self.beak_blast_on_hit(r, t, mv);
                hit.push(t);
            }
        }
        // runMoveEffects: a move whose every effect returned false fails
        // (one with a `self` effect doesn't).
        if !any && plain_fail && mv.self_boosts.is_none() && mv.self_volatile.is_none() {
            self.fail_move(r);
        }
        // selfDrops runs for targets the move didn't fail on, and consumes a
        // roll even without a chance.
        if let (Some(b), true) = (mv.self_boosts, any) {
            let _ = self.rng.random(100);
            let effect = self.move_ref(mv);
            self.boost_listed(r, &b, Some(r), effect, false, true);
        }
        if let (Some(v), true) = (&mv.self_volatile, any) {
            self.add_self_volatile(r, v);
        }
        if mv.force_switch {
            for &t in hit.iter().filter(|t| !blocked.contains(t)) {
                self.force_switch_target(r, t);
            }
        }
        // A substitute's null, a failure and NOT_FAIL (Heal Pulse at full HP)
        // all drop the target from targetsCopy (no gotAttacked).
        let reached: Vec<MonRef> = hit
            .iter()
            .copied()
            .filter(|t| !blocked.contains(t) && !self.active_move.not_fail.contains(t))
            .collect();
        self.active_move.status_reached = reached;
        hit
    }

    /// Perish Song's `onHitField`: each active Pokémon the move reaches (not
    /// stopped by TryHit: Soundproof, Good as Gold, Psychic Terrain against
    /// a Prankster one) and not already counting down starts at three.
    /// False when it reached none.
    /// `move_type` is after ModifyType: Liquid Voice makes it Water, which
    /// Dry Skin and Water Absorb take in TryHit.
    pub(super) fn perish_song(&mut self, r: MonRef, mv: &Move, move_type: TypeId) -> bool {
        let mut result = false;
        let mut message = false;
        for p in self.all_active() {
            // Invulnerability first: the move misses a Pokémon out of reach.
            if self.invulnerable(r, p, &mv.id) {
                let (ri, pi) = (self.ident(r), self.ident(p));
                log_line!(self, "|-miss|{ri}|{pi}");
                result = true;
                continue;
            }
            let blocked = p != r
                && (self.try_hit(r, p, mv, 4) != TryHit::Hit
                    || self.try_hit(r, p, mv, 3) != TryHit::Hit
                    || self.ability_try_hit(r, p, mv, move_type));
            if blocked {
                result = true;
            } else if self.mon(p).vol.perish == 0 {
                self.try_add_volatile_sort(p);
                self.mon_mut(p).vol.perish = 4;
                self.stamp_volatile(p, ResidualVol::Perish);
                let pi = self.ident(p);
                log_line!(self, "|-start|{pi}|perish3|[silent]");
                result = true;
                message = true;
            }
        }
        if message {
            self.add("|-fieldactivate|move: Perish Song");
        }
        result
    }

    /// The forceSwitch step (Roar, Dragon Tail): mark the target to be
    /// dragged out after the action, unless Guard Dog or Suction Cups hold it.
    pub(super) fn force_switch_target(&mut self, src: MonRef, t: MonRef) {
        if self.mon(t).hp == 0 || self.mon(src).hp == 0 || !self.can_switch_side(t.0) {
            return;
        }
        if self.drag_out_blocked(t) {
            return;
        }
        self.mon_mut(t).force_switch_flag = true;
    }

    /// `runEvent('DragOut')`: Guard Dog and Suction Cups, then Ingrain, hold
    /// the Pokémon in place (each announcing itself).
    pub(crate) fn drag_out_blocked(&mut self, t: MonRef) -> bool {
        if matches!(
            self.ability_kind(t),
            crate::kinds::AbilityKind::GuardDog | crate::kinds::AbilityKind::SuctionCups
        ) {
            let ti = self.ident(t);
            let name = self.ability_name(t);
            log_line!(self, "|-activate|{ti}|ability: {name}");
            return true;
        }
        self.ingrain_on_drag_out(t)
    }

    /// Trick, Switcheroo: swap held items. Fails when neither holds one, or
    /// a Mega Stone would leave or reach a Pokémon that uses it (TakeItem).
    fn trick(&mut self, src: MonRef, t: MonRef, mv: &Move) -> bool {
        let yours = self.mon(t).item;
        let mine = self.mon(src).item;
        // takeItem on the target, then the user, before any check: each
        // runs TakeItem, where Unburden (an ability, so before the item's
        // own handler) takes note even if the swap then fails.
        for holder in [t, src] {
            if self.mon(holder).item.is_some() {
                self.after_use_item(holder);
            }
        }
        let stuck =
            |b: &Battle, holder: MonRef| b.mon(holder).item.is_some() && !b.item_removable(holder);
        let refused = |b: &Battle, receiver: MonRef, item: Option<ItemId>| {
            item.map(|i| !b.item_receivable(receiver, i))
                .unwrap_or(false)
        };
        if (yours.is_none() && mine.is_none())
            || stuck(self, t)
            || stuck(self, src)
            || refused(self, t, mine)
            || refused(self, src, yours)
        {
            self.fail_move(src);
            return false;
        }
        let (si, ti) = (self.ident(src), self.ident(t));
        log_line!(self, "|-activate|{si}|move: Trick|[of] {ti}");
        for (who, gets, lost) in [(t, mine, yours), (src, yours, mine)] {
            self.mon_mut(who).item = gets;
            let wi = self.ident(who);
            match gets {
                Some(i) => {
                    // setItem: the item's Start (Air Balloon, seeds), then the message.
                    self.item_on_start(who);
                    let name = self.dex.item(i).name.clone();
                    log_line!(self, "|-item|{wi}|{name}|[from] move: {}", mv.name);
                }
                None => {
                    let name = lost
                        .map(|i| self.dex.item(i).name.clone())
                        .unwrap_or_default();
                    log_line!(
                        self,
                        "|-enditem|{wi}|{name}|[silent]|[from] move: {}",
                        mv.name
                    );
                }
            }
        }
        true
    }

    /// Showdown `skillSwap`: the two abilities trade places, unless either
    /// is flagged `failskillswap`. Each old ability ends on its holder, then
    /// each starts on its new one (the user's first). Between allies the
    /// message doesn't name them.
    fn skill_swap(&mut self, src: MonRef, t: MonRef) -> bool {
        let done = self.skill_swap_quiet(src, t);
        if !done {
            self.fail_move(src);
        }
        done
    }

    /// `skillSwap` itself: false, without a message, when it can't happen
    /// (Wandering Spirit's swap; the move adds its `-fail`).
    pub(crate) fn skill_swap_quiet(&mut self, src: MonRef, t: MonRef) -> bool {
        let (sa, ta) = (self.mon(src).ability, self.mon(t).ability);
        let blocked =
            |b: &Battle, a: AbilityId| b.dex.ability(a).flags.iter().any(|f| f == "failskillswap");
        if self.mon(src).fainted || self.mon(t).fainted || blocked(self, sa) || blocked(self, ta) {
            return false;
        }
        let (si, ti) = (self.ident(src), self.ident(t));
        if src.0 == t.0 {
            log_line!(self, "|-activate|{si}|Skill Swap|||[of] {ti}");
        } else {
            let (sn, tn) = (
                self.dex.ability(sa).name.clone(),
                self.dex.ability(ta).name.clone(),
            );
            log_line!(self, "|-activate|{si}|Skill Swap|{tn}|{sn}|[of] {ti}");
        }
        self.ability_on_end(src);
        self.ability_on_end(t);
        for (who, a) in [(src, ta), (t, sa)] {
            self.mon_mut(who).ability = a;
            self.reset_ability_state(who);
        }
        self.run_start(t);
        self.run_start(src);
        true
    }

    /// Encore (Champions): lock the target into its last move for 3 turns
    /// (4 if it has already moved); a target still to move this turn has its
    /// action changed to that move.
    fn encore(&mut self, r: MonRef, t: MonRef) -> bool {
        // addVolatile: TryAddVolatile (Aroma Veil) before the start's checks.
        if self.mon(t).vol.encore.is_none() && self.run_try_add_volatile(t, "encore", true) {
            return false;
        }
        let m = self.mon(t);
        let last = m.last_move.filter(|&lm| {
            !self.dex.mv(lm).has(MoveFlag::FailEncore)
                && m.moves.iter().any(|ms| ms.id == lm && ms.pp > 0)
        });
        let Some(lm) = last.filter(|_| m.vol.encore.is_none()) else {
            self.fail_move(r);
            return false;
        };
        let ti = self.ident(t);
        log_line!(self, "|-start|{ti}|Encore");
        let mut duration = 3;
        match self.queued_move_slot(t) {
            None => duration += 1,
            Some(slot) => {
                let queued = if slot < EXTERNAL {
                    Some(self.mon(t).moves[slot].id)
                } else {
                    None
                };
                // Champions: a Mental Herb holder keeps its chosen move.
                if queued != Some(lm)
                    && self.item_kind(t) != Some(crate::kinds::ItemKind::MentalHerb)
                {
                    let s = self.mon(t).moves.iter().position(|ms| ms.id == lm).unwrap();
                    self.change_action_to_move(t, s);
                }
            }
        }
        self.stamp_volatile(t, ResidualVol::Encore);
        let v = &mut self.mon_mut(t).vol;
        v.encore = Some(lm);
        v.encore_duration = duration;
        true
    }

    /// Yawn's volatile: blocked by Insomnia and Vital Spirit, by Flower Veil
    /// for Grass types, and by Electric Terrain (TryAddVolatile).
    fn add_yawn(&mut self, src: MonRef, t: MonRef) -> bool {
        if self.mon(t).vol.yawn > 0 {
            self.fail_move(src);
            return false;
        }
        let ri = self.ident(src);
        let ti = self.ident(t);
        // runEvent('TryAddVolatile'): the handlers act in their sorted
        // order, so a faster ally's Sweet Veil blocks before the target's
        // own Insomnia, and of two Sweet Veil holders the faster one does.
        let grass = self
            .dex
            .type_id("Grass")
            .is_some_and(|g| self.has_type_of(t, g));
        let sun = self.effective_weather() == Some(crate::state::Weather::Sun);
        let holders = self.try_add_volatile_sort(t);
        // The terrain and Safeguard have no speed (0): ahead of an ability
        // whose holder's speed is 0 or less (Trick Room negates it).
        let mut field_done = false;
        for h in holders {
            use crate::kinds::AbilityKind as A;
            if !field_done && self.mon(h).speed <= 0 {
                field_done = true;
                if self.yawn_field_blocks(t, src) {
                    return false;
                }
            }
            let block = match self.ability_kind(h) {
                A::StatusImmune(Status::Slp) | A::PurifyingSalt if h == t => "immune",
                A::LeafGuard if h == t && sun => "immune",
                A::SweetVeil => "block",
                A::FlowerVeil if grass => "block",
                _ => continue,
            };
            let name = self.ability_name(h);
            if block == "immune" {
                log_line!(self, "|-immune|{ti}|[from] ability: {name}");
            } else {
                let hi = self.ident(h);
                log_line!(self, "|-block|{ti}|ability: {name}|[of] {hi}");
            }
            return false;
        }
        if !field_done && self.yawn_field_blocks(t, src) {
            return false;
        }
        self.mon_mut(t).vol.yawn = 2;
        self.stamp_volatile(t, ResidualVol::Yawn);
        log_line!(self, "|-start|{ti}|move: Yawn|[of] {ri}");
        true
    }

    /// Yawn's `TryAddVolatile` handlers that have no speed: Electric Terrain
    /// (subOrder 0) goes before Safeguard (side condition, 4).
    fn yawn_field_blocks(&mut self, t: MonRef, src: MonRef) -> bool {
        if self.field.terrain == Some(Terrain::Electric)
            && self.is_grounded(t)
            && !self.semi_invulnerable(t)
        {
            let ti = self.ident(t);
            log_line!(self, "|-activate|{ti}|move: Electric Terrain");
            return true;
        }
        self.safeguard_on_try_add_volatile(t, Some(src), true)
    }

    /// Disable from Cursed Body: the attacker's last move, for 4 turns (5,
    /// less the turn in progress).
    pub(crate) fn add_disable(&mut self, r: MonRef, holder: MonRef) {
        let m = self.mon(r);
        if m.hp == 0 || m.vol.disable.is_some() {
            return;
        }
        // TryAddVolatile (Aroma Veil, silent for an ability's Disable) comes
        // before the start's checks.
        if self.run_try_add_volatile(r, "disable", false) {
            return;
        }
        let m = self.mon(r);
        let Some(lm) = m.last_move else { return };
        if m.moves.iter().any(|ms| ms.id == lm && ms.pp == 0) {
            return;
        }
        self.stamp_volatile(r, ResidualVol::Disable);
        let v = &mut self.mon_mut(r).vol;
        v.disable = Some(lm);
        v.disable_duration = 4;
        let ri = self.ident(r);
        let hi = self.ident(holder);
        let name = self.dex.mv(lm).name.clone();
        let ability = self.ability_name(holder);
        log_line!(
            self,
            "|-start|{ri}|Disable|{name}|[from] ability: {ability}|[of] {hi}"
        );
    }

    /// A status move's effect on one target (`onHit`, or its slot
    /// condition). Returns whether it did something.
    pub(crate) fn status_move_on_hit(
        &mut self,
        r: MonRef,
        t: MonRef,
        mv: &Move,
        kind: MoveKind,
    ) -> bool {
        match kind {
            MoveKind::WeatherHeal => {
                self.mega_sol_message(r);
                let modifier = match self.move_weather() {
                    Some(Weather::Sun) => 2732,
                    Some(_) => 1024,
                    None => 2048,
                };
                let amount = crate::moves::damage::modify_4096(self.mon(t).maxhp as u32, modifier);
                // At full HP: NOT_FAIL, which the hit loop counts as a hit.
                if self.heal(t, amount, "") == 0 {
                    let ti = self.ident(t);
                    log_line!(self, "|-fail|{ti}|heal");
                    self.note_not_fail(t);
                }
                true
            }
            MoveKind::Rest => self.rest(t, mv),
            MoveKind::TidyUp => self.tidy_up(r, mv),
            MoveKind::CorrosiveGas => {
                // takeItem(source): TakeItem (Unburden takes note, a Mega Stone
                // refuses), then the message; onHit returns nothing, so the
                // move counts as done either way.
                let (ti, ri) = (self.ident(t), self.ident(r));
                let held = self.mon(t).item;
                match held {
                    // Sticky Hold refuses the takeItem, so the move fails.
                    Some(_) if self.sticky_hold_on_take_item(t, Some(r), false) => {
                        log_line!(self, "|-fail|{ti}|move: Corrosive Gas");
                    }
                    Some(item) => {
                        self.after_use_item(t);
                        if self.item_removable(t) {
                            self.mon_mut(t).item = None;
                            let name = self.dex.item(item).name.clone();
                            log_line!(
                                self,
                                "|-enditem|{ti}|{name}|[from] move: Corrosive Gas|[of] {ri}"
                            );
                        } else {
                            log_line!(self, "|-fail|{ti}|move: Corrosive Gas");
                        }
                    }
                    None => log_line!(self, "|-fail|{ti}|move: Corrosive Gas"),
                }
                true
            }
            MoveKind::Wish => {
                let slot = &mut self.slot_conds[t.0][t.1];
                if slot.wish.is_some() {
                    self.fail_move(r);
                    return false;
                }
                let m = self.mon(r);
                let wish = WishState {
                    hp: m.maxhp as f64 / 2.0,
                    starting_turn: self.turn.wrapping_sub(1) & 255,
                    source_slot: r.1,
                    wisher: m.name.to_string(),
                };
                self.slot_conds[t.0][t.1].wish = Some(wish);
                true
            }
            MoveKind::HealingWish => {
                // addSlotCondition fails on a wish still waiting in the slot
                // (its user came in unhurt). selfdestruct 'ifHit' still faints
                // the user (runMoveEffects checks it before the result is
                // combined); the move did nothing, without a `-fail`.
                let slot = &mut self.slot_conds[t.0][t.1];
                let added = slot.healing_wish.is_none();
                if added {
                    slot.healing_wish = Some(r.1);
                }
                self.mon_mut(r).hp = 0;
                self.queue_faint(r);
                added
            }
            MoveKind::Defog => {
                let ok = self.defog(r, t, mv);
                if !ok {
                    self.fail_move(r);
                }
                ok
            }
            // onHit: heal 1/4, 1/2 or all by the layers, then the stockpile goes.
            MoveKind::Swallow => {
                let layers = self.mon(t).vol.stockpile.clamp(1, 3);
                let modifier = [1024, 2048, 4096][layers as usize - 1];
                let amount = crate::moves::damage::modify_4096(self.mon(t).maxhp as u32, modifier);
                let healed = self.heal(t, amount, "") > 0;
                if !healed {
                    let ti = self.ident(t);
                    log_line!(self, "|-fail|{ti}|heal");
                }
                self.end_stockpile(t);
                if !healed {
                    self.note_not_fail(t);
                }
                // Failing to heal returns NOT_FAIL: still a hit.
                true
            }
            _ => false,
        }
    }

    /// Rest's `onHit`: `setStatus('slp')` replaces any other status (its
    /// start draws the usual 2-3 turns, then Rest makes it 3), and the
    /// Pokémon heals fully.
    pub(crate) fn rest(&mut self, t: MonRef, mv: &Move) -> bool {
        let ti = self.ident(t);
        self.event_sort(Event::Immunity, t, None, &[]);
        // runEvent('SetStatus') in Speed order: an ability or terrain
        // returns false (the move fails), Sweet Veil and Uproar null (it
        // fails quietly).
        match self.run_set_status(
            t,
            Status::Slp,
            t,
            crate::kinds::EffectRef::Move(mv.idx),
            false,
            true,
        ) {
            Some(StatusHandler::Own | StatusHandler::Terrain) => {
                self.fail_move(t);
                return false;
            }
            Some(_) => return false,
            None => {}
        }
        let _ = self.rng.sample(&[2u8, 3, 3]);
        let m = self.mon_mut(t);
        m.status = Status::Slp;
        m.status_state = Status::Slp;
        m.status_time = 3;
        m.status_start_time = 3;
        m.toxic_stage = 0;
        log_line!(self, "|-status|{ti}|slp|[from] move: {}", mv.name);
        self.after_set_status(t, Status::Slp, t, crate::kinds::EffectRef::Move(mv.idx));
        // Rest sets the times on whatever status state is left (a Lum
        // Berry's cure leaves an empty one).
        let m = self.mon_mut(t);
        m.status_state = Status::Slp;
        m.status_time = 3;
        m.status_start_time = 3;
        let maxhp = self.mon(t).maxhp as u32;
        self.heal(t, maxhp, "|[silent]");
        true
    }

    /// Status moves whose effect on a target this module handles
    /// (`status_move_hit`).
    pub(crate) fn handles_status_move(kind: MoveKind) -> bool {
        matches!(
            kind,
            MoveKind::PsychUp
                | MoveKind::FocusEnergy
                | MoveKind::DragonCheer
                | MoveKind::BellyDrum
                | MoveKind::Acupressure
                | MoveKind::HealPulse
                | MoveKind::Growth
                | MoveKind::Endure
                | MoveKind::MagicPowder
                | MoveKind::Memento
                | MoveKind::AllySwitch
                | MoveKind::BatonPass
                | MoveKind::AfterYou
                | MoveKind::Quash
                | MoveKind::SwapBoosts
                | MoveKind::SpeedSwap
                | MoveKind::Split
                | MoveKind::TopsyTurvy
                | MoveKind::PainSplit
                | MoveKind::DestinyBond
                | MoveKind::Curse
                | MoveKind::Entrainment
                | MoveKind::RolePlay
                | MoveKind::WorrySeed
                | MoveKind::Trap
                | MoveKind::LockOn
                | MoveKind::PollenPuff
                | MoveKind::Recycle
                | MoveKind::ReflectType
                | MoveKind::ShedTail
                | MoveKind::Spite
                | MoveKind::StrengthSap
                | MoveKind::StuffCheeks
                | MoveKind::AddType
                | MoveKind::SimpleBeam
                | MoveKind::Copycat
                | MoveKind::SleepTalk
                | MoveKind::Instruct
        )
    }

    /// A status move's effect on one target (`runMoveEffects`):
    /// whether it did something. A failure is announced here.
    pub(crate) fn status_move_effects(
        &mut self,
        r: MonRef,
        t: MonRef,
        mv: &Move,
        kind: MoveKind,
    ) -> bool {
        match kind {
            // onTryHit made it a heal (half the ally's max HP): nothing to
            // heal is NOT_FAIL.
            MoveKind::PollenPuff => {
                let amount = self.mon(t).maxhp as u32 / 2;
                // Heal Block's onTryHeal on the target: the heal names Pollen
                // Puff as `cant` (`[still]` on the move line) unless the
                // target is at full HP.
                if self.mon(t).vol.heal_block > 0 && self.mon(t).hp != self.mon(t).maxhp {
                    if let Some(line) = self.last_move_line {
                        self.attr_move_line(line, "[still]");
                    }
                    let ri = self.ident(r);
                    log_line!(self, "|cant|{ri}|move: Heal Block|{}", mv.name);
                    self.note_not_fail(t);
                    return true;
                }
                if self.heal(t, amount, "") == 0 {
                    self.note_not_fail(t);
                } else if (self.kings_rock_flinch(r, mv)
                    || mv
                        .secondaries
                        .iter()
                        .any(|s| s.volatile.as_deref() == Some("flinch")))
                    && self.rng.random(100) < 10
                {
                    // King's Rock's or Stench's secondary rolls for a target
                    // that was healed.
                    self.add_volatile(t, "flinch", false);
                }
                true
            }
            MoveKind::LockOn => {
                self.lock_on(r, t);
                true
            }
            MoveKind::Recycle => self.recycle(r, t),
            MoveKind::ReflectType => self.reflect_type(r, t),
            MoveKind::ShedTail => self.shed_tail(r, t, mv),
            MoveKind::Spite => self.spite(r, t),
            MoveKind::StrengthSap => self.strength_sap(r, t, mv),
            MoveKind::StuffCheeks => self.stuff_cheeks(r, mv),
            MoveKind::FocusEnergy | MoveKind::DragonCheer => {
                let v = if kind == MoveKind::FocusEnergy {
                    "focusenergy"
                } else {
                    "dragoncheer"
                };
                match self.add_volatile(t, v, true) {
                    Some(true) => true,
                    Some(false) => {
                        self.fail_move(r);
                        false
                    }
                    None => false,
                }
            }
            MoveKind::PsychUp => {
                self.psych_up(r, t);
                true
            }
            MoveKind::BellyDrum => {
                let m = self.mon(t);
                if m.hp as u32 * 2 <= m.maxhp as u32
                    || m.boosts[Boost::Atk as usize] >= 6
                    || m.maxhp == 1
                {
                    self.fail_move(r);
                    return false;
                }
                let half = m.maxhp as u32 / 2;
                self.direct_damage(t, half, "");
                let mut b = [0i8; 7];
                b[Boost::Atk as usize] = 12;
                self.boost(t, &b, Some(r), EffectRef::Move(mv.idx), false, false);
                true
            }
            MoveKind::Acupressure => {
                let stats: Vec<usize> = (0..7).filter(|&i| self.mon(t).boosts[i] < 6).collect();
                if stats.is_empty() {
                    self.fail_move(r);
                    return false;
                }
                let stat = self.rng.sample(&stats);
                let mut b = [0i8; 7];
                b[stat] = 2;
                self.boost(t, &b, Some(r), EffectRef::Move(mv.idx), false, false);
                true
            }
            MoveKind::HealPulse => {
                let maxhp = self.mon(t).maxhp as u32;
                let amount = if self.ability_id(r) == "megalauncher" {
                    crate::moves::damage::modify(maxhp, 3, 4)
                } else {
                    maxhp.div_ceil(2)
                };
                if self.heal(t, amount, "") == 0 {
                    // NOT_FAIL: no `[still]`, and the move counts as done.
                    let ti = self.ident(t);
                    log_line!(self, "|-fail|{ti}|heal");
                    self.note_not_fail(t);
                }
                true
            }
            MoveKind::Growth => {
                let sun = self.move_weather() == Some(Weather::Sun);
                let did = if sun {
                    let mut grown = mv.clone();
                    if let Some(b) = grown.boosts.as_mut() {
                        for x in b.vals.iter_mut().filter(|x| **x != 0) {
                            *x = 2;
                        }
                    }
                    self.primary_effects(r, t, &grown, true)
                } else {
                    self.primary_effects(r, t, mv, true)
                };
                matches!(did, Did::Done | Did::Nothing)
            }
            MoveKind::Endure => {
                // The volatile (TryAddVolatile, then its start), then onHit's stall.
                self.try_add_volatile_sort(t);
                self.mon_mut(t).vol.endure = true;
                self.stamp_volatile(t, ResidualVol::Endure);
                let ti = self.ident(t);
                log_line!(self, "|-singleturn|{ti}|move: Endure");
                self.add_stall(t);
                true
            }
            MoveKind::MagicPowder => {
                let psychic = self.dex.ids.types.psychic.unwrap_or(0);
                if self.types_of(t) == [psychic, NO_TYPE, NO_TYPE] {
                    self.fail_move(r);
                    return false;
                }
                self.set_type(t, [psychic, NO_TYPE]);
                let ti = self.ident(t);
                log_line!(self, "|-start|{ti}|typechange|Psychic");
                true
            }
            MoveKind::AllySwitch => {
                // onHit: swap with the ally, or fail without counting as a
                // failure; always in singles (no ally to swap with).
                let new = if r.1 == 0 { 1 } else { 0 };
                match self.active(r.0, new) {
                    Some(a) if self.sides[r.0].n_active > 1 && !self.mon(a).fainted => {
                        self.swap_position(r, new, "|[from] move: Ally Switch")
                    }
                    _ => {
                        let ri = self.ident(r);
                        log_line!(self, "|-fail|{ri}|move: Ally Switch");
                        if let Some(line) = self.last_move_line {
                            self.attr_move_line(line, "[still]");
                        }
                    }
                }
                true
            }
            MoveKind::BatonPass => {
                // onHit fails (NOT_FAIL) with nothing to switch to; then
                // selfSwitch makes the move fail.
                if !self.can_switch_side(r.0) {
                    self.fail_move(r);
                    return false;
                }
                // self.onHit, then selfSwitch.
                self.mon_mut(r).skip_before_switch_out = true;
                self.set_self_switch(r, mv.idx);
                true
            }
            MoveKind::AfterYou | MoveKind::Quash => {
                // The target's queued move goes first (order 3, at the front)
                // or last (order 201, re-sorted after this action).
                let at = self.queue.iter().position(
                    |a| matches!(a.kind, ActionKind::Move { side, mon, .. } if (side, mon) == t),
                );
                let Some(i) = at.filter(|_| self.sides[t.0].n_active > 1 && !self.mon(t).fainted)
                else {
                    self.fail_move(r);
                    return false;
                };
                if kind == MoveKind::AfterYou {
                    if let Some(mut a) = self.queue.remove(i) {
                        a.order = 3;
                        self.queue.push_front(a);
                    }
                } else {
                    self.queue[i].order = 201;
                }
                let ti = self.ident(t);
                let name = if kind == MoveKind::AfterYou {
                    "After You"
                } else {
                    "Quash"
                };
                log_line!(self, "|-activate|{ti}|move: {name}");
                true
            }
            MoveKind::SwapBoosts => {
                let (stats, list) = if mv.id == "powerswap" {
                    ([Boost::Atk, Boost::Spa], "atk, spa")
                } else {
                    ([Boost::Def, Boost::Spd], "def, spd")
                };
                for b in stats {
                    let (x, y) = (
                        self.mon(r).boosts[b as usize],
                        self.mon(t).boosts[b as usize],
                    );
                    self.mon_mut(r).boosts[b as usize] = y;
                    self.mon_mut(t).boosts[b as usize] = x;
                }
                let (ri, ti) = (self.ident(r), self.ident(t));
                log_line!(
                    self,
                    "|-swapboost|{ri}|{ti}|{list}|[from] move: {}",
                    mv.name
                );
                true
            }
            MoveKind::SpeedSwap => {
                let i = Stat::Spe as usize;
                let (x, y) = (self.mon(r).stats[i], self.mon(t).stats[i]);
                for (who, v) in [(t, x), (r, y)] {
                    let m = self.mon_mut(who);
                    m.stats[i] = v;
                    m.vol.stats_changed = true;
                }
                let (ri, ti) = (self.ident(r), self.ident(t));
                log_line!(self, "|-activate|{ri}|move: Speed Swap|[of] {ti}");
                true
            }
            MoveKind::Split => {
                let stats = if mv.id == "powersplit" {
                    [Stat::Atk, Stat::Spa]
                } else {
                    [Stat::Def, Stat::Spd]
                };
                for st in stats {
                    let i = st as usize;
                    let avg = (self.mon(t).stats[i] as u32 + self.mon(r).stats[i] as u32) / 2;
                    for who in [t, r] {
                        let m = self.mon_mut(who);
                        m.stats[i] = avg as u16;
                        m.vol.stats_changed = true;
                    }
                }
                let (ri, ti) = (self.ident(r), self.ident(t));
                log_line!(self, "|-activate|{ri}|move: {}|[of] {ti}", mv.name);
                true
            }
            MoveKind::TopsyTurvy => {
                let m = self.mon_mut(t);
                if m.boosts.iter().all(|&b| b == 0) {
                    self.fail_move(r);
                    return false;
                }
                for b in m.boosts.iter_mut() {
                    *b = -*b;
                }
                let ti = self.ident(t);
                log_line!(self, "|-invertboost|{ti}|[from] move: Topsy-Turvy");
                true
            }
            MoveKind::PainSplit => {
                let (th, uh) = (self.mon(t).hp as u32, self.mon(r).hp as u32);
                let avg = ((th + uh) / 2).max(1);
                for (who, silent) in [(t, "|[silent]"), (r, "")] {
                    let m = self.mon_mut(who);
                    m.hp = avg.min(m.maxhp as u32) as u16;
                    let wi = self.ident(who);
                    self.add_health(
                        format_args!("|-sethp|{wi}"),
                        who,
                        &format!("|[from] move: Pain Split{silent}"),
                    );
                }
                true
            }
            MoveKind::DestinyBond => {
                self.try_add_volatile_sort(t);
                self.mon_mut(t).vol.destiny_bond = true;
                let ti = self.ident(t);
                log_line!(self, "|-singlemove|{ti}|Destiny Bond");
                true
            }
            MoveKind::Curse => self.curse(r, t, mv),
            MoveKind::Entrainment
            | MoveKind::RolePlay
            | MoveKind::WorrySeed
            | MoveKind::SimpleBeam => self.give_ability(r, t, mv, kind),
            MoveKind::AddType => self.add_type_move(r, t, mv),
            MoveKind::Copycat => self.copycat(r, mv),
            MoveKind::SleepTalk => self.sleep_talk(r, mv),
            MoveKind::Instruct => self.instruct(r, t),
            // onHit returns addVolatile's result: false fails the move.
            MoveKind::Trap => {
                if !self.add_trapped(t, r) {
                    self.fail_move(r);
                    return false;
                }
                true
            }
            MoveKind::Memento => {
                let did = self.primary_effects(r, t, mv, true);
                // selfdestruct: 'ifHit'.
                self.status_move_after_effects(r, kind);
                matches!(did, Did::Done | Did::Nothing)
            }
            _ => false,
        }
    }

    /// Curse's onHit (Champions). A non-Ghost raises its Attack and Defense
    /// and lowers its Speed; a Ghost pays half its max HP to curse the
    /// target, or a random foe when it aimed at an ally.
    pub(crate) fn curse(&mut self, r: MonRef, t: MonRef, mv: &Move) -> bool {
        if !self.is_ghost(r) {
            let mut b = crate::dex::types::MoveBoosts::default();
            b.vals[Boost::Spe as usize] = -1;
            b.vals[Boost::Atk as usize] = 1;
            b.vals[Boost::Def as usize] = 1;
            b.order = [
                Boost::Spe as u8,
                Boost::Atk as u8,
                Boost::Def as u8,
                2,
                3,
                5,
                6,
            ];
            if !self.boost_listed(r, &b, Some(r), EffectRef::Move(mv.idx), false, false) {
                self.fail_move(r);
                return false;
            }
            return true;
        }
        let half = (self.mon(r).maxhp as u32 / 2).max(1);
        self.direct_damage(r, half, "");
        let mut t = t;
        if t.0 == r.0 {
            // getRandomTarget(source, 'Curse'): the base move's target type.
            match self.get_random_target(r, Target::Normal) {
                Some(x) => t = x,
                None => {
                    self.fail_move(r);
                    return false;
                }
            }
        }
        // delete target.volatiles['curse'], then addVolatile (no hp: nothing).
        if self.mon(t).hp == 0 {
            return true;
        }
        self.mon_mut(t).vol.curse = false;
        self.try_add_volatile_sort(t);
        self.mon_mut(t).vol.curse = true;
        self.stamp_volatile(t, ResidualVol::Curse);
        let (ti, ri) = (self.ident(t), self.ident(r));
        log_line!(self, "|-start|{ti}|Curse|[of] {ri}");
        true
    }

    pub(crate) fn is_ghost(&self, r: MonRef) -> bool {
        self.dex
            .type_id("Ghost")
            .is_some_and(|g| self.has_type_of(r, g))
    }

    /// Psych Up's onHit: the user takes the target's stat stages and its
    /// critical-hit volatiles (its own are removed first, silently).
    pub(crate) fn psych_up(&mut self, src: MonRef, t: MonRef) {
        let boosts = self.mon(t).boosts;
        self.mon_mut(src).boosts = boosts;
        let theirs = self.mon(t).vol;
        self.clear_crit_volatiles(src, &theirs);
        let (si, ti) = (self.ident(src), self.ident(t));
        log_line!(self, "|-copyboost|{si}|{ti}|[from] move: Psych Up");
    }

    /// Spite's onHit: the target's last move loses 4 PP (what it has left).
    /// `deductPP` marks the slot used even when it has no PP to lose.
    pub(crate) fn spite(&mut self, r: MonRef, t: MonRef) -> bool {
        let slot = self
            .mon(t)
            .last_move
            .and_then(|lm| self.mon(t).moves.iter().position(|ms| ms.id == lm));
        let Some(slot) = slot else {
            self.fail_move(r);
            return false;
        };
        let ms = &mut self.mon_mut(t).moves[slot];
        ms.used = true;
        let by = ms.pp.min(4);
        ms.pp -= by;
        let id = ms.id;
        if by == 0 {
            self.fail_move(r);
            return false;
        }
        let ti = self.ident(t);
        let name = self.dex.mv(id).name.clone();
        log_line!(self, "|-activate|{ti}|move: Spite|{name}|{by}");
        true
    }

    /// Strength Sap's onHit: the user heals by the target's Attack (with its
    /// stage, before the drop) and the target loses a stage. Liquid Ooze
    /// turns the heal into damage; the move fails only if neither the heal
    /// nor the drop happened.
    pub(crate) fn strength_sap(&mut self, r: MonRef, t: MonRef, mv: &Move) -> bool {
        if self.mon(t).boosts[Boost::Atk as usize] == -6 {
            self.fail_move(r);
            return false;
        }
        let atk = self.get_stat(t, Stat::Atk, true, false);
        let mut b = [0i8; 7];
        b[Boost::Atk as usize] = -1;
        let success = self.boost(t, &b, Some(r), EffectRef::Move(mv.idx), false, true);
        let healed = if self.liquid_ooze_on_source_try_heal(r, t, atk) {
            0
        } else {
            let atk = self.drain_heal_mod(r, atk);
            self.heal(r, atk, "")
        };
        if healed == 0 && !success {
            self.fail_move(r);
            return false;
        }
        true
    }

    /// Stuff Cheeks' onHit: +2 Defense, then the berry is eaten (forced:
    /// no TryEatItem, so Unnerve doesn't stop it). A Defense that can't
    /// rise returns null: no failure line, no effect.
    pub(crate) fn stuff_cheeks(&mut self, r: MonRef, mv: &Move) -> bool {
        let mut b = [0i8; 7];
        b[Boost::Def as usize] = 2;
        if !self.boost(r, &b, Some(r), EffectRef::Move(mv.idx), false, false) {
            return false;
        }
        self.eat_item_forced(r);
        true
    }

    /// Supersweet Syrup's onStart: once per Pokémon, each adjacent foe loses
    /// a stage of evasion (a substitute is immune).
    pub(crate) fn super_sweet_syrup(&mut self, r: MonRef) {
        if self.mon(r).syrup_triggered {
            return;
        }
        self.mon_mut(r).syrup_triggered = true;
        let ability = self.mon(r).ability;
        let (ri, name) = (self.ident(r), self.dex.ability(ability).name.clone());
        log_line!(self, "|-ability|{ri}|{name}");
        for t in self.foes_of(r) {
            if self.mon(t).vol.substitute_halves > 0 {
                let ti = self.ident(t);
                log_line!(self, "|-immune|{ti}");
            } else {
                let mut b = [0i8; 7];
                b[Boost::Evasion as usize] = -1;
                self.boost(t, &b, Some(r), EffectRef::Ability(ability), true, false);
            }
        }
    }

    /// Recycle's onHit: the user's `lastItem` comes back, if it holds none.
    pub(crate) fn recycle(&mut self, r: MonRef, t: MonRef) -> bool {
        let m = self.mon(t);
        let Some(item) = m.last_item.filter(|_| m.item.is_none()) else {
            self.fail_move(r);
            return false;
        };
        self.mon_mut(t).last_item = None;
        let ti = self.ident(t);
        let name = self.dex.item(item).name.clone();
        log_line!(self, "|-item|{ti}|{name}|[from] move: Recycle");
        // setItem: the item's Start.
        self.mon_mut(t).item = Some(item);
        self.item_on_start(t);
        true
    }

    /// Reflect Type's onHit: the user takes the target's types but `???`
    /// (Normal when that leaves none but an added type), and its added type.
    /// Arceus and Silvally keep theirs; a target with only `???` has
    /// nothing to copy.
    pub(crate) fn reflect_type(&mut self, r: MonRef, t: MonRef) -> bool {
        let base = self.dex.species(self.mon(r).species).base_species.as_str();
        let typeless = self.dex.ids.types.typeless;
        let mut types: Vec<TypeId> = self
            .base_types_of(t)
            .into_iter()
            .filter(|&x| x != NO_TYPE && Some(x) != typeless)
            .collect();
        let added = self.mon(t).added_type;
        if types.is_empty() && added != NO_TYPE {
            types.push(self.dex.ids.types.normal.unwrap_or(0));
        }
        if matches!(base, "Arceus" | "Silvally") || types.is_empty() {
            self.fail_move(r);
            return false;
        }
        let (ri, ti) = (self.ident(r), self.ident(t));
        log_line!(
            self,
            "|-start|{ri}|typechange|[from] move: Reflect Type|[of] {ti}"
        );
        let old = self.mon(r).types;
        // `source.knownType = target.isAlly(source) && target.knownType`; an
        // unknown type keeps the old apparentType.
        let stale = self
            .mon(r)
            .vol
            .apparent_types
            .map(|(a, _)| a)
            .unwrap_or(old);
        let new = [types[0], types.get(1).copied().unwrap_or(NO_TYPE)];
        let unknown = t.0 != r.0 || self.mon(t).vol.type_unknown;
        self.mon_mut(r).types = new;
        self.mon_mut(r).added_type = added;
        self.mon_mut(r).vol.type_unknown = unknown;
        self.mon_mut(r).vol.apparent_types = unknown.then_some((stale, new));
        true
    }

    /// Shed Tail: onTryHit fails without a Pokémon to switch to, with a
    /// substitute already up, or at half HP or less (all NOT_FAIL); then a
    /// substitute (with a quarter of max HP, `[from] move: Shed Tail`),
    /// onHit pays half of max HP, and the user switches out passing it on.
    pub(crate) fn shed_tail(&mut self, r: MonRef, t: MonRef, mv: &Move) -> bool {
        let ti = self.ident(t);
        let m = self.mon(t);
        if !self.can_switch_side(r.0) {
            log_line!(self, "|-fail|{ti}");
            return false;
        }
        if m.vol.substitute_halves > 0 {
            log_line!(self, "|-fail|{ti}|move: Shed Tail");
            return false;
        }
        if m.hp as u32 <= (m.maxhp as u32).div_ceil(2) {
            log_line!(self, "|-fail|{ti}|move: Shed Tail|[weak]");
            return false;
        }
        let (quarter, half) = (m.maxhp / 4, (m.maxhp as u32).div_ceil(2));
        self.try_add_volatile_sort(t);
        self.mon_mut(t).vol.substitute_halves = quarter * 2;
        log_line!(self, "|-start|{ti}|Substitute|[from] move: Shed Tail");
        self.end_partial_trap_silently(t);
        self.direct_damage(t, half, "");
        // self.onHit, then selfSwitch.
        self.mon_mut(r).skip_before_switch_out = true;
        self.set_self_switch(r, mv.idx);
        true
    }

    /// Swap the stored Attack and Defense (Power Trick); the species' stats
    /// come back when the volatiles clear.
    pub(crate) fn swap_atk_def(&mut self, r: MonRef) {
        let m = self.mon_mut(r);
        m.stats.swap(Stat::Atk as usize, Stat::Def as usize);
        m.vol.stats_changed = true;
    }

    /// Entrainment (the target takes the user's ability), Role Play (the
    /// user takes the target's) and Worry Seed (the target takes Insomnia,
    /// with no source, then wakes up): `setAbility`, whose failure fails
    /// the move.
    pub(crate) fn give_ability(&mut self, r: MonRef, t: MonRef, mv: &Move, kind: MoveKind) -> bool {
        let (who, ability, source) = match kind {
            MoveKind::Entrainment => (t, self.mon(r).ability, Some(r)),
            MoveKind::RolePlay => (r, self.mon(t).ability, Some(t)),
            MoveKind::SimpleBeam => (t, self.dex.ids.abilities.simple.expect("simple"), None),
            _ => (t, self.dex.ids.abilities.insomnia.expect("insomnia"), None),
        };
        if !self.set_ability_from(who, ability, source, &format!("move: {}", mv.name)) {
            self.fail_move(r);
            return false;
        }
        if kind == MoveKind::WorrySeed && self.mon(t).status == Status::Slp {
            self.cure_status(t);
        }
        true
    }

    /// Forest's Curse (Grass) and Trick-or-Treat (Ghost): onHit fails on a
    /// Pokémon of that type, else `addType` replaces any added type. On the
    /// right-hand Pokémon of a doubles side, Trick-or-Treat's "Curse glitch"
    /// points a Curse it will still use at its ally (`targetLoc = -1`).
    pub(crate) fn add_type_move(&mut self, r: MonRef, t: MonRef, mv: &Move) -> bool {
        let trick = mv.id == "trickortreat";
        let ty = self
            .dex
            .type_id(if trick { "Ghost" } else { "Grass" })
            .unwrap_or(0);
        if self.has_type_of(t, ty) {
            self.fail_move(r);
            return false;
        }
        self.mon_mut(t).added_type = ty;
        let (ti, name) = (self.ident(t), self.dex.type_names[ty as usize].clone());
        log_line!(self, "|-start|{ti}|typeadd|{name}|[from] move: {}", mv.name);
        if trick && self.sides[t.0].n_active == 2 && t.1 == 1 && !self.mon(t).fainted {
            let curse = self.dex.ids.moves.curse;
            let queued = self
                .queue
                .iter_mut()
                .find(|a| matches!(a.kind, ActionKind::Move { side, mon, .. } if (side, mon) == t));
            if let Some(a) = queued {
                if let ActionKind::Move {
                    id, ref mut target, ..
                } = a.kind
                {
                    if Some(id) == curse {
                        *target = -1;
                    }
                }
            }
        }
        true
    }

    /// Showdown `setType`: the Pokémon's types, and its added type is gone.
    pub(crate) fn set_type(&mut self, r: MonRef, types: [TypeId; 2]) {
        self.reset_type_knowledge(r);
        let m = self.mon_mut(r);
        m.types = types;
        m.added_type = NO_TYPE;
    }

    /// What follows a move's effects on a target even when a substitute
    /// took the hit: Memento's user faints (`selfdestruct: 'ifHit'`).
    pub(crate) fn status_move_after_effects(&mut self, r: MonRef, kind: MoveKind) {
        if kind == MoveKind::Memento && self.mon(r).hp > 0 {
            self.mon_mut(r).hp = 0;
            self.queue_faint(r);
        }
    }

    /// Thief, Covet: an item-less user takes the target's item
    /// (`takeItem`, the item's own `TakeItem` with the user as the taker,
    /// then `setItem`); when the user can't have it, the target keeps it.
    pub(crate) fn thief(&mut self, src: MonRef, t: MonRef, mv: &Move) {
        // A gem used for this hit (the `gem` volatile) counts as an item.
        if self.mon(src).item.is_some() || self.mon(src).vol.gem {
            return;
        }
        let Some(item) = self.mon(t).item else { return };
        // `target.takeItem(source)`: Sticky Hold, or Unburden (an ability)
        // takes note before a Mega Stone refuses.
        if self.sticky_hold_on_take_item(t, Some(src), false) {
            return;
        }
        self.after_use_item(t);
        if !self.item_removable(t) {
            return;
        }
        let s = self.mon(src);
        if !self.item_receivable(src, item) || s.hp == 0 || !s.is_active {
            return;
        }
        self.mon_mut(t).item = None;
        self.mon_mut(src).item = Some(item);
        // setItem: the item's Start (Air Balloon, seeds), then the messages.
        self.item_on_start(src);
        let (si, ti) = (self.ident(src), self.ident(t));
        let name = self.dex.item(item).name.clone();
        if mv.id == "thief" {
            log_line!(
                self,
                "|-enditem|{ti}|{name}|[silent]|[from] move: Thief|[of] {si}"
            );
        }
        log_line!(
            self,
            "|-item|{si}|{name}|[from] move: {}|[of] {ti}",
            mv.name
        );
    }
}
