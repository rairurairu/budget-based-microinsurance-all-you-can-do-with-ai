"""Evaluate the pilot (docs/07-pilot-and-metrics.md, ADR-012) against gates fixed before it started.

    a gate passes when its metric is measured and within its bound
    stop    when any `stop` gate is breached (measured and outside its bound)
    scale   when every `scale` gate passes and no `stop` gate is breached
    hold    otherwise: a metric not measured yet, or a `scale` gate not passed

A metric not measured never passes: a gate cannot be met by missing data. Metrics come from the
book (`GET /ops/pilot`: rates in basis points, hours as hours) and from outside it (a satisfaction
survey, the partner's incremental revenue, the insurer value equation), passed in in the gates'
own terms (a rate as a fraction).

    python -m microinsurance_economics.pilot_evaluation report.json [external.json]

prints the evaluation of the API's report, with any external metrics, as JSON.
"""

from __future__ import annotations

import json
import sys
from collections.abc import Mapping, Sequence
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Literal

Kind = Literal["scale", "stop"]
Status = Literal["pass", "breach", "not_measured"]
Decision = Literal["scale", "hold", "stop"]

BPS = 10_000


@dataclass(frozen=True)
class Gate:
    metric: str
    kind: Kind
    at_least: float | None = None
    at_most: float | None = None


@dataclass(frozen=True)
class GateResult:
    metric: str
    kind: Kind
    value: float | None
    status: Status


@dataclass(frozen=True)
class Evaluation:
    decision: Decision
    gates: list[GateResult]


def check(gate: Gate, value: float | None) -> Status:
    if value is None:
        return "not_measured"
    if gate.at_least is not None and value < gate.at_least:
        return "breach"
    if gate.at_most is not None and value > gate.at_most:
        return "breach"
    return "pass"


def evaluate(metrics: Mapping[str, float | None], gates: Sequence[Gate]) -> Evaluation:
    """Pass, breach or not measured per gate, and the pilot's decision."""
    results = [GateResult(g.metric, g.kind, metrics.get(g.metric), "pass") for g in gates]
    results = [
        GateResult(r.metric, r.kind, r.value, check(g, r.value))
        for g, r in zip(gates, results, strict=True)
    ]
    if any(r.kind == "stop" and r.status == "breach" for r in results):
        decision: Decision = "stop"
    elif all(r.status == "pass" for r in results if r.kind == "scale"):
        decision = "scale"
    else:
        decision = "hold"
    return Evaluation(decision, results)


def metrics_from_report(report: Mapping[str, object]) -> dict[str, float | None]:
    """The API's pilot metrics in the gates' terms: basis points as fractions."""
    out: dict[str, float | None] = {}
    rows = report.get("metrics", [])
    assert isinstance(rows, list)
    for row in rows:
        assert isinstance(row, dict)
        value = row.get("value")
        unit = row.get("unit")
        name = str(row.get("metric"))
        if value is None:
            out[name] = None
        elif unit == "bps":
            out[name] = float(value) / BPS
        else:
            out[name] = float(value)
    return out


def gates_from_report(report: Mapping[str, object]) -> list[Gate]:
    rows = report.get("gates", [])
    assert isinstance(rows, list)
    gates: list[Gate] = []
    for row in rows:
        assert isinstance(row, dict)
        kind = row["kind"]
        assert kind in ("scale", "stop")
        gates.append(
            Gate(
                metric=str(row["metric"]),
                kind=kind,
                at_least=None if row.get("at_least") is None else float(row["at_least"]),
                at_most=None if row.get("at_most") is None else float(row["at_most"]),
            )
        )
    return gates


def evaluate_report(
    report: Mapping[str, object], external: Mapping[str, float | None] | None = None
) -> Evaluation:
    """The report's metrics, with metrics from outside the book, against its gates."""
    metrics = metrics_from_report(report)
    metrics.update(external or {})
    return evaluate(metrics, gates_from_report(report))


def main(argv: Sequence[str]) -> int:
    if len(argv) < 2:
        print(__doc__)
        return 2
    report = json.loads(Path(argv[1]).read_text(encoding="utf-8"))
    external = json.loads(Path(argv[2]).read_text(encoding="utf-8")) if len(argv) > 2 else {}
    print(json.dumps(asdict(evaluate_report(report, external)), indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
