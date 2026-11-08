"""Ninjask depends only on what pyproject.toml declares.

Nothing in the checkout (the `ninjask` package, the scripts and the tests)
imports a third-party package other than NumPy, SciPy and pytest, so a fresh
install has everything it needs; and nothing reads a `data/` directory
directly: generated data is found through `ninjask.paths`.
"""

from __future__ import annotations

import ast
import sys
from pathlib import Path

# tests/python/test_imports.py -> the checkout's root
ROOT = Path(__file__).resolve().parents[2]
ALLOWED = {"ninjask", "numpy", "scipy", "pytest"} | set(sys.stdlib_module_names)


def _imports(path: Path) -> list[str]:
    names: list[str] = []
    for node in ast.walk(ast.parse(path.read_text(encoding="utf-8"))):
        if isinstance(node, ast.Import):
            names += [a.name for a in node.names]
        elif isinstance(node, ast.ImportFrom) and node.level == 0 and node.module:
            names.append(node.module)
    return names


def _sources() -> list[Path]:
    skip = {"target", "__pycache__", ".venv", "third_party", "data"}
    files = [p for p in ROOT.rglob("*.py") if not skip & set(p.relative_to(ROOT).parts)]
    assert ROOT / "python" / "ninjask" / "__init__.py" in files
    return files


def test_imports_only_declared_dependencies() -> None:
    bad = [
        f"{p.relative_to(ROOT)}: {name}"
        for p in _sources()
        for name in _imports(p)
        if name.split(".")[0] not in ALLOWED
    ]
    assert not bad, "\n".join(bad)


def test_does_not_read_data_directory() -> None:
    quoted = [q + "da" + "ta/" for q in "\"'"]  # built so this file doesn't match itself
    bad = [str(p.relative_to(ROOT)) for p in _sources() if any(q in p.read_text(encoding="utf-8") for q in quoted)]
    assert not bad, "\n".join(bad)
