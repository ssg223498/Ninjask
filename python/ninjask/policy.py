"""What the game loops ask for each choice: a policy, plus a random one for checks."""

from __future__ import annotations

import random
from typing import Protocol

from ninjask.choices import joint_choices, team_preview_choices


class Policy(Protocol):
    def choose(self, request: dict, log: list[str]) -> str:
        """Return a choice string for ``request``.

        ``log`` holds the protocol lines this player has seen since the
        previous call. When a choice is rejected, the policy is asked again
        (with the updated request, if there is one) and ``log`` holds only
        lines that arrived in between, usually none.
        """


class RandomPolicy:
    """A uniformly random legal choice. Never targets its own partner."""

    def __init__(self, seed: int = 0, game_type: str | None = "doubles") -> None:
        # The game loops this serves play doubles; naming the game type
        # keeps move targets enumerated in a one-active endgame, whose
        # request reads as singles (ninjask.choices.slot_options).
        self.rng = random.Random(seed)
        self.game_type = game_type

    def choose(self, request: dict, log: list[str]) -> str:
        if request.get("teamPreview"):
            return self.rng.choice(team_preview_choices(request, self.game_type))
        return self.rng.choice(joint_choices(request, game_type=self.game_type))
