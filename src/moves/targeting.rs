//! Move targets: `getTarget`, `getRandomTarget` and `getMoveTargets`.

use crate::battle::{ActiveList, Battle, MonRef};
use crate::dex::types::*;
use crate::dex::Move;
use crate::kinds::AbilityKind;
use crate::kinds::MoveKind;
use crate::state::*;

impl Battle {
    /// Targets for a move (Showdown `getTarget`, `getRandomTarget`, `getMoveTargets`
    /// with `RedirectTarget`). `move_type` is the type after `ModifyType`.
    /// The Pokémon in a target location seen from `r` (Showdown `getAtLoc`):
    /// foes at 1 and 2, allies (and `r`) at -1 and -2.
    pub(crate) fn at_loc(&self, r: MonRef, loc: i8) -> Option<MonRef> {
        match loc {
            0 => None,
            l if l > 0 => self.active(1 - r.0, (l - 1) as usize),
            l => self.active(r.0, (-l - 1) as usize),
        }
    }

    /// Showdown `getLocOf`.
    pub(crate) fn loc_of(&self, r: MonRef, t: MonRef) -> i8 {
        let pos = self.mon(t).position as i8 + 1;
        if t.0 == r.0 {
            -pos
        } else {
            pos
        }
    }

    /// Showdown `getRandomTarget`: the user for moves on itself, its
    /// side or the field; a random adjacent ally for ally moves; otherwise a
    /// random foe still standing (one draw, even with a single foe), or the
    /// foe in slot a when none stands. In singles there is no draw: no ally
    /// for ally moves, else the foe's only slot, standing or not.
    #[track_caller]
    pub(crate) fn get_random_target(&mut self, r: MonRef, target: Target) -> Option<MonRef> {
        let singles = !self.dex.format.doubles;
        match target {
            Target::SelfTarget
            | Target::All
            | Target::AllySide
            | Target::AllyTeam
            | Target::AdjacentAllyOrSelf => Some(r),
            Target::AdjacentAlly if singles => None,
            _ if singles => self.active(1 - r.0, 0),
            Target::AdjacentAlly => {
                let allies = if self.mon(r).fainted {
                    ActiveList::new()
                } else {
                    self.allies_of(r)
                };
                if allies.is_empty() {
                    None
                } else {
                    Some(self.rng.sample(&allies))
                }
            }
            _ => {
                let foes = self.foes_of(r);
                if foes.is_empty() {
                    self.active(1 - r.0, 0)
                } else {
                    Some(self.rng.sample(&foes))
                }
            }
        }
    }

    /// Showdown `getTarget`: the chosen target while it can be hit, a fainted
    /// ally as chosen, otherwise a random target.
    #[track_caller]
    pub(crate) fn get_target(
        &mut self,
        r: MonRef,
        target: Target,
        loc: i8,
        smart: bool,
        future: bool,
    ) -> Option<MonRef> {
        // Dragon Darts: whatever stands at the chosen spot, else a random target.
        if smart {
            return match self.at_loc(r, loc).filter(|&t| !self.mon(t).fainted) {
                Some(t) => Some(t),
                None => self.get_random_target(r, target),
            };
        }
        let self_loc = -(self.mon(r).position as i8 + 1);
        if matches!(target, Target::AdjacentAlly | Target::Any | Target::Normal)
            && loc == self_loc
            && self.mon(r).vol.two_turn.is_none()
        {
            // A future move aimed at its own spot (Ally Switch moved it there)
            // targets the user.
            return future.then_some(r);
        }
        if target != Target::RandomNormal && self.target_ok(r.0, self.mon(r).position, loc, target)
        {
            if let Some(t) = self.at_loc(r, loc) {
                if !self.mon(t).fainted {
                    return Some(t);
                }
                if t.0 == r.0 {
                    return Some(if target == Target::AdjacentAllyOrSelf {
                        r
                    } else {
                        t
                    });
                }
            }
        }
        self.get_random_target(r, target)
    }

    /// Showdown `getMoveTargets`: who a move hits given its target, and the
    /// new target to name in the move line when it changed. (This Showdown
    /// redirects a two-turn move on its charging turn too.)
    pub(super) fn get_move_targets(
        &mut self,
        r: MonRef,
        target_type: Target,
        target: Option<MonRef>,
        move_type: TypeId,
        future: bool,
    ) -> (ActiveList, Option<MonRef>) {
        let s = r.0;
        let foe = 1 - s;
        let standing = |b: &Battle, x: MonRef| !b.mon(x).fainted && b.mon(x).hp > 0;
        let allies_and_self = |b: &Battle| -> ActiveList {
            (0..b.sides[s].n_active)
                .filter_map(|sl| b.active(s, sl))
                .filter(|&x| standing(b, x))
                .collect()
        };
        match target_type {
            Target::All | Target::FoeSide | Target::AllySide | Target::AllyTeam => {
                let mut targets = ActiveList::new();
                if target_type != Target::FoeSide {
                    targets.extend(allies_and_self(self));
                }
                if !matches!(target_type, Target::AllySide | Target::AllyTeam) {
                    // `foes(true)`: fainted actives too.
                    targets.extend(
                        (0..self.sides[foe].n_active).filter_map(|sl| self.active(foe, sl)),
                    );
                }
                let retarget = match (targets.last(), target) {
                    (Some(&last), t) if !t.map(|t| targets.contains(&t)).unwrap_or(false) => {
                        Some(last)
                    }
                    _ => None,
                };
                (targets, retarget)
            }
            Target::AllAdjacent | Target::AllAdjacentFoes => {
                let mut targets = ActiveList::new();
                if target_type == Target::AllAdjacent && !self.mon(r).fainted {
                    targets.extend(self.allies_of(r));
                }
                if !self.mon(r).fainted {
                    targets.extend(self.foes_of(r));
                }
                let retarget = match (targets.last(), target) {
                    (Some(&last), t) if !t.map(|t| targets.contains(&t)).unwrap_or(false) => {
                        Some(last)
                    }
                    _ => None,
                };
                (targets, retarget)
            }
            Target::Allies => (allies_and_self(self), None),
            _ => {
                let selected = target;
                let mut t = match target {
                    Some(t) if !(self.mon(t).fainted && t.0 != s) => t,
                    // A fainted foe target: retarget at random.
                    _ => match self.get_random_target(r, target_type) {
                        Some(t) => t,
                        None => return (ActiveList::new(), None),
                    },
                };
                // RedirectTarget (doubles): Follow Me, Rage Powder, Lightning Rod,
                // Storm Drain; their holders are valid targets only for moves
                // that can hit a foe.
                let redirectable = matches!(
                    target_type,
                    Target::Normal
                        | Target::Any
                        | Target::AdjacentFoe
                        | Target::RandomNormal
                        | Target::Scripted
                );
                if self.sides[s].n_active > 1 && redirectable && !self.active_move.tracks_target {
                    t = self.redirect_target(r, t, move_type);
                }
                // getSmartTargets (unless a redirection turned it off): the
                // target and its ally, while both stand and the ally isn't
                // the user; else the one that stands.
                let mut list: ActiveList = [t].into_iter().collect();
                if self.smart_target {
                    let ally = (0..self.sides[t.0].n_active)
                        .filter_map(|sl| self.active(t.0, sl))
                        .find(|&x| x != t && !self.mon(x).fainted && !self.mon(t).fainted);
                    match ally {
                        Some(a) if a != r && self.mon(a).hp > 0 => {
                            list = if self.mon(t).hp == 0 {
                                [a].into_iter().collect()
                            } else {
                                [t, a].into_iter().collect()
                            };
                        }
                        _ => {}
                    }
                    t = list[0];
                    self.smart_target = list.len() > 1;
                }
                // A future move still aims at a fainted Pokémon's slot.
                if self.mon(t).fainted && !future {
                    return (ActiveList::new(), None);
                }
                let retarget = if selected != Some(t) { Some(t) } else { None };
                (list, retarget)
            }
        }
    }

    /// The move's target type after `ModifyMove` (Expanding Force in Psychic Terrain).
    pub(crate) fn move_target_type(&self, r: MonRef, mv: &Move) -> Target {
        if self.dex.move_kind[mv.idx as usize] == MoveKind::ExpandingForce
            && self.field.terrain == Some(Terrain::Psychic)
            && self.is_grounded(r)
        {
            Target::AllAdjacentFoes
        } else {
            mv.target
        }
    }

    /// `onModifyTarget`: Comeuppance aims at the slot of the last attacker.
    pub(crate) fn comeuppance_on_modify_target(&self, r: MonRef, kind: MoveKind) -> Option<MonRef> {
        if !matches!(kind, MoveKind::Comeuppance | MoveKind::MetalBurst) {
            return None;
        }
        let a = self.last_damaged_by(r)?;
        let (s, p) = (a.slot.0 as usize, a.slot.1 as usize);
        (p < self.sides[s].n_active).then_some((s, p))
    }

    /// Counter's `onRedirectTarget` (priority -1, after Follow Me): the slot
    /// that hit its user last.
    pub(crate) fn counter_on_redirect_target(&self, src: MonRef, target: MonRef) -> MonRef {
        let active = self.active_move.used_type.map(|(m, _)| m);
        let vol = if active == self.dex.ids.moves.counter {
            self.mon(src).vol.counter
        } else if active == self.dex.ids.moves.mirror_coat {
            self.mon(src).vol.mirror_coat
        } else {
            return target;
        };
        match vol.and_then(|c| c.slot) {
            Some((s, p)) if (p as usize) < self.sides[s as usize].n_active => {
                (s as usize, p as usize)
            }
            _ => target,
        }
    }

    /// Long Reach's `onModifyMove`: the move loses its contact flag.
    pub(crate) fn long_reach_on_modify_move(&self, r: MonRef, mv: &Move) -> Option<Move> {
        if !self.has_ability(r, AbilityKind::LongReach) || !mv.has(MoveFlag::Contact) {
            return None;
        }
        let mut m = mv.clone();
        m.flags.remove(MoveFlag::Contact);
        Some(m)
    }

    /// References to Pokémon (Syrup Bomb's thrower) follow a position swap.
    pub(crate) fn follow_swap_volatile_refs(
        &mut self,
        fix: impl Fn(Option<MonRef>) -> Option<MonRef>,
    ) {
        for side in self.sides.iter_mut() {
            for m in side.mons.iter_mut() {
                m.vol.syrup_bomb = fix(m.vol.syrup_bomb);
                // Partial trapping's user, kept while it is off the field.
                m.vol.partial_trap_source = fix(m.vol.partial_trap_source);
            }
        }
    }
}
