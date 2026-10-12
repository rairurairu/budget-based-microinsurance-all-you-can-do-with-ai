"""Run golden cases through the AI layer exactly as production does.

Cases are YAML files in ``golden/<component>/``. They go, as JSON lines, to the ``ai-eval``
binary (``crates/ai/src/bin/ai_eval.rs``), which runs each through its component: harness,
fixed checks and saved fallback. By default the model is the saved replies in
``fixtures/ai_replies/`` and nothing leaves the machine; with ``EVAL_LIVE=1`` it is the
configured provider.
"""

from __future__ import annotations

import json
import os
import subprocess
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, cast

import yaml

ROOT = Path(__file__).resolve().parents[3]
GOLDEN = Path(__file__).resolve().parents[1] / "golden"
REPLIES = ROOT / "fixtures" / "ai_replies"

Json = dict[str, Any]


@dataclass
class Case:
    """One golden case: the input, the rules it protects and the expected properties."""

    case: str
    component: str
    rules: list[str]
    input: Json
    expect: Json
    # A harness case: its saved reply is deliberately bad, to test the fallback. Skipped live.
    harness_only: bool = False
    path: Path = field(default_factory=Path)
    # A market overlay (config/markets/<market>, ADR-012) the case runs in; none: the base market.
    market: str | None = None


@dataclass
class Outcome:
    """What the component returned for a case, and what the model was sent."""

    case: Case
    value: Any
    source: str | None
    reason: str | None
    sent: str | None
    key: str | None
    reply: str | None
    error: str | None


def live() -> bool:
    return os.environ.get("EVAL_LIVE") == "1"


def load_cases(components: list[str] | None = None) -> list[Case]:
    cases: list[Case] = []
    for path in sorted(GOLDEN.glob("*/*.yaml")):
        component = path.parent.name
        if components is not None and component not in components:
            continue
        raw = cast(Json, yaml.safe_load(path.read_text(encoding="utf-8")))
        cases.append(
            Case(
                case=path.stem,
                component=component,
                rules=[str(r) for r in cast(list[Any], raw.get("rules", []))],
                input=cast(Json, raw["input"]),
                expect=cast(Json, raw.get("expect", {})),
                harness_only=bool(raw.get("harness_only", False)),
                path=path,
                market=cast(str | None, raw.get("market")),
            )
        )
    return cases


def run(cases: list[Case]) -> list[Outcome]:
    """Runs the cases, one process per market, and returns the outcomes in case order."""
    markets = sorted({c.market or "" for c in cases})
    by_market = {m: _run([c for c in cases if (c.market or "") == m], m or None) for m in markets}
    by_case = {o.case.case: o for outcomes in by_market.values() for o in outcomes}
    return [by_case[c.case] for c in cases if c.case in by_case]


def _run(cases: list[Case], market: str | None) -> list[Outcome]:
    """Runs every case of one market in one process."""
    lines = "".join(
        json.dumps({"case": c.case, "component": c.component, "input": c.input}) + "\n"
        for c in cases
    )
    env = dict(os.environ, REPO_ROOT=str(ROOT))
    env.pop("MARKET", None)
    if market is not None:
        env["MARKET"] = market
    done = subprocess.run(
        ["cargo", "run", "-q", "-p", "microinsurance-ai", "--bin", "ai-eval"],
        input=lines,
        capture_output=True,
        text=True,
        encoding="utf-8",
        cwd=ROOT,
        env=env,
        check=False,
    )
    if done.returncode != 0:
        raise RuntimeError(f"ai-eval failed:\n{done.stderr}")
    by_case = {c.case: c for c in cases}
    outcomes: list[Outcome] = []
    for line in done.stdout.splitlines():
        row = cast(Json, json.loads(line))
        result = cast(Json, row.get("result") or {})
        request = cast(Json, row.get("request") or {})
        outcomes.append(
            Outcome(
                case=by_case[str(row["case"])],
                value=result.get("value"),
                source=cast(str | None, result.get("source")),
                reason=cast(str | None, result.get("reason")),
                sent=cast(str | None, request.get("user")),
                key=cast(str | None, request.get("key")),
                reply=cast(str | None, row.get("reply")),
                error=cast(str | None, row.get("error")),
            )
        )
    return outcomes
