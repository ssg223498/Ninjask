//! What one player knows about a battle, built only from that player's
//! protocol lines (Showdown's or the engine's) and requests.
//!
//! The engine's per-player log and Showdown's stream build the same view, so
//! a player reading either knows the same things. An item or ability counts
//! as known when a message names it, a move when the Pokémon uses it or a
//! message names it, and HP is what the line shows (exact for one's own side,
//! out of 100 for the foe).

use std::sync::Arc;

use serde_json::Value;

use crate::dex::types::*;
use crate::dex::Dex;
use crate::kinds::MoveKind;
use crate::state::{SideCond, Terrain, Weather};

/// What a player knows about a held item.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ItemView {
    #[default]
    Unknown,
    /// Known to hold nothing (consumed, knocked off, given away).
    None,
    Known(ItemId),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MoveView {
    pub id: MoveId,
    /// Times the player saw it used (PP spent, for a foe's moves).
    pub uses: u16,
    /// PP from requests (own side only).
    pub pp: Option<u8>,
    pub maxpp: Option<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct MonView {
    /// The name in this side's idents ("p2a: NAME").
    pub name: String,
    /// The species as last shown, after Mega Evolution or a forme change.
    /// `None` while only a hidden preview forme ("Urshifu-*") is known.
    pub species: Option<SpeciesId>,
    /// The species as shown at team preview.
    pub preview: String,
    pub level: u8,
    pub gender: Option<char>,
    /// HP as shown: exact on one's own side, out of 100 for the foe.
    pub hp: u16,
    pub maxhp: u16,
    pub status: Status,
    pub fainted: bool,
    /// Has been on the field (for a foe: known to have been brought).
    pub seen: bool,
    /// Active slot (0 = a, 1 = b).
    pub slot: Option<usize>,
    pub boosts: [i8; 7],
    /// Announced volatile effects, by id ("confusion", "encore", "yawn"...).
    pub volatiles: Vec<String>,
    pub moves: Vec<MoveView>,
    pub ability: Option<AbilityId>,
    /// The ability before Trace or a Mega Evolution replaced it, when shown.
    pub base_ability: Option<AbilityId>,
    /// Skill Swap gave the Pokémon another's ability while its own is
    /// unknown: leaving the field makes the ability unknown again.
    pub ability_swapped: bool,
    pub item: ItemView,
    /// The item last shown leaving (eaten, popped, knocked off): the heal or
    /// boost line naming it afterwards doesn't mean it is still held.
    pub prev_item: Option<ItemId>,
    pub mega: bool,
    /// The move last seen used (Encore, Disable and Stomping Tantrum depend on it).
    pub last_move: Option<MoveId>,
    /// Own side: position in the latest request's Pokémon list (0 and 1 are
    /// on the field; `switch k` brings in position k - 1).
    pub team_pos: Option<usize>,
    /// Own side: atk, def, spa, spd, spe from the request.
    pub stats: Option<[u16; 5]>,
    /// The name an Illusion shows this Pokémon under, when the tracker can
    /// tell (one's own; a foe appearing as a teammate already on the field).
    pub shown_as: Option<String>,
    /// When it last came in, or was shown for what it is (a count of
    /// switch and replace lines).
    pub entered: u32,
    /// The slot an Illusion-user guess fainted in, so a later `replace` that
    /// shows the guess was wrong can still swap the two identities.
    pub faint_slot: Option<usize>,
    /// The type Forest's Curse or Trick-or-Treat added (`-start|X|typeadd|T`),
    /// until a type change or the Pokémon leaves the field.
    pub added_type: Option<TypeId>,
    /// Transformed (`-transform`) into this species, until it leaves the field.
    pub transformed: Option<SpeciesId>,
    /// What was known before it transformed, restored when it leaves.
    pre_transform: Option<Box<PreTransform>>,
    /// The turn each volatile in `volatiles` started, for those with a
    /// known length (Magnet Rise, Uproar, Syrup Bomb, partial trapping).
    pub volatile_since: Vec<(String, u32)>,
    /// The full length, in turns, of a volatile whose length depends on
    /// when in the turn it started: Taunt and Encore last a turn more, and
    /// a move's Disable a turn more, when the target had already acted that
    /// turn (Showdown's `queue.willMove`). The log's move order tells.
    pub volatile_length: Vec<(String, u8)>,
    /// `partiallytrapped`: its user was known to hold a Binding Band (1/6
    /// of max HP a turn rather than 1/8).
    pub bound_band: bool,
    /// The types a type change set (`-start|X|typechange|T`: Soak, Reflect
    /// Type, Protean, Libero, Burn Up), until a forme change or it leaves.
    pub types: Option<Vec<TypeId>>,
    /// A rampage (Outrage, Petal Dance...) seen landing: its uses in a row
    /// and the turn of the last, until fatigue, sleep or a turn without it.
    /// One's own comes from the request.
    pub locked: Option<(u8, u32)>,
    /// Ally Switch's counter (3, 9, 27...: the next use succeeds with
    /// chance 1/counter) and the turn it was last used.
    pub ally_switch: Option<(u16, u32)>,
    /// Took damage this turn (Showdown `hurtThisTurn`): any `-damage` line
    /// that isn't direct damage, even when a foe's HP percentage is unchanged.
    pub hurt_this_turn: bool,
    /// Who holds each trapping volatile on it, as (volatile, (side, index,
    /// `entered` count)): `trapped` ends silently when its holder leaves;
    /// partial trapping and Octolock trap only while theirs is on the field.
    pub trap_sources: Vec<(String, (usize, usize, u32))>,
    /// A stat was raised / lowered this turn (Showdown
    /// `statsRaisedThisTurn` / `statsLoweredThisTurn`).
    pub stats_raised_this_turn: bool,
    pub stats_lowered_this_turn: bool,
    /// Ate a berry that Cud Chew, if it has it, eats again.
    pub cud_chew: bool,
    /// Has no move left to make this turn: it moved, couldn't move, or
    /// came in during the turn (Showdown's `!queue.willMove`).
    pub acted_this_turn: bool,
    /// Came in during this turn (Showdown's `activeTurns` is still 0).
    pub entered_this_turn: bool,
    /// Its Illusion was broken (`replace`) since it came in.
    pub illusion_broken: bool,
    /// The nature an open team sheet showed.
    pub nature: Option<String>,
    /// Where its last move of its own was aimed (Showdown `lastMoveTargetLoc`:
    /// -1 its own slot a, 1 the foe's slot a...). It stays when the Pokémon
    /// leaves the field.
    pub last_move_target_loc: i8,
    /// The result of its move this turn and last turn (Showdown
    /// `moveThisTurnResult`, `moveLastTurnResult`: Stomping Tantrum):
    /// `Some(false)` a failure, `None` neither (no move, recharging, a
    /// charging turn, a move that hit nothing without failing, Protect's).
    pub move_this_turn: Option<bool>,
    pub move_last_turn: Option<bool>,
    /// Hits taken since it came in (`timesAttacked`: Rage Fist).
    pub times_attacked: u16,
    /// The damaging hits taken since it came in (`attackedBy`: Revenge,
    /// Avalanche, Counter), oldest first.
    pub attacked_by: Vec<AttackView>,
    /// The consecutive-protection counter (`stall`): the counter and the
    /// residual phases it lasts.
    pub stall: Option<(u16, u8)>,
    /// The item it last used up (`lastItem`: Recycle, Harvest), kept off
    /// the field.
    pub last_item: Option<ItemId>,
    /// Has eaten a berry (`ateBerry`: Belch).
    pub ate_berry: bool,
}

/// A damaging hit a Pokémon took (Showdown's `attackedBy` entry).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttackView {
    pub side: usize,
    /// The attacker's name in its side's idents.
    pub name: String,
    /// The attacker's slot when it hit.
    pub slot: usize,
    pub mv: MoveId,
    /// HP the last hit took: exact on one's own side, percentage points on
    /// the foe's.
    pub damage: u16,
    pub this_turn: bool,
}

/// A transformed Pokémon's view from before: its moves and abilities.
#[derive(Clone, Debug, Default)]
struct PreTransform {
    moves: Vec<MoveView>,
    ability: Option<AbilityId>,
    base_ability: Option<AbilityId>,
    ability_swapped: bool,
}

impl MonView {
    fn has_volatile(&self, id: &str) -> bool {
        self.volatiles.iter().any(|v| v == id)
    }
    fn add_volatile(&mut self, id: &str) {
        if !self.has_volatile(id) {
            self.volatiles.push(id.to_string());
        }
    }
    /// A volatile that starts on `turn` (kept when already there).
    fn add_volatile_at(&mut self, id: &str, turn: u32) {
        if !self.has_volatile(id) {
            self.volatiles.push(id.to_string());
            self.volatile_since.retain(|(v, _)| v != id);
            self.volatile_since.push((id.to_string(), turn));
        }
    }
    fn remove_volatile(&mut self, id: &str) {
        self.volatiles.retain(|v| v != id);
    }
    /// One's own rampage, from the request: locked into `shown` (the
    /// request's one move) when that is a rampage, else not.
    fn set_locked(&mut self, dex: &Dex, shown: Option<MoveId>, turn: u32) {
        let rampage =
            shown.is_some_and(|id| dex.mv(id).self_volatile.as_deref() == Some("lockedmove"));
        self.locked = if rampage {
            // Used last turn, if the log didn't show it landing.
            Some(self.locked.unwrap_or((1, turn.saturating_sub(1))))
        } else {
            None
        };
    }
    /// The full length of a present volatile, when the view knows it
    /// differs from the usual one (see `volatile_length`).
    pub fn volatile_length(&self, id: &str) -> Option<u8> {
        if !self.has_volatile(id) {
            return None;
        }
        self.volatile_length
            .iter()
            .find(|(v, _)| v == id)
            .map(|(_, n)| *n)
    }
    /// The turn a present volatile started, when known.
    pub fn volatile_since(&self, id: &str) -> Option<u32> {
        if !self.has_volatile(id) {
            return None;
        }
        self.volatile_since
            .iter()
            .find(|(v, _)| v == id)
            .map(|(_, t)| *t)
    }
    /// What lasts only while it stays on the field, besides boosts and
    /// volatiles: cleared when it leaves.
    fn clear_field_state(&mut self) {
        self.volatile_since.clear();
        self.volatile_length.clear();
        self.acted_this_turn = false;
        self.entered_this_turn = false;
        self.trap_sources.clear();
        self.bound_band = false;
        self.types = None;
        self.locked = None;
        self.ally_switch = None;
        self.hurt_this_turn = false;
        self.stats_raised_this_turn = false;
        self.stats_lowered_this_turn = false;
        self.cud_chew = false;
        self.illusion_broken = false;
        self.move_this_turn = None;
        self.move_last_turn = None;
        self.times_attacked = 0;
        self.attacked_by.clear();
        self.stall = None;
    }
    /// `src`'s field state, for the Pokémon that turned out to be the one
    /// on the field (an Illusion seen through).
    fn copy_field_state(&mut self, src: &MonView) {
        self.volatile_since.clone_from(&src.volatile_since);
        self.volatile_length.clone_from(&src.volatile_length);
        self.acted_this_turn = src.acted_this_turn;
        self.entered_this_turn = src.entered_this_turn;
        self.trap_sources.clone_from(&src.trap_sources);
        self.bound_band = src.bound_band;
        self.types.clone_from(&src.types);
        self.locked = src.locked;
        self.ally_switch = src.ally_switch;
        self.hurt_this_turn = src.hurt_this_turn;
        self.stats_raised_this_turn = src.stats_raised_this_turn;
        self.stats_lowered_this_turn = src.stats_lowered_this_turn;
        self.cud_chew = src.cud_chew;
        self.move_this_turn = src.move_this_turn;
        self.move_last_turn = src.move_last_turn;
        self.times_attacked = src.times_attacked;
        self.attacked_by.clone_from(&src.attacked_by);
        self.stall = src.stall;
    }
    /// Effects that end when the Pokémon leaves the field; a copied ability
    /// (Trace) reverts to the base one.
    fn leave_field(&mut self) {
        self.slot = None;
        self.added_type = None;
        self.clear_field_state();
        self.shown_as = None;
        self.transformed = None;
        if let Some(p) = self.pre_transform.take() {
            self.moves = p.moves;
            self.ability = p.ability;
            self.base_ability = p.base_ability;
            self.ability_swapped = p.ability_swapped;
        }
        self.boosts = [0; 7];
        self.volatiles.clear();
        self.last_move = None;
        if self.base_ability.is_some() {
            self.ability = self.base_ability;
        } else if self.ability_swapped {
            self.ability = None;
        }
        self.ability_swapped = false;
    }

    /// Skill Swap: `new` replaces the ability, `old`, which is the one to
    /// revert to unless an earlier change already recorded it.
    fn swap_ability(&mut self, new: Option<AbilityId>, old: Option<AbilityId>) {
        if self.base_ability.is_none() && !self.ability_swapped {
            self.base_ability = old;
            self.ability_swapped = old.is_none();
        }
        self.ability = new;
    }
}

#[derive(Clone, Debug, Default)]
pub struct SideView {
    pub name: String,
    pub mons: Vec<MonView>,
    /// Pokémon brought to battle (`|teamsize|`).
    pub team_size: usize,
    /// Side conditions with the turn they started (Toxic Spikes: layers in
    /// `toxic_spikes`; Stealth Rock in `stealth_rock`).
    pub conditions: Vec<(SideCond, u32)>,
    pub toxic_spikes: u8,
    pub stealth_rock: bool,
    /// Spikes layers, Sticky Web, and the turn Safeguard started.
    pub spikes: u8,
    pub sticky_web: bool,
    pub safeguard: Option<u32>,
    /// Slot conditions of active slots a and b.
    pub slots: [SlotView; 2],
}

/// What waits on one active slot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlotView {
    /// Wish, and the turn it was made (it heals at the end of the next).
    pub wish: Option<u32>,
    /// Healing Wish, until a hurt or statused Pokémon comes in.
    pub healing_wish: bool,
    /// Future Sight (or Doom Desire), and the turn it was used (it hits at
    /// the end of the turn after next).
    pub future: Option<u32>,
}

impl SideView {
    pub fn active(&self, slot: usize) -> Option<&MonView> {
        self.mons.iter().find(|m| m.slot == Some(slot))
    }
    pub fn condition_since(&self, c: SideCond) -> Option<u32> {
        self.conditions
            .iter()
            .find(|(x, _)| *x == c)
            .map(|(_, t)| *t)
    }
}

#[derive(Clone, Debug, Default)]
pub struct FieldView {
    /// Weather and the turn it started.
    pub weather: Option<(Weather, u32)>,
    pub terrain: Option<(Terrain, u32)>,
    pub trick_room: Option<u32>,
    /// Gravity and the turn it started.
    pub gravity: Option<u32>,
    /// Magic Room and the turn it started.
    pub magic_room: Option<u32>,
    pub wonder_room: Option<u32>,
    /// Fairy Lock and the turn it was used (it lasts through the next).
    pub fairy_lock: Option<u32>,
}

/// The latest `move` line, until the next: what it may still undo (a `-fail`)
/// or confirm (a rampage's hit).
#[derive(Clone, Debug, Default)]
struct MoveLine {
    user: String,
    id: Option<MoveId>,
    target: String,
    /// The slot condition or volatile it set, undone by the user's `-fail`.
    wish: Option<(usize, usize)>,
    healing_wish: Option<(usize, usize)>,
    minimize: bool,
    /// A rampage's use count, once a hit shows it landed.
    lock_uses: Option<u8>,
}

/// The move being carried out, from its `move` line until the next move,
/// switch, `cant` or turn line: whose result and hit records its lines
/// change.
#[derive(Clone, Debug, Default)]
struct Act {
    user: String,
    id: Option<MoveId>,
    /// Some target (another Pokémon) has yet to act this turn (Showdown's
    /// `queue.willAct()`, which Protect needs).
    will_act: bool,
    /// Per target hit (side, index): the hits counted so far.
    hits: Vec<((usize, usize), u16)>,
    /// A status move changed some stat stage.
    changed: bool,
}

#[derive(Clone, Debug)]
pub struct Tracker {
    dex: Arc<Dex>,
    /// The player this view belongs to (0 = p1, 1 = p2).
    pub me: usize,
    pub turn: u32,
    /// This turn's residual phase is over (`|upkeep|`): forced switches
    /// before the next turn see the durations already counted down.
    pub upkeep_done: bool,
    pub sides: [SideView; 2],
    pub field: FieldView,
    pub ended: bool,
    /// The winner's name, or "" for a tie.
    pub winner: Option<String>,
    /// Open team sheets: the sets were shown (`|showteam|`).
    pub open_sheets: bool,
    /// Per side and slot: the Pokémon a switch line named, as viewed just
    /// before it (index and view), so a `replace` (Illusion) can undo what
    /// was attributed to the disguise.
    switch_snapshots: [[Option<(usize, MonView)>; 3]; 2],
    switches: u32,
    /// The move the latest `move` line showed when another move (Copycat,
    /// Instruct, Metronome) called it, with its user: not one of the user's
    /// own, so a `-prepare` or a block message naming it reveals nothing.
    called_move: Option<(String, String)>,
    move_line: Option<MoveLine>,
    /// The Pokémon whose Cud Chew just activated: its next `-enditem` is
    /// the berry eaten again.
    cud_eat: Option<String>,
    /// The last move used on the field (Showdown `battle.lastMove`: Copycat).
    pub last_move: Option<MoveId>,
    act: Option<Act>,
    /// `|gametype|singles` (`Some(false)` for another game type).
    singles: Option<bool>,
}

/// A parsed protocol line: kind, positional arguments, `[key] value` tags.
struct Line<'a> {
    kind: &'a str,
    args: Vec<&'a str>,
    tags: Vec<(&'a str, &'a str)>,
}

impl<'a> Line<'a> {
    fn parse(raw: &'a str) -> Option<Line<'a>> {
        let raw = raw.strip_prefix('|')?;
        let mut parts = raw.split('|');
        let kind = parts.next()?;
        let mut args = Vec::new();
        let mut tags = Vec::new();
        for p in parts {
            if let Some(rest) = p.strip_prefix('[') {
                if let Some(close) = rest.find(']') {
                    let key = &rest[..close];
                    if key.chars().all(|c| c.is_ascii_lowercase()) && !key.is_empty() {
                        tags.push((key, rest[close + 1..].trim_start()));
                        continue;
                    }
                }
            }
            args.push(p);
        }
        Some(Line { kind, args, tags })
    }
    fn arg(&self, i: usize) -> &'a str {
        self.args.get(i).copied().unwrap_or("")
    }
    fn tag(&self, key: &str) -> Option<&'a str> {
        self.tags.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    }
}

/// "p2a: Name" -> (side, slot, name); "p2: Name" has no slot.
fn parse_ref(r: &str) -> Option<(usize, Option<usize>, &str)> {
    let (head, name) = r.split_once(": ")?;
    let side = match head.get(..2)? {
        "p1" => 0,
        "p2" => 1,
        _ => return None,
    };
    let slot = match head.get(2..3) {
        Some("a") => Some(0),
        Some("b") => Some(1),
        Some("c") => Some(2),
        _ => None,
    };
    Some((side, slot, name))
}

/// "Garchomp-Mega, L50, F" -> (species, level, gender).
fn parse_details(d: &str) -> (&str, u8, Option<char>) {
    let mut parts = d.split(", ");
    let species = parts.next().unwrap_or("");
    let mut level = 100;
    let mut gender = None;
    for p in parts {
        if let Some(l) = p.strip_prefix('L') {
            level = l.parse().unwrap_or(100);
        } else if p == "M" || p == "F" {
            gender = p.chars().next();
        }
    }
    (species, level, gender)
}

/// "154/172 par", "72/100y", "0 fnt" -> (hp, maxhp, status, fainted).
fn parse_condition(c: &str) -> Option<(u16, u16, Status, bool)> {
    let mut parts = c.split(' ');
    let hp_part = parts.next()?;
    let status_part = parts.next().unwrap_or("");
    if hp_part == "0" || status_part == "fnt" {
        return Some((0, 0, Status::None, true));
    }
    let (hp, max) = hp_part.split_once('/')?;
    let max: String = max.chars().take_while(|c| c.is_ascii_digit()).collect();
    let status = Status::from_id(status_part).unwrap_or(Status::None);
    Some((hp.parse().ok()?, max.parse().ok()?, status, false))
}

/// The effect named by "[from] ability: X" / "item: X" / "move: X".
enum Named<'a> {
    Ability(&'a str),
    Item(&'a str),
    Move(&'a str),
    Other(&'a str),
}

fn named(effect: &str) -> Named<'_> {
    if let Some(a) = effect.strip_prefix("ability: ") {
        Named::Ability(a)
    } else if let Some(i) = effect.strip_prefix("item: ") {
        Named::Item(i)
    } else if let Some(m) = effect.strip_prefix("move: ") {
        Named::Move(m)
    } else {
        Named::Other(effect)
    }
}

fn side_cond(name: &str) -> Option<SideCond> {
    let id = to_id(name.strip_prefix("move: ").unwrap_or(name));
    SideCond::from_id(&id)
}

impl Tracker {
    pub fn new(dex: Arc<Dex>, me: usize) -> Tracker {
        Tracker {
            dex,
            me,
            turn: 0,
            upkeep_done: false,
            sides: [SideView::default(), SideView::default()],
            field: FieldView::default(),
            ended: false,
            winner: None,
            open_sheets: false,
            switch_snapshots: Default::default(),
            switches: 0,
            called_move: None,
            move_line: None,
            cud_eat: None,
            last_move: None,
            act: None,
            singles: None,
        }
    }

    /// The format this view reads.
    pub fn dex(&self) -> &Arc<Dex> {
        &self.dex
    }

    /// The turn to record for a condition that starts now, so that
    /// `residuals_since` counts the residual phases it has seen: after this
    /// turn's upkeep, or before turn 1 (a lead's ability), the next residual
    /// phase is the next turn's.
    fn residual_mark(&self) -> u32 {
        self.turn + u32::from(self.upkeep_done || self.turn == 0)
    }

    /// End-of-turn residual phases since `since` (a turn something started):
    /// a condition of `n` turns started then has `n` minus this left.
    pub fn residuals_since(&self, since: u32) -> u32 {
        self.turn.saturating_sub(since) + u32::from(self.upkeep_done)
    }

    /// Feed one protocol line from this player's stream.
    pub fn feed(&mut self, raw: &str) {
        self.feed_line(raw);
        self.sweep_traps();
    }

    /// A `trapped` volatile ends silently when the Pokémon that trapped
    /// it leaves the field (Showdown's linked volatiles).
    fn sweep_traps(&mut self) {
        for s in 0..2 {
            for k in 0..self.sides[s].mons.len() {
                let gone: Vec<String> = self.sides[s].mons[k]
                    .trap_sources
                    .iter()
                    .filter(|(v, src)| v == "trapped" && !self.source_active(*src))
                    .map(|(v, _)| v.clone())
                    .collect();
                if !gone.is_empty() {
                    let m = &mut self.sides[s].mons[k];
                    m.trap_sources.retain(|(v, _)| !gone.contains(v));
                    for v in gone {
                        m.remove_volatile(&v);
                    }
                }
            }
        }
    }

    /// Whether a trap's holder is still on the field since it trapped.
    fn source_active(&self, (s, k, entered): (usize, usize, u32)) -> bool {
        self.sides[s]
            .mons
            .get(k)
            .is_some_and(|m| m.slot.is_some() && !m.fainted && m.entered == entered)
    }

    /// Remember who holds the trapping volatile `vol` on `who`.
    fn record_trap_source(&mut self, who: &str, vol: &str, holder: &str) {
        let Some((hs, hk)) = self.mon_index(holder) else {
            return;
        };
        let src = (hs, hk, self.sides[hs].mons[hk].entered);
        if let Some(m) = self.mon_mut(who) {
            m.trap_sources.retain(|(v, _)| v != vol);
            m.trap_sources.push((vol.to_string(), src));
        }
    }

    fn feed_line(&mut self, raw: &str) {
        let Some(l) = Line::parse(raw) else { return };
        if matches!(l.kind, "turn" | "upkeep" | "cant") {
            self.called_move = None;
            self.move_line = None;
        }
        // A move's own lines end before the next action (a switch, or a
        // `drag` after the move) and the residual phase.
        if matches!(l.kind, "turn" | "upkeep" | "cant" | "switch" | "drag" | "") {
            self.act = None;
        }
        match l.kind {
            "player" => {
                if let Some(s) = side_index(l.arg(0)) {
                    if !l.arg(1).is_empty() {
                        self.sides[s].name = l.arg(1).to_string();
                    }
                }
            }
            "gametype" => self.singles = Some(l.arg(0) == "singles"),
            "teamsize" => {
                if let Some(s) = side_index(l.arg(0)) {
                    self.sides[s].team_size = l.arg(1).parse().unwrap_or(0);
                }
            }
            "clearpoke" => {
                for side in self.sides.iter_mut() {
                    side.mons.retain(|m| m.seen);
                }
            }
            "poke" => self.on_poke(&l),
            "showteam" => self.on_showteam(raw),
            "switch" | "drag" => self.on_switch(&l),
            // Ally Switch: "swap|p1a: X|1": X moves to slot 1, its ally to X's slot.
            "swap" => {
                if let (Some((s, Some(from), name)), Ok(to)) =
                    (parse_ref(l.arg(0)), l.arg(1).parse::<usize>())
                {
                    for m in self.sides[s].mons.iter_mut() {
                        if m.name == name {
                            m.slot = Some(to);
                        } else if m.slot == Some(to) {
                            m.slot = Some(from);
                        }
                    }
                    // What each slot showed at its switch-in moves with it
                    // (an Illusion broken after Ally Switch).
                    self.swap_switch_snapshots(s, from, to);
                }
            }
            "detailschange" => self.on_detailschange(&l),
            "replace" => self.on_replace(&l),
            // A temporary forme (Stance Change), until the Pokémon leaves.
            "-formechange" => {
                let id = self.dex.species_id(l.arg(1));
                if let Some(m) = self.mon_mut(l.arg(0)) {
                    m.species = id.or(m.species);
                    m.types = None;
                }
                self.attribute(&l, l.arg(0));
            }
            "-mega" => {
                self.mega_unmask(l.arg(0));
                if let Some(m) = self.mon_mut(l.arg(0)) {
                    m.mega = true;
                }
                let stone = l.arg(2);
                self.set_item(l.arg(0), stone);
            }
            "move" => self.on_move(&l),
            "-transform" => self.on_transform(&l),
            "cant" => self.on_cant(&l),
            "faint" => {
                if let Some(m) = self.mon_mut(l.arg(0)) {
                    m.fainted = true;
                    m.hp = 0;
                    if m.shown_as.is_some() {
                        m.faint_slot = m.slot;
                    }
                    m.leave_field();
                }
            }
            "-damage" | "-heal" | "-sethp" => {
                // Hitting itself in confusion aborts the move: Destiny Bond
                // ends, and the move counts as failed.
                if l.tag("from") == Some("confusion") {
                    if let Some(m) = self.mon_mut(l.arg(0)) {
                        m.remove_volatile("destinybond");
                        m.move_this_turn = Some(false);
                    }
                }
                let before = self.mon_mut(l.arg(0)).map(|m| m.hp);
                self.on_hp(&l);
                if l.kind == "-damage" {
                    self.on_damage(&l);
                    self.on_hit(&l, before);
                }
                if l.kind == "-heal" {
                    self.on_absorbed(l.arg(0), l.tag("from"));
                }
                // "-heal|X|...|[from] move: Wish|[wisher] W", Healing Wish's.
                if l.kind == "-heal" {
                    if let Some(Named::Move(mv)) = l.tag("from").map(named) {
                        self.end_slot_condition(l.arg(0), &to_id(mv));
                    }
                }
            }
            "-status" => {
                if let Some(st) = Status::from_id(l.arg(1)) {
                    if let Some(m) = self.mon_mut(l.arg(0)) {
                        m.status = st;
                    }
                }
                self.attribute(&l, l.arg(0));
            }
            "-curestatus" => {
                if let Some(m) = self.mon_mut(l.arg(0)) {
                    m.status = Status::None;
                }
                self.attribute(&l, l.arg(0));
            }
            "-boost" | "-unboost" => {
                let sign = if l.kind == "-boost" { 1 } else { -1 };
                let n: i8 = l.arg(2).parse().unwrap_or(0);
                if let (Some(b), Some(m)) = (Boost::from_id(l.arg(1)), self.mon_mut(l.arg(0))) {
                    let v = &mut m.boosts[b as usize];
                    *v = (*v + sign * n).clamp(-6, 6);
                    if n > 0 && sign > 0 {
                        m.stats_raised_this_turn = true;
                    } else if n > 0 {
                        m.stats_lowered_this_turn = true;
                    }
                }
                // Item boosts (seeds, White Herb) follow the item's `-enditem`.
                self.attribute(&l, l.arg(0));
                if l.kind == "-boost" {
                    self.on_absorbed(l.arg(0), l.tag("from"));
                }
                // A status move whose stat changes all changed nothing (at +6)
                // failed.
                let dex = self.dex.clone();
                if let Some(act) = self.act.as_mut().filter(|a| {
                    l.tag("from").is_none()
                        && a.id.is_some_and(|i| dex.mv(i).category == Category::Status)
                }) {
                    let user = act.user.clone();
                    if n != 0 {
                        act.changed = true;
                        self.act_result(&user, Some(true));
                    } else if !act.changed {
                        self.act_result(&user, Some(false));
                    }
                }
            }
            "-setboost" => {
                let n: i8 = l.arg(2).parse().unwrap_or(0);
                let belly_drum = l.tag("from") == Some("move: Belly Drum");
                if let (Some(b), Some(m)) = (Boost::from_id(l.arg(1)), self.mon_mut(l.arg(0))) {
                    m.boosts[b as usize] = n.clamp(-6, 6);
                    // Belly Drum boosts (to +6); Anger Point sets the stage.
                    if belly_drum {
                        m.stats_raised_this_turn = true;
                    }
                }
                // Anger Point names itself.
                self.attribute(&l, l.arg(0));
            }
            "-clearboost" => {
                if let Some(m) = self.mon_mut(l.arg(0)) {
                    m.boosts = [0; 7];
                }
            }
            "-clearnegativeboost" => {
                if let Some(m) = self.mon_mut(l.arg(0)) {
                    for b in m.boosts.iter_mut() {
                        if *b < 0 {
                            *b = 0;
                        }
                    }
                }
            }
            "-clearallboost" => {
                for side in self.sides.iter_mut() {
                    for m in side.mons.iter_mut() {
                        m.boosts = [0; 7];
                    }
                }
            }
            // Psych Up: the user takes the target's stages and crit volatiles.
            "-copyboost" => {
                let from = self
                    .mon_mut(l.arg(1))
                    .map(|m| (m.boosts, m.volatiles.clone()));
                if let (Some((boosts, vols)), Some(m)) = (from, self.mon_mut(l.arg(0))) {
                    m.boosts = boosts;
                    for v in ["focusenergy", "dragoncheer"] {
                        m.remove_volatile(v);
                        if vols.iter().any(|x| x == v) {
                            m.add_volatile(v);
                        }
                    }
                }
            }
            // Power Swap, Guard Swap: "-swapboost|USER|TARGET|atk, spa".
            "-swapboost" => {
                let stats: Vec<usize> = l
                    .arg(2)
                    .split(", ")
                    .filter_map(Boost::from_id)
                    .map(|b| b as usize)
                    .collect();
                let a = self.mon_mut(l.arg(0)).map(|m| m.boosts);
                let b = self.mon_mut(l.arg(1)).map(|m| m.boosts);
                if let (Some(a), Some(b)) = (a, b) {
                    for (who, from) in [(l.arg(0), b), (l.arg(1), a)] {
                        if let Some(m) = self.mon_mut(who) {
                            for &i in &stats {
                                m.boosts[i] = from[i];
                            }
                        }
                    }
                }
            }
            "-invertboost" => {
                if let Some(m) = self.mon_mut(l.arg(0)) {
                    for b in m.boosts.iter_mut() {
                        *b = -*b;
                    }
                }
            }
            "-weather" => self.on_weather(&l),
            "-fieldstart" | "-fieldend" => self.on_field(&l),
            "-sidestart" | "-sideend" => self.on_side(&l),
            "-swapsideconditions" => self.swap_side_conditions(),
            "-start" => {
                self.on_absorbed(l.arg(0), Some(l.arg(1)));
                self.on_start(&l);
            }
            "-end" => self.on_end(&l),
            // Wide Guard and Quick Guard are announced on their user and last the turn.
            "-singleturn" => {
                if let (Some((s, _, _)), Some(c)) = (parse_ref(l.arg(0)), side_cond(l.arg(1))) {
                    if c.is_guard() {
                        let turn = self.turn;
                        let side = &mut self.sides[s];
                        side.conditions.retain(|(x, _)| *x != c);
                        side.conditions.push((c, turn));
                    }
                }
                // Endure lasts the turn.
                if l.arg(1) == "move: Endure" {
                    if let Some(m) = self.mon_mut(l.arg(0)) {
                        m.add_volatile("endure");
                    }
                }
                // Protection (and Endure, Quick Guard, Wide Guard) went up:
                // the stall counter starts at 3 or triples (to 729), for two
                // residual phases.
                let guard = side_cond(l.arg(1)).is_some_and(|c| c.is_guard());
                if guard || matches!(l.arg(1), "Protect" | "move: Protect" | "move: Endure") {
                    if let Some(m) = self.mon_mut(l.arg(0)) {
                        let counter = match m.stall {
                            None => 3,
                            Some((c, _)) if c < 729 => c * 3,
                            Some((c, _)) => c,
                        };
                        m.stall = Some((counter, 2));
                    }
                }
            }
            // Fairy Lock: no switching until the end of the next turn.
            "-fieldactivate" => {
                if l.arg(0) == "move: Fairy Lock" {
                    self.field.fairy_lock = Some(self.turn);
                }
            }
            "-singlemove" => {
                let v = match l.arg(1) {
                    "Glaive Rush" => Some("glaiverush"),
                    "Destiny Bond" => Some("destinybond"),
                    _ => None,
                };
                if let (Some(v), Some(m)) = (v, self.mon_mut(l.arg(0))) {
                    m.add_volatile(v);
                }
            }
            "-mustrecharge" => {
                if let Some(m) = self.mon_mut(l.arg(0)) {
                    m.add_volatile("mustrecharge");
                }
            }
            // A charging move that fires at once (Electro Shot in rain).
            "-anim" => {
                let charged = self.mon_mut(l.arg(0)).is_some_and(|m| {
                    let charging = m.has_volatile("twoturnmove");
                    m.remove_volatile("twoturnmove");
                    charging
                });
                if charged {
                    self.act_result(l.arg(0), Some(true));
                }
            }
            "-prepare" => {
                let mv = l.arg(1).to_string();
                self.reveal_move(l.arg(0), &mv, 0);
                // Chilly Reception's `[premajor]` announces a move, not a charge.
                if l.tag("premajor").is_none() {
                    if let Some(m) = self.mon_mut(l.arg(0)) {
                        m.add_volatile("twoturnmove");
                    }
                    // A charging turn is no result.
                    self.act_result(l.arg(0), None);
                }
            }
            // A move's miss is its failure.
            "-miss" => {
                if let Some(user) = self.act.as_ref().map(|a| a.user.clone()) {
                    if user == l.arg(0) {
                        self.act_result(&user, Some(false));
                    }
                }
            }
            // "-hitcount|TARGET|N": a hit a substitute took counts too, when
            // the last one landed.
            "-hitcount" => {
                let n: u16 = l.arg(1).parse().unwrap_or(0);
                let t = self.mon_index(l.arg(0));
                let seen = self
                    .act
                    .as_ref()
                    .and_then(|a| a.hits.iter().find(|(r, _)| Some(*r) == t).map(|(_, c)| *c));
                if let (Some((s, k)), Some(c)) = (t, seen) {
                    if n > c {
                        let m = &mut self.sides[s].mons[k];
                        m.times_attacked = m.times_attacked.saturating_add(n - c);
                    }
                }
            }
            // Gastro Acid ("-endability|X") suppresses the ability until X leaves.
            "-endability" => {
                if let Some(m) = self.mon_mut(l.arg(0)) {
                    m.add_volatile("gastroacid");
                }
            }
            "-item" => self.on_item(&l),
            "-enditem" => self.on_enditem(&l),
            "-ability" => {
                // "-ability|X|Lightning Rod|boost": the ability took the move.
                let named = format!("ability: {}", l.arg(1));
                self.on_absorbed(l.arg(0), Some(&named));
                self.on_ability(&l);
            }
            "-activate" | "-immune" | "-fail" | "-block" => self.on_activate(&l),
            "turn" => {
                self.turn = l.arg(0).parse().unwrap_or(self.turn);
                self.on_new_turn();
            }
            // The guards end in the residual phase, which `|upkeep|` closes.
            "upkeep" => self.on_upkeep(),
            "win" => {
                self.ended = true;
                self.winner = Some(l.arg(0).to_string());
            }
            "tie" => {
                self.ended = true;
                self.winner = Some(String::new());
            }
            _ => {}
        }
    }

    /// Feed this player's request: exact information about its own team.
    pub fn feed_request(&mut self, req: &Value) {
        let me = self.me;
        let Some(list) = req["side"]["pokemon"].as_array() else {
            return;
        };
        // Battle requests list only the Pokémon brought: forget the others.
        if req["teamPreview"].as_bool() != Some(true) {
            let names: Vec<&str> = list
                .iter()
                .filter_map(|p| {
                    p["ident"]
                        .as_str()
                        .and_then(|i| i.split_once(": "))
                        .map(|(_, n)| n)
                })
                .collect();
            self.sides[me]
                .mons
                .retain(|m| m.name.is_empty() || names.contains(&m.name.as_str()));
        }
        let mut actives: Vec<(String, usize)> = Vec::new();
        for (i, p) in list.iter().enumerate() {
            let ident = p["ident"].as_str().unwrap_or("");
            let name = ident
                .split_once(": ")
                .map(|(_, n)| n)
                .unwrap_or(ident)
                .to_string();
            let details = p["details"].as_str().unwrap_or("");
            let (species, level, gender) = parse_details(details);
            let species_id = self.dex.species_id(species);
            let idx = match self.sides[me].mons.iter().position(|m| m.name == name) {
                Some(k) => k,
                None => {
                    // First request: match the preview entry by species, else add.
                    let k = self.sides[me]
                        .mons
                        .iter()
                        .position(|m| m.name.is_empty() && same_species(&m.preview, species));
                    match k {
                        Some(k) => k,
                        None => {
                            self.sides[me].mons.push(MonView {
                                preview: species.to_string(),
                                ..Default::default()
                            });
                            self.sides[me].mons.len() - 1
                        }
                    }
                }
            };
            let dex = self.dex.clone();
            let turn = self.turn;
            let m = &mut self.sides[me].mons[idx];
            m.name = name;
            m.species = species_id.or(m.species);
            m.level = level;
            m.gender = gender;
            m.team_pos = Some(i);
            // The request knows which of one's own Pokémon are on the field.
            let fainted = p["condition"].as_str().is_some_and(|c| c.ends_with("fnt"));
            if p["active"].as_bool() == Some(true) && !fainted {
                actives.push((m.name.clone(), i));
            }
            let st = &p["stats"];
            if st.is_object() {
                let g = |k: &str| st[k].as_u64().unwrap_or(0) as u16;
                m.stats = Some([g("atk"), g("def"), g("spa"), g("spd"), g("spe")]);
            }
            if let Some((hp, maxhp, status, fainted)) =
                p["condition"].as_str().and_then(parse_condition)
            {
                m.hp = hp;
                if maxhp > 0 {
                    m.maxhp = maxhp;
                }
                m.status = status;
                m.fainted = fainted;
            }
            m.ability = p["ability"].as_str().and_then(|a| dex.ability_id(a));
            m.base_ability = p["baseAbility"].as_str().and_then(|a| dex.ability_id(a));
            m.item = match p["item"].as_str() {
                Some("") => ItemView::None,
                Some(it) => dex
                    .item_id(it)
                    .map(ItemView::Known)
                    .unwrap_or(ItemView::Unknown),
                None => ItemView::Unknown,
            };
            // Moves in the request's order, which is the order of `move N` choices.
            let ids: Vec<MoveId> = p["moves"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().and_then(|x| dex.move_id(x)))
                .collect();
            let mut moves: Vec<MoveView> = ids
                .iter()
                .map(|&id| {
                    m.moves
                        .iter()
                        .find(|mv| mv.id == id)
                        .cloned()
                        .unwrap_or(MoveView {
                            id,
                            ..Default::default()
                        })
                })
                .collect();
            moves.extend(m.moves.iter().filter(|mv| !ids.contains(&mv.id)).cloned());
            m.moves = moves;
            if p["active"].as_bool() == Some(true) {
                // Exact PP comes with the active Pokémon's move list.
                let slot = i;
                if let Some(active) = req["active"].get(slot) {
                    // A rampage shows as its one move, without PP.
                    let shown = active["moves"].as_array().map(Vec::as_slice).unwrap_or(&[]);
                    let lock = match shown {
                        [only] if only.get("pp").is_none() => {
                            only["id"].as_str().and_then(|x| dex.move_id(x))
                        }
                        _ => None,
                    };
                    m.set_locked(&dex, lock, turn);
                    for am in active["moves"].as_array().into_iter().flatten() {
                        let Some(id) = am["id"].as_str().and_then(|x| dex.move_id(x)) else {
                            continue;
                        };
                        if let Some(mv) = m.moves.iter_mut().find(|mv| mv.id == id) {
                            mv.pp = am["pp"].as_u64().map(|x| x as u8);
                            mv.maxpp = am["maxpp"].as_u64().map(|x| x as u8);
                        }
                    }
                }
            }
        }
        if req["teamPreview"].as_bool() != Some(true) {
            self.fix_own_slots(&actives);
        }
    }

    /// Own-side information straight from the engine: the same view as
    /// [`Tracker::feed_request`] on the engine's request (tested), without
    /// building the request. For self-play.
    pub fn feed_engine(&mut self, b: &crate::battle::Battle) {
        use crate::moves::LockedMove;
        use crate::state::RequestState;
        let me = self.me;
        if b.request == RequestState::Ended {
            return;
        }
        let dex = self.dex.clone();
        let side = &b.sides[me];
        let needs = b.side_needs_choice(me);
        if !(b.request == RequestState::TeamPreview && needs) {
            self.sides[me]
                .mons
                .retain(|m| m.name.is_empty() || side.mons.iter().any(|x| *x.name == *m.name));
        }
        // The request's move list, with PP, comes with a move request only.
        let has_active = b.request == RequestState::Move && needs;
        let mut actives: Vec<(String, usize)> = Vec::new();
        for (i, em) in side.mons.iter().enumerate() {
            // The request's details: a temporary forme (Aegislash-Blade) shows
            // the base species.
            let species_name = &dex.species(em.base_species).name;
            let idx = match self.sides[me].mons.iter().position(|m| *m.name == *em.name) {
                Some(k) => k,
                None => {
                    let k = self.sides[me]
                        .mons
                        .iter()
                        .position(|m| m.name.is_empty() && same_species(&m.preview, species_name));
                    match k {
                        Some(k) => k,
                        None => {
                            self.sides[me].mons.push(MonView {
                                preview: species_name.clone(),
                                ..Default::default()
                            });
                            self.sides[me].mons.len() - 1
                        }
                    }
                }
            };
            let m = &mut self.sides[me].mons[idx];
            m.name = em.name.to_string();
            m.species = Some(em.base_species);
            m.level = em.level;
            m.gender = match em.gender.as_str() {
                "M" => Some('M'),
                "F" => Some('F'),
                _ => None,
            };
            m.team_pos = Some(i);
            if em.is_active && em.hp > 0 && i < side.n_active {
                actives.push((em.name.to_string(), i));
            }
            // The request's stats: its own, also while transformed.
            let st = b.base_stored_stats((me, i));
            m.stats = Some([st[1], st[2], st[3], st[4], st[5]]);
            if em.hp == 0 {
                m.hp = 0;
                m.status = Status::None;
                m.fainted = true;
            } else {
                m.hp = em.hp;
                if em.maxhp > 0 {
                    m.maxhp = em.maxhp;
                }
                m.status = if em.status == Status::Fnt {
                    Status::None
                } else {
                    em.status
                };
                m.fainted = false;
            }
            m.ability = Some(em.ability);
            m.base_ability = Some(em.base_ability);
            m.item = em.item.map(ItemView::Known).unwrap_or(ItemView::None);
            if m.moves.len() != em.moves.len()
                || m.moves.iter().zip(&em.moves).any(|(a, b)| a.id != b.id)
            {
                let mut moves: Vec<MoveView> = em
                    .moves
                    .iter()
                    .map(|ms| {
                        m.moves
                            .iter()
                            .find(|mv| mv.id == ms.id)
                            .cloned()
                            .unwrap_or(MoveView {
                                id: ms.id,
                                ..Default::default()
                            })
                    })
                    .collect();
                moves.extend(
                    m.moves
                        .iter()
                        .filter(|mv| !em.moves.iter().any(|ms| ms.id == mv.id))
                        .cloned(),
                );
                m.moves = moves;
            }
            if has_active && em.is_active && i < side.n_active {
                let lock = match b.locked_move((me, i)) {
                    Some(LockedMove::Move(id)) => Some(id),
                    _ => None,
                };
                m.set_locked(&dex, lock, self.turn);
                match b.locked_move((me, i)) {
                    // The request shows only the locked move, without PP.
                    Some(LockedMove::Move(id)) => {
                        if let Some(mv) = m.moves.iter_mut().find(|mv| mv.id == id) {
                            mv.pp = None;
                            mv.maxpp = None;
                        }
                    }
                    Some(LockedMove::Recharge) => {}
                    // With no move shown usable the request shows only Struggle.
                    None if (0..em.moves.len()).all(|j| b.move_shown_disabled((me, i), j)) => {}
                    None => {
                        for ms in &em.moves {
                            if let Some(mv) = m.moves.iter_mut().find(|mv| mv.id == ms.id) {
                                mv.pp = Some(ms.pp);
                                mv.maxpp = Some(ms.max_pp);
                            }
                        }
                    }
                }
            }
        }
        if b.request != RequestState::TeamPreview {
            self.fix_own_slots(&actives);
        }
    }

    // ------------------------------------------------------------ lookups

    fn mon_index(&self, r: &str) -> Option<(usize, usize)> {
        let (s, slot, name) = parse_ref(r)?;
        let side = &self.sides[s];
        // A Pokémon known to be shown under another's name.
        if let Some(k) = slot.and_then(|sl| {
            side.mons
                .iter()
                .position(|m| m.slot == Some(sl) && m.shown_as.as_deref() == Some(name))
        }) {
            return Some((s, k));
        }
        // One's own active Pokémon are known by slot, whatever name an
        // Illusion shows.
        if s == self.me {
            if let Some(k) = slot.and_then(|sl| side.mons.iter().position(|m| m.slot == Some(sl))) {
                return Some((s, k));
            }
        }
        if let Some(k) = side.mons.iter().position(|m| m.name == name) {
            return Some((s, k));
        }
        slot.and_then(|sl| side.mons.iter().position(|m| m.slot == Some(sl)))
            .map(|k| (s, k))
    }

    fn mon_mut(&mut self, r: &str) -> Option<&mut MonView> {
        let (s, k) = self.mon_index(r)?;
        Some(&mut self.sides[s].mons[k])
    }

    // ------------------------------------------------------------ handlers

    fn on_poke(&mut self, l: &Line) {
        let Some(s) = side_index(l.arg(0)) else {
            return;
        };
        let (species, level, gender) = parse_details(l.arg(1));
        let species_id = if species.ends_with("-*") {
            None
        } else {
            self.dex.species_id(species)
        };
        self.sides[s].mons.push(MonView {
            preview: species.to_string(),
            species: species_id,
            level,
            gender,
            hp: 100,
            maxhp: 100,
            ..Default::default()
        });
    }

    /// `|showteam|p2|PACKED` (open team sheets): the foe's sets become known.
    fn on_showteam(&mut self, raw: &str) {
        let rest = raw.trim_start_matches("|showteam|");
        let Some((side, packed)) = rest.split_once('|') else {
            return;
        };
        let Some(s) = side_index(side) else { return };
        let Ok(sets) = crate::dex::team::parse_packed(&self.dex, packed) else {
            return;
        };
        self.open_sheets = true;
        for set in sets {
            let name = self.dex.species(set.species).name.clone();
            let k = match self.sides[s]
                .mons
                .iter()
                .position(|m| same_species(&m.preview, &name))
            {
                Some(k) => k,
                None => {
                    self.sides[s].mons.push(MonView {
                        preview: name.clone(),
                        hp: 100,
                        maxhp: 100,
                        ..Default::default()
                    });
                    self.sides[s].mons.len() - 1
                }
            };
            let m = &mut self.sides[s].mons[k];
            m.species = Some(set.species);
            m.ability = Some(set.ability);
            m.item = set.item.map(ItemView::Known).unwrap_or(ItemView::None);
            m.nature = (!set.nature.is_empty()).then(|| set.nature.clone());
            for &id in &set.moves {
                if !m.moves.iter().any(|mv| mv.id == id) {
                    m.moves.push(MoveView {
                        id,
                        ..Default::default()
                    });
                }
            }
        }
    }

    fn on_switch(&mut self, l: &Line) {
        let Some((s, Some(slot), name)) = parse_ref(l.arg(0)) else {
            return;
        };
        let (species, level, gender) = parse_details(l.arg(1));
        let species_id = self.dex.species_id(species);
        // Baton Pass hands stat stages and most volatiles to the newcomer.
        let mut passed = None;
        let mut passed_since = None;
        if l.tag("from") == Some("Shed Tail") {
            // Only the substitute goes along, and no stat stages.
            if let Some(m) = self.sides[s].mons.iter().find(|m| m.slot == Some(slot)) {
                let vols: Vec<String> = m
                    .volatiles
                    .iter()
                    .filter(|v| *v == "substitute")
                    .cloned()
                    .collect();
                passed = Some(([0; 7], vols));
            }
        }
        if l.tag("from") == Some("Baton Pass") {
            if let Some(m) = self.sides[s].mons.iter().find(|m| m.slot == Some(slot)) {
                const NO_COPY: [&str; 11] = [
                    "choicelock",
                    "encore",
                    "disable",
                    "imprison",
                    "yawn",
                    "glaiverush",
                    "minimize",
                    "attract",
                    "saltcure",
                    "destinybond",
                    "trapped",
                ];
                let vols: Vec<String> = m
                    .volatiles
                    .iter()
                    .filter(|v| !NO_COPY.contains(&v.as_str()) && !v.starts_with("stockpile"))
                    .cloned()
                    .collect();
                passed = Some((m.boosts, vols));
                passed_since = Some((
                    m.volatile_since.clone(),
                    m.volatile_length.clone(),
                    m.bound_band,
                ));
            }
        }
        // The Pokémon that held the slot leaves the field (seen until now
        // for what it is, unless an Illusion showed it as another).
        let now = self.switches;
        for m in self.sides[s].mons.iter_mut() {
            if m.slot == Some(slot) {
                if m.shown_as.is_none() {
                    m.entered = now;
                }
                m.leave_field();
            }
        }
        // One's own Illusion user enters under its disguise's name: its own
        // max HP tells them apart.
        let own_illusion = if s == self.me {
            self.own_illusion_entrant(s, slot, name, l.arg(2))
        } else {
            self.foe_illusion_entrant(s, slot, name)
        };
        let side = &mut self.sides[s];
        let k = own_illusion.unwrap_or_else(|| {
            side.mons
                .iter()
                .position(|m| m.name == name)
                .or_else(|| {
                    side.mons
                        .iter()
                        .position(|m| m.name.is_empty() && same_species(&m.preview, species))
                })
                .unwrap_or_else(|| {
                    side.mons.push(MonView {
                        preview: species.to_string(),
                        ..Default::default()
                    });
                    side.mons.len() - 1
                })
        });
        if slot < 3 {
            self.switch_snapshots[s][slot] = Some((k, self.sides[s].mons[k].clone()));
        }
        self.switches += 1;
        let dex = self.dex.clone();
        let side = &mut self.sides[s];
        let m = &mut side.mons[k];
        m.entered = self.switches;
        m.faint_slot = None;
        if own_illusion.is_none() {
            m.name = name.to_string();
            m.species = species_id.or(m.species);
            m.level = level;
            m.gender = gender;
        } else {
            m.shown_as = Some(name.to_string());
        }
        m.seen = true;
        m.slot = Some(slot);
        m.boosts = [0; 7];
        m.volatiles.clear();
        m.added_type = None;
        m.clear_field_state();
        // A Pokémon coming in during the turn has no move queued.
        m.acted_this_turn = true;
        m.entered_this_turn = true;
        if let Some((since, length, band)) = passed_since {
            m.volatile_since = since;
            m.volatile_length = length;
            m.bound_band = band;
        }
        if let Some((boosts, mut vols)) = passed {
            // Gastro Acid's onCopy drops it for an ability it can't suppress.
            if m.ability.is_some_and(|a| dex.ability(a).cant_suppress) {
                vols.retain(|v| v != "gastroacid");
            }
            m.boosts = boosts;
            m.volatiles = vols;
        }
        if let Some((hp, maxhp, status, fainted)) = parse_condition(l.arg(2)) {
            m.hp = hp;
            m.maxhp = maxhp;
            m.status = status;
            m.fainted = fainted;
        }
    }

    /// A permanent forme change (Mega Evolution): a Mega's single ability
    /// is known from its data, and becomes its base ability.
    fn on_detailschange(&mut self, l: &Line) {
        self.mega_unmask(l.arg(0));
        let (species, _, _) = parse_details(l.arg(1));
        let id = self.dex.species_id(species);
        let mega_ability = id.filter(|&s| self.dex.species(s).is_mega).and_then(|s| {
            self.dex
                .species(s)
                .abilities
                .first()
                .and_then(|(_, a)| self.dex.ability_id(a))
        });
        if let Some(m) = self.mon_mut(l.arg(0)) {
            m.species = id.or(m.species);
            // setSpecies drops the added type and any type change.
            m.added_type = None;
            m.types = None;
            if mega_ability.is_some() {
                m.ability = mega_ability;
                m.base_ability = mega_ability;
            }
        }
    }

    fn on_move(&mut self, l: &Line) {
        let user = l.arg(0);
        let name = l.arg(1).to_string();
        self.called_move = None;
        self.move_line = None;
        let id = self.dex.move_id(&name);
        // The field's last move: every move used, called and bounced ones too.
        if id.is_some() {
            self.last_move = id;
        }
        let prev = self.act.take();
        // A move an ability made the Pokémon use (Magic Bounce) shows the
        // ability, not one of its moves.
        if let Some(Named::Ability(a)) = l.tag("from").map(named) {
            let (user, a) = (user.to_string(), a.to_string());
            self.reveal_ability(&user, &a);
            // The bounced move failed for its user (TryHit's null counts as a
            // failure), and its result is the holder's.
            if let Some(p) = prev {
                if p.user != user {
                    self.act_result(&p.user, Some(false));
                }
            }
            self.start_act(&user, id);
            return;
        }
        // The Pokémon's own action (`moveUsed`): not a move another move
        // called; a rampage's or a charged move's later turns are.
        let own = match l.tag("from").map(named) {
            None => true,
            Some(Named::Move(caller)) => caller == name,
            Some(Named::Other(o)) => o == "lockedmove",
            _ => false,
        };
        if own {
            if let Some(loc) = id.and_then(|id| self.target_loc(user, id, l.arg(2))) {
                if let Some(m) = self.mon_mut(user) {
                    m.last_move_target_loc = loc;
                }
            }
        }
        self.start_act(user, id);
        if let Some(m) = self.mon_mut(user) {
            m.acted_this_turn = true;
        }
        self.move_effects(l);
        // A move another move called (Copycat, Sleep Talk) spends no PP and
        // leaves the caller as the last move; Sleep Talk only picks the
        // user's own moves. (Round's "[from] move: Round" names itself.)
        if let Some(Named::Move(caller)) = l.tag("from").map(named) {
            if caller != name {
                let user = user.to_string();
                if caller == "Sleep Talk" {
                    self.reveal_move(&user, &name, 0);
                } else {
                    self.called_move = Some((user, name));
                }
                return;
            }
        }
        // Struggle is no move of the Pokémon's own, and a locked move's later
        // turns repeat a move already shown, or one a called move started.
        if name != "Struggle" && l.tag("from") != Some("lockedmove") {
            self.reveal_move(user, &name, 1);
        }
        let id = self.dex.move_id(&name);
        if let Some(m) = self.mon_mut(user) {
            if id.is_some() {
                m.last_move = id;
            }
            m.remove_volatile("twoturnmove");
            m.remove_volatile("glaiverush");
            m.remove_volatile("mustrecharge");
            // Destiny Bond ends before any move (a second Destiny Bond
            // removes it and fails; a first one's `-singlemove` follows).
            m.remove_volatile("destinybond");
        }
    }

    /// What a move line starts that the log shows no other way: Wish and
    /// Healing Wish on the user's slot, Minimize, Ally Switch's counter,
    /// and a rampage (once a hit shows it landed). A called move (Copycat,
    /// Metronome) counts too.
    fn move_effects(&mut self, l: &Line) {
        let user = l.arg(0).to_string();
        let dex = self.dex.clone();
        let id = dex.move_id(l.arg(1));
        let mut ml = MoveLine {
            user: user.clone(),
            id,
            target: l.arg(2).to_string(),
            ..Default::default()
        };
        let pos = parse_ref(&user).and_then(|(s, slot, _)| slot.filter(|&x| x < 2).map(|x| (s, x)));
        let mv = id.map(|i| dex.mv(i));
        let turn = self.turn;
        match mv.map(|m| m.id.as_str()).unwrap_or("") {
            "wish" => {
                if let Some((s, x)) = pos {
                    let slot = &mut self.sides[s].slots[x];
                    if slot.wish.is_none() {
                        slot.wish = Some(turn);
                        ml.wish = Some((s, x));
                    }
                }
            }
            "healingwish" => {
                if let Some((s, x)) = pos {
                    let slot = &mut self.sides[s].slots[x];
                    if !slot.healing_wish {
                        slot.healing_wish = true;
                        ml.healing_wish = Some((s, x));
                    }
                }
            }
            "minimize" => {
                if let Some(m) = self.mon_mut(&user) {
                    if !m.has_volatile("minimize") {
                        m.add_volatile("minimize");
                        ml.minimize = true;
                    }
                }
            }
            // Used again the next turn, the counter triples (to 729 at most).
            "allyswitch" => {
                if let Some(m) = self.mon_mut(&user) {
                    let counter = match m.ally_switch {
                        Some((c, _)) if c < 729 => c * 3,
                        Some((c, _)) => c,
                        None => 3,
                    };
                    m.ally_switch = Some((counter, turn));
                }
            }
            _ => {}
        }
        if mv.is_some_and(|m| m.self_volatile.as_deref() == Some("lockedmove")) {
            let more = l.tag("from") == Some("lockedmove");
            if let Some(m) = self.mon_mut(&user) {
                let prev = m.locked.map(|(u, _)| u).unwrap_or(0);
                // A miss or a block ends it: the hit confirms it again.
                m.locked = None;
                ml.lock_uses = Some(if more { prev + 1 } else { 1 });
            }
        }
        self.move_line = Some(ml);
    }

    /// A `move` line starts the move's own lines: it succeeds unless they
    /// show otherwise.
    fn start_act(&mut self, user: &str, id: Option<MoveId>) {
        let me = self.mon_index(user);
        let will_act = (0..2).any(|s| {
            self.sides[s].mons.iter().enumerate().any(|(k, m)| {
                Some((s, k)) != me && m.slot.is_some() && !m.fainted && !m.acted_this_turn
            })
        });
        self.act = Some(Act {
            user: user.to_string(),
            id,
            will_act,
            hits: Vec::new(),
            changed: false,
        });
        self.act_result(user, Some(true));
    }

    /// `act` is a status move aimed at a foe behind a substitute, which it
    /// doesn't bypass.
    fn status_move_subbed(&self, act: &Act) -> bool {
        let Some(id) = act.id else { return false };
        let mv = self.dex.mv(id);
        let aimed = !matches!(
            mv.target,
            Target::SelfTarget
                | Target::All
                | Target::AllySide
                | Target::AllyTeam
                | Target::FoeSide
                | Target::AdjacentAllyOrSelf
        );
        let Some((us, uk)) = self.mon_index(&act.user) else {
            return false;
        };
        let infiltrator = self.dex.ability_id("Infiltrator");
        mv.category == Category::Status
            && aimed
            && !mv.has(MoveFlag::BypassSub)
            && (infiltrator.is_none() || self.sides[us].mons[uk].ability != infiltrator)
            && self.sides[1 - us]
                .mons
                .iter()
                .any(|m| m.slot.is_some() && !m.fainted && m.has_volatile("substitute"))
    }

    /// The result of `who`'s move this turn.
    fn act_result(&mut self, who: &str, result: Option<bool>) {
        if let Some(m) = self.mon_mut(who) {
            m.move_this_turn = result;
        }
    }

    /// An ability that took the move in (Volt Absorb's heal, Motor Drive's
    /// boost, Flash Fire): TryHit's null, a failure.
    fn on_absorbed(&mut self, target: &str, effect: Option<&str>) {
        const ABSORB: [&str; 12] = [
            "Volt Absorb",
            "Water Absorb",
            "Dry Skin",
            "Earth Eater",
            "Motor Drive",
            "Lightning Rod",
            "Storm Drain",
            "Sap Sipper",
            "Flash Fire",
            "Well-Baked Body",
            "Wind Rider",
            "Eelevate",
        ];
        let Some(Named::Ability(a)) = effect.map(named) else {
            return;
        };
        if !ABSORB.contains(&a) {
            return;
        }
        if let Some(user) = self.act.as_ref().map(|x| x.user.clone()) {
            if user != target {
                self.act_result(&user, Some(false));
            }
        }
    }

    /// Showdown's target location for a move chosen without a target: the
    /// user's own slot for a move on itself or its side, nothing for one
    /// only an ally can take in singles, else the foe (in doubles, the
    /// Pokémon the line names).
    fn target_loc(&self, user: &str, id: MoveId, shown: &str) -> Option<i8> {
        let (s, Some(slot), _) = parse_ref(user)? else {
            return None;
        };
        let mv = self.dex.mv(id);
        let singles = self.singles.unwrap_or_else(|| {
            (0..2).all(|x| self.sides[x].mons.iter().all(|m| m.slot.unwrap_or(0) == 0))
        });
        // Champions: Curse is a move on the user unless it is a Ghost type.
        let ghost = self.dex.type_id("Ghost");
        let curse_self = mv.id == "curse"
            && !self.mon_index(user).is_some_and(|(us, uk)| {
                let m = &self.sides[us].mons[uk];
                let types = self.current_types(m).unwrap_or_default();
                ghost.is_some_and(|g| types.contains(&g))
            });
        let own = -(slot as i8 + 1);
        Some(match mv.target {
            _ if curse_self => own,
            Target::SelfTarget
            | Target::All
            | Target::AllySide
            | Target::AllyTeam
            | Target::AdjacentAllyOrSelf => own,
            Target::AdjacentAlly if singles => 0,
            _ if singles => 1,
            _ => match parse_ref(shown) {
                Some((ts, Some(tslot), _)) if ts == s => -(tslot as i8 + 1),
                Some((_, Some(tslot), _)) => tslot as i8 + 1,
                _ => 1,
            },
        })
    }

    /// A move's hit on another Pokémon (`-damage` without a source): it
    /// counts in the target's `timesAttacked`, and its `attackedBy` keeps the
    /// attacker with the HP the last hit took. `before` is the target's HP
    /// before the line.
    fn on_hit(&mut self, l: &Line, before: Option<u16>) {
        if l.tag("from").is_some() {
            return;
        }
        let Some(act) = self.act.as_ref() else { return };
        let Some(id) = act.id else { return };
        if self.dex.mv(id).category == Category::Status || act.user == l.arg(0) {
            return;
        }
        let (Some(t), Some((us, uk))) = (self.mon_index(l.arg(0)), self.mon_index(&act.user))
        else {
            return;
        };
        if t == (us, uk) {
            return;
        }
        let slot = parse_ref(&act.user).and_then(|(_, sl, _)| sl).unwrap_or(0);
        let name = self.sides[us].mons[uk].name.clone();
        let act = self.act.as_mut().unwrap();
        let first = match act.hits.iter_mut().find(|(r, _)| *r == t) {
            Some(e) => {
                e.1 += 1;
                false
            }
            None => {
                act.hits.push((t, 1));
                true
            }
        };
        let m = &mut self.sides[t.0].mons[t.1];
        let damage = before.unwrap_or(m.hp).saturating_sub(m.hp);
        m.times_attacked = m.times_attacked.saturating_add(1);
        if first {
            m.attacked_by.push(AttackView {
                side: us,
                name,
                slot,
                mv: id,
                damage,
                this_turn: true,
            });
        } else if let Some(a) = m.attacked_by.last_mut() {
            a.damage = damage;
        }
    }

    /// `-fail|USER`: the move just used did nothing.
    fn on_fail(&mut self, l: &Line) {
        let who = l.arg(0);
        // The move failed (`-fail` on its user or a target). One an ability
        // or item caused stopped only an effect (Clear Body against Parting
        // Shot's drops): a move that fails with it shows a plain `-fail` of
        // its user too. A user without the type for Burn Up or Double Shock
        // (and Teatime with no berry) is no result; some lines are no failure
        // at all: the heal moves that return NOT_FAIL at full HP (Moonlight,
        // Swallow, Heal Pulse, Pollen Puff; not Recover or Roost), Ally
        // Switch without an ally, Corrosive Gas on no item.
        if let Some(act) = self.act.clone() {
            let kind = act.id.map(|i| self.dex.move_kind[i as usize]);
            let fine = match l.arg(1) {
                "heal" => matches!(
                    kind,
                    Some(
                        MoveKind::WeatherHeal
                            | MoveKind::Swallow
                            | MoveKind::PollenPuff
                            | MoveKind::HealPulse
                    )
                ),
                "move: Ally Switch" | "move: Corrosive Gas" => true,
                // A status move a substitute blocked did nothing, as a
                // success (its TryPrimaryHit's null keeps the target).
                "" => act.user == who && self.status_move_subbed(&act),
                _ => false,
            };
            if l.tag("from").is_none() && !fine {
                let none = matches!(
                    l.arg(1),
                    "move: Burn Up" | "move: Double Shock" | "move: Teatime" | "move: Aura Wheel"
                );
                self.act_result(&act.user, (!none).then_some(false));
            }
            // Protection that lost its roll (another Pokémon still to act):
            // the counter ends.
            let stalls = act.id.is_some_and(|i| {
                matches!(
                    self.dex.move_kind[i as usize],
                    MoveKind::Protect(_) | MoveKind::Endure
                )
            });
            if act.user == who && act.will_act && stalls {
                if let Some(m) = self.mon_mut(who) {
                    m.stall = None;
                }
            }
        }
        let Some(ml) = self.move_line.clone() else {
            return;
        };
        if ml.user != who {
            return;
        }
        if let Some((s, x)) = ml.wish {
            self.sides[s].slots[x].wish = None;
        }
        if let Some((s, x)) = ml.healing_wish {
            self.sides[s].slots[x].healing_wish = false;
        }
        let ally_switch = ml.id.is_some_and(|i| self.dex.mv(i).id == "allyswitch");
        if let Some(m) = self.mon_mut(who) {
            if ml.minimize {
                m.remove_volatile("minimize");
            }
            // Its PrepareHit failed (the counter's roll) and the counter
            // goes; "-fail|X|move: Ally Switch" (no ally) keeps it.
            if ally_switch && l.arg(1).is_empty() {
                m.ally_switch = None;
            }
        }
        if let Some(ml) = self.move_line.as_mut() {
            ml.wish = None;
            ml.healing_wish = None;
            ml.minimize = false;
        }
    }

    /// The move line's rampage hit `target`: the user is locked into it
    /// (for a third use, it ends now).
    fn confirm_lock(&mut self, target: &str) {
        let Some(ml) = self.move_line.as_mut() else {
            return;
        };
        let Some(uses) = ml.lock_uses else { return };
        if ml.user == target {
            return;
        }
        ml.lock_uses = None;
        let user = ml.user.clone();
        let turn = self.turn;
        if let Some(m) = self.mon_mut(&user) {
            m.locked = (uses < 3).then_some((uses, turn));
        }
    }

    /// `-damage`: Showdown's `hurtThisTurn`, and a rampage's hit.
    fn on_damage(&mut self, l: &Line) {
        let who = l.arg(0);
        let from = l.tag("from");
        if from.is_none() {
            self.confirm_lock(who);
        }
        // Direct damage doesn't count: Struggle's recoil, and the HP a
        // status move costs its user (Substitute, Belly Drum, Curse...).
        let dex = self.dex.clone();
        let paid = from.is_none()
            && self.move_line.as_ref().is_some_and(|ml| {
                ml.user == who
                    && ml
                        .id
                        .is_some_and(|i| dex.mv(i).category == Category::Status)
            });
        if from == Some("recoil") || paid {
            return;
        }
        if let Some(m) = self.mon_mut(who) {
            // A foe's HP is a percentage, so a small hit can leave it as it
            // was: the line itself (shown only for nonzero damage) counts.
            m.hurt_this_turn = true;
        }
    }

    /// `|turn|`: the per-turn flags reset, and a rampage not used last
    /// turn is over.
    fn on_new_turn(&mut self) {
        let turn = self.turn;
        self.upkeep_done = false;
        // endTurn: the hits taken stop being this turn's, and those whose
        // attacker has left the field are forgotten.
        let on_field: Vec<(usize, String)> = (0..2)
            .flat_map(|s| {
                self.sides[s]
                    .mons
                    .iter()
                    .filter(|m| m.slot.is_some() && !m.fainted)
                    .map(move |m| (s, m.name.clone()))
            })
            .collect();
        for side in self.sides.iter_mut() {
            for m in side.mons.iter_mut().filter(|m| m.slot.is_some()) {
                m.attacked_by.retain_mut(|a| {
                    a.this_turn = false;
                    on_field.iter().any(|(s, n)| *s == a.side && *n == a.name)
                });
                m.move_last_turn = m.move_this_turn.take();
            }
        }
        for side in self.sides.iter_mut() {
            for m in side.mons.iter_mut() {
                m.hurt_this_turn = false;
                m.acted_this_turn = false;
                m.entered_this_turn = false;
                // Showdown keeps them going into turn 1: what the leads'
                // abilities did (Intimidate) still counts.
                if turn != 1 {
                    m.stats_raised_this_turn = false;
                    m.stats_lowered_this_turn = false;
                }
                if m.locked.is_some_and(|(_, last)| last + 1 < turn) {
                    m.locked = None;
                }
            }
        }
    }

    /// `|upkeep|` closes the residual phase: what lasted the turn ends, and
    /// the slot conditions due have landed (silently, on an empty slot).
    fn on_upkeep(&mut self) {
        let turn = self.turn;
        self.upkeep_done = true;
        for side in self.sides.iter_mut() {
            side.conditions.retain(|(c, _)| !c.is_guard());
            for sl in side.slots.iter_mut() {
                if sl.wish.is_some_and(|t| turn > t) {
                    sl.wish = None;
                }
                if sl.future.is_some_and(|t| turn >= t + 2) {
                    sl.future = None;
                }
            }
            for m in side.mons.iter_mut() {
                // The stall counter's residual: it ends after two.
                if let Some((c, d)) = m.stall {
                    m.stall = (d > 1).then_some((c, d - 1));
                }
                m.remove_volatile("endure");
                if m.ally_switch.is_some_and(|(_, t)| turn > t) {
                    m.ally_switch = None;
                }
                // Sleep ends a rampage (onResidual).
                if m.status == Status::Slp {
                    m.locked = None;
                }
            }
        }
        if self.field.fairy_lock.is_some_and(|t| turn > t) {
            self.field.fairy_lock = None;
        }
    }

    /// A slot condition used up: Wish's and Healing Wish's heal, Future
    /// Sight's hit, on the Pokémon now in the slot.
    fn end_slot_condition(&mut self, who: &str, effect: &str) {
        let Some((s, Some(x), _)) = parse_ref(who) else {
            return;
        };
        if x >= 2 {
            return;
        }
        let slot = &mut self.sides[s].slots[x];
        match effect {
            "wish" => slot.wish = None,
            "healingwish" => slot.healing_wish = false,
            "futuresight" | "doomdesire" => slot.future = None,
            _ => {}
        }
    }

    /// `-transform|USER|TARGET`: the user takes the target's species, boosts,
    /// moves (5 PP each at most) and ability, as far as they are known,
    /// until it leaves the field. Imposter names itself.
    fn on_transform(&mut self, l: &Line) {
        let (user, target) = (l.arg(0).to_string(), l.arg(1).to_string());
        self.attribute(l, &user);
        let Some((ts, tk)) = self.mon_index(&target) else {
            return;
        };
        let t = self.sides[ts].mons[tk].clone();
        let dex = self.dex.clone();
        let Some(m) = self.mon_mut(&user) else { return };
        if m.pre_transform.is_none() {
            m.pre_transform = Some(Box::new(PreTransform {
                moves: std::mem::take(&mut m.moves),
                ability: m.ability,
                base_ability: m.base_ability,
                ability_swapped: m.ability_swapped,
            }));
        }
        m.transformed = t.transformed.or(t.species);
        m.boosts = t.boosts;
        m.times_attacked = t.times_attacked;
        m.added_type = t.added_type;
        m.types.clone_from(&t.types);
        m.moves = t
            .moves
            .iter()
            .map(|mv| {
                let pp = dex.mv(mv.id).pp.min(5);
                MoveView {
                    id: mv.id,
                    uses: 0,
                    pp: None,
                    maxpp: Some(pp),
                }
            })
            .collect();
        m.ability = t.ability;
        m.base_ability = None;
        m.ability_swapped = false;
    }

    fn on_cant(&mut self, l: &Line) {
        let reason = l.arg(1);
        // The Pokémon whose move is stopped: the line's own, or for Armor
        // Tail and the like (`cant|holder|ability: X|move|[of] attacker`)
        // the attacker.
        let stopped = match (named(reason), l.tag("of")) {
            (Named::Ability(_), Some(of)) => of.to_string(),
            _ => l.arg(0).to_string(),
        };
        if let Some(m) = self.mon_mut(&stopped) {
            m.acted_this_turn = true;
            // A move that didn't happen failed, unless the Pokémon had no
            // choice (recharging).
            m.move_this_turn = (reason != "recharge").then_some(false);
        }
        if let Some(m) = self.mon_mut(l.arg(0)) {
            // Glaive Rush's drawback ends before the next move, used or not;
            // a charged move is lost when the Pokémon can't move.
            m.remove_volatile("glaiverush");
            m.remove_volatile("twoturnmove");
            m.remove_volatile("destinybond");
            if reason == "recharge" {
                m.remove_volatile("mustrecharge");
            }
        }
        // "cant|X|Disable|Move", "cant|X|move: Taunt|Move" name the chosen move.
        let mv = l.arg(2);
        if !mv.is_empty() {
            match named(reason) {
                // Armor Tail and the like: the holder blocks the attacker's move.
                Named::Ability(a) => {
                    let a = a.to_string();
                    self.reveal_ability(l.arg(0), &a);
                    if let Some(of) = l.tag("of") {
                        let mv = mv.to_string();
                        self.reveal_move(of, &mv, 0);
                    }
                }
                _ => {
                    let mv = mv.to_string();
                    self.reveal_move(l.arg(0), &mv, 0);
                }
            }
        }
    }

    fn on_hp(&mut self, l: &Line) {
        let who = l.arg(0);
        if let Some((hp, maxhp, status, fainted)) = parse_condition(l.arg(1)) {
            if let Some(m) = self.mon_mut(who) {
                m.hp = hp;
                if maxhp > 0 {
                    m.maxhp = maxhp;
                }
                if !fainted {
                    m.status = status;
                }
                if fainted {
                    m.hp = 0;
                }
                // Revival Blessing brings a fainted teammate back.
                if !fainted && l.kind == "-heal" && l.tag("from") == Some("move: Revival Blessing")
                {
                    m.fainted = false;
                }
            }
        }
        let Some(from) = l.tag("from") else { return };
        // Hurting itself in confusion aborts the Pokémon's move: a charged
        // move is lost, and Glaive Rush's drawback had already ended.
        if from == "confusion" && l.kind == "-damage" {
            if let Some(m) = self.mon_mut(who) {
                m.remove_volatile("twoturnmove");
                m.remove_volatile("glaiverush");
            }
            return;
        }
        let of = l.tag("of");
        match named(from) {
            Named::Item(item) => {
                // Damage: Rocky Helmet names its holder in [of]; Life Orb, berries
                // and Leftovers belong to the Pokémon on the line.
                // A berry flung at the Pokémon heals it with [of] the flinger,
                // whose item the -enditem line settles; it isn't the target's.
                if l.kind == "-heal" && of.is_some() && item.ends_with("Berry") {
                    return;
                }
                let holder = if l.kind == "-damage" {
                    of.unwrap_or(who)
                } else {
                    who
                };
                let item = item.to_string();
                let holder = holder.to_string();
                self.set_item(&holder, &item);
            }
            Named::Ability(a) => {
                // Damage: Rough Skin, Solar Power name the holder in [of]. Heal: the
                // healed Pokémon's own ability (Volt Absorb, Regenerator), except
                // Hospitality, which heals an ally and names its holder in [of].
                let holder = if l.kind == "-damage" || a == "Hospitality" {
                    of.unwrap_or(who)
                } else {
                    who
                };
                let (a, holder) = (a.to_string(), holder.to_string());
                self.reveal_ability(&holder, &a);
            }
            _ => {}
        }
    }

    fn on_weather(&mut self, l: &Line) {
        let name = l.arg(0);
        if name == "none" {
            self.field.weather = None;
        } else if l.tag("upkeep").is_none() {
            if let Some(w) = Weather::from_id(&to_id(name)) {
                self.field.weather = Some((w, self.residual_mark()));
            }
        }
        self.attribute(l, "");
    }

    fn on_field(&mut self, l: &Line) {
        let id = to_id(l.arg(0).strip_prefix("move: ").unwrap_or(l.arg(0)));
        let start = l.kind == "-fieldstart";
        if id == "trickroom" {
            self.field.trick_room = if start { Some(self.turn) } else { None };
        } else if id == "gravity" {
            self.field.gravity = if start { Some(self.turn) } else { None };
        } else if id == "magicroom" {
            self.field.magic_room = if start { Some(self.turn) } else { None };
        } else if id == "wonderroom" {
            self.field.wonder_room = if start { Some(self.turn) } else { None };
        } else if let Some(t) = Terrain::from_id(&id) {
            if start {
                self.field.terrain = Some((t, self.residual_mark()));
            } else if self.field.terrain.map(|(x, _)| x) == Some(t) {
                self.field.terrain = None;
            }
        }
        self.attribute(l, "");
    }

    fn on_side(&mut self, l: &Line) {
        let Some(s) = side_index(l.arg(0).get(..2).unwrap_or("")) else {
            return;
        };
        let start = l.kind == "-sidestart";
        let cond = l.arg(1);
        match to_id(cond.strip_prefix("move: ").unwrap_or(cond)).as_str() {
            "toxicspikes" => {
                let side = &mut self.sides[s];
                side.toxic_spikes = if start {
                    (side.toxic_spikes + 1).min(2)
                } else {
                    0
                };
                return;
            }
            "stealthrock" => {
                self.sides[s].stealth_rock = start;
                return;
            }
            "spikes" => {
                let side = &mut self.sides[s];
                side.spikes = if start { (side.spikes + 1).min(3) } else { 0 };
                return;
            }
            "stickyweb" => {
                self.sides[s].sticky_web = start;
                return;
            }
            "safeguard" => {
                self.sides[s].safeguard = start.then_some(self.turn);
                return;
            }
            _ => {}
        }
        let Some(c) = side_cond(cond) else { return };
        let turn = self.turn;
        let side = &mut self.sides[s];
        side.conditions.retain(|(x, _)| *x != c);
        if start {
            side.conditions.push((c, turn));
        }
    }

    /// Court Change: the sides trade screens, Tailwind, Safeguard and hazards.
    fn swap_side_conditions(&mut self) {
        let [a, b] = &mut self.sides;
        let swapped = |c: &SideCond| !c.is_guard();
        let from_a: Vec<_> = a
            .conditions
            .iter()
            .copied()
            .filter(|(c, _)| swapped(c))
            .collect();
        let from_b: Vec<_> = b
            .conditions
            .iter()
            .copied()
            .filter(|(c, _)| swapped(c))
            .collect();
        a.conditions.retain(|(c, _)| !swapped(c));
        b.conditions.retain(|(c, _)| !swapped(c));
        a.conditions.extend(from_b);
        b.conditions.extend(from_a);
        std::mem::swap(&mut a.toxic_spikes, &mut b.toxic_spikes);
        std::mem::swap(&mut a.stealth_rock, &mut b.stealth_rock);
        std::mem::swap(&mut a.spikes, &mut b.spikes);
        std::mem::swap(&mut a.sticky_web, &mut b.sticky_web);
        std::mem::swap(&mut a.safeguard, &mut b.safeguard);
    }

    fn on_start(&mut self, l: &Line) {
        let who = l.arg(0);
        let effect = l.arg(1);
        let id = match named(effect) {
            Named::Ability(a) => {
                let a = a.to_string();
                self.reveal_ability(who, &a);
                to_id(&a)
            }
            Named::Move(m) => to_id(m),
            Named::Item(i) => to_id(i),
            Named::Other(o) => to_id(o),
        };
        // An added type stays until a type change (Reflect Type copies the
        // target's); it is no volatile, while "typechange" is kept as one.
        if id == "typeadd" || id == "typechange" {
            let added = if id == "typeadd" {
                self.dex.type_id(l.arg(2))
            } else if l.tag("from") == Some("move: Reflect Type") {
                l.tag("of")
                    .and_then(|o| self.mon_index(o))
                    .and_then(|(s, k)| self.sides[s].mons[k].added_type)
            } else {
                None
            };
            // The types set: listed ("Fire/Water"), or the target's for Reflect Type.
            let types = if id == "typeadd" {
                None
            } else if l.tag("from") == Some("move: Reflect Type") {
                l.tag("of")
                    .and_then(|o| self.mon_index(o))
                    .and_then(|(s, k)| self.current_types(&self.sides[s].mons[k]))
            } else {
                let ts: Vec<TypeId> = l
                    .arg(2)
                    .split('/')
                    .filter_map(|t| self.dex.type_id(t))
                    .collect();
                (!ts.is_empty()).then_some(ts)
            };
            if let Some(m) = self.mon_mut(who) {
                m.added_type = added;
                if id == "typechange" {
                    m.types = types;
                }
            }
            if id == "typeadd" {
                self.attribute(l, who);
                return;
            }
        }
        // Future Sight's start is a slot condition on its target's slot
        // (the move line's target), not a volatile.
        if id == "futuresight" || id == "doomdesire" {
            let target = self
                .move_line
                .as_ref()
                .filter(|ml| ml.user == who)
                .map(|ml| ml.target.clone());
            if let Some((s, Some(x), _)) = target.as_deref().and_then(parse_ref) {
                if x < 2 {
                    self.sides[s].slots[x].future = Some(self.turn);
                }
            }
            self.attribute(l, who);
            return;
        }
        // Rampage fatigue: the lock is over.
        if id == "confusion" && l.tag("fatigue").is_some() {
            if let Some(m) = self.mon_mut(who) {
                m.locked = None;
            }
        }
        // Disable names the disabled move.
        if id == "disable" && !l.arg(2).is_empty() {
            let mv = l.arg(2).to_string();
            self.reveal_move(who, &mv, 0);
        }
        if id == "octolock" {
            if let Some(of) = l.tag("of").map(str::to_string) {
                self.record_trap_source(who, "octolock", &of);
            }
        }
        let turn = self.turn;
        if let Some(m) = self.mon_mut(who) {
            // Perish Song's count replaces the previous one, and so does
            // Stockpile's ("stockpile1" to "stockpile3").
            if id.starts_with("perish") {
                m.volatiles.retain(|v| !v.starts_with("perish"));
            }
            if id.starts_with("stockpile") {
                m.volatiles.retain(|v| !v.starts_with("stockpile"));
            }
            // Smack Down grounds: Magnet Rise ends silently.
            if id == "smackdown" {
                m.remove_volatile("magnetrise");
            }
            m.add_volatile_at(&id, turn);
            // Showdown's onStart: Taunt is a turn longer on a Pokémon that
            // has already acted (and was on the field when the turn
            // began), Encore on one that has acted, and a move's Disable a
            // turn shorter on one still to act (Cursed Body's: always 4).
            let length = match id.as_str() {
                "taunt" => Some(3 + (m.acted_this_turn && !m.entered_this_turn) as u8),
                "encore" => Some(3 + m.acted_this_turn as u8),
                "disable" if l.tag("from").is_none() => Some(if m.acted_this_turn { 5 } else { 4 }),
                _ => None,
            };
            if let Some(n) = length {
                m.volatile_length.retain(|(v, _)| v != &id);
                m.volatile_length.push((id.clone(), n));
            }
        }
        self.attribute(l, who);
    }

    /// Whether the active Pokémon `k` of side `s` can't switch out, as far as
    /// the log and the player's own team show, and whether it could escape
    /// with a Shed Shell: its item is unknown and may be one, or is one.
    /// Trapping volatiles (`trapped`, partial trapping while its user is on
    /// the field, Octolock likewise, Ingrain, No Retreat), Fairy Lock, and a
    /// foe's known Shadow Tag trap it, unless it is a Ghost (by its known
    /// types) or, for Shadow Tag, has that ability itself.
    pub fn trap_state(&self, s: usize, k: usize) -> (bool, bool) {
        let m = &self.sides[s].mons[k];
        if m.fainted {
            return (false, false);
        }
        let shell = self.dex.item_id("Shed Shell");
        let shed = match m.item {
            ItemView::Known(i) => Some(i) == shell,
            ItemView::None => false,
            // Item Clause: one Shed Shell on a team.
            ItemView::Unknown => !self.sides[s]
                .mons
                .iter()
                .any(|o| shell.is_some_and(|i| o.item == ItemView::Known(i))),
        };
        if m.slot.is_none() {
            return (false, shed);
        }
        let dex = &self.dex;
        let ghost = self
            .current_types(m)
            .into_iter()
            .flatten()
            .chain(m.added_type)
            .any(|t| dex.type_immune_to_effect(t, "trapped"));
        if ghost {
            return (false, shed);
        }
        let held = |v: &str| m.has_volatile(v);
        let source_ok = |v: &str| {
            m.trap_sources
                .iter()
                .find(|(x, _)| x == v)
                .is_none_or(|(_, src)| self.source_active(*src))
        };
        let mut trapped = held("trapped")
            || held("ingrain")
            || held("noretreat")
            || (held("partiallytrapped") && source_ok("partiallytrapped"))
            || (held("octolock") && source_ok("octolock"))
            || self.field.fairy_lock.is_some();
        if !trapped {
            let tag = dex.ability_id("Shadow Tag");
            let can_tag = |ab: Option<AbilityId>, sp: Option<SpeciesId>| match ab {
                Some(a) => Some(a) == tag,
                None => sp.is_some_and(|sp| {
                    dex.species(sp)
                        .abilities
                        .iter()
                        .any(|(_, a)| a == "Shadow Tag")
                }),
            };
            // Unknown whether it has Shadow Tag itself: not counted as trapped.
            let own_tag = can_tag(m.ability, m.transformed.or(m.species));
            trapped = !own_tag
                && self.sides[1 - s].mons.iter().any(|f| {
                    f.slot.is_some()
                        && !f.fainted
                        && f.ability.is_some()
                        && f.ability == tag
                        && !f.has_volatile("gastroacid")
                });
        }
        (trapped, shed)
    }

    /// A Pokémon's types as far as known: a type change's, else its
    /// (copied) species'.
    fn current_types(&self, m: &MonView) -> Option<Vec<TypeId>> {
        if let Some(t) = &m.types {
            return Some(t.clone());
        }
        let sp = m.transformed.or(m.species)?;
        Some(
            self.dex
                .species(sp)
                .types
                .iter()
                .copied()
                .filter(|&t| t != NO_TYPE)
                .collect(),
        )
    }

    fn on_end(&mut self, l: &Line) {
        let who = l.arg(0);
        let effect = l.arg(1);
        let id = match named(effect) {
            Named::Ability(a) | Named::Move(a) | Named::Item(a) | Named::Other(a) => to_id(a),
        };
        // Future Sight hits the Pokémon in its slot.
        if id == "futuresight" || id == "doomdesire" {
            self.end_slot_condition(who, &id);
            return;
        }
        // A substitute broken by a rampage's hit: it landed.
        if id == "substitute" && l.tag("from").is_none() {
            self.confirm_lock(who);
        }
        // An Illusion broken shows its ability.
        if id == "illusion" {
            self.reveal_ability(who, "Illusion");
        }
        // "-end|X|Fire Spin|[partiallytrapped]" (Octolock's names Octolock).
        let trap = l.tag("partiallytrapped").is_some() && id != "octolock";
        if let Some(m) = self.mon_mut(who) {
            m.remove_volatile(&id);
            if trap {
                m.remove_volatile("partiallytrapped");
            }
            // A type change back (Mimicry) is a setType.
            if id == "typechange" {
                m.added_type = None;
                m.types = None;
            }
            // "-end|X|Stockpile" ends "stockpile1" to "stockpile3".
            if id == "stockpile" {
                m.volatiles.retain(|v| !v.starts_with("stockpile"));
            }
        }
    }

    fn on_item(&mut self, l: &Line) {
        let Some(id) = self.dex.item_id(l.arg(1)) else {
            return;
        };
        let tricked = matches!(l.tag("from"), Some("move: Trick" | "move: Switcheroo"));
        if let Some(m) = self.mon_mut(l.arg(0)) {
            // An item Trick gave can be used up (White Herb) before the line
            // that names it.
            if tricked && m.item == ItemView::None && m.prev_item == Some(id) {
                return;
            }
            m.item = ItemView::Known(id);
            m.prev_item = None;
        }
        // An item used up comes back (Recycle, Harvest), or another's is
        // picked up (Pickup): it is no longer the last one used.
        match l.tag("from") {
            Some("move: Recycle" | "ability: Harvest") => {
                if let Some(m) = self.mon_mut(l.arg(0)) {
                    m.last_item = None;
                }
            }
            Some("ability: Pickup") => {
                let picker = self.mon_index(l.arg(0));
                let from = (0..2)
                    .flat_map(|x| (0..self.sides[x].mons.len()).map(move |k| (x, k)))
                    .find(|&(x, k)| {
                        let m = &self.sides[x].mons[k];
                        Some((x, k)) != picker && m.slot.is_some() && m.last_item == Some(id)
                    });
                if let Some((x, k)) = from {
                    self.sides[x].mons[k].last_item = None;
                }
            }
            _ => {}
        }
        // Pickpocket is the new holder's ability; Frisk's holder is "[of]".
        match l.tag("from") {
            Some("ability: Pickpocket") => self.reveal_ability(l.arg(0), "Pickpocket"),
            Some("ability: Frisk") => {
                if let Some(of) = l.tag("of").map(str::to_string) {
                    self.reveal_ability(&of, "Frisk");
                }
            }
            // Covet and Magician take the item from "[of]" without an
            // `-enditem` line; Magician is the new holder's ability.
            Some(from @ ("move: Covet" | "ability: Magician")) => {
                if from == "ability: Magician" {
                    self.reveal_ability(l.arg(0), "Magician");
                }
                if let Some(m) = l.tag("of").and_then(|of| self.mon_mut(of)) {
                    m.item = ItemView::None;
                    m.prev_item = Some(id);
                }
            }
            _ => {}
        }
    }

    fn on_enditem(&mut self, l: &Line) {
        let who = l.arg(0);
        // The item was shown leaving; the Pokémon now holds nothing (Trick's
        // silent line is followed by the `-item` that gives it the new one).
        let id = self.dex.item_id(l.arg(1));
        // Bug Bite, Pluck: the berry's effect follows on the user ("[of]"),
        // who doesn't hold it.
        if l.tag("from") == Some("stealeat") {
            if let Some(m) = l.tag("of").and_then(|of| self.mon_mut(of)) {
                if m.item != ItemView::Known(id.unwrap_or(0)) || id.is_none() {
                    m.prev_item = id;
                }
            }
        }
        // A berry eaten ("[eat]") is one Cud Chew eats again, unless Cud
        // Chew is eating it now.
        let again = self.cud_eat.take().is_some_and(|c| c == who);
        let berry = id.is_some_and(|i| self.dex.item(i).is_berry);
        // Used up (Showdown's `lastItem`): eaten, or used (`useItem`: Focus
        // Sash, a seed, Red Card's "[of]"; a gem's; Fling's). Not when taken
        // (Knock Off, Thief, Bug Bite, Corrosive Gas) or popped (Air
        // Balloon); Cud Chew eats the same berry again.
        let eaten = l.tag("eat").is_some();
        let used = match l.tag("from") {
            None => {
                !eaten
                    && l.tag("silent").is_none()
                    && l.tag("weaken").is_none()
                    && l.arg(1) != "Air Balloon"
            }
            Some(f) => f == "gem" || f == "move: Fling",
        };
        if let Some(m) = self.mon_mut(who) {
            if eaten && berry {
                m.ate_berry = true;
            }
            if id.is_some() && ((eaten && !again) || used) {
                m.last_item = id;
            }
        }
        // Bug Bite, Pluck: the user ate the berry.
        if l.tag("from") == Some("stealeat") && berry {
            if let Some(m) = l.tag("of").and_then(|of| self.mon_mut(of)) {
                m.ate_berry = true;
            }
        }
        if l.tag("eat").is_some() && berry && !again {
            if let Some(m) = self.mon_mut(who) {
                m.cud_chew = true;
            }
        }
        let weaken = l.tag("weaken").is_some();
        if let Some(m) = self.mon_mut(who) {
            // A resist berry's "[weaken]" follows its "[eat]" line, and
            // Symbiosis may have handed over a new item in between.
            if weaken && m.prev_item == id && id.is_some() {
                return;
            }
            m.item = ItemView::None;
            m.prev_item = id;
        }
    }

    fn on_ability(&mut self, l: &Line) {
        let who = l.arg(0).to_string();
        let name = l.arg(1).to_string();
        let dex = self.dex.clone();
        match l.tag("from").map(named) {
            // Trace: the new ability replaces the old one, which is shown too.
            Some(Named::Ability("Trace")) => {
                let old = l.arg(2).to_string();
                if let Some(m) = self.mon_mut(&who) {
                    m.ability = dex.ability_id(&name);
                    m.base_ability = dex.ability_id(&old);
                }
            }
            // Entrainment, Role Play, Worry Seed: "-ability|X|NEW|OLD|[from]
            // move: M|[of] Y". X has NEW until it leaves the field; Y, when
            // named, is the one NEW was copied from.
            // Receiver: "-ability|X|NEW|Receiver|[from] ability: Receiver|[of]
            // FAINTED", the same.
            Some(n)
                if matches!(&n, Named::Move(_))
                    || matches!(&n, Named::Ability(a) if *a == "Receiver") =>
            {
                let (new, old) = (dex.ability_id(&name), dex.ability_id(l.arg(2)));
                if let Some(m) = self.mon_mut(&who) {
                    m.swap_ability(new, old);
                }
                if let Some(of) = l.tag("of").map(str::to_string) {
                    self.reveal_ability(&of, &name);
                }
            }
            _ => {
                // "-ability|X|Intimidate|boost": X's own ability, except the
                // boost a bounced Mirror Armor drop shows on its target.
                if name != "Mirror Armor" || l.arg(2) != "boost" {
                    self.reveal_ability(&who, &name);
                }
            }
        }
    }

    /// `-activate`, `-immune`, `-fail`, `-block`: effects named in the line.
    fn on_activate(&mut self, l: &Line) {
        let who = l.arg(0).to_string();
        let effect = l.arg(1);
        if l.kind == "-fail" {
            self.on_fail(l);
        }
        // What stopped a move without failing it (Showdown's NOT_FAIL hit
        // result): protection. An immunity fails it (a type's, Levitate's,
        // and an ability's TryHit null: Soundproof, Good as Gold), and so
        // does Psychic Terrain against a priority move; not a move on the
        // whole field or a side (Perish Song), whose hit is the field's.
        if let Some((user, id)) = self.act.as_ref().map(|a| (a.user.clone(), a.id)) {
            if user != who {
                let blocked = l.kind == "-activate"
                    && matches!(
                        effect,
                        "move: Protect" | "move: Quick Guard" | "move: Wide Guard"
                    );
                if blocked {
                    self.act_result(&user, None);
                }
                if l.kind == "-activate" && effect == "move: Psychic Terrain" {
                    self.act_result(&user, Some(false));
                }
                let field = id.is_some_and(|i| {
                    matches!(
                        self.dex.mv(i).target,
                        Target::All | Target::FoeSide | Target::AllySide | Target::AllyTeam
                    )
                });
                if l.kind == "-immune" && !field {
                    self.act_result(&user, Some(false));
                }
            }
        }
        // Confusion shows before the confused Pokémon's move, whether it
        // then moves or hurts itself: its action has begun.
        if l.kind == "-activate" && effect == "confusion" {
            if let Some(m) = self.mon_mut(&who) {
                m.acted_this_turn = true;
            }
        }
        // Gravity brings a Pokémon in the air (Fly, Bounce) down mid-charge,
        // and ends Magnet Rise.
        if l.kind == "-activate" && effect == "move: Gravity" {
            if let Some(m) = self.mon_mut(&who) {
                m.remove_volatile("twoturnmove");
                m.remove_volatile("magnetrise");
            }
        }
        // Mean Look, Block, Spirit Shackle and Jaw Lock: "-activate|X|trapped".
        // The trapper is the move's user, or for Jaw Lock's own trap the
        // target; the trap ends silently when it leaves the field.
        if l.kind == "-activate" && effect == "trapped" {
            let holder = self.move_line.as_ref().map(|ml| {
                if ml.user == who {
                    ml.target.clone()
                } else {
                    ml.user.clone()
                }
            });
            let turn = self.turn;
            if let Some(m) = self.mon_mut(&who) {
                m.add_volatile_at("trapped", turn);
            }
            if let Some(h) = holder {
                self.record_trap_source(&who, "trapped", &h);
            }
        }
        // "-activate|TARGET|move: Fire Spin|[of] USER": partial trapping starts.
        if l.kind == "-activate" {
            if let (Named::Move(mv), Some(of)) = (named(effect), l.tag("of")) {
                let traps = self.dex.move_id(mv).is_some_and(|id| {
                    self.dex.mv(id).volatile_status.as_deref() == Some("partiallytrapped")
                });
                if traps {
                    let band = self.dex.item_id("Binding Band");
                    let bound = self
                        .mon_mut(of)
                        .is_some_and(|u| band.is_some_and(|b| u.item == ItemView::Known(b)));
                    let turn = self.turn;
                    if let Some(m) = self.mon_mut(&who) {
                        m.add_volatile_at("partiallytrapped", turn);
                        m.bound_band = bound;
                    }
                    self.record_trap_source(&who, "partiallytrapped", of);
                }
            }
        }
        // Cud Chew eats its berry again: the `-enditem` that follows.
        if l.kind == "-activate" && effect == "ability: Cud Chew" {
            if let Some(m) = self.mon_mut(&who) {
                m.cud_chew = false;
            }
            self.cud_eat = Some(who.clone());
        }
        // A rampage that hits a substitute has landed.
        if l.kind == "-activate" && effect.ends_with("Substitute") && l.tag("damage").is_some() {
            self.confirm_lock(&who);
        }
        // Trick: what left either Pokémon before no longer matters.
        if l.kind == "-activate" && effect == "move: Trick" {
            let other = l.tag("of").unwrap_or("").to_string();
            for who in [who.as_str(), other.as_str()] {
                if let Some(m) = self.mon_mut(who) {
                    m.prev_item = None;
                }
            }
        }
        // Symbiosis: "-activate|HOLDER|ability: Symbiosis|ITEM|[of] ALLY".
        if l.kind == "-activate" && effect == "ability: Symbiosis" {
            let ally = l.tag("of").unwrap_or("").to_string();
            let item = l.arg(2).to_string();
            // The ally keeps the item it used as `prev_item`: a resist
            // berry's "[weaken]" line follows.
            let id = self.dex.item_id(&item);
            // (A holder passing an item it was just given back to itself
            // changes nothing.)
            if let (Some(m), true) = (self.mon_mut(&who), who != ally) {
                m.item = ItemView::None;
                m.prev_item = id;
            }
            if let (Some(id), Some(m)) = (id, self.mon_mut(&ally)) {
                m.item = ItemView::Known(id);
            }
        }
        // Feint, and other moves that break protection ("[broken]"), end the
        // guards on the target's side.
        if l.kind == "-activate" && (effect == "move: Feint" || l.tag("broken").is_some()) {
            if let Some((s, _, _)) = parse_ref(&who) {
                self.sides[s].conditions.retain(|(c, _)| !c.is_guard());
            }
            // and the target's stall counter.
            if let Some(m) = self.mon_mut(&who) {
                m.stall = None;
            }
        }
        // "-activate|HOLDER|ability: Forewarn|MOVE|[of] FOE": one of the foe's moves.
        if l.kind == "-activate" && effect == "ability: Forewarn" {
            if let Some(of) = l.tag("of").map(str::to_string) {
                let mv = l.arg(2).to_string();
                self.reveal_move(&of, &mv, 0);
            }
        }
        // Mummy: "-activate|HOLDER|ability: Mummy|ATTACKER|[ability] OLD"; the
        // attacker has Mummy until it leaves the field.
        if l.kind == "-activate" && effect == "ability: Mummy" {
            let attacker = l.arg(2).to_string();
            let dex = self.dex.clone();
            let (new, old) = (
                dex.ability_id("Mummy"),
                l.tag("ability").and_then(|a| dex.ability_id(a)),
            );
            if let Some(m) = self.mon_mut(&attacker) {
                m.swap_ability(new, old);
            }
        }
        // "-activate|TARGET|move: Poltergeist|ITEM": the target's item.
        if l.kind == "-activate" && effect == "move: Poltergeist" {
            let item = l.arg(2).to_string();
            self.set_item(&who, &item);
            return;
        }
        // "-activate|USER|Skill Swap|TARGET'S OLD|USER'S OLD|[of] TARGET"; the
        // abilities are left out between allies, and the known ones swap.
        if l.kind == "-activate" && effect == "Skill Swap" {
            let target = l.tag("of").unwrap_or("").to_string();
            let dex = self.dex.clone();
            let shown = |name: &str| {
                if name.is_empty() {
                    None
                } else {
                    dex.ability_id(name)
                }
            };
            let (named_target, named_user) = (shown(l.arg(2)), shown(l.arg(3)));
            let target_old = named_target.or_else(|| self.mon_mut(&target).and_then(|m| m.ability));
            let user_old = named_user.or_else(|| self.mon_mut(&who).and_then(|m| m.ability));
            if let Some(m) = self.mon_mut(&who) {
                m.swap_ability(target_old, user_old);
            }
            if let Some(m) = self.mon_mut(&target) {
                m.swap_ability(user_old, target_old);
            }
            return;
        }
        match l.kind {
            "-activate" | "-block" => match named(effect) {
                Named::Ability(a) => {
                    // "-block|target|ability: Flower Veil|[of] holder".
                    let holder = if l.kind == "-block" {
                        l.tag("of").unwrap_or(&who).to_string()
                    } else {
                        who.clone()
                    };
                    let a = a.to_string();
                    self.reveal_ability(&holder, &a);
                }
                // "[consumed]": the item was eaten just before (Leppa Berry).
                Named::Item(i) if l.tag("consumed").is_none() => {
                    let i = i.to_string();
                    self.set_item(&who, &i);
                }
                _ => {}
            },
            _ => {}
        }
        // "-immune|X|[from] ability: Levitate", "-fail|X|unboost|[from] ability: Clear Body|[of] X".
        if let Some(from) = l.tag("from") {
            if let Named::Ability(a) = named(from) {
                let holder = if l.kind == "-fail" {
                    l.tag("of").unwrap_or(&who).to_string()
                } else {
                    who.clone()
                };
                let a = a.to_string();
                self.reveal_ability(&holder, &a);
            }
        }
    }

    /// "[from] ability: X|[of] HOLDER" on weather, terrain, status and other lines.
    fn attribute(&mut self, l: &Line, subject: &str) {
        let Some(from) = l.tag("from") else { return };
        match named(from) {
            Named::Ability(a) => {
                let holder = l.tag("of").unwrap_or(subject).to_string();
                if !holder.is_empty() {
                    let a = a.to_string();
                    self.reveal_ability(&holder, &a);
                }
            }
            Named::Item(i) if !subject.is_empty() => {
                let (i, subject) = (i.to_string(), subject.to_string());
                self.set_item(&subject, &i);
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------ reveals

    fn reveal_move(&mut self, who: &str, name: &str, uses: u16) {
        if self
            .called_move
            .as_ref()
            .is_some_and(|(u, m)| u == who && m == name)
        {
            return;
        }
        let Some(id) = self.dex.move_id(name) else {
            return;
        };
        if let Some(m) = self.mon_mut(who) {
            match m.moves.iter_mut().find(|mv| mv.id == id) {
                Some(mv) => mv.uses += uses,
                None => m.moves.push(MoveView {
                    id,
                    uses,
                    ..Default::default()
                }),
            }
        }
    }

    fn reveal_ability(&mut self, who: &str, name: &str) {
        let Some(id) = self.dex.ability_id(name) else {
            return;
        };
        if let Some(m) = self.mon_mut(who) {
            if m.ability.is_none() {
                m.ability = Some(id);
            }
        }
    }

    /// An item named as the source of an effect: still held, unless it is
    /// the one just shown leaving (a berry's heal after it was eaten).
    fn set_item(&mut self, who: &str, name: &str) {
        let Some(id) = self.dex.item_id(name) else {
            return;
        };
        if let Some(m) = self.mon_mut(who) {
            if m.prev_item != Some(id) {
                m.item = ItemView::Known(id);
            }
        }
    }
}

impl Tracker {
    /// One's own Illusion user switching in under another's name: the
    /// named Pokémon is already out elsewhere, or its max HP isn't the line's.
    fn own_illusion_entrant(
        &self,
        s: usize,
        slot: usize,
        name: &str,
        condition: &str,
    ) -> Option<usize> {
        let (_, maxhp, _, _) = parse_condition(condition)?;
        let side = &self.sides[s];
        let named = side.mons.iter().find(|m| m.name == name)?;
        let elsewhere = named.slot.is_some_and(|x| x != slot);
        if !elsewhere && (named.maxhp == 0 || named.maxhp == maxhp) {
            return None;
        }
        let illusion = self.dex.ability_id("Illusion");
        side.mons.iter().position(|m| {
            m.ability.is_some()
                && m.ability == illusion
                && m.maxhp == maxhp
                && !m.fainted
                && m.slot.is_none()
        })
    }

    /// A foe switching in under the name of a teammate already on the field
    /// can only be its Illusion user: the one Pokémon on its team that can
    /// have Illusion and isn't out.
    /// The one already out is taken for the Illusion user and unmasked: it
    /// is when the entrant is to its right (an Illusion only shows a
    /// teammate to its right), and most likely otherwise, unless the rest
    /// of the team is down or the user was seen since. A `replace` corrects
    /// a wrong guess.
    fn foe_illusion_entrant(&mut self, s: usize, slot: usize, name: &str) -> Option<usize> {
        let side = &self.sides[s];
        let k = side.mons.iter().position(|m| m.name == name)?;
        let named = &side.mons[k];
        let other = named.slot.filter(|&x| x != slot && !named.fainted)?;
        let dex = &self.dex;
        let can_illusion = |m: &MonView| {
            let sp = m.species.or_else(|| dex.species_id(&m.preview));
            sp.is_some_and(|sp| {
                dex.species(sp)
                    .abilities
                    .iter()
                    .any(|(_, a)| a == "Illusion")
            })
        };
        let mut found = side
            .mons
            .iter()
            .enumerate()
            .filter(|(_, m)| m.slot.is_none() && !m.fainted && can_illusion(m));
        let user = match (found.next(), found.next()) {
            (Some((u, _)), None) => u,
            _ => return None,
        };
        // Seen for what it is since the other came in: the other is real.
        if side.mons[user].entered > named.entered {
            return Some(user);
        }
        // The entrant to the left could have taken the look of the one to
        // its right only if no other teammate was left on the bench; then
        // it is taken for the user.
        let seen = side.mons.iter().filter(|m| m.seen).count();
        let unseen = side
            .team_size
            .saturating_sub(seen + usize::from(!side.mons[user].seen));
        let bench = side
            .mons
            .iter()
            .enumerate()
            .filter(|&(i, m)| i != user && i != k && m.seen && !m.fainted && m.slot.is_none())
            .count();
        if slot < other && bench + unseen == 0 {
            return Some(user);
        }
        let snap = self.switch_snapshots[s]
            .get(other)
            .cloned()
            .flatten()
            .filter(|(c, _)| *c == k)
            .map(|(_, v)| v);
        self.unmask(s, other, k, user, snap);
        self.sides[s].mons[user].shown_as = Some(name.to_string());
        None
    }

    /// One's own Pokémon on the field, as the request lists them. When the
    /// log put another in that slot (an Illusion shown under a teammate's
    /// name with the same max HP), what it was seen doing there moves over.
    fn fix_own_slots(&mut self, actives: &[(String, usize)]) {
        let me = self.me;
        let side = &mut self.sides[me];
        for (name, i) in actives {
            let Some(k) = side.mons.iter().position(|m| &m.name == name) else {
                continue;
            };
            if side.mons[k].slot == Some(*i) {
                continue;
            }
            if let Some(j) = side.mons.iter().position(|m| m.slot == Some(*i)) {
                let shown = side.mons[j].clone();
                side.mons[j].clear_field_state();
                side.mons[k].copy_field_state(&shown);
                let (boosts, volatiles, last) = {
                    let o = &mut side.mons[j];
                    o.slot = None;
                    o.shown_as = None;
                    (
                        std::mem::take(&mut o.boosts),
                        std::mem::take(&mut o.volatiles),
                        o.last_move.take(),
                    )
                };
                let m = &mut side.mons[k];
                m.boosts = boosts;
                m.volatiles = volatiles;
                m.last_move = last;
            }
            side.mons[k].slot = Some(*i);
        }
        for m in side.mons.iter_mut() {
            if m.slot
                .is_some_and(|x| !actives.iter().any(|(n, i)| *i == x && *n == m.name))
            {
                m.slot = None;
            }
        }
    }

    /// `base` with what `shown` shows was learned since `before`: its field
    /// state, and the moves and item seen since.
    fn merged(base: &MonView, shown: &MonView, before: &MonView) -> MonView {
        let mut m = base.clone();
        m.slot = shown.slot;
        m.seen = true;
        m.hp = shown.hp;
        m.maxhp = shown.maxhp;
        m.status = shown.status;
        m.fainted = shown.fainted;
        m.boosts = shown.boosts;
        m.volatiles = shown.volatiles.clone();
        m.copy_field_state(shown);
        m.last_move = shown.last_move;
        m.shown_as = None;
        m.faint_slot = None;
        // An ability shown since (Psychic Surge's terrain line) is theirs too.
        if shown.ability != before.ability {
            m.ability = shown.ability;
            m.base_ability = shown.base_ability;
        }
        for mv in &shown.moves {
            let had = before.moves.iter().find(|x| x.id == mv.id).map(|x| x.uses);
            if had == Some(mv.uses) {
                continue;
            }
            let uses = mv.uses - had.unwrap_or(0).min(mv.uses);
            match m.moves.iter_mut().find(|x| x.id == mv.id) {
                Some(x) => x.uses += uses,
                None => m.moves.push(MoveView {
                    id: mv.id,
                    uses,
                    ..Default::default()
                }),
            }
        }
        if shown.item != before.item || shown.prev_item != before.prev_item {
            m.item = shown.item;
            m.prev_item = shown.prev_item;
        }
        m
    }

    /// What the slot's switch line showed as `cur` since it came in was
    /// `real` (an Illusion user): that moves over, and `cur` goes back to
    /// `snap` (its view before the switch) if it is still in the slot.
    fn unmask(&mut self, s: usize, slot: usize, cur: usize, real: usize, snap: Option<MonView>) {
        let side = &mut self.sides[s];
        let shown = side.mons[cur].clone();
        let restore = shown.slot == Some(slot);
        let before = snap.clone().unwrap_or_default();
        side.mons[real] = Self::merged(&side.mons[real], &shown, &before);
        side.mons[real].slot = Some(slot);
        match snap {
            Some(v) if restore => side.mons[cur] = v,
            _ if restore => side.mons[cur].slot = None,
            _ => {}
        }
    }

    /// Two Pokémon showing one name, `a` in `a_slot` and `b` in `b_slot`,
    /// were each the other: they swap what was seen of them since they came
    /// in (against the views their switch lines found them in).
    fn swap_identities(&mut self, s: usize, a: usize, a_slot: usize, b: usize, b_slot: usize) {
        let snap = |t: &Tracker, slot: usize, k: usize| {
            t.switch_snapshots[s]
                .get(slot)
                .cloned()
                .flatten()
                .filter(|(x, _)| *x == k)
                .map(|(_, v)| v)
                .unwrap_or_default()
        };
        let (snap_a, snap_b) = (snap(self, a_slot, a), snap(self, b_slot, b));
        let side = &mut self.sides[s];
        let (shown_a, shown_b) = (side.mons[a].clone(), side.mons[b].clone());
        side.mons[a] = Self::merged(&snap_a, &shown_b, &snap_b);
        side.mons[b] = Self::merged(&snap_b, &shown_a, &snap_a);
        // `b` keeps the name its switch line gave it (its snapshot predates it).
        if side.mons[b].name.is_empty() {
            side.mons[b].name = shown_b.name.clone();
            side.mons[b].species = shown_b.species.or(side.mons[b].species);
            side.mons[b].level = shown_b.level;
            side.mons[b].gender = shown_b.gender;
        }
    }

    /// A Mega Evolution shows the real Pokémon: if the tracker took it for
    /// an Illusion user, the two swap.
    fn mega_unmask(&mut self, who: &str) {
        let Some((s, Some(slot), name)) = parse_ref(who) else {
            return;
        };
        let side = &self.sides[s];
        let Some(h) = side
            .mons
            .iter()
            .position(|m| m.slot == Some(slot) && m.shown_as.as_deref() == Some(name))
        else {
            return;
        };
        let Some(y) = side.mons.iter().position(|m| m.name == name) else {
            return;
        };
        let Some(y_slot) = side.mons[y].slot.filter(|&x| x != slot) else {
            return;
        };
        self.swap_identities(s, h, slot, y, y_slot);
        self.sides[s].mons[h].shown_as = Some(name.to_string());
    }

    /// `|replace|POKEMON|DETAILS`: an Illusion ends. What the slot's
    /// disguise was seen doing since it switched in belongs to this Pokémon;
    /// the disguise goes back to how it was viewed before.
    /// Ally Switch moves the switch-in snapshots of the two slots.
    fn swap_switch_snapshots(&mut self, s: usize, a: usize, b: usize) {
        if a < 3 && b < 3 {
            self.switch_snapshots[s].swap(a, b);
        }
    }

    fn on_replace(&mut self, l: &Line) {
        let Some((s, Some(slot), name)) = parse_ref(l.arg(0)) else {
            return;
        };
        let (species, level, gender) = parse_details(l.arg(1));
        let species_id = self.dex.species_id(species);
        let side = &mut self.sides[s];
        // The entry the slot's switch line went to, wherever it is now.
        let snapshot = self.switch_snapshots[s].get(slot).cloned().flatten();
        let current = snapshot
            .as_ref()
            .map(|(k, _)| *k)
            .or_else(|| side.mons.iter().position(|m| m.slot == Some(slot)));
        let real = side
            .mons
            .iter()
            .position(|m| m.name == name)
            .or_else(|| {
                side.mons
                    .iter()
                    .position(|m| m.name.is_empty() && same_species(&m.preview, species))
            })
            .unwrap_or_else(|| {
                side.mons.push(MonView {
                    preview: species.to_string(),
                    ..Default::default()
                });
                side.mons.len() - 1
            });
        // Nothing to move when the real one already holds the slot (one's
        // own, known from the request).
        let guessed = side.mons[real]
            .slot
            .filter(|&o| o != slot && side.mons[real].shown_as.is_some())
            .or(side.mons[real]
                .faint_slot
                .filter(|&o| o != slot && side.mons[real].fainted));
        if let (Some(other), Some(cur)) = (guessed, current.filter(|&c| c != real)) {
            // The tracker took the other one for the Illusion user (two
            // Pokémon showed one name and either could have been it).
            self.swap_identities(s, real, other, cur, slot);
        } else if let Some(cur) =
            current.filter(|&c| c != real && side.mons[real].slot != Some(slot))
        {
            let snap = snapshot.filter(|(k, _)| *k == cur).map(|(_, v)| v);
            self.unmask(s, slot, cur, real, snap);
        }
        let side = &mut self.sides[s];
        // The slot may already show another Pokémon of that name (the real
        // one, switched in beside its disguise).
        for (i, m) in side.mons.iter_mut().enumerate() {
            if i != real && m.slot == Some(slot) {
                m.slot = None;
            }
        }
        let m = &mut side.mons[real];
        m.name = name.to_string();
        m.species = species_id.or(m.species);
        m.level = level;
        m.gender = gender;
        m.slot = Some(slot);
        m.seen = true;
        m.shown_as = None;
        m.faint_slot = None;
        m.illusion_broken = true;
        self.switches += 1;
        self.sides[s].mons[real].entered = self.switches;
    }
}

impl Tracker {
    /// The view as JSON (ids, not names), for tests and inspection.
    pub fn to_json(&self) -> Value {
        let dex = &self.dex;
        let mon = |si: usize, k: usize, m: &MonView| {
            let (trapped, shed) = self.trap_state(si, k);
            let mut v = serde_json::json!({
                "name": m.name,
                "species": m.species.map(|s| dex.species(s).id.clone()),
                "preview": m.preview,
                "hp": m.hp,
                "maxhp": m.maxhp,
                "status": m.status.id(),
                "fainted": m.fainted,
                "seen": m.seen,
                "slot": m.slot,
                "boosts": m.boosts,
                "volatiles": m.volatiles,
                "moves": m.moves.iter().map(|mv| serde_json::json!({
                    "id": dex.mv(mv.id).id, "uses": mv.uses, "pp": mv.pp, "maxpp": mv.maxpp,
                })).collect::<Vec<_>>(),
                "ability": m.ability.map(|a| dex.ability(a).id.clone()),
                "baseAbility": m.base_ability.map(|a| dex.ability(a).id.clone()),
                "item": match m.item {
                    ItemView::Unknown => Value::Null,
                    ItemView::None => serde_json::json!(""),
                    ItemView::Known(i) => serde_json::json!(dex.item(i).id),
                },
                "mega": m.mega,
                "lastMove": m.last_move.map(|x| dex.mv(x).id.clone()),
                "teamPos": m.team_pos,
                "stats": m.stats,
                "transformed": m.transformed.map(|s| dex.species(s).id.clone()),
                "addedType": m.added_type.map(|t| dex.type_names[t as usize].clone()),
                "types": m.types.as_ref().map(|ts| ts.iter().map(|&t| dex.type_names[t as usize].clone()).collect::<Vec<_>>()),
                "volatileSince": m.volatiles.iter().filter_map(|v| m.volatile_since(v).map(|t| (v.clone(), serde_json::json!(t)))).collect::<serde_json::Map<_, _>>(),
                "boundBand": m.bound_band,
                "locked": m.locked,
                "allySwitch": m.ally_switch,
                "hurtThisTurn": m.hurt_this_turn,
                "trapped": trapped,
                "shedShell": shed,
                "statsRaisedThisTurn": m.stats_raised_this_turn,
                "statsLoweredThisTurn": m.stats_lowered_this_turn,
                "cudChew": m.cud_chew,
                "illusionBroken": m.illusion_broken,
                "shownAs": m.shown_as,
            });
            let attacked: Vec<Value> = m
                .attacked_by
                .iter()
                .map(|a| {
                    serde_json::json!({
                        "side": a.side, "name": a.name, "slot": a.slot, "move": dex.mv(a.mv).id,
                        "damage": a.damage, "thisTurn": a.this_turn,
                    })
                })
                .collect();
            let history = serde_json::json!({
                "lastMoveTargetLoc": m.last_move_target_loc,
                "moveThisTurnResult": m.move_this_turn,
                "moveLastTurnResult": m.move_last_turn,
                "timesAttacked": m.times_attacked,
                "attackedBy": attacked,
                "stall": m.stall,
                "lastItem": m.last_item.map(|i| dex.item(i).id.clone()),
                "ateBerry": m.ate_berry,
            });
            if let (Value::Object(o), Value::Object(h)) = (&mut v, history) {
                o.extend(h);
            }
            v
        };
        let side = |si: usize, s: &SideView| {
            serde_json::json!({
                "name": s.name,
                "teamSize": s.team_size,
                "conditions": s
                    .conditions
                    .iter()
                    .map(|(c, t)| (c.id().to_string(), serde_json::json!(t)))
                    .chain(s.safeguard.map(|t| ("safeguard".to_string(), serde_json::json!(t))))
                    .chain((s.spikes > 0).then(|| ("spikes".to_string(), serde_json::json!(s.spikes))))
                    .chain(s.sticky_web.then(|| ("stickyweb".to_string(), serde_json::json!(true))))
                    .collect::<serde_json::Map<_, _>>(),
                "spikes": s.spikes,
                "toxicSpikes": s.toxic_spikes,
                "stealthRock": s.stealth_rock,
                "slots": s.slots.iter().map(|x| serde_json::json!({
                    "wish": x.wish, "healingWish": x.healing_wish, "future": x.future,
                })).collect::<Vec<_>>(),
                "pokemon": s.mons.iter().enumerate().map(|(k, m)| mon(si, k, m)).collect::<Vec<_>>(),
            })
        };
        let me = ["p1", "p2"][self.me];
        serde_json::json!({
            "me": me,
            "turn": self.turn,
            "upkeepDone": self.upkeep_done,
            "ended": self.ended,
            "winner": self.winner,
            "openSheets": self.open_sheets,
            "lastMove": self.last_move.map(|x| dex.mv(x).id.clone()),
            "field": {
                "weather": self.field.weather.map(|(w, t)| serde_json::json!([w.id(), t])),
                "terrain": self.field.terrain.map(|(x, t)| serde_json::json!([x.id(), t])),
                "trickRoom": self.field.trick_room,
                "gravity": self.field.gravity,
                "magicRoom": self.field.magic_room,
                "wonderRoom": self.field.wonder_room,
                "fairyLock": self.field.fairy_lock,
            },
            "sides": [side(0, &self.sides[0]), side(1, &self.sides[1])],
        })
    }
}

fn side_index(s: &str) -> Option<usize> {
    match s {
        "p1" => Some(0),
        "p2" => Some(1),
        _ => None,
    }
}

/// Whether a species shown at preview (possibly "Urshifu-*") matches one
/// shown later, Megas included ("Charizard" and "Charizard-Mega-X").
fn same_species(preview: &str, shown: &str) -> bool {
    if preview == shown {
        return true;
    }
    if let Some(base) = preview.strip_suffix("-*") {
        return shown == base || shown.starts_with(&format!("{base}-"));
    }
    shown.starts_with(&format!("{preview}-Mega"))
}
