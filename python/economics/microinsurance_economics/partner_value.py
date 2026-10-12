"""Value to a distribution or reward partner (docs/03-economic-models.md §7), per period:

    value = incremental_engagement + conversion + retention + use_of_low_marginal_cost_capacity

Each term is the partner's own money for the period, measured against its baseline (the
partner's data, not the book): the margin on extra engagement, on customers converted through
the journey and on customers retained, and the value its spare capacity earns as rewards (the
price paid on supplier statements above its marginal cost, R56). The pilot's
`partner_incremental_revenue` gate reads this value.
"""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class PartnerInputs:
    incremental_engagement: float
    conversion: float
    retention: float
    spare_capacity_value: float


def value(
    incremental_engagement: float,
    conversion: float,
    retention: float,
    spare_capacity_value: float,
) -> float:
    return incremental_engagement + conversion + retention + spare_capacity_value


def spare_capacity_value(
    deliveries: int, price_per_delivery: float, marginal_cost_per_delivery: float
) -> float:
    """What spare capacity earns as rewards: the price the insurer pays per delivery on the
    supplier's statement (R56), less what the delivery costs the partner at the margin."""
    return deliveries * (price_per_delivery - marginal_cost_per_delivery)


def of(i: PartnerInputs) -> float:
    return value(i.incremental_engagement, i.conversion, i.retention, i.spare_capacity_value)
