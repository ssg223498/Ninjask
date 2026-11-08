//! One side of the battle: its Pokémon, side conditions and revealed choices.

use super::{Mon, SlotChoice};
use crate::battle::MonRef;

/// Side conditions the engine models.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SideCond {
    Tailwind,
    Reflect,
    LightScreen,
    AuroraVeil,
    /// One-turn guards against spread moves and priority moves.
    WideGuard,
    QuickGuard,
}

impl SideCond {
    pub const ALL: [SideCond; 6] = [
        SideCond::Tailwind,
        SideCond::Reflect,
        SideCond::LightScreen,
        SideCond::AuroraVeil,
        SideCond::WideGuard,
        SideCond::QuickGuard,
    ];

    pub fn id(self) -> &'static str {
        match self {
            SideCond::Tailwind => "tailwind",
            SideCond::Reflect => "reflect",
            SideCond::LightScreen => "lightscreen",
            SideCond::AuroraVeil => "auroraveil",
            SideCond::WideGuard => "wideguard",
            SideCond::QuickGuard => "quickguard",
        }
    }
    pub fn from_id(s: &str) -> Option<SideCond> {
        SideCond::ALL.into_iter().find(|c| c.id() == s)
    }
    /// The effect name in `-sidestart` / `-sideend` lines.
    pub fn log_name(self) -> &'static str {
        match self {
            SideCond::Tailwind => "move: Tailwind",
            SideCond::Reflect => "Reflect",
            SideCond::LightScreen => "move: Light Screen",
            SideCond::AuroraVeil => "move: Aurora Veil",
            SideCond::WideGuard => "Wide Guard",
            SideCond::QuickGuard => "Quick Guard",
        }
    }
    /// Wide Guard and Quick Guard last one turn and announce like single-turn moves.
    pub fn is_guard(self) -> bool {
        matches!(self, SideCond::WideGuard | SideCond::QuickGuard)
    }
    /// `onSideResidualSubOrder` (all have `onSideResidualOrder: 26`).
    pub fn residual_sub_order(self) -> u32 {
        match self {
            SideCond::Reflect => 1,
            SideCond::LightScreen => 2,
            SideCond::Tailwind => 5,
            SideCond::AuroraVeil => 10,
            SideCond::WideGuard | SideCond::QuickGuard => 4,
        }
    }
}

/// Side conditions with their remaining turns (0 = not active).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SideConditions {
    pub tailwind: u8,
    pub reflect: u8,
    pub light_screen: u8,
    pub aurora_veil: u8,
    pub wide_guard: u8,
    pub quick_guard: u8,
    /// Toxic Spikes layers (no duration; not a `SideCond`).
    pub toxic_spikes: u8,
    /// Stealth Rock (no duration; not a `SideCond`).
    pub stealth_rock: bool,
    /// When the entry hazards were added (`next_added`): Showdown runs a
    /// side's hazards at a switch-in in the order they were created.
    pub toxic_spikes_added: u16,
    pub stealth_rock_added: u16,
    /// When each condition (indexed by `SideCond`) was last added. Showdown
    /// keeps side conditions in insertion order, which orders their handlers.
    pub added: [u16; 6],
    pub next_added: u16,
}

impl SideConditions {
    /// Record that `c` was just added: it goes last in insertion order.
    pub fn stamp(&mut self, c: crate::state::SideCond) {
        self.next_added += 1;
        self.added[c as usize] = self.next_added;
    }

    /// The active conditions, in the order they were added.
    pub fn in_order(&self) -> arrayvec::ArrayVec<crate::state::SideCond, 6> {
        let mut v: arrayvec::ArrayVec<_, 6> = crate::state::SideCond::ALL
            .into_iter()
            .filter(|&c| self.get(c) > 0)
            .collect();
        v.sort_by_key(|&c| self.added[c as usize]);
        v
    }

    pub fn get(&self, c: crate::state::SideCond) -> u8 {
        use crate::state::SideCond::*;
        match c {
            Tailwind => self.tailwind,
            Reflect => self.reflect,
            LightScreen => self.light_screen,
            AuroraVeil => self.aurora_veil,
            WideGuard => self.wide_guard,
            QuickGuard => self.quick_guard,
        }
    }
    pub fn get_mut(&mut self, c: crate::state::SideCond) -> &mut u8 {
        use crate::state::SideCond::*;
        match c {
            Tailwind => &mut self.tailwind,
            Reflect => &mut self.reflect,
            LightScreen => &mut self.light_screen,
            AuroraVeil => &mut self.aurora_veil,
            WideGuard => &mut self.wide_guard,
            QuickGuard => &mut self.quick_guard,
        }
    }
}

/// The most Pokémon a side has on the field at once (doubles).
pub const MAX_ACTIVE_PER_SIDE: usize = 2;
/// The most Pokémon on the field at once.
pub const MAX_ACTIVE: usize = 2 * MAX_ACTIVE_PER_SIDE;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Side {
    pub index: usize,
    pub name: String,
    /// Pokémon in Showdown order: positions 0..active are on the field.
    pub mons: Vec<Mon>,
    pub n_active: usize,
    pub pokemon_left: usize,
    pub mega_used: bool,
    /// Pokémon fainted so far (Showdown `totalFainted`, for Last Respects).
    pub total_fainted: u8,
    pub conds: SideConditions,
    /// The choice made for the current request, if any.
    pub choice: Option<Vec<SlotChoice>>,
    pub team_preview_choice: Option<Vec<usize>>,
    /// The team as open team sheets show it (`|showteam|`).
    pub sheet: String,
    /// What refused choices revealed for the request numbered
    /// `reveals_rqid`, per active slot: Showdown answers a choice that only
    /// hidden information rules out ("[Unavailable choice]") with an updated
    /// request.
    pub reveals: Vec<(usize, Reveal)>,
    pub reveals_rqid: u32,
    /// Revival Blessing's slot condition, per active slot.
    pub revival: [bool; 2],
}

/// Hidden information a refused choice reveals.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reveal {
    /// Every move the end-of-turn pass disabled, Imprison's hidden ones included.
    Disabled,
    /// A hidden trap (Shadow Tag).
    Trapped,
    /// The move (its index) a refused choice picked: the updated request
    /// shows it disabled, with an empty `disabledSource`. In singles it is
    /// the only one shown so (`updateDisabledRequest` skips singles).
    Move(usize),
}

impl Side {
    pub fn id(&self) -> &'static str {
        ["p1", "p2"][self.index]
    }
}

/// Wish's slot condition (`effectState`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct WishState {
    /// Half the wisher's max HP (`hp`, kept as Showdown's float).
    pub hp: f64,
    /// `startingTurn`: the overflowed turn count when it was made.
    pub starting_turn: u32,
    /// The wisher's slot and name (`[wisher]`).
    pub source_slot: usize,
    pub wisher: String,
}

/// One slot's conditions, with when each was added (insertion order).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SlotConds {
    pub wish: Option<WishState>,
    /// Healing Wish, with the slot of its user.
    pub healing_wish: Option<usize>,
    /// Its `onSwap` ran in a `runEvent('Swap')` (Ally Switch) without
    /// healing: `runEvent` set its state's `target` to that Pokémon, so
    /// `fieldEvent('SwitchIn')` now skips it (it looks for the state in the
    /// Pokémon's volatiles). Only another swap can still use it.
    pub healing_wish_detached: bool,
    /// Future Sight (`futuremove`).
    pub future: Option<FutureState>,
}

/// Future Sight's slot condition: the turn it lands, and its user.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct FutureState {
    pub ending_turn: u32,
    pub source: MonRef,
}

/// The side conditions on one side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct SideExtras {
    /// Safeguard: turns left, 0 = none.
    pub safeguard: u8,
    /// Spikes layers (0-3) and Sticky Web.
    pub spikes: u8,
    pub sticky_web: bool,
    /// When the hazards were added (the side's `conds.next_added`): their
    /// switch-in handlers tie and run in the order they were created.
    pub spikes_added: u16,
    pub sticky_web_added: u16,
}

/// Entry hazards (switch-in handlers, battle/switching.rs runs them).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum AHazard {
    Spikes,
    StickyWeb,
}
