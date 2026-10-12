"""Parametric pricing on a data source's historical record (R31, R32, R55).

The record is replayed through the production matcher (PyO3 `Engine.payout_by_month_ppm`), so
declared hours (R08), stages, minutes in force and the payout basis count exactly as they will
in a live payout. For one declared work pattern:

    expected cost per calendar month = what the record would have paid in that month,
                                       averaged over the record's years (ppm of benefit amount)
    rate = expected cost / target loss ratio            (Engine.rate_ppm, R31)

The monthly costs feed `RatingBasis.expected_cost`, where the engine applies the rating horizon
(R32). Money stays in the engine's integer units: ppm of the benefit amount.
"""

from __future__ import annotations

import csv
import json
from collections.abc import Iterable
from datetime import date, datetime
from pathlib import Path
from typing import Protocol


class PricingEngine(Protocol):
    def payout_by_month_ppm(
        self, reports_json: str, cover_id: str, work_windows_json: str
    ) -> dict[str, int]: ...
    def rate_ppm(self, base_cost_ppm: int, relativity_bps: int) -> int: ...


def load_history(path: Path, source: str) -> list[dict[str, str]]:
    """A warnings-database export (`code,issued,cancelled`, local times with offset) as
    `SourceReport` JSON objects. Lines starting with `#` are comments."""
    rows = [
        line
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.startswith("#")
    ]
    reports = []
    for row in csv.DictReader(rows):
        issued = datetime.fromisoformat(row["issued"])
        cancelled = datetime.fromisoformat(row["cancelled"])
        reports.append(
            {
                "report_id": f"{row['code']}:{row['issued']}",
                "source": source,
                "code": row["code"],
                "starts": issued.isoformat(),
                "ends": cancelled.isoformat(),
            }
        )
    return reports


def work_week(weekdays: Iterable[int], start_minute: int, end_minute: int) -> list[dict[str, int]]:
    """Declared hours as `WorkWindow` JSON objects (ISO weekdays, minutes from local midnight)."""
    return [
        {"weekday": d, "start_minute": start_minute, "end_minute": end_minute} for d in weekdays
    ]


def monthly_cost_ppm(
    engine: PricingEngine,
    reports: list[dict[str, str]],
    cover_id: str,
    work_windows: list[dict[str, int]],
    first_year: int,
    last_year: int,
) -> dict[int, int]:
    """Expected cost by calendar month (1-12), ppm of the benefit amount: the record's payouts
    in that month over `first_year..last_year`, divided by the number of years, rounded to the
    nearest ppm."""
    years = last_year - first_year + 1
    if years <= 0:
        raise ValueError("the record needs at least one year")
    paid = engine.payout_by_month_ppm(json.dumps(reports), cover_id, json.dumps(work_windows))
    totals = dict.fromkeys(range(1, 13), 0)
    for month, ppm in paid.items():
        year, m = (int(x) for x in month.split("-"))
        if first_year <= year <= last_year:
            totals[m] += ppm
    return {m: (total + years // 2) // years for m, total in totals.items()}


def expected_cost_curve(costs: dict[int, int], start: date, months: int) -> dict[str, int]:
    """A `RatingBasis.expected_cost` curve: each policy period from `start` (a first of the
    month) gets its calendar month's expected cost."""
    curve = {}
    for i in range(months):
        y, m = divmod(start.month - 1 + i, 12)
        curve[date(start.year + y, m + 1, 1).isoformat()] = costs[m + 1]
    return curve


def rate_ppm(engine: PricingEngine, cost_ppm: int, relativity_bps: int = 10_000) -> int:
    """R31: premium per unit of benefit amount, from the engine's integer rating."""
    return engine.rate_ppm(cost_ppm, relativity_bps)
