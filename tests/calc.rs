//! The read-only damage calculation against the engine's own random-draw
//! outcomes, and the turn-order odds. Needs the format export
//! (`data/derived/format/`, see `common`); skipped without it.

mod common;

use std::sync::Arc;

use ninjask::calc::{check_samples, sample_games, Plan};
use ninjask::dex::team::parse_packed;
use ninjask::rng::Rng;
use ninjask::state::RequestState;
use ninjask::{Battle, Dex};

fn setup() -> Option<(Arc<Dex>, Vec<Vec<ninjask::dex::team::MonSet>>)> {
    let json = common::format_export("gen9championsvgc2026regmc")?;
    let root = env!("CARGO_MANIFEST_DIR");
    let dex = Arc::new(Dex::from_json(&json).ok()?);
    let teams = std::fs::read_to_string(format!("{root}/tests/fixtures/regmc_teams.txt"))
        .ok()?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| parse_packed(&dex, l).unwrap())
        .collect();
    Some((dex, teams))
}

#[test]
fn damage_matches_played_hits() {
    let Some((dex, teams)) = setup() else { return };
    let samples = sample_games(&dex, &teams, 400, 11);
    let r = check_samples(&samples);
    println!("{r}");
    assert!(r.hits > 5000);
    assert_eq!(r.outside, 0, "damage outside the predicted values");
    assert_eq!(r.ko_wrong, 0, "a certain KO went otherwise");
    assert_eq!(r.full_agree, r.full_compared, "full calculation disagrees");
    assert!(r.crit_z.abs() < 4.0 && r.ko_z.abs() < 4.0 && r.full_ko_z.abs() < 4.0);
    assert_eq!(
        r.full_ko_wrong, 0,
        "a KO the calculation called certain went otherwise"
    );
    assert!(r.pit_chi2.2 > 1e-4 && r.ko_chi2.2 > 1e-4);
}

#[test]
fn calc_leaves_the_battle_unchanged_and_orders_make_sense() {
    let Some((dex, teams)) = setup() else { return };
    let mut rng = Rng::new(5);
    for g in 0..20 {
        let mut b = Battle::from_sets(
            dex.clone(),
            [&teams[g % teams.len()], &teams[(g * 3 + 1) % teams.len()]],
            ["a", "b"],
            g as u64,
        );
        b.log_enabled = false;
        b.choose_team(0, vec![0, 1, 2, 3]);
        b.choose_team(1, vec![0, 1, 2, 3]);
        while !b.ended && b.turn < 4 {
            for s in 0..2 {
                if b.request != RequestState::Ended && b.side_needs_choice(s) {
                    let c = b.sample_joint_choice(s, &mut rng);
                    b.choose_structured(s, c);
                }
            }
        }
        if b.ended || b.request != RequestState::Move {
            continue;
        }
        let before = b.projection();
        let actives = b.all_active();
        for &a in &actives {
            for &t in &actives {
                for slot in 0..b.mon(a).moves.len() {
                    let id = b.mon(a).moves[slot].id;
                    let c = b.calc(a, id, t);
                    assert!(
                        (0.0..=1.0 + 1e-9).contains(&c.ko) && c.ko2 >= c.ko - 1e-9,
                        "{c:?}"
                    );
                    assert!(c.accuracy > 0.0 && c.accuracy <= 1.0);
                    if c.immune {
                        assert_eq!(c.expected, 0.0);
                    }
                }
            }
        }
        assert_eq!(before, b.projection(), "calc changed the battle");
        for &a in &actives {
            for &o in &actives {
                if a == o {
                    continue;
                }
                let x = b.order_odds(a, Plan::Priority(0), o, Plan::Priority(0));
                let y = b.order_odds(o, Plan::Priority(0), a, Plan::Priority(0));
                assert!((x.first + y.first - 1.0).abs() < 1e-9);
                assert!(b.order_odds(a, Plan::Switch, o, Plan::Priority(0)).first == 1.0);
                assert!(
                    b.order_odds(a, Plan::Priority(1), o, Plan::Priority(0))
                        .first
                        >= 0.8
                );
                let (sa, so) = (b.action_speed(a), b.action_speed(o));
                if sa == so {
                    assert!(x.tie > 0.0);
                }
            }
        }
        // Trick Room reverses the order of different speeds.
        let (a, o) = (actives[0], actives[actives.len() - 1]);
        let x = b.order_odds(a, Plan::Priority(0), o, Plan::Priority(0));
        b.field.trick_room = 5;
        let y = b.order_odds(a, Plan::Priority(0), o, Plan::Priority(0));
        if x.tie == 0.0 && y.tie == 0.0 && (x.first == 0.0 || x.first == 1.0) {
            assert!((x.first - (1.0 - y.first)).abs() < 1e-9);
        }
    }
}

/// The grouped calculation (`calc_all`) and the turn-order matrix
/// (`order_matrix`) give exactly the per-target results: they share the
/// preparation and the order keys, never the arithmetic.
#[test]
fn grouped_calculations_match_single_ones() {
    let Some((dex, teams)) = setup() else { return };
    let mut rng = Rng::new(7);
    let mut compared = 0usize;
    for g in 0..24 {
        let mut b = Battle::from_sets(
            dex.clone(),
            [&teams[g % teams.len()], &teams[(g * 5 + 2) % teams.len()]],
            ["a", "b"],
            100 + g as u64,
        );
        b.log_enabled = false;
        b.choose_team(0, vec![0, 1, 2, 3]);
        b.choose_team(1, vec![0, 1, 2, 3]);
        for _ in 0..(g % 5) {
            if b.ended {
                break;
            }
            for s in 0..2 {
                if b.request != RequestState::Ended && b.side_needs_choice(s) {
                    let c = b.sample_joint_choice(s, &mut rng);
                    b.choose_structured(s, c);
                }
            }
        }
        if b.ended {
            continue;
        }
        let actives = b.all_active();
        // With and without Wonder Room (its ModifyMove step logs a hint
        // and swaps offensive stats, so the shared preparation must not
        // change what a target sees).
        for wonder in [0u8, 3] {
            b.field.wonder_room = wonder;
            let matrix = b.order_matrix(&actives);
            for (i, &a) in actives.iter().enumerate() {
                for (j, &o) in actives.iter().enumerate() {
                    if a != o {
                        assert_eq!(
                            matrix[i][j],
                            b.order_odds(a, Plan::Priority(0), o, Plan::Priority(0))
                                .first,
                            "order matrix disagrees with order_odds"
                        );
                    }
                }
                for slot in 0..b.mon(a).moves.len() {
                    let id = b.mon(a).moves[slot].id;
                    let grouped = b.calc_all(a, id, &actives);
                    assert_eq!(grouped.len(), actives.len());
                    for (n, &t) in actives.iter().enumerate() {
                        assert_eq!(
                            grouped[n],
                            b.calc(a, id, t),
                            "calc_all disagrees with calc (wonder {wonder})"
                        );
                        compared += 1;
                    }
                }
            }
        }
    }
    assert!(compared > 1000, "only {compared} calculations compared");
}

/// Damage and turn order for any Pokémon of a party (`damage_calc_party`,
/// `order_odds_party`): a benched one counts as if it were in its side's
/// slot. In singles (`gen9championsbssregmc`): pointing at the Pokémon on
/// the field gives exactly `damage_calc` and `order_odds`, a benched one's
/// calculation targets its own HP, and the battle is left as it was.
#[test]
fn party_calculations_place_benched_pokemon_in_the_slot() {
    let Some(json) = common::format_export("gen9championsbssregmc") else {
        return;
    };
    let root = env!("CARGO_MANIFEST_DIR");
    let dex = Arc::new(Dex::from_json(&json).unwrap());
    let teams: Vec<Vec<ninjask::dex::team::MonSet>> =
        std::fs::read_to_string(format!("{root}/tests/fixtures/regmc_teams.txt"))
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| parse_packed(&dex, l).unwrap())
            .collect();
    let mut rng = Rng::new(9);
    let (mut same, mut bench) = (0usize, 0usize);
    for g in 0..16 {
        let mut b = Battle::from_sets(
            dex.clone(),
            [&teams[g % teams.len()], &teams[(g * 3 + 1) % teams.len()]],
            ["a", "b"],
            200 + g as u64,
        );
        b.log_enabled = false;
        b.choose_team(0, vec![0, 1, 2]);
        b.choose_team(1, vec![2, 0, 1]);
        for _ in 0..(g % 4) {
            for s in 0..2 {
                if b.request != RequestState::Ended && b.side_needs_choice(s) {
                    let c = b.sample_joint_choice(s, &mut rng);
                    b.choose_structured(s, c);
                }
            }
        }
        if b.ended || b.request != RequestState::Move {
            continue;
        }
        let before = b.projection();
        let (a, o) = (b.active(0, 0).unwrap(), b.active(1, 0).unwrap());
        for slot in 0..b.mon(a).moves.len() {
            let id = b.mon(a).moves[slot].id;
            assert_eq!(
                b.damage_calc_party(a, id, o).unwrap(),
                b.damage_calc(a, id, o)
            );
            let p = Plan::Move(id);
            assert_eq!(
                b.order_odds_party(a, p, o, Plan::Priority(0)).unwrap(),
                b.order_odds(a, p, o, Plan::Priority(0))
            );
            same += 1;
        }
        for x in 0..b.sides[0].mons.len() {
            for y in 0..b.sides[1].mons.len() {
                let (x, y) = ((0, x), (1, y));
                if b.mon(x).fainted || b.mon(y).fainted {
                    continue;
                }
                let (view, refs) = b.with_in_slot(&[x, y]).unwrap();
                // Placed in the slot, with the Pokémon it displaced on the bench.
                assert_eq!(refs, vec![(0, 0), (1, 0)]);
                assert_eq!(view.mon(refs[0]).name, b.mon(x).name);
                assert_eq!(view.mon(refs[1]).name, b.mon(y).name);
                assert!(view.mon(refs[0]).is_active && view.mon(refs[1]).is_active);
                assert_eq!(view.all_active().len(), 2);
                for slot in 0..b.mon(x).moves.len() {
                    let id = b.mon(x).moves[slot].id;
                    let c = b.damage_calc_party(x, id, y).unwrap();
                    assert_eq!(c.target_maxhp, b.mon(y).maxhp as u32);
                    assert_eq!(c.target_hp, b.mon(y).hp as u32);
                    assert_eq!(c, view.damage_calc(refs[0], id, refs[1]));
                    bench += 1;
                }
                let o = b
                    .order_odds_party(x, Plan::Switch, y, Plan::Priority(0))
                    .unwrap();
                assert_eq!(o.first, 1.0);
            }
        }
        assert_eq!(
            before,
            b.projection(),
            "a party calculation changed the battle"
        );
        // Out of range, and two Pokémon of one side in its one slot.
        let n = b.sides[0].mons.len();
        assert!(b.with_in_slot(&[(0, n)]).is_err());
        assert!(b.with_in_slot(&[(2, 0)]).is_err());
        assert!(b.with_in_slot(&[(0, 1), (0, 2)]).is_err());
        assert!(b.with_in_slot(&[(0, 0), (0, 2)]).is_err());
        assert!(b.with_in_slot(&[(0, 2), (0, 2)]).is_ok());
    }
    assert!(same >= 20 && bench >= 200, "{same} {bench}");
}
