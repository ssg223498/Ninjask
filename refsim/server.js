#!/usr/bin/env node
/**
 * Reference simulator server: drives the pinned Pokémon Showdown simulator
 * over JSON lines on stdin/stdout. One JSON object per line in, one out.
 *
 * Commands (all take "cmd"; battle commands take "id"):
 *   start    {id, format, seed, p1: {name, team}, p2: {name, team}, ots?}
 *            team is packed or export text. ots: show open team sheets at
 *            preview. Returns the first observations (see `observation`).
 *   choose   {id, side, choice}   -> observations after the choice
 *   state    {id}                 -> canonical state projection
 *   inputlog {id}                 -> the battle's input log (exact replay)
 *   replay   {inputlog, states?}  -> replays an input log; with states, the
 *                                   projection after every input line
 *   branch   {id, choices: {p1, p2}, seeds: [...]}
 *                                -> from the battle's current decision point,
 *                                   apply the same choices once per seed on a
 *                                   copy and return each resulting projection
 *                                   (the outcome distribution for that turn)
 *   close    {id}
 *   validate {format, team}       -> {problems: [...] | null}
 *   pack     {team}               -> {packed}
 *   version                       -> Showdown commit info
 *
 * Observations: {requests: {p1, p2}, logs: {p1: [...], p2: [...],
 * spectator: [...]}, errors: {p1, p2}, ended, winner}. Requests, errors and
 * log lines are those produced since the previous response (each is
 * delivered once); logs are split per viewer exactly as Showdown splits them
 * for real clients. A side with no new request has nothing new to answer.
 */
'use strict';

const path = require('path');
const readline = require('readline');

const PS = require('./showdown_path').showdownPath();
const sim = require(path.join(PS, 'dist', 'sim'));
const { extractChannelMessages } = require(path.join(PS, 'dist', 'sim', 'battle'));
const { Battle, PRNG, Teams, TeamValidator } = sim;

const battles = new Map();

function toPackedTeam(team) {
	if (typeof team !== 'string') throw new Error('team must be a string');
	const trimmed = team.trim();
	// Export format has newlines; packed format has none.
	if (trimmed.includes('\n')) {
		const sets = Teams.import(trimmed);
		if (!sets) throw new Error('could not import team');
		return Teams.pack(sets);
	}
	return trimmed;
}

/**
 * The levels a server's team validation leaves (TeamValidator: `adjustLevel`,
 * and `adjustLevelDown` for higher levels). Battles here skip validation,
 * so a set without a level would otherwise stay at 100.
 */
function adjustLevels(format, packed) {
	const rules = sim.Dex.formats.getRuleTable(sim.Dex.formats.get(format));
	if (!rules.adjustLevel && !rules.adjustLevelDown) return packed;
	const sets = Teams.unpack(packed);
	if (!sets) throw new Error('could not unpack team');
	for (const set of sets) {
		const level = set.level || 100;
		if (rules.adjustLevelDown && level >= rules.adjustLevelDown) set.level = rules.adjustLevelDown;
		else if (rules.adjustLevel) set.level = rules.adjustLevel;
	}
	return Teams.pack(sets);
}

function newRecord(battle) {
	return {
		battle,
		logs: { p1: [], p2: [], spectator: [], omniscient: [] },
		requests: { p1: null, p2: null },
		errors: { p1: null, p2: null },
		ended: false,
	};
}

function makeSend(rec) {
	return (type, data) => {
		if (Array.isArray(data)) data = data.join('\n');
		if (type === 'update') {
			const ch = extractChannelMessages(data, [-1, 0, 1, 2]);
			rec.logs.omniscient.push(...ch[-1]);
			rec.logs.spectator.push(...ch[0]);
			rec.logs.p1.push(...ch[1]);
			rec.logs.p2.push(...ch[2]);
		} else if (type === 'sideupdate') {
			const nl = data.indexOf('\n');
			const side = data.slice(0, nl);
			const line = data.slice(nl + 1);
			if (line.startsWith('|request|')) {
				rec.requests[side] = JSON.parse(line.slice('|request|'.length));
			} else if (line.startsWith('|error|')) {
				rec.errors[side] = line.slice('|error|'.length);
			} else {
				rec.logs[side].push(line);
			}
		} else if (type === 'end') {
			rec.ended = true;
		}
	};
}

/**
 * Record every draw of the battle's PRNG with a short call stack, as
 * "random(n) = r @ frames" (random(m, n) is written as random(n - m) = r - m,
 * the engine's form).
 */
function traceRng(rec) {
	const prng = rec.battle.prng;
	const orig = prng.random.bind(prng);
	rec.rngTrace = [];
	prng.random = (from, to) => {
		const r = orig(from, to);
		const frames = new Error().stack.split('\n').slice(2)
			.map(l => l.trim().replace(/^at /, ''))
			// The checkout's dist/ in any directory, with / or \ (Windows) separators.
			.filter(l => /[\\/]dist[\\/](sim|data)[\\/]/.test(l) && !/prng\.js/.test(l))
			.slice(0, 5)
			.map(l => l.replace(/\(.*?[\\/]dist[\\/]/, '(').replace(/\\/g, '/').replace(/:\d+\)$/, ')'))
			.join(' < ');
		let call;
		if (from === undefined) call = `random() = ${r}`;
		else if (!to) call = `random(${from}) = ${r}`;
		else call = `random(${to - from}) = ${r - from}`;
		rec.rngTrace.push(`${call} @ ${frames}`);
		return r;
	};
	// Handler lists sorted by speedSort, as "# handlers id@holder/order/priority/speed/subOrder,...".
	const battle = rec.battle;
	const origSpeedSort = battle.speedSort.bind(battle);
	battle.speedSort = (list, comparator) => {
		if (list.length > 1 && list[0] && list[0].effect) {
			const holder = h => (h.effectHolder && h.effectHolder.side ? h.effectHolder.side.id + h.effectHolder.position :
				h.effectHolder && h.effectHolder.id ? h.effectHolder.id : 'field');
			rec.rngTrace.push('# handlers ' + list.map(h => `${h.effect.id}@${holder(h)}/${h.order}/${h.priority}/${h.speed}/${h.subOrder}`).join(','));
		} else if (list.length > 1 && list[0] && list[0].side && list[0].species) {
			// Pokémon sorted by stored speed (runSwitch, eachEvent).
			rec.rngTrace.push('# pokemon ' + list.map(p => `${p.side.id}${p.position}:${p.species.id}/${p.speed}`).join(','));
		}
		return origSpeedSort(list, comparator);
	};
	// Queue contents around every sort, as "# queue <before> -> <after>".
	const queue = rec.battle.queue;
	const origSort = queue.sort.bind(queue);
	const show = () => queue.list.map(a => `${a.choice}:${a.pokemon ? a.pokemon.side.id + a.pokemon.position : ''}` +
		`/${a.order}/${a.priority}/${a.speed}`).join(',');
	queue.sort = () => {
		const before = show();
		const out = origSort();
		rec.rngTrace.push(`# queue ${before} -> ${show()}`);
		return out;
	};
}

function observation(rec, includeOmniscient) {
	const b = rec.battle;
	const out = {
		requests: rec.requests,
		logs: { p1: rec.logs.p1, p2: rec.logs.p2, spectator: rec.logs.spectator },
		errors: rec.errors,
		ended: b.ended,
		winner: b.ended ? (b.winner || null) : null,
		turn: b.turn,
	};
	if (includeOmniscient) out.logs.omniscient = rec.logs.omniscient;
	if (rec.rngTrace) {
		out.rngTrace = rec.rngTrace;
		rec.rngTrace = [];
	}
	// Each request, log line and error is delivered exactly once.
	rec.logs = { p1: [], p2: [], spectator: [], omniscient: [] };
	rec.requests = { p1: null, p2: null };
	rec.errors = { p1: null, p2: null };
	return out;
}

/**
 * Primitive-valued fields of an effect state (durations, layers, counters),
 * without Showdown's bookkeeping: the id, name, effectOrder and target, and
 * the source's slot, which only Leech Seed and Wish read (they keep it).
 */
function prim(state, id) {
	const out = {};
	for (const [k, v] of Object.entries(state || {})) {
		if (['id', 'name', 'effectOrder', 'target'].includes(k)) continue;
		if (k === 'sourceSlot' && id !== 'leechseed' && id !== 'wish' && id !== 'healingwish') continue;
		if (v === null || ['number', 'string', 'boolean'].includes(typeof v)) out[k] = v;
	}
	return out;
}

function effects(table) {
	const out = {};
	for (const id of Object.keys(table || {}).sort()) out[id] = prim(table[id], id);
	return out;
}

/** A Pokémon as the projection names one: its side and its position in the team. */
function mref(p) {
	return `${p.side.id}:${p.position}`;
}

/** Volatiles whose `effectState.source` (a Pokémon) the engine keeps. */
const SOURCED = new Set(['attract', 'octolock', 'syrupbomb', 'lockon']);

/**
 * A Pokémon's volatiles: `prim` plus the references the engine restores.
 * `source` is the Pokémon that applied the volatile; `linked` holds
 * Showdown's `linkedPokemon` (Block, Mean Look: `trapped` names its trapper,
 * `trapper` lists everyone it traps); `sourceEffect` is the move of
 * `partiallytrapped`. A trapper that left the field is no source.
 */
function volatileEffects(table) {
	const out = effects(table);
	for (const id of Object.keys(out)) {
		const state = table[id];
		if (state.source && state.source.side && (SOURCED.has(id) || (id === 'partiallytrapped' && state.source.isActive))) {
			out[id].source = mref(state.source);
		}
		if (Array.isArray(state.linkedPokemon)) out[id].linked = state.linkedPokemon.map(mref).sort();
		if (id === 'partiallytrapped' && state.sourceEffect) out[id].sourceEffect = state.sourceEffect.id;
	}
	return out;
}

/**
 * A slot's conditions: Healing Wish is `detached` once a Swap event moved
 * its state's target from the side to a Pokémon; Future Sight names its
 * `source` and Wish the `wisher` (the source's name).
 */
function slotEffects(table) {
	const out = effects(table);
	for (const id of Object.keys(out)) {
		const state = table[id];
		if (id === 'healingwish' && state.target && state.target.side) out[id].detached = true;
		if (id === 'futuremove' && state.source && state.source.side) out[id].source = mref(state.source);
		if (id === 'wish' && state.source) out[id].wisher = state.source.name;
	}
	return out;
}

/** Pseudo-weathers: Magic Room's end names its source. */
function fieldEffects(table) {
	const out = effects(table);
	for (const id of Object.keys(out)) {
		const state = table[id];
		if (id === 'magicroom' && state.source && state.source.side) out[id].source = mref(state.source);
	}
	return out;
}

/**
 * Volatiles with a residual or update handler, which the engine keeps in
 * insertion order (that order breaks ties between their handlers).
 */
const ORDERED_VOLATILES = new Set([
	'throatchop', 'taunt', 'encore', 'disable', 'yawn', 'roost', 'perishsong', 'healblock', 'flinch',
	'protect', 'spikyshield', 'banefulbunker', 'kingsshield', 'obstruct', 'stall', 'helpinghand', 'followme',
	'ragepowder', 'mustrecharge', 'twoturnmove', 'partiallytrapped', 'leechseed', 'aquaring', 'ingrain',
	'saltcure', 'chillyreception', 'magnetrise', 'uproar', 'gem', 'counter', 'beakblast', 'mirrorcoat',
	'focuspunch', 'syrupbomb', 'fling', 'endure', 'allyswitch', 'curse', 'lockedmove', 'charge', 'electrify',
	'lockon', 'octolock', 'attract',
]);

/** The ordered volatiles' ids in the order they were added (the charging move's own volatile too). */
function volatileOrder(p) {
	// Only a charging move with a duration (Phantom Force) has handlers of its own.
	let charging = p.volatiles.twoturnmove && p.volatiles.twoturnmove.move;
	if (!(p.volatiles[charging] && p.volatiles[charging].duration)) charging = null;
	return Object.keys(p.volatiles).filter(id => ORDERED_VOLATILES.has(id) || id === charging);
}

/** Side conditions with a handler the engine orders (Safeguard has none). */
const ORDERED_SIDE_CONDITIONS = new Set([
	'tailwind', 'reflect', 'lightscreen', 'auroraveil', 'wideguard', 'quickguard', 'stealthrock', 'toxicspikes',
	'spikes', 'stickyweb',
]);

/** Ability state: Cud Chew's berry and Opportunist's boosts (in the order they were first seen) are objects. */
function abilityStateOf(a) {
	const out = prim(a);
	if (a && a.berry && a.berry.id) out.berry = a.berry.id;
	if (a && a.boosts && typeof a.boosts === 'object') out.oppBoosts = Object.entries(a.boosts);
	return out;
}

/** Canonical, engine-agnostic projection of the full battle state. */
function project(battle) {
	const f = battle.field;
	// Ties between abilities (redirection) go to the older ability state: its rank among the active Pokémon.
	const orders = [...new Set(battle.sides.filter(side => side).flatMap(side => side.pokemon.filter(p => p.isActive)
		.map(p => p.abilityState.effectOrder)))].sort((a, b) => a - b);
	return {
		turn: battle.turn,
		ended: battle.ended,
		winner: battle.winner || null,
		prng: battle.prng.getSeed(),
		field: {
			weather: f.weather || null,
			weatherState: prim(f.weatherState),
			terrain: f.terrain || null,
			terrainState: prim(f.terrainState),
			pseudoWeather: fieldEffects(f.pseudoWeather),
		},
		requestState: battle.requestState || null,
		queueLength: battle.queue.list.length,
		// Copycat copies the battle's last move.
		lastMove: battle.lastMove ? battle.lastMove.id : null,
		sides: battle.sides.filter(side => side).map(side => ({
			id: side.id,
			pokemonLeft: side.pokemonLeft,
			totalFainted: side.totalFainted,
			forceSwitch: (side.activeRequest && side.activeRequest.forceSwitch) || null,
			sideConditions: effects(side.sideConditions),
			sideConditionOrder: Object.keys(side.sideConditions).filter(id => ORDERED_SIDE_CONDITIONS.has(id)),
			slotConditions: side.slotConditions.map(slotEffects),
			active: side.active.map(p => (p ? p.set.name || p.species.name : null)),
			pokemon: side.pokemon.map(p => ({
				name: p.set.name,
				species: p.species.name,
				baseSpecies: p.baseSpecies.name,
				set: { nature: p.set.nature || '', evs: p.set.evs || null, level: p.level, gender: p.gender || '', shiny: !!p.set.shiny },
				position: p.position,
				isActive: p.isActive,
				hp: p.hp,
				maxhp: p.maxhp,
				status: p.status || null,
				statusState: prim(p.statusState),
				fainted: p.fainted,
				boosts: { ...p.boosts },
				storedStats: { ...p.storedStats },
				types: p.getTypes(),
				// Forest's Curse, Trick-or-Treat: the type getTypes() ends with.
				addedType: p.addedType || null,
				knownType: !!p.knownType,
				item: p.item || null,
				// Harvest, Pickup and Recycle read it.
				lastItem: p.lastItem || null,
				usedItemThisTurn: !!p.usedItemThisTurn,
				syrupTriggered: !!p.syrupTriggered,
				// Belch reads ateBerry and Rage Fist timesAttacked.
				ateBerry: !!p.ateBerry,
				timesAttacked: p.timesAttacked || 0,
				itemState: prim(p.itemState),
				ability: p.ability || null,
				baseAbility: p.baseAbility || null,
				abilityState: abilityStateOf(p.abilityState),
				abilityOrder: p.isActive ? orders.indexOf(p.abilityState.effectOrder) + 1 : null,
				moves: p.moveSlots.map(m => ({ id: m.id, pp: m.pp, maxpp: m.maxpp, disabled: !!m.disabled, used: !!m.used })),
				volatiles: volatileEffects(p.volatiles),
				volatileOrder: volatileOrder(p),
				lastMove: p.lastMove ? p.lastMove.id : null,
				lastMoveTargetLoc: p.lastMoveTargetLoc || 0,
				moveThisTurnResult: p.moveThisTurnResult ?? null,
				moveLastTurnResult: p.moveLastTurnResult ?? null,
				canMegaEvo: p.canMegaEvo || null,
				activeMoveActions: p.activeMoveActions,
				activeTurns: p.activeTurns,
				illusion: p.illusion ? p.illusion.set.name : null,
				heroMessageDisplayed: !!p.heroMessageDisplayed,
				// Per-turn flags that moves read: Assurance, Alluring Voice, Lash Out.
				hurtThisTurn: p.hurtThisTurn ?? null,
				statsRaisedThisTurn: !!p.statsRaisedThisTurn,
				statsLoweredThisTurn: !!p.statsLoweredThisTurn,
				// Avalanche, Comeuppance: the hits that dealt a damage number.
				attackedBy: p.attackedBy.filter(a => typeof a.damageValue === 'number').map(a => ({
					source: a.source.side.id + ':' + a.source.set.name,
					damage: a.damage, move: a.move, thisTurn: a.thisTurn, slot: a.slot,
				})),
				// Beat Up reads the base Attack of the set's species.
				setSpecies: p.set.species,
				// Transform: the Pokémon's own move slots and stats while it is transformed.
				transformed: p.transformed,
				...(p.transformed ? {
					baseMoveSlots: p.baseMoveSlots.map(m => ({ id: m.id, pp: m.pp, maxpp: m.maxpp, disabled: !!m.disabled, used: !!m.used })),
					baseStoredStats: {
						atk: p.baseStoredStats.atk, def: p.baseStoredStats.def, spa: p.baseStoredStats.spa,
						spd: p.baseStoredStats.spd, spe: p.baseStoredStats.spe,
					},
				} : {}),
			})),
		})),
	};
}

function startBattle(msg) {
	if (battles.has(msg.id)) throw new Error(`battle ${msg.id} already exists`);
	const rec = newRecord(null);
	const battle = new Battle({ formatid: msg.format, seed: msg.seed, send: makeSend(rec) });
	rec.battle = battle;
	battles.set(msg.id, rec);
	if (msg.trace) traceRng(rec);
	battle.setPlayer('p1', { name: msg.p1.name, team: adjustLevels(msg.format, toPackedTeam(msg.p1.team)) });
	battle.setPlayer('p2', { name: msg.p2.name, team: adjustLevels(msg.format, toPackedTeam(msg.p2.team)) });
	if (msg.ots) {
		battle.inputLog.push('>show-openteamsheets');
		battle.showOpenTeamSheets();
	}
	battle.sendUpdates();
	return observation(rec, msg.omniscient);
}

function getRecord(id) {
	const rec = battles.get(id);
	if (!rec) throw new Error(`no battle ${id}`);
	return rec;
}

/** Re-run an input log; optionally record the projection after each input. */
function replay(msg) {
	const lines = msg.inputlog;
	let battle = null;
	const states = [];
	const rec = newRecord(null);
	for (const line of lines) {
		const [type, rest] = splitFirst(line.slice(1), ' ');
		if (type === 'start') {
			const options = JSON.parse(rest);
			options.send = makeSend(rec);
			battle = new Battle(options);
			rec.battle = battle;
		} else if (type === 'player') {
			const [slot, opts] = splitFirst(rest, ' ');
			battle.setPlayer(slot, JSON.parse(opts));
		} else if (type === 'show-openteamsheets') {
			battle.showOpenTeamSheets();
		} else if (type === 'p1' || type === 'p2') {
			battle.choose(type, rest);
		} else if (type === 'forcelose') {
			battle.lose(rest);
		} else if (type === 'version' || type === 'version-origin' || type === 'chat') {
			continue;
		} else {
			throw new Error(`unsupported input line: ${line}`);
		}
		if (msg.states && battle) states.push(project(battle));
	}
	battle.sendUpdates();
	return {
		ended: battle.ended,
		winner: battle.winner || null,
		turn: battle.turn,
		final: project(battle),
		states: msg.states ? states : undefined,
		spectator: msg.logs ? rec.logs.spectator : undefined,
	};
}

/** Re-roll the next turn from the current state under different seeds. */
function branch(msg) {
	const rec = getRecord(msg.id);
	// Deserialize from a string every time: Battle.fromJSON mutates the
	// object it is given, and reusing one object breaks later copies.
	const snapshot = JSON.stringify(rec.battle.toJSON());
	const results = [];
	for (const seed of msg.seeds) {
		const copy = Battle.fromJSON(snapshot);
		const copyRec = newRecord(copy);
		copy.send = makeSend(copyRec);
		copy.prng = new PRNG(seed);
		let accepted = true;
		for (const side of ['p1', 'p2']) {
			if (msg.choices[side]) accepted = copy.choose(side, msg.choices[side]) && accepted;
		}
		copy.sendUpdates();
		results.push({
			seed,
			accepted,
			errors: copyRec.errors,
			state: project(copy),
			spectator: msg.logs ? copyRec.logs.spectator : undefined,
		});
	}
	return { results };
}

function splitFirst(str, sep) {
	const i = str.indexOf(sep);
	return i < 0 ? [str, ''] : [str.slice(0, i), str.slice(i + sep.length)];
}

function handle(msg) {
	switch (msg.cmd) {
	case 'start':
		return startBattle(msg);
	case 'choose': {
		const rec = getRecord(msg.id);
		const ok = rec.battle.choose(msg.side, msg.choice);
		rec.battle.sendUpdates();
		const out = observation(rec, msg.omniscient);
		out.accepted = ok;
		return out;
	}
	case 'state':
		return project(getRecord(msg.id).battle);
	case 'branch':
		return branch(msg);
	case 'inputlog':
		return { inputlog: getRecord(msg.id).battle.inputLog.slice() };
	case 'replay':
		return replay(msg);
	case 'close':
		battles.delete(msg.id);
		return { closed: msg.id };
	case 'validate': {
		const validator = TeamValidator.get(msg.format);
		const sets = Teams.unpack(toPackedTeam(msg.team));
		return { problems: validator.validateTeam(sets) };
	}
	case 'pack':
		return { packed: toPackedTeam(msg.team) };
	case 'reseed': {
		// Replace the live battle's RNG (for tests comparing live play with branch).
		const rec = getRecord(msg.id);
		rec.battle.prng = new PRNG(msg.seed);
		return { seed: msg.seed };
	}
	case 'version':
		return { showdown: PS };
	default:
		throw new Error(`unknown command ${msg.cmd}`);
	}
}

const rl = readline.createInterface({ input: process.stdin, terminal: false });
rl.on('line', line => {
	if (!line.trim()) return;
	let reply;
	try {
		const msg = JSON.parse(line);
		reply = { ok: true, result: handle(msg) };
	} catch (err) {
		reply = { ok: false, error: String(err && err.stack || err) };
	}
	process.stdout.write(JSON.stringify(reply) + '\n');
});
