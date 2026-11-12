//! A simulator of Pokémon Champions VGC doubles (Reg M-C) that matches
//! Pokémon Showdown at a pinned commit exactly, built for bots, self-play
//! and search. The README (<https://github.com/dcabatin/Ninjask>) covers
//! installation, the parity checks and the Python package.
//!
//! A battle is a [`Battle`] over a loaded format, a [`Dex`]. Choices and
//! requests use Showdown's own syntax and JSON:
//!
//! ```no_run
//! use std::sync::Arc;
//! use ninjask::{rng::Rng, Battle, Dex};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // A format export, as the Python package bundles them (ninjask/data/formats/)
//! let dex = Arc::new(Dex::from_json(&std::fs::read_to_string("gen9championsvgc2026regmc.json")?)?);
//! # let (team1, team2) = ("", "");
//! let mut battle = Battle::new(dex, [team1, team2], ["Alice", "Bob"], 1)?; // packed teams
//! let mut rng = Rng::new(1);
//! while !battle.ended && battle.turn < 150 {
//!     for side in 0..2 {
//!         if battle.side_needs_choice(side) {
//!             let choice = battle.random_choice(side, &mut rng); // or "move 1 1, switch 3"
//!             battle.choose(side, &choice)?;
//!         }
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! Beyond the battle: [`tracker`] builds one player's view from the protocol,
//! [`world`] turns a view back into a battle for search under hidden
//! information, and [`calc`] gives exact damage and turn-order odds.

/// Append a line to the battle log, formatting it only when logging is on
/// (self-play runs with logging off).
macro_rules! log_line {
    ($battle:expr, $($arg:tt)*) => {
        if $battle.log_enabled {
            let line = format!($($arg)*);
            $battle.add(line);
        }
    };
}

pub mod abilities;
pub mod approx;
pub mod batch;
pub mod battle;
pub mod boosts;
pub mod calc;
pub mod conditions;
pub mod dex;
pub mod encode;
pub mod events;
pub mod features;
pub mod items;
pub mod kinds;
pub mod moves;
pub mod outcome;
pub mod projection;
pub mod rng;
pub mod state;
pub mod tracker;
pub mod transform;
pub mod world;

#[cfg(feature = "python")]
pub mod python;

pub use battle::Battle;
pub use dex::Dex;

pub use battle::{BattleStateKey, DEFAULT_MAX_PATHS};
pub use outcome::{EnumerationError, EnumerationMode, Outcome};
