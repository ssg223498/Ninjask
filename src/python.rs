//! Python bindings (feature `python`): the `Dex`, `Battle` and `Tracker` classes.
//!
//! `register` adds them to a Python module. With the feature `python-module`
//! this crate is itself the `ninjask._core` extension module (built with
//! maturin from `pyproject.toml`), which the `ninjask` Python package
//! re-exports. A crate that adds classes of its own taking these (say, an
//! observation encoder that reads a `Battle`) depends on this one with the
//! feature `python` only, calls `register` from its own `#[pymodule]`, and
//! declares that module under the `ninjask.extension` entry-point group so
//! that the `ninjask` package uses its classes: one `Battle` class per
//! process, not two incompatible ones (README, "Embedding in another
//! extension"). The classes report `ninjask` as their module, which is where
//! the package exposes them either way.

use std::sync::Arc;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::battle::Battle;
use crate::dex::support;
use crate::dex::Dex;

/// "team 1234" for team positions (0-based) in the order sent.
fn team_string(order: &[usize]) -> String {
    let picks: String = order.iter().map(|p| (p + 1).to_string()).collect();
    format!("team {picks}")
}

/// The `index`-th of the side's legal choices, as Showdown syntax.
fn indexed_choice(b: &Battle, s: usize, index: usize) -> PyResult<String> {
    let out_of_range =
        |n: usize| PyValueError::new_err(format!("choice {index} out of range: {n} legal choices"));
    if b.request == crate::state::RequestState::TeamPreview {
        let orders = b.team_preview_orders(s);
        return orders
            .get(index)
            .map(|o| team_string(o))
            .ok_or_else(|| out_of_range(orders.len()));
    }
    let choices = b.player_choices(s);
    choices
        .get(index)
        .map(|c| b.choice_string(c))
        .ok_or_else(|| out_of_range(choices.len()))
}

/// Little-endian bytes of a numeric buffer, for `numpy.frombuffer`.
fn le_bytes<'py, const N: usize, T: Copy>(
    py: Python<'py>,
    v: &[T],
    to: impl Fn(T) -> [u8; N],
) -> Bound<'py, pyo3::types::PyBytes> {
    let raw: Vec<u8> = v.iter().flat_map(|&x| to(x)).collect();
    pyo3::types::PyBytes::new(py, &raw)
}

/// Damage enumeration policy. Other chance events are exhaustive in both modes.
#[pyclass(
    name = "EnumerationMode",
    module = "ninjask",
    eq,
    eq_int,
    from_py_object
)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PyEnumerationMode {
    Precise,
    DamageEndpoints,
}
impl From<PyEnumerationMode> for crate::EnumerationMode {
    fn from(m: PyEnumerationMode) -> Self {
        match m {
            PyEnumerationMode::Precise => Self::Precise,
            PyEnumerationMode::DamageEndpoints => Self::DamageEndpoints,
        }
    }
}

#[pyclass(name = "Outcome", module = "ninjask", frozen)]
pub struct PyOutcome {
    inner: crate::Outcome,
}
#[pymethods]
impl PyOutcome {
    #[getter]
    fn probability(&self) -> f64 {
        self.inner.probability
    }
    #[getter]
    fn battle(&self) -> PyBattle {
        PyBattle {
            inner: self.inner.battle.clone(),
        }
    }
}

#[pyclass(name = "BattleStateKey", module = "ninjask", frozen, eq)]
#[derive(Clone, PartialEq, Eq)]
pub struct PyBattleStateKey {
    inner: crate::BattleStateKey,
}
#[pymethods]
impl PyBattleStateKey {
    fn __hash__(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.inner.hash(&mut h);
        h.finish()
    }
}

/// "p1" or "p2" as a side index.
pub fn side_index(side: &str) -> PyResult<usize> {
    match side {
        "p1" => Ok(0),
        "p2" => Ok(1),
        _ => Err(PyValueError::new_err(format!("bad side {side}"))),
    }
}

#[pyclass(name = "Dex", module = "ninjask", frozen)]
#[derive(Clone)]
pub struct PyDex {
    pub inner: Arc<Dex>,
}

#[pymethods]
impl PyDex {
    /// Load an export written by refsim/export_format.js.
    #[staticmethod]
    fn load(path: &str) -> PyResult<PyDex> {
        let text =
            std::fs::read_to_string(path).map_err(|e| PyValueError::new_err(e.to_string()))?;
        let dex = Dex::from_json(&text).map_err(PyValueError::new_err)?;
        Ok(PyDex {
            inner: Arc::new(dex),
        })
    }

    #[getter]
    fn format_id(&self) -> String {
        self.inner.format.id.clone()
    }

    /// Reasons a packed team is not fully simulated (empty when it is).
    fn team_problems(&self, packed: &str) -> PyResult<Vec<String>> {
        support::team_problems(&self.inner, packed).map_err(PyValueError::new_err)
    }

    /// Support level of a move by id: "generic", "implemented" or "unsupported (...)".
    fn move_support(&self, move_id: &str) -> PyResult<String> {
        let id = self
            .inner
            .move_id(move_id)
            .ok_or_else(|| PyValueError::new_err("unknown move"))?;
        Ok(support::move_support(&self.inner, self.inner.mv(id)).to_string())
    }

    /// What `Battle.eval_features` reads of a move: `(setup, heal,
    /// priority)`. `setup`: a status move whose boosts to its user add up
    /// to 1 or more over the positive ones (`boosts` for a move targeting
    /// its user, else `self.boosts`, else its secondary effect's
    /// `self.boosts`); `heal`: the heal flag on a move targeting its user;
    /// `priority`: the dex priority.
    fn eval_move_marks(&self, move_id: &str) -> PyResult<(bool, bool, i8)> {
        let id = self
            .inner
            .move_id(move_id)
            .ok_or_else(|| PyValueError::new_err(format!("unknown move {move_id}")))?;
        let m = self.inner.move_marks(id);
        Ok((m.setup, m.heal, m.priority))
    }

    /// Ids of all species in this format's data.
    fn species_ids(&self) -> Vec<String> {
        self.inner.species.iter().map(|s| s.id.clone()).collect()
    }

    /// Embedding-table sizes for `Tracker.encode_ids()`: one more than the
    /// dex's count (0 is unknown), two more for items (1 is known to hold
    /// nothing). Index `k - 1` of `species_ids()` (or `move_ids()`) is the
    /// entity behind value `k`.
    fn vocab(&self) -> std::collections::HashMap<&'static str, usize> {
        let d = &self.inner;
        std::collections::HashMap::from([
            ("species", d.species.len() + 1),
            ("items", d.items.len() + 2),
            ("abilities", d.abilities.len() + 1),
            ("moves", d.moves.len() + 1),
        ])
    }

    /// Ids of all moves in this format's data, in dex order.
    fn move_ids(&self) -> Vec<String> {
        self.inner.moves.iter().map(|m| m.id.clone()).collect()
    }

    /// Ids of all moves the engine simulates generically.
    fn generic_moves(&self) -> Vec<String> {
        self.inner
            .moves
            .iter()
            .filter(|m| {
                support::move_support(&self.inner, m) == crate::dex::types::Support::Generic
            })
            .map(|m| m.id.clone())
            .collect()
    }

    /// Ids of moves with hand-ported handlers.
    fn implemented_moves(&self) -> Vec<String> {
        self.inner
            .moves
            .iter()
            .filter(|m| {
                support::move_support(&self.inner, m) == crate::dex::types::Support::Implemented
            })
            .map(|m| m.id.clone())
            .collect()
    }

    /// Ids of abilities the engine simulates (ported, or No Ability).
    fn supported_abilities(&self) -> Vec<String> {
        (0..self.inner.abilities.len())
            .filter(|&i| {
                !matches!(
                    support::ability_support(&self.inner, i as u16),
                    crate::dex::types::Support::Unsupported(_)
                )
            })
            .map(|i| self.inner.abilities[i].id.clone())
            .collect()
    }

    /// Ids of items the engine simulates.
    fn supported_items(&self) -> Vec<String> {
        (0..self.inner.items.len())
            .filter(|&i| {
                !matches!(
                    support::item_support(&self.inner, Some(i as u16)),
                    crate::dex::types::Support::Unsupported(_)
                )
            })
            .map(|i| self.inner.items[i].id.clone())
            .collect()
    }
}

#[pyclass(name = "Battle", module = "ninjask")]
pub struct PyBattle {
    pub inner: Battle,
}

#[pymethods]
impl PyBattle {
    #[new]
    #[pyo3(signature = (dex, team1, team2, seed, names=None))]
    fn new(
        dex: &PyDex,
        team1: &str,
        team2: &str,
        seed: u64,
        names: Option<(String, String)>,
    ) -> PyResult<PyBattle> {
        let (n1, n2) = names.unwrap_or(("Player 1".into(), "Player 2".into()));
        let inner = Battle::new(dex.inner.clone(), [team1, team2], [&n1, &n2], seed)
            .map_err(PyValueError::new_err)?;
        Ok(PyBattle { inner })
    }

    /// Reference mode: a battle whose RNG is Showdown's, seeded like Showdown's
    /// ("sodium,<hex>"), so it draws the same numbers.
    #[staticmethod]
    #[pyo3(signature = (dex, team1, team2, seed, names=None))]
    fn reference(
        dex: &PyDex,
        team1: &str,
        team2: &str,
        seed: &str,
        names: Option<(String, String)>,
    ) -> PyResult<PyBattle> {
        let (n1, n2) = names.unwrap_or(("Player 1".into(), "Player 2".into()));
        let rng = crate::rng::Rng::from_showdown_seed(seed).map_err(PyValueError::new_err)?;
        let inner = Battle::new_with_rng(dex.inner.clone(), [team1, team2], [&n1, &n2], rng)
            .map_err(PyValueError::new_err)?;
        Ok(PyBattle { inner })
    }

    /// Record every RNG draw with its call site (see `rng_trace`).
    fn set_rng_trace(&mut self, on: bool) {
        self.inner.rng.trace = if on { Some(Vec::new()) } else { None };
    }

    /// The draws recorded since the last call.
    fn rng_trace(&mut self) -> Vec<String> {
        self.inner
            .rng
            .trace
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    }

    /// The RNG state as Showdown prints it (reference mode), else None.
    #[getter]
    fn rng_seed(&self) -> Option<String> {
        self.inner.rng.showdown_seed()
    }

    /// Continue from a Showdown state projection (JSON text) with a new seed.
    #[staticmethod]
    #[pyo3(signature = (dex, projection, seed, names=None))]
    fn from_projection(
        dex: &PyDex,
        projection: &str,
        seed: u64,
        names: Option<(String, String)>,
    ) -> PyResult<PyBattle> {
        let (n1, n2) = names.unwrap_or(("Player 1".into(), "Player 2".into()));
        let v: serde_json::Value =
            serde_json::from_str(projection).map_err(|e| PyValueError::new_err(e.to_string()))?;
        let inner = Battle::from_projection(dex.inner.clone(), &v, [&n1, &n2], seed)
            .map_err(PyValueError::new_err)?;
        Ok(PyBattle { inner })
    }

    /// The side's current request as Showdown JSON text, or None.
    fn request(&self, side: &str) -> PyResult<Option<String>> {
        Ok(self
            .inner
            .request_json(side_index(side)?)
            .map(|v| v.to_string()))
    }

    /// Whether the side has to answer a request now.
    fn needs_choice(&self, side: &str) -> PyResult<bool> {
        Ok(self.inner.side_needs_choice(side_index(side)?))
    }

    /// Submit a choice; raises ValueError with Showdown's message if invalid.
    fn choose(&mut self, side: &str, choice: &str) -> PyResult<()> {
        self.inner
            .choose(side_index(side)?, choice)
            .map_err(PyValueError::new_err)
    }

    /// The side's legal choices in Showdown's choice syntax, as its request
    /// offers them: every ordered pick at team preview (the first two lead),
    /// else every joint move or switch choice, ally targets included. The
    /// same choices as `ninjask.choices` lists from the request, in the same
    /// order, built without it: they depend only on what the player can
    /// see, so one can still be refused (a hidden disable or trap), as in
    /// Showdown. Empty when the side has nothing to choose.
    fn legal_choices(&self, side: &str) -> PyResult<Vec<String>> {
        let s = side_index(side)?;
        let b = &self.inner;
        if b.request == crate::state::RequestState::TeamPreview {
            return Ok(b
                .team_preview_orders(s)
                .iter()
                .map(|o| team_string(o))
                .collect());
        }
        Ok(b.player_choices(s)
            .iter()
            .map(|c| b.choice_string(c))
            .collect())
    }

    /// `legal_choices` as numbers, `action_width(side)` int8 values per
    /// choice, little-endian bytes for `numpy.frombuffer(..., numpy.int8)`.
    /// Move and switch requests: for each active slot, kind (0 pass, 1 move,
    /// 2 switch, 3 Struggle or recharge), index (move slot 1-4 or team
    /// position 1-6), target (1, 2 foes; -1, -2 allies; 0 none), Mega
    /// Evolution (`ninjask.ACTION_FIELDS` values). Team preview: the team
    /// positions sent, 1-based, leads first.
    fn legal_actions<'py>(
        &self,
        py: Python<'py>,
        side: &str,
    ) -> PyResult<Bound<'py, pyo3::types::PyBytes>> {
        let s = side_index(side)?;
        let b = &self.inner;
        let mut out: Vec<i8> = Vec::new();
        if b.request == crate::state::RequestState::TeamPreview {
            for o in b.team_preview_orders(s) {
                out.extend(o.iter().map(|&p| p as i8 + 1));
            }
        } else {
            for c in b.player_choices(s) {
                crate::encode::action_row(&c, &mut out);
            }
        }
        Ok(le_bytes(py, &out, |x: i8| x.to_le_bytes()))
    }

    /// Values per choice in `legal_actions(side)`: the number of Pokémon
    /// brought at team preview, else `ninjask.ACTION_FIELDS` per active
    /// slot; 0 when the side has nothing to choose.
    fn action_width(&self, side: &str) -> PyResult<usize> {
        let s = side_index(side)?;
        let b = &self.inner;
        if !b.side_needs_choice(s) {
            return Ok(0);
        }
        Ok(match b.request {
            crate::state::RequestState::TeamPreview => {
                let n = b.sides[s].mons.len();
                b.dex.format.picked_team_size.unwrap_or(n).min(n)
            }
            crate::state::RequestState::Move | crate::state::RequestState::Switch => {
                b.sides[s].n_active * crate::encode::ACTION_FIELDS
            }
            crate::state::RequestState::Ended => 0,
        })
    }

    /// Submit `legal_choices(side)[index]` without formatting the list.
    /// Raises ValueError as `choose` does: when Showdown refuses the choice
    /// (something hidden), or for an index out of range.
    fn choose_index(&mut self, side: &str, index: usize) -> PyResult<()> {
        let s = side_index(side)?;
        let choice = indexed_choice(&self.inner, s, index)?;
        self.inner.choose(s, &choice).map_err(PyValueError::new_err)
    }

    /// Experimental: enumerate to the next request. Empty strings preserve
    /// already-submitted choices.
    /// Explores at most `max_paths` chance paths (each replays the turn) and
    /// raises ValueError beyond that, so a turn with many attacks fails fast
    /// instead of running for minutes.
    #[pyo3(signature = (choice_p1, choice_p2, mode=PyEnumerationMode::Precise, max_paths=crate::DEFAULT_MAX_PATHS))]
    fn enumerate_outcomes(
        &self,
        py: Python<'_>,
        choice_p1: &str,
        choice_p2: &str,
        mode: PyEnumerationMode,
        max_paths: u64,
    ) -> PyResult<Vec<PyOutcome>> {
        py.detach(|| {
            self.inner
                .enumerate_outcomes_text_within(choice_p1, choice_p2, mode.into(), max_paths)
        })
        .map(|outcomes| {
            outcomes
                .into_iter()
                .map(|inner| PyOutcome { inner })
                .collect()
        })
        .map_err(|e| PyValueError::new_err(e.to_string()))
    }

    fn state_key(&self) -> PyBattleStateKey {
        PyBattleStateKey {
            inner: self.inner.state_key(),
        }
    }

    fn log(&self, viewer: &str, start: usize) -> Vec<String> {
        self.inner.log_for(viewer, start)
    }

    fn log_len(&self) -> usize {
        self.inner.log.len()
    }

    fn set_logging(&mut self, on: bool) {
        self.inner.log_enabled = on;
    }

    /// Open team sheets: log both teams' sets, as Showdown does at team preview.
    fn show_team_sheets(&mut self) {
        self.inner.show_team_sheets();
    }

    fn projection(&self) -> String {
        self.inner.projection().to_string()
    }

    fn copy(&self) -> PyBattle {
        PyBattle {
            inner: self.inner.clone(),
        }
    }

    /// Reseed the engine's RNG (for drawing independent outcomes from a copy).
    fn reseed(&mut self, seed: u64) {
        self.inner.rng = crate::rng::Rng::new(seed);
    }

    /// The read-only damage calculation (`ninjask::calc`) for the move
    /// `move_id` of the Pokémon in slot `attacker` ("p1a") against the one in
    /// `target` ("p2b"), as JSON: immune, accuracy, crit chance, spread,
    /// hit counts, the 16 rolls of each hit without and with a crit, the
    /// expected damage, and the chances to KO this turn and within two uses.
    fn damage(&self, attacker: &str, move_id: &str, target: &str) -> PyResult<String> {
        let (a, t) = (self.slot_ref(attacker)?, self.slot_ref(target)?);
        let id = self.move_id(move_id)?;
        Ok(damage_json(&self.inner.damage_calc(a, id, t)))
    }

    /// `damage` for any Pokémon of the two parties, named by side and index
    /// into the projection's `sides[i]["pokemon"]` ("p1:2"). A benched
    /// Pokémon counts as if it were in its side's slot, with the battle's
    /// field, weather and side conditions and no boosts (only Pokémon on
    /// the field have them); see `Battle::with_in_slot`. For two Pokémon
    /// on the field, exactly `damage`. ValueError for two Pokémon of one
    /// side when one of them is benched.
    fn damage_party(&self, attacker: &str, move_id: &str, target: &str) -> PyResult<String> {
        let (a, t) = (party_ref(attacker)?, party_ref(target)?);
        let id = self.move_id(move_id)?;
        let c = self
            .inner
            .damage_calc_party(a, id, t)
            .map_err(PyValueError::new_err)?;
        Ok(damage_json(&c))
    }

    /// `order` for any Pokémon of the two parties, named as in
    /// `damage_party` ("p1:2"), benched ones counting as in their side's slot.
    fn order_party(&self, a: &str, plan_a: &str, b: &str, plan_b: &str) -> PyResult<(f64, f64)> {
        let (ra, rb) = (party_ref(a)?, party_ref(b)?);
        let o = self
            .inner
            .order_odds_party(ra, self.plan(plan_a)?, rb, self.plan(plan_b)?)
            .map_err(PyValueError::new_err)?;
        Ok((o.first, o.tie))
    }

    /// battle-ai's "v2" evaluator features from `side`'s point of view: 22
    /// floats in the order of Rust's `ninjask::features::FEATURES_V2` (and
    /// battle-ai's), counted from the battle with the calculations
    /// of `damage_party` and `order_party`, without building the
    /// projection. `eval_features("p2")` is `eval_features("p1")` negated.
    /// The battle is left as it was.
    fn eval_features(&self, py: Python<'_>, side: &str) -> PyResult<Vec<f64>> {
        let s = side_index(side)?;
        let f = py
            .detach(|| self.inner.eval_features(s))
            .map_err(PyValueError::new_err)?;
        Ok(f.to_vec())
    }

    /// The chance that the Pokémon in slot `a` doing `plan_a` acts before
    /// the one in slot `b` doing `plan_b`, and that a speed tie decides it:
    /// `(first, tie)`. A plan is a move id, "switch", or "priority:N" (a
    /// damaging move of priority N).
    fn order(&self, a: &str, plan_a: &str, b: &str, plan_b: &str) -> PyResult<(f64, f64)> {
        let (ra, rb) = (self.slot_ref(a)?, self.slot_ref(b)?);
        let o = self
            .inner
            .order_odds(ra, self.plan(plan_a)?, rb, self.plan(plan_b)?);
        Ok((o.first, o.tie))
    }

    /// Each active Pokémon's effective Speed: `{"p1a": speed, ...}` as JSON.
    fn speeds(&self) -> String {
        let mut m = serde_json::Map::new();
        for r in self.inner.all_active() {
            m.insert(
                slot_name(r),
                serde_json::json!(self.inner.effective_speed(r)),
            );
        }
        serde_json::Value::Object(m).to_string()
    }

    #[getter]
    fn ended(&self) -> bool {
        self.inner.ended
    }

    #[getter]
    fn winner(&self) -> Option<String> {
        self.inner.winner_name().map(str::to_string)
    }

    /// The winning side ("p1" or "p2"), or None for a tie or a battle
    /// still running. Unlike `winner`, it doesn't depend on player names.
    #[getter]
    fn winner_side(&self) -> Option<&'static str> {
        self.inner.winner.map(|s| if s == 0 { "p1" } else { "p2" })
    }

    #[getter]
    fn turn(&self) -> u32 {
        self.inner.turn
    }
}

impl PyBattle {
    /// The active Pokémon in a slot named like "p1a".
    fn slot_ref(&self, slot: &str) -> PyResult<crate::battle::MonRef> {
        let b = slot.as_bytes();
        let s = side_index(slot.get(..2).unwrap_or(""))?;
        let k = match b.get(2) {
            Some(b'a') => 0,
            Some(b'b') => 1,
            _ => return Err(PyValueError::new_err(format!("bad slot {slot}"))),
        };
        self.inner
            .active(s, k)
            .ok_or_else(|| PyValueError::new_err(format!("no Pokémon in {slot}")))
    }

    fn move_id(&self, move_id: &str) -> PyResult<crate::dex::types::MoveId> {
        self.inner
            .dex
            .move_id(move_id)
            .ok_or_else(|| PyValueError::new_err(format!("unknown move {move_id}")))
    }

    fn plan(&self, plan: &str) -> PyResult<crate::calc::Plan> {
        if plan == "switch" {
            return Ok(crate::calc::Plan::Switch);
        }
        if let Some(p) = plan.strip_prefix("priority:") {
            let p: i8 = p
                .parse()
                .map_err(|_| PyValueError::new_err(format!("bad plan {plan}")))?;
            return Ok(crate::calc::Plan::Priority(p));
        }
        self.inner
            .dex
            .move_id(plan)
            .map(crate::calc::Plan::Move)
            .ok_or_else(|| PyValueError::new_err(format!("unknown move {plan}")))
    }
}

/// A Pokémon of a party named like "p1:2": the side and its index into
/// the projection's `sides[i]["pokemon"]` (the engine's party order). The
/// index is checked against the party by the calculation.
fn party_ref(name: &str) -> PyResult<crate::battle::MonRef> {
    let bad = || PyValueError::new_err(format!("bad Pokémon {name:?} (want \"p1:2\")"));
    let (side, index) = name.split_once(':').ok_or_else(bad)?;
    let s = side_index(side).map_err(|_| bad())?;
    let i: usize = index.parse().map_err(|_| bad())?;
    Ok((s, i))
}

/// A damage calculation as the JSON `damage` and `damage_party` return.
fn damage_json(c: &crate::calc::DamageCalc) -> String {
    let hits: Vec<serde_json::Value> = c
        .hits
        .iter()
        .map(|h| serde_json::json!({"normal": h.normal, "crit": h.crit}))
        .collect();
    serde_json::json!({
        "damaging": c.damaging, "immune": c.immune, "accuracy": c.accuracy,
        "critChance": c.crit_chance, "spread": c.spread, "hitCounts": c.hit_counts,
        "hits": hits, "fixed": c.fixed, "targetHp": c.target_hp, "targetMaxhp": c.target_maxhp,
        "expected": c.expected, "ko": c.ko, "ko2": c.ko2,
    })
    .to_string()
}

fn slot_name(r: crate::battle::MonRef) -> String {
    format!("p{}{}", r.0 + 1, ["a", "b"][r.1.min(1)])
}

/// What one player knows, built from its protocol lines and requests.
#[pyclass(name = "Tracker", module = "ninjask")]
pub struct PyTracker {
    pub inner: crate::tracker::Tracker,
}

#[pymethods]
impl PyTracker {
    #[new]
    fn new(dex: &PyDex, side: &str) -> PyResult<PyTracker> {
        Ok(PyTracker {
            inner: crate::tracker::Tracker::new(dex.inner.clone(), side_index(side)?),
        })
    }

    /// Feed protocol lines from this player's stream.
    fn feed(&mut self, lines: Vec<String>) {
        for l in &lines {
            self.inner.feed(l);
        }
    }

    /// A copy of this tracker: search advances the copy into a stepped
    /// world battle, leaving the live tracker untouched.
    fn copy(&self) -> PyTracker {
        PyTracker {
            inner: self.inner.clone(),
        }
    }

    /// Feed this player's request (JSON).
    fn feed_request(&mut self, request: &str) -> PyResult<()> {
        let v: serde_json::Value = serde_json::from_str(request)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        self.inner.feed_request(&v);
        Ok(())
    }

    /// Feed this player's own side straight from an engine battle (self-play;
    /// the same view as `feed_request` on the battle's request).
    fn feed_battle(&mut self, battle: &PyBattle) {
        self.inner.feed_engine(&battle.inner);
    }

    /// The view as JSON.
    fn view(&self) -> String {
        self.inner.to_json().to_string()
    }

    /// The view as `ninjask.OBS_LEN` float32 values, little-endian bytes for
    /// `numpy.frombuffer(..., numpy.float32)`: the player's own side, then
    /// the foe's, then the field (layout: `ninjask::encode::observation`).
    /// Only what the view holds is encoded, so nothing hidden leaks.
    fn encode<'py>(&self, py: Python<'py>) -> Bound<'py, pyo3::types::PyBytes> {
        let mut v = Vec::with_capacity(crate::encode::OBS_LEN);
        crate::encode::observation(&self.inner, &mut v);
        le_bytes(py, &v, |x: f32| x.to_le_bytes())
    }

    /// The view's categorical part as `ninjask.IDS_LEN` int16 values
    /// (`numpy.frombuffer(..., numpy.int16)`): per Pokémon its species,
    /// item, ability and four moves, 0 when unknown (layout:
    /// `ninjask::encode::ids`; table sizes: `Dex.vocab()`).
    fn encode_ids<'py>(&self, py: Python<'py>) -> Bound<'py, pyo3::types::PyBytes> {
        let mut v = Vec::with_capacity(crate::encode::IDS_LEN);
        crate::encode::ids(&self.inner, &mut v);
        le_bytes(py, &v, |x: i16| x.to_le_bytes())
    }

    /// A world: the battle this view describes, with `assumptions` (JSON,
    /// see `World::from_view`; None for the defaults) for the foe's hidden
    /// sets. Raises ValueError on bad assumptions.
    #[pyo3(signature = (assumptions=None, seed=0))]
    fn world(&self, assumptions: Option<&str>, seed: u64) -> PyResult<PyBattle> {
        let mut a = match assumptions {
            Some(text) => {
                let v: serde_json::Value =
                    serde_json::from_str(text).map_err(|e| PyValueError::new_err(e.to_string()))?;
                crate::world::Assumptions::from_json(self.inner.dex(), &v)
                    .map_err(PyValueError::new_err)?
            }
            None => crate::world::Assumptions::default(),
        };
        a.seed = seed;
        let inner =
            crate::world::World::from_view(&self.inner, &a).map_err(PyValueError::new_err)?;
        Ok(PyBattle { inner })
    }
}

/// Add the engine's classes to `m`.
/// Many battles stepped together (`ninjask::batch`), for training: both
/// players' trackers are kept in Rust, `observe` reports every pending
/// decision as numeric buffers, `step` applies one choice to each, and both
/// run the battles in parallel with the GIL released.
#[pyclass(name = "Batch", module = "ninjask")]
pub struct PyBatch {
    pub inner: crate::batch::Batch,
    /// (battle, side) of each decision the last `observe` reported.
    pending: Vec<(usize, usize)>,
}

#[pymethods]
impl PyBatch {
    /// `n` empty battle slots for `dex`'s format; `reset` starts a battle.
    #[new]
    fn new(dex: &PyDex, n: usize) -> PyBatch {
        PyBatch {
            inner: crate::batch::Batch::new(dex.inner.clone(), n),
            pending: Vec::new(),
        }
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    /// The turn after which a battle stops as a draw (default 150).
    #[getter]
    fn turn_limit(&self) -> u32 {
        self.inner.turn_limit
    }

    #[setter]
    fn set_turn_limit(&mut self, turns: u32) {
        self.inner.turn_limit = turns;
    }

    /// Start a battle in slot `i` from two packed teams. Raises ValueError
    /// on a bad team or slot.
    fn reset(&mut self, i: usize, team1: &str, team2: &str, seed: u64) -> PyResult<()> {
        self.inner
            .reset(i, [team1, team2], seed)
            .map_err(PyValueError::new_err)
    }

    /// Every pending decision, as a dict of little-endian bytes for
    /// `numpy.frombuffer` (shapes for `m` decisions and `r` legal choices
    /// in all):
    ///
    /// - `who`: uint32 (m, 2), the battle and side (0 is p1) deciding;
    /// - `preview`: uint8 (m,), 1 for a team preview decision;
    /// - `obs`: float32 (m, OBS_LEN), each player's `Tracker.encode()`;
    /// - `ids`: int16 (m, IDS_LEN), each player's `Tracker.encode_ids()`;
    /// - `actions`: int8 (r, ACTION_WIDTH), the legal choices of every
    ///   decision in turn, as `Battle.legal_actions` (padded with 0);
    /// - `offsets`: uint32 (m + 1,), decision k's choices are rows
    ///   `offsets[k]:offsets[k + 1]`;
    ///
    /// and `n`, the number of decisions. Battles that are over report none.
    fn observe<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        let inner = &mut self.inner;
        let o = py.detach(|| inner.observe());
        self.pending = o
            .who
            .chunks(2)
            .map(|w| (w[0] as usize, w[1] as usize))
            .collect();
        let d = pyo3::types::PyDict::new(py);
        d.set_item("n", self.pending.len())?;
        d.set_item("who", le_bytes(py, &o.who, |x: u32| x.to_le_bytes()))?;
        d.set_item("preview", pyo3::types::PyBytes::new(py, &o.preview))?;
        d.set_item("obs", le_bytes(py, &o.obs, |x: f32| x.to_le_bytes()))?;
        d.set_item("ids", le_bytes(py, &o.ids, |x: i16| x.to_le_bytes()))?;
        d.set_item("actions", le_bytes(py, &o.actions, |x: i8| x.to_le_bytes()))?;
        d.set_item(
            "offsets",
            le_bytes(py, &o.offsets, |x: u32| x.to_le_bytes()),
        )?;
        Ok(d)
    }

    /// Apply one choice to each decision the last `observe` reported:
    /// `choices[k]` indexes decision k's legal choices. Returns uint8 bytes,
    /// one per decision: bit 0 set when Showdown refused the choice (the
    /// decision stays pending with what the refusal revealed), bit 1 when
    /// the battle ended. Raises ValueError on a wrong count or index.
    fn step<'py>(
        &mut self,
        py: Python<'py>,
        choices: Vec<usize>,
    ) -> PyResult<Bound<'py, pyo3::types::PyBytes>> {
        if choices.len() != self.pending.len() {
            return Err(PyValueError::new_err(format!(
                "{} choices for {} pending decisions",
                choices.len(),
                self.pending.len()
            )));
        }
        let picks: Vec<(usize, usize, usize)> = self
            .pending
            .iter()
            .zip(&choices)
            .map(|(&(i, s), &c)| (i, s, c))
            .collect();
        let inner = &mut self.inner;
        let res = py
            .detach(|| inner.step(&picks))
            .map_err(PyValueError::new_err)?;
        self.pending.clear();
        let flags: Vec<u8> = res
            .iter()
            .map(|r| r.refused as u8 | (r.ended as u8) << 1)
            .collect();
        Ok(pyo3::types::PyBytes::new(py, &flags))
    }

    /// Start several battles: `reset(slots[k], teams1[k], teams2[k],
    /// seeds[k])` for each k, built in parallel. Raises ValueError (and
    /// starts none) on a bad team or slot, or lists of different lengths.
    fn reset_many(
        &mut self,
        py: Python<'_>,
        slots: Vec<usize>,
        teams1: Vec<String>,
        teams2: Vec<String>,
        seeds: Vec<u64>,
    ) -> PyResult<()> {
        let n = slots.len();
        if teams1.len() != n || teams2.len() != n || seeds.len() != n {
            return Err(PyValueError::new_err(
                "slots, teams1, teams2 and seeds differ in length",
            ));
        }
        let starts: Vec<(usize, [&str; 2], u64)> = (0..n)
            .map(|k| (slots[k], [teams1[k].as_str(), teams2[k].as_str()], seeds[k]))
            .collect();
        let inner = &mut self.inner;
        py.detach(|| inner.reset_many(&starts))
            .map_err(PyValueError::new_err)
    }

    /// uint8 bytes, one per slot: 1 when its battle is over (`done`).
    fn dones<'py>(&self, py: Python<'py>) -> Bound<'py, pyo3::types::PyBytes> {
        let d: Vec<u8> = self.inner.dones().into_iter().map(u8::from).collect();
        pyo3::types::PyBytes::new(py, &d)
    }

    /// Whether battle `i` is over (ended, past the turn limit, or never
    /// started).
    fn done(&self, i: usize) -> bool {
        self.inner.done(i)
    }

    /// The winning side of battle `i` ("p1" or "p2"), or None (a draw, a
    /// battle still running or stopped at the turn limit).
    fn winner(&self, i: usize) -> Option<&'static str> {
        self.inner
            .battle(i)?
            .winner
            .map(|s| if s == 0 { "p1" } else { "p2" })
    }

    /// The turn of battle `i` (0 before it starts).
    fn turn(&self, i: usize) -> u32 {
        self.inner.battle(i).map_or(0, |b| b.turn)
    }

    /// A copy of battle `i`, to inspect or search from.
    fn battle(&self, i: usize) -> PyResult<PyBattle> {
        self.inner
            .battle(i)
            .map(|b| PyBattle { inner: b.clone() })
            .ok_or_else(|| PyValueError::new_err(format!("no battle in slot {i}")))
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyEnumerationMode>()?;
    m.add_class::<PyOutcome>()?;
    m.add_class::<PyBattleStateKey>()?;
    m.add_class::<PyDex>()?;
    m.add_class::<PyBattle>()?;
    m.add_class::<PyTracker>()?;
    m.add_class::<PyBatch>()?;
    m.add("ACTION_WIDTH", crate::batch::ACTION_WIDTH)?;
    m.add("DEFAULT_MAX_PATHS", crate::DEFAULT_MAX_PATHS)?;
    m.add("OBS_LEN", crate::encode::OBS_LEN)?;
    m.add("IDS_LEN", crate::encode::IDS_LEN)?;
    m.add("ACTION_FIELDS", crate::encode::ACTION_FIELDS)?;
    #[cfg(feature = "approx")]
    m.add_function(wrap_pyfunction!(set_approx, m)?)?;
    Ok(())
}

/// Research builds only (feature `approx`): set the switches of
/// `ninjask::approx` for every battle in the process (0 is exact play).
#[cfg(feature = "approx")]
#[pyfunction]
fn set_approx(flags: u32) {
    crate::approx::set(flags);
}

/// `ninjask._core`, the extension module the `ninjask` package re-exports.
#[cfg(feature = "python-module")]
#[pymodule(name = "_core")]
fn core_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    register(m)
}
