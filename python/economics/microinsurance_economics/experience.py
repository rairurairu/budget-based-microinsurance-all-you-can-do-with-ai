"""Indications from an experience study (ADR-012) for the actuary: the data loop's last step.

The study (`GET /ops/experience`, `learning::experience`) gives, per class of business and policy
period, the incurred claims against the expected cost of the rating basis in force, and per
prevention measure the claim frequency with and without a verified activity. Here:

    A/E(class)          = Σ incurred / Σ expected, over the study's periods
    Z(class)            = min(1, sqrt(claims / full_credibility_claims))   limited fluctuation
    indicated factor    = Z × A/E + (1 − Z) × 1
    proposed cost(cover, period) = basis cost × indicated factor, for the class's covers

    p_without, p_with (measure) = Z × observed + (1 − Z) × basis, in basis points

Nothing here changes a rate or a reward: the proposal is for the actuary to review and file as the
next rating basis and prevention basis (R18, R31, R43). Experience from a period of known shocks
(a typhoon season) is credible only to the extent its claim count allows, which is the point of Z.
"""

from __future__ import annotations

import json
import math
import sys
from collections.abc import Mapping, Sequence
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any

BPS = 10_000


def credibility(claims: int, full: int) -> float:
    return min(1.0, math.sqrt(claims / max(full, 1)))


@dataclass(frozen=True)
class ClassIndication:
    class_of_business: str
    periods: int
    claims: int
    actual_to_expected: float | None
    credibility: float
    indicated_factor: float


def class_indications(study: Mapping[str, Any]) -> list[ClassIndication]:
    """One indication per class, over every period in the study."""
    full = int(study.get("full_credibility_claims", 1_082))
    by_class: dict[str, list[Mapping[str, Any]]] = {}
    for row in study.get("classes", []):
        by_class.setdefault(str(row["class_of_business"]), []).append(row)
    out: list[ClassIndication] = []
    for cls, rows in sorted(by_class.items()):
        expected = sum(int(r["expected_minor"]) for r in rows)
        incurred = sum(int(r["incurred_minor"]) for r in rows)
        claims = sum(int(r["claims"]) for r in rows)
        a_e = incurred / expected if expected > 0 else None
        z = credibility(claims, full)
        factor = 1.0 if a_e is None else z * a_e + (1 - z) * 1.0
        out.append(ClassIndication(cls, len(rows), claims, a_e, z, factor))
    return out


def propose_rating_basis(
    basis: Mapping[str, Any],
    study: Mapping[str, Any],
    class_of_cover: Mapping[str, str],
    from_period: str,
) -> dict[str, Any]:
    """The rating basis with each class's indicated factor applied to its covers' expected costs
    from `from_period` (`YYYY-MM-DD`) on, in ppm, rounded to the unit. Entries before stay."""
    factors = {i.class_of_business: i.indicated_factor for i in class_indications(study)}
    proposed: dict[str, Any] = json.loads(json.dumps(basis))
    for cover, curve in proposed.get("expected_cost", {}).items():
        factor = factors.get(class_of_cover.get(cover, ""))
        if factor is None:
            continue
        for start, cost in curve.items():
            if start >= from_period:
                curve[start] = max(0, round(cost * factor))
    return proposed


@dataclass(frozen=True)
class MeasureIndication:
    measure: str
    credibility: float
    claim_prob_without: int
    claim_prob_with: int
    observed_without: int | None
    observed_with: int | None


def prevention_indications(
    study: Mapping[str, Any], prevention_basis: Mapping[str, Any]
) -> list[MeasureIndication]:
    """Each measure's claim probabilities (basis points per policy period), credibility-weighted
    between what the book shows and the prevention basis in force."""
    measures = prevention_basis.get("measures", {})
    out: list[MeasureIndication] = []
    for row in study.get("prevention", []):
        name = str(row["measure"])
        basis = measures.get(name)
        if basis is None:
            continue
        z = float(row["credibility_bps"]) / BPS
        observed_with = row.get("frequency_with_bps")
        observed_without = row.get("frequency_without_bps")

        def blend(observed: int | None, prior: int, z: float = z) -> int:
            return prior if observed is None else round(z * observed + (1 - z) * prior)

        out.append(
            MeasureIndication(
                measure=name,
                credibility=z,
                claim_prob_without=blend(observed_without, int(basis["claim_prob_without"])),
                claim_prob_with=blend(observed_with, int(basis["claim_prob_with"])),
                observed_without=observed_without,
                observed_with=observed_with,
            )
        )
    return out


def main(argv: Sequence[str]) -> int:
    if len(argv) < 2:
        print(__doc__)
        return 2
    study = json.loads(Path(argv[1]).read_text(encoding="utf-8"))
    print(json.dumps([asdict(i) for i in class_indications(study)], indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
