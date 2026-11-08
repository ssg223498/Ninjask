//! Format data loaded from `refsim/export_format.js` output, interned to ids.

pub mod events;
pub mod known;
pub mod support;
pub mod team;
pub mod types;

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use serde_json::Value;

use crate::dex::events::EventHandlers;
use crate::dex::known::KnownIds;
use crate::dex::types::*;
use crate::kinds::{AbilityKind, ItemKind, MoveKind};

#[derive(Clone, Debug)]
pub struct Species {
    pub id: String,
    pub name: String,
    pub base_species: String,
    pub types: [TypeId; 2],
    pub base_stats: [u16; 6],
    /// Ability names by slot ("0", "1", "H", "S").
    pub abilities: Vec<(String, String)>,
    pub weight_kg: f32,
    pub battle_only: bool,
    pub is_mega: bool,
    /// A fixed gender ("M", "F", "N" for none), or "" when it varies.
    pub gender: String,
}

/// User volatiles from `self.volatileStatus` that the engine models.
pub const MODELED_SELF_VOLATILES: &[&str] = &[
    "mustrecharge",
    "roost",
    "glaiverush",
    "lockedmove",
    "uproar",
];

#[derive(Clone, Debug, Default)]
pub struct Secondary {
    /// Percent chance; `None` means always.
    pub chance: Option<u8>,
    pub status: Option<Status>,
    pub volatile: Option<String>,
    pub boosts: Option<MoveBoosts>,
    pub self_boosts: Option<MoveBoosts>,
}

#[derive(Clone, Debug)]
pub struct Move {
    /// This move's index in `Dex::moves`.
    pub idx: MoveId,
    pub id: String,
    pub name: String,
    pub type_: TypeId,
    pub category: Category,
    pub base_power: u16,
    /// `None` means the move can't miss (`accuracy: true`).
    pub accuracy: Option<u8>,
    pub pp: u8,
    pub priority: i8,
    pub target: Target,
    pub flags: MoveFlags,
    pub crit_ratio: u8,
    pub drain: Option<(u32, u32)>,
    pub recoil: Option<(u32, u32)>,
    pub multihit: Option<(u8, u8)>,
    pub secondaries: Vec<Secondary>,
    pub self_boosts: Option<MoveBoosts>,
    /// A volatile the move puts on its user (`self.volatileStatus`), when
    /// modelled: `mustrecharge` (Hyper Beam).
    pub self_volatile: Option<String>,
    pub boosts: Option<MoveBoosts>,
    pub status: Option<Status>,
    pub volatile_status: Option<String>,
    /// Showdown `ignoreImmunity`: `Some(true)` / `Some(false)` when set
    /// (status moves default to ignoring type immunity).
    pub ignore_immunity: Option<bool>,
    /// Heals this fraction of the target's max HP (Recover, Life Dew).
    pub heal: Option<(u32, u32)>,
    /// Always a critical hit.
    pub will_crit: bool,
    /// Thaws a frozen target after the move (Scald).
    pub thaws_target: bool,
    /// Boosts the user once after the move hits (Clanging Scales, Scale Shot).
    pub self_boost_after: Option<MoveBoosts>,
    /// Ignores the target's defensive / evasion stages (Darkest Lariat).
    pub ignore_defensive: bool,
    pub ignore_evasion: bool,
    /// Fixed damage: the user's level (`Some(None)`) or a number.
    pub fixed_damage: Option<Option<u32>>,
    /// Uses this stat of the attacker (Body Press: Def) or defender (Psyshock: Def).
    pub override_offensive_stat: Option<Stat>,
    pub override_defensive_stat: Option<Stat>,
    /// Uses the target's attacking stat (Foul Play).
    pub offensive_pokemon_is_target: bool,
    /// One-hit KO (Sheer Cold, Fissure): `Some(None)`, or `Some(Some(t))`
    /// when Pokémon of type `t` are immune (Sheer Cold: Ice).
    pub ohko: Option<Option<TypeId>>,
    /// The user switches out after hitting (U-turn, Volt Switch, Flip Turn).
    pub self_switch: bool,
    /// Drags the target out for a random teammate (Roar, Dragon Tail).
    pub force_switch: bool,
    /// Removes the target's Protect and its side's guards (Feint, Phantom Force).
    pub breaks_protect: bool,
    /// Dragon Darts: one hit on the target and one on its ally when both can
    /// be hit (`smartTarget`).
    pub smart_target: bool,
    /// Sheer Force boosts it though its secondary effect is in code (Electro Shot).
    pub has_sheer_force_boost: bool,
    /// PP Ups don't apply (Revival Blessing): max PP is the base PP.
    pub no_pp_boosts: bool,
    /// Set on the copy a Sheer Force user strikes with, its secondaries gone
    /// (`move.hasSheerForce`); never in the dex.
    pub sheer_force: bool,
    pub callbacks: Vec<String>,
    /// Handler sort keys (`onBasePowerPriority`...), as `nums_of` reads them.
    pub nums: Vec<(String, f64)>,
    pub core_ref: bool,
    /// Non-null top-level fields in the export, for the support check.
    pub fields: Vec<String>,
    /// Problems found while reading data the engine doesn't model.
    pub unmodeled: Vec<String>,
}

impl Move {
    #[inline]
    pub fn has(&self, flag: MoveFlag) -> bool {
        self.flags.contains(flag)
    }

    /// `has` by the flag's id, for flags named in data; false for an id
    /// that is no move flag.
    pub fn has_flag(&self, flag: &str) -> bool {
        MoveFlag::from_id(flag).is_some_and(|f| self.has(f))
    }
}

#[derive(Clone, Debug)]
pub struct Item {
    pub id: String,
    pub name: String,
    /// Base species name -> Mega forme name.
    pub mega_stone: Vec<(String, String)>,
    pub is_berry: bool,
    /// Fling's base power with this item (0 if it can't be flung).
    pub fling_power: u16,
    pub callbacks: Vec<String>,
    pub nums: Vec<(String, f64)>,
    pub core_ref: bool,
    /// Non-null top-level fields in the export (`isChoice`, `isGem`, `boosts`...).
    pub fields: Vec<String>,
}

impl Item {
    pub fn has_field(&self, field: &str) -> bool {
        self.fields.iter().any(|f| f == field)
    }
}

#[derive(Clone, Debug)]
pub struct Ability {
    pub id: String,
    pub name: String,
    /// Showdown ability flags (`notrace`, `cantsuppress`...).
    pub flags: Vec<String>,
    /// The `breakable` flag: Mold Breaker and its kind ignore the ability.
    pub breakable: bool,
    /// The `cantsuppress` flag: Gastro Acid and ability changes can't touch it.
    pub cant_suppress: bool,
    /// The `notransform` flag: a transformed Pokémon's copy of it does nothing.
    pub no_transform: bool,
    /// Showdown's rating of the ability's strength (-1 to 5).
    pub rating: f32,
    pub callbacks: Vec<String>,
    pub nums: Vec<(String, f64)>,
    pub core_ref: bool,
}

/// An effect's handlers: which callbacks it has, with their sort keys.
pub trait Handlers {
    fn callbacks(&self) -> &[String];
    fn nums(&self) -> &[(String, f64)];
    fn has_callback(&self, name: &str) -> bool {
        self.callbacks().iter().any(|c| c == name)
    }
    /// A handler's `...Priority`, `...Order` or `...SubOrder` value.
    fn num(&self, key: &str) -> Option<f64> {
        self.nums().iter().find(|(k, _)| k == key).map(|&(_, v)| v)
    }
}

impl Handlers for Move {
    fn callbacks(&self) -> &[String] {
        &self.callbacks
    }
    fn nums(&self) -> &[(String, f64)] {
        &self.nums
    }
}

impl Handlers for Item {
    fn callbacks(&self) -> &[String] {
        &self.callbacks
    }
    fn nums(&self) -> &[(String, f64)] {
        &self.nums
    }
}

impl Handlers for Ability {
    fn callbacks(&self) -> &[String] {
        &self.callbacks
    }
    fn nums(&self) -> &[(String, f64)] {
        &self.nums
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Nature {
    pub plus: Option<Stat>,
    pub minus: Option<Stat>,
}

#[derive(Clone, Debug)]
pub struct FormatInfo {
    pub id: String,
    pub name: String,
    pub doubles: bool,
    pub picked_team_size: Option<usize>,
    pub level: Option<u8>,
    pub stat_point_limit: Option<u16>,
    pub champions: bool,
    /// The Cancel Mod rule (in Flat Rules): a player may change a choice
    /// until the foe has chosen, so a request says `noCancel` only when it
    /// is the only one (Showdown `getRequests`).
    pub cancel_mod: bool,
}

/// FxHash (as in rustc) for the dex's lookups by id, which run during
/// play (condition numbers, natures): far cheaper than the default
/// SipHash on short keys. The keys come from the format's own data, so
/// SipHash's resistance to crafted collisions is not needed.
#[derive(Clone, Copy, Debug, Default)]
pub struct IdHasher(u64);

impl IdHasher {
    /// FxHash's multiplier (from Firefox): odd, with well-spread bits.
    const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

    #[inline]
    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(IdHasher::SEED);
    }
}

impl Hasher for IdHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let (chunks, rest) = bytes.as_chunks::<8>();
        for chunk in chunks {
            self.add(u64::from_le_bytes(*chunk));
        }
        let mut tail = [0u8; 8];
        tail[..rest.len()].copy_from_slice(rest);
        self.add(u64::from_le_bytes(tail));
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
}

/// A map keyed by id, with [`IdHasher`].
type IdMap<V> = HashMap<String, V, BuildHasherDefault<IdHasher>>;

/// All data for one format.
#[derive(Clone, Debug)]
pub struct Dex {
    pub format: FormatInfo,
    pub type_names: Vec<String>,
    type_index: IdMap<TypeId>,
    /// `taken[defender][attacker]`: Showdown damageTaken code (0 normal, 1 weak, 2 resist, 3 immune).
    taken: Vec<Vec<u8>>,
    /// `immune[defender_type]` to these effect ids (brn, par, psn, tox, frz, sandstorm, hail, powder, prankster, trapped).
    type_effect_immunity: Vec<Vec<String>>,
    pub species: Vec<Species>,
    species_index: IdMap<SpeciesId>,
    pub moves: Vec<Move>,
    move_index: IdMap<MoveId>,
    pub items: Vec<Item>,
    item_index: IdMap<ItemId>,
    pub abilities: Vec<Ability>,
    ability_index: IdMap<AbilityId>,
    natures: IdMap<Nature>,
    pub learnsets: HashMap<String, Vec<String>>,
    /// Handler kind per move, ability and item id.
    pub move_kind: Vec<MoveKind>,
    pub ability_kind: Vec<AbilityKind>,
    pub item_kind: Vec<ItemKind>,
    /// Sorted-event handler priorities per ability and item id.
    pub ability_handlers: Vec<EventHandlers>,
    pub item_handlers: Vec<EventHandlers>,
    /// Ids the mechanics refer to by name.
    pub ids: KnownIds,
    /// Numeric fields of Showdown conditions (duration, residual orders...).
    conditions: IdMap<IdMap<f64>>,
    /// Per item, its `mega_stone` pairs as species: the species named as the
    /// base, and the Mega forme (`None` for a name not in the dex).
    mega_forms: Vec<Vec<(Option<SpeciesId>, Option<SpeciesId>)>>,
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// A boosts object, keeping its key order (serde_json's `preserve_order`).
fn boosts_of(v: &Value) -> Option<MoveBoosts> {
    let obj = v.as_object()?;
    let mut vals = [0i8; 7];
    let mut order = Vec::with_capacity(7);
    for (k, x) in obj {
        let i = Boost::from_id(k)? as usize;
        vals[i] = x.as_i64()? as i8;
        order.push(i as u8);
    }
    order.extend(
        BOOST_ORDER
            .into_iter()
            .filter(|i| !order.contains(i))
            .collect::<Vec<_>>(),
    );
    Some(MoveBoosts {
        vals,
        order: order.try_into().ok()?,
    })
}

/// Numeric handler keys of an exported effect: fields ending in Priority,
/// Order or SubOrder.
fn nums_of(v: &Value) -> Vec<(String, f64)> {
    v.as_object()
        .map(|o| {
            o.iter()
                .filter(|(k, _)| k.ends_with("Priority") || k.ends_with("Order"))
                .filter_map(|(k, x)| Some((k.clone(), x.as_f64()?)))
                .collect()
        })
        .unwrap_or_default()
}

fn fraction(v: &Value) -> Option<(u32, u32)> {
    let a = v.as_array()?;
    Some((a.first()?.as_u64()? as u32, a.get(1)?.as_u64()? as u32))
}

impl Dex {
    pub fn from_json(text: &str) -> Result<Dex, String> {
        let root: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let f = &root["format"];
        let format = FormatInfo {
            id: s(f, "id"),
            name: s(f, "name"),
            doubles: s(f, "gameType") == "doubles",
            picked_team_size: f["pickedTeamSize"].as_u64().map(|n| n as usize),
            level: f["adjustLevel"].as_u64().map(|n| n as u8),
            stat_point_limit: f["evLimit"].as_u64().map(|n| n as u16),
            champions: s(f, "mod").starts_with("champions"),
            cancel_mod: f["rules"]
                .as_array()
                .is_some_and(|rules| rules.iter().any(|r| r["rule"] == "cancelmod")),
        };

        // Types: every key of the type chart is a type; inner keys that are
        // not types are effect immunities (brn, par, sandstorm, ...).
        let chart = root["typechart"].as_object().ok_or("missing typechart")?;
        let mut type_names: Vec<String> = chart.keys().cloned().collect();
        type_names.sort();
        // Showdown's typeless "???" (Double Shock's user) has no chart entry:
        // neutral to everything and immune to nothing.
        type_names.push("???".to_string());
        let type_index: IdMap<TypeId> = type_names
            .iter()
            .enumerate()
            .map(|(i, t)| (t.clone(), i as TypeId))
            .collect();
        let n = type_names.len();
        let mut taken = vec![vec![0u8; n]; n];
        let mut type_effect_immunity = vec![Vec::new(); n];
        for (def_name, row) in chart {
            let d = type_index[def_name] as usize;
            for (k, code) in row.as_object().ok_or("bad typechart row")? {
                let code = code.as_u64().unwrap_or(0) as u8;
                if let Some(&a) = type_index.get(k) {
                    taken[d][a as usize] = code;
                } else if code == 3 {
                    type_effect_immunity[d].push(k.clone());
                }
            }
        }
        let tid = |name: &str| -> Result<TypeId, String> {
            type_index
                .get(name)
                .copied()
                .ok_or_else(|| format!("unknown type {name}"))
        };

        let mut species = Vec::new();
        for sp in root["species"].as_array().ok_or("missing species")? {
            let types = strings(&sp["types"]);
            let t0 = tid(&types[0])?;
            let t1 = if types.len() > 1 {
                tid(&types[1])?
            } else {
                NO_TYPE
            };
            let bs = &sp["baseStats"];
            let mut base_stats = [0u16; 6];
            for st in Stat::ALL {
                base_stats[st as usize] = bs[st.id()].as_u64().unwrap_or(0) as u16;
            }
            let abilities = sp["abilities"]
                .as_object()
                .map(|o| {
                    o.iter()
                        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                        .collect()
                })
                .unwrap_or_default();
            species.push(Species {
                id: s(sp, "id"),
                name: s(sp, "name"),
                base_species: s(sp, "baseSpecies"),
                types: [t0, t1],
                base_stats,
                abilities,
                weight_kg: sp["weightkg"].as_f64().unwrap_or(0.0) as f32,
                battle_only: !sp["battleOnly"].is_null(),
                is_mega: sp["isMega"].as_bool().unwrap_or(false),
                gender: sp["gender"].as_str().unwrap_or("").to_string(),
            });
        }

        let mut moves = Vec::new();
        for m in root["moves"].as_array().ok_or("missing moves")? {
            let obj = m.as_object().ok_or("bad move")?;
            let mut unmodeled = Vec::new();
            let mut flags = MoveFlags::EMPTY;
            for f in m["flags"].as_object().into_iter().flatten().map(|(f, _)| f) {
                match MoveFlag::from_id(f) {
                    Some(flag) => flags.insert(flag),
                    None => unmodeled.push(format!("flag {f}")),
                }
            }
            let accuracy = match &m["accuracy"] {
                Value::Bool(true) => None,
                v => Some(v.as_u64().unwrap_or(100) as u8),
            };
            let category = match s(m, "category").as_str() {
                "Physical" => Category::Physical,
                "Special" => Category::Special,
                _ => Category::Status,
            };
            let mut secondaries = Vec::new();
            for sec in m["secondaries"].as_array().into_iter().flatten() {
                let mut out = Secondary {
                    chance: sec["chance"].as_u64().map(|c| c as u8),
                    ..Default::default()
                };
                for (k, v) in sec.as_object().into_iter().flatten() {
                    match k.as_str() {
                        "chance" => {}
                        "status" => out.status = v.as_str().and_then(Status::from_id),
                        "volatileStatus" => out.volatile = v.as_str().map(str::to_string),
                        "boosts" => out.boosts = boosts_of(v),
                        "self" => {
                            for (sk, sv) in v.as_object().into_iter().flatten() {
                                if sk == "boosts" {
                                    out.self_boosts = boosts_of(sv);
                                } else {
                                    unmodeled.push(format!("secondary.self.{sk}"));
                                }
                            }
                        }
                        other => unmodeled.push(format!("secondary.{other}")),
                    }
                }
                secondaries.push(out);
            }
            let mut self_boosts = None;
            let mut self_volatile = None;
            if let Some(selfv) = m.get("self").filter(|v| !v.is_null()) {
                for (k, v) in selfv.as_object().into_iter().flatten() {
                    match (k.as_str(), v.as_str()) {
                        ("boosts", _) => self_boosts = boosts_of(v),
                        ("volatileStatus", Some(vol)) if MODELED_SELF_VOLATILES.contains(&vol) => {
                            self_volatile = Some(vol.to_string())
                        }
                        _ => unmodeled.push(format!("self.{k}")),
                    }
                }
            }
            let multihit = match &m["multihit"] {
                Value::Number(n) => {
                    Some((n.as_u64().unwrap_or(1) as u8, n.as_u64().unwrap_or(1) as u8))
                }
                Value::Array(a) => Some((
                    a.first().and_then(|x| x.as_u64()).unwrap_or(1) as u8,
                    a.get(1).and_then(|x| x.as_u64()).unwrap_or(1) as u8,
                )),
                _ => None,
            };
            let fields: Vec<String> = obj
                .iter()
                .filter(|(_, v)| !v.is_null() && !matches!(v, Value::Bool(false)))
                .map(|(k, _)| k.clone())
                .collect();
            moves.push(Move {
                idx: moves.len() as MoveId,
                id: s(m, "id"),
                name: s(m, "name"),
                type_: tid(&s(m, "type"))?,
                category,
                base_power: m["basePower"].as_u64().unwrap_or(0) as u16,
                accuracy,
                pp: m["pp"].as_u64().unwrap_or(1) as u8,
                priority: m["priority"].as_i64().unwrap_or(0) as i8,
                target: Target::from_id(&s(m, "target"))
                    .ok_or_else(|| format!("bad target in {}", s(m, "id")))?,
                flags,
                crit_ratio: m["critRatio"].as_u64().unwrap_or(1) as u8,
                drain: fraction(&m["drain"]),
                recoil: fraction(&m["recoil"]),
                multihit,
                secondaries,
                self_boosts,
                self_volatile,
                boosts: boosts_of(&m["boosts"]),
                status: m["status"].as_str().and_then(Status::from_id),
                volatile_status: m["volatileStatus"].as_str().map(str::to_string),
                ignore_immunity: m["ignoreImmunity"].as_bool(),
                heal: fraction(&m["heal"]),
                will_crit: m["willCrit"].as_bool().unwrap_or(false),
                thaws_target: m["thawsTarget"].as_bool().unwrap_or(false),
                self_boost_after: boosts_of(&m["selfBoost"]["boosts"]),
                ignore_defensive: m["ignoreDefensive"].as_bool().unwrap_or(false),
                ignore_evasion: m["ignoreEvasion"].as_bool().unwrap_or(false),
                fixed_damage: match &m["damage"] {
                    Value::String(s) if s == "level" => Some(None),
                    Value::Number(n) => Some(n.as_u64().map(|x| x as u32)),
                    _ => None,
                },
                override_offensive_stat: m["overrideOffensiveStat"]
                    .as_str()
                    .and_then(Stat::from_id),
                override_defensive_stat: m["overrideDefensiveStat"]
                    .as_str()
                    .and_then(Stat::from_id),
                offensive_pokemon_is_target: m["overrideOffensivePokemon"].as_str()
                    == Some("target"),
                ohko: match &m["ohko"] {
                    Value::Bool(true) => Some(None),
                    Value::String(t) => Some(Some(tid(t)?)),
                    _ => None,
                },
                self_switch: m["selfSwitch"].as_bool().unwrap_or(false),
                force_switch: m["forceSwitch"].as_bool().unwrap_or(false),
                breaks_protect: m["breaksProtect"].as_bool().unwrap_or(false),
                smart_target: m["smartTarget"].as_bool().unwrap_or(false),
                has_sheer_force_boost: m["hasSheerForceBoost"].as_bool().unwrap_or(false),
                no_pp_boosts: m["noPPBoosts"].as_bool().unwrap_or(false),
                sheer_force: false,
                callbacks: strings(&m["callbacks"]),
                nums: nums_of(m),
                core_ref: m["coreRef"].as_bool().unwrap_or(false),
                fields,
                unmodeled,
            });
        }

        let mut items = Vec::new();
        for it in root["items"].as_array().ok_or("missing items")? {
            let mega_stone = it["megaStone"]
                .as_object()
                .map(|o| {
                    o.iter()
                        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let fields = it
                .as_object()
                .map(|o| {
                    o.iter()
                        .filter(|(_, v)| !v.is_null() && !matches!(v, Value::Bool(false)))
                        .map(|(k, _)| k.clone())
                        .collect()
                })
                .unwrap_or_default();
            items.push(Item {
                id: s(it, "id"),
                name: s(it, "name"),
                mega_stone,
                is_berry: it["isBerry"].as_bool().unwrap_or(false),
                fling_power: it["fling"]["basePower"].as_u64().unwrap_or(0) as u16,
                callbacks: strings(&it["callbacks"]),
                nums: nums_of(it),
                core_ref: it["coreRef"].as_bool().unwrap_or(false),
                fields,
            });
        }

        let mut abilities = Vec::new();
        for a in root["abilities"].as_array().ok_or("missing abilities")? {
            let flags: Vec<String> = a["flags"]
                .as_object()
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default();
            let has_flag = |f: &str| flags.iter().any(|x| x == f);
            abilities.push(Ability {
                id: s(a, "id"),
                name: s(a, "name"),
                breakable: has_flag("breakable"),
                cant_suppress: has_flag("cantsuppress"),
                no_transform: has_flag("notransform"),
                flags,
                rating: a["rating"].as_f64().unwrap_or(0.0) as f32,
                callbacks: strings(&a["callbacks"]),
                nums: nums_of(a),
                core_ref: a["coreRef"].as_bool().unwrap_or(false),
            });
        }
        // Custom formats allow "No Ability"; keep it available everywhere.
        if !abilities.iter().any(|a| a.id == "noability") {
            abilities.push(Ability {
                id: "noability".into(),
                name: "No Ability".into(),
                flags: vec![],
                breakable: false,
                cant_suppress: false,
                no_transform: false,
                rating: 0.0,
                callbacks: vec![],
                nums: vec![],
                core_ref: false,
            });
        }

        let mut natures = IdMap::default();
        for (id, nv) in root["natures"].as_object().ok_or("missing natures")? {
            natures.insert(
                id.clone(),
                Nature {
                    plus: nv["plus"].as_str().and_then(Stat::from_id),
                    minus: nv["minus"].as_str().and_then(Stat::from_id),
                },
            );
        }

        let learnsets = root["learnsets"]
            .as_object()
            .map(|o| o.iter().map(|(k, v)| (k.clone(), strings(v))).collect())
            .unwrap_or_default();

        let mut conditions = IdMap::default();
        for (id, c) in root["conditions"].as_object().into_iter().flatten() {
            let nums: IdMap<f64> = c
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(k, v)| v.as_f64().map(|x| (k.clone(), x)))
                .collect();
            conditions.insert(id.clone(), nums);
        }
        // No Pokémon learns Struggle, so the export leaves it out; add it as
        // Showdown defines it (its onModifyMove makes it typeless).
        if !moves.iter().any(|m| m.id == "struggle") {
            let flags = [
                MoveFlag::Contact,
                MoveFlag::Protect,
                MoveFlag::FailEncore,
                MoveFlag::FailMeFirst,
                MoveFlag::NoSleepTalk,
                MoveFlag::NoAssist,
                MoveFlag::FailCopycat,
                MoveFlag::FailMimic,
                MoveFlag::FailInstruct,
                MoveFlag::NoSketch,
            ];
            moves.push(Move {
                idx: moves.len() as MoveId,
                id: "struggle".into(),
                name: "Struggle".into(),
                type_: tid("Normal")?,
                category: Category::Physical,
                base_power: 50,
                accuracy: None,
                pp: 1,
                priority: 0,
                target: Target::RandomNormal,
                flags: flags.into_iter().collect(),
                crit_ratio: 1,
                drain: None,
                recoil: None,
                multihit: None,
                secondaries: vec![],
                self_boosts: None,
                self_volatile: None,
                boosts: None,
                status: None,
                volatile_status: None,
                ignore_immunity: None,
                heal: None,
                will_crit: false,
                thaws_target: false,
                self_boost_after: None,
                ignore_defensive: false,
                ignore_evasion: false,
                fixed_damage: None,
                override_offensive_stat: None,
                override_defensive_stat: None,
                offensive_pokemon_is_target: false,
                ohko: None,
                self_switch: false,
                force_switch: false,
                breaks_protect: false,
                smart_target: false,
                has_sheer_force_boost: false,
                no_pp_boosts: false,
                sheer_force: false,
                callbacks: vec!["onModifyMove".into()],
                nums: vec![],
                core_ref: false,
                fields: vec![],
                unmodeled: vec![],
            });
        }
        let move_kind = moves.iter().map(|m| MoveKind::from_id(&m.id)).collect();
        let ability_kind = abilities
            .iter()
            .map(|a| AbilityKind::from_id(&a.id, |t| type_index.get(t).copied()))
            .collect();
        let item_kind = items
            .iter()
            .map(|it| {
                ItemKind::from_id(&it.id, !it.mega_stone.is_empty(), |t| {
                    type_index.get(t).copied()
                })
            })
            .collect();
        let ability_handlers = abilities.iter().map(EventHandlers::new).collect();
        let item_handlers = items.iter().map(EventHandlers::new).collect();
        let index = |names: Vec<String>| -> IdMap<u16> {
            names
                .into_iter()
                .enumerate()
                .map(|(i, n)| (n, i as u16))
                .collect()
        };
        let mut dex = Dex {
            format,
            type_index,
            type_names,
            taken,
            type_effect_immunity,
            species_index: index(species.iter().map(|x| x.id.clone()).collect()),
            move_index: index(moves.iter().map(|x| x.id.clone()).collect()),
            item_index: index(items.iter().map(|x| x.id.clone()).collect()),
            ability_index: index(abilities.iter().map(|x| x.id.clone()).collect()),
            species,
            moves,
            items,
            abilities,
            natures,
            learnsets,
            move_kind,
            ability_kind,
            item_kind,
            ability_handlers,
            item_handlers,
            ids: KnownIds::default(),
            conditions,
            mega_forms: Vec::new(),
        };
        dex.ids = KnownIds::resolve(&dex);
        // Species names are unique: the first species with a name is the one.
        let mut by_name: HashMap<&str, SpeciesId> = HashMap::new();
        for (i, sp) in dex.species.iter().enumerate() {
            by_name.entry(sp.name.as_str()).or_insert(i as SpeciesId);
        }
        dex.mega_forms = dex
            .items
            .iter()
            .map(|it| {
                it.mega_stone
                    .iter()
                    .map(|(base, mega)| (by_name.get(base.as_str()).copied(), dex.species_id(mega)))
                    .collect()
            })
            .collect();
        Ok(dex)
    }

    /// A numeric field of a Showdown condition, such as "duration" or "onResidualOrder".
    pub fn cond_num(&self, id: &str, field: &str) -> Option<f64> {
        self.conditions.get(id).and_then(|c| c.get(field)).copied()
    }

    pub fn species_id(&self, name: &str) -> Option<SpeciesId> {
        self.species_index.get(&to_id(name)).copied()
    }
    pub fn move_id(&self, name: &str) -> Option<MoveId> {
        self.move_index.get(&to_id(name)).copied()
    }
    pub fn item_id(&self, name: &str) -> Option<ItemId> {
        self.item_index.get(&to_id(name)).copied()
    }
    pub fn ability_id(&self, name: &str) -> Option<AbilityId> {
        self.ability_index.get(&to_id(name)).copied()
    }
    pub fn type_id(&self, name: &str) -> Option<TypeId> {
        self.type_index.get(name).copied()
    }
    pub fn nature(&self, name: &str) -> Nature {
        self.natures.get(&to_id(name)).copied().unwrap_or_default()
    }

    /// The Mega forme an item's Mega Stone turns `base` into: the first of
    /// its `mega_stone` pairs whose base is that species' name, resolved
    /// (`None` without one, or when the forme is not in the dex).
    pub fn mega_forme(&self, item: ItemId, base: SpeciesId) -> Option<SpeciesId> {
        self.mega_forms[item as usize]
            .iter()
            .find(|(b, _)| *b == Some(base))?
            .1
    }

    pub fn species(&self, id: SpeciesId) -> &Species {
        &self.species[id as usize]
    }
    pub fn mv(&self, id: MoveId) -> &Move {
        &self.moves[id as usize]
    }
    pub fn item(&self, id: ItemId) -> &Item {
        &self.items[id as usize]
    }
    pub fn ability(&self, id: AbilityId) -> &Ability {
        &self.abilities[id as usize]
    }

    /// Showdown's type effectiveness exponent of one attacking type on one
    /// defending type: +1 super effective, -1 resisted, 0 neutral.
    /// Immunity is reported separately by [`Dex::is_immune`].
    pub fn effectiveness_exp(&self, attack: TypeId, defend: TypeId) -> i32 {
        if defend == NO_TYPE {
            return 0;
        }
        match self.taken[defend as usize][attack as usize] {
            1 => 1,
            2 => -1,
            _ => 0,
        }
    }

    pub fn is_immune(&self, attack: TypeId, defend: TypeId) -> bool {
        defend != NO_TYPE && self.taken[defend as usize][attack as usize] == 3
    }

    /// Whether a defending type is immune to an effect such as "brn" or "sandstorm".
    pub fn type_immune_to_effect(&self, defend: TypeId, effect: &str) -> bool {
        defend != NO_TYPE
            && self.type_effect_immunity[defend as usize]
                .iter()
                .any(|e| e == effect)
    }
}
