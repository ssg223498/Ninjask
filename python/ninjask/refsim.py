"""Python client for refsim/server.js, the pinned Pokémon Showdown simulator."""

from __future__ import annotations

import hashlib
import itertools
import json
import subprocess
from dataclasses import dataclass, field
from functools import lru_cache

from ninjask.format import FormatSpec, load as load_format
from ninjask.paths import REFSIM_DIR, SHOWDOWN_DIR, node_env, require_checkout

SERVER = REFSIM_DIR / "server.js"


@lru_cache(maxsize=8)
def _spec(format_id: str) -> FormatSpec:
    return load_format(format_id)


class RefSimError(RuntimeError):
    pass


def seed_from_int(n: int, salt: str = "ninjask") -> str:
    """A Showdown 'sodium' PRNG seed derived deterministically from an integer."""
    return "sodium," + hashlib.sha256(f"{salt}:{n}".encode()).hexdigest()


class RefSim:
    """One Node process running the reference simulator. Not thread-safe."""

    def __init__(self) -> None:
        require_checkout("the reference simulator")
        if not (SHOWDOWN_DIR / "dist").exists():
            raise RefSimError(f"Pokémon Showdown is not built at {SHOWDOWN_DIR}; run scripts/setup_showdown.sh")
        self.proc = subprocess.Popen(
            ["node", str(SERVER)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
            encoding="utf-8",  # Node speaks UTF-8, whatever the locale (Windows: cp932, cp1252)
            bufsize=1,
            env=node_env(),
        )
        self._ids = itertools.count()

    def call(self, **msg) -> dict:
        assert self.proc.stdin and self.proc.stdout
        self.proc.stdin.write(json.dumps(msg) + "\n")
        self.proc.stdin.flush()
        line = self.proc.stdout.readline()
        if not line:
            raise RefSimError("reference simulator exited")
        reply = json.loads(line)
        if not reply["ok"]:
            raise RefSimError(reply["error"])
        return reply["result"]

    def close(self) -> None:
        if self.proc.poll() is None:
            self.proc.stdin.close()
            self.proc.wait(timeout=10)

    def __enter__(self) -> RefSim:
        return self

    def __exit__(self, *exc) -> None:
        self.close()

    def new_battle_id(self) -> str:
        return f"b{next(self._ids)}"

    def validate(self, format_id: str, team: str) -> list[str] | None:
        return self.call(cmd="validate", format=_spec(format_id).sim_format_id, team=team)["problems"]

    def pack(self, team: str) -> str:
        return self.call(cmd="pack", team=team)["packed"]


@dataclass
class Observation:
    """What the simulator sent since the previous call."""

    requests: dict[str, dict | None]
    logs: dict[str, list[str]]
    errors: dict[str, str | None]
    ended: bool
    winner: str | None  # player name
    turn: int
    accepted: bool | None = None
    # PRNG draws since the previous call, when the battle traces them.
    rng_trace: list[str] | None = None

    @classmethod
    def from_reply(cls, r: dict) -> Observation:
        return cls(
            requests=r["requests"],
            logs=r["logs"],
            errors=r["errors"],
            ended=r["ended"],
            winner=r["winner"],
            turn=r["turn"],
            accepted=r.get("accepted"),
            rng_trace=r.get("rngTrace"),
        )


@dataclass
class Battle:
    """A battle in the reference simulator, driven one choice at a time."""

    sim: RefSim
    format_id: str
    seed: str
    p1: tuple[str, str]  # (name, team)
    p2: tuple[str, str]
    ots: bool = False
    id: str = field(default="")

    def start(self, omniscient: bool = False, trace: bool = False) -> Observation:
        """Start the battle; `omniscient` also returns the full log
        (logs["omniscient"]), `trace` records every PRNG draw (rng_trace).

        The battle is created under the format's simulator id
        (`FormatSpec.sim_format_id`); a format that forces open team
        sheets opens them here whether or not `ots` was passed, so
        selecting such a format is enough."""
        self.id = self.id or self.sim.new_battle_id()
        spec = _spec(self.format_id)
        r = self.sim.call(
            cmd="start",
            id=self.id,
            format=spec.sim_format_id,
            seed=self.seed,
            p1={"name": self.p1[0], "team": self.p1[1]},
            p2={"name": self.p2[0], "team": self.p2[1]},
            ots=self.ots or spec.open_team_sheets,
            omniscient=omniscient,
            trace=trace,
        )
        return Observation.from_reply(r)

    def choose(self, side: str, choice: str, omniscient: bool = False) -> Observation:
        r = self.sim.call(cmd="choose", id=self.id, side=side, choice=choice, omniscient=omniscient)
        return Observation.from_reply(r)

    def state(self) -> dict:
        return self.sim.call(cmd="state", id=self.id)

    def inputlog(self) -> list[str]:
        return self.sim.call(cmd="inputlog", id=self.id)["inputlog"]

    def close(self) -> None:
        self.sim.call(cmd="close", id=self.id)
