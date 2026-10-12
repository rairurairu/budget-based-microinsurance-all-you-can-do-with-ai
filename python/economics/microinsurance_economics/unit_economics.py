"""Per-subscriber monthly P&L. Defaults are illustrative (docs/03-economic-models.md §1)."""
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class UnitInputs:
    premium: float = 80
    loss_ratio: float = 0.55
    prevention_rewards: float = 5
    commission_rate: float = 0.15
    ops_cost: float = 4


@dataclass(frozen=True)
class UnitPnL:
    premium: float
    expected_claims: float
    prevention_rewards: float
    partner_commission: float
    ops_cost: float
    remainder: float
    """Insurer margin + platform fee, shared under the partnership agreement."""


def pnl(i: UnitInputs) -> UnitPnL:
    expected_claims = i.premium * i.loss_ratio
    commission = i.premium * i.commission_rate
    return UnitPnL(
        premium=i.premium,
        expected_claims=expected_claims,
        prevention_rewards=i.prevention_rewards,
        partner_commission=commission,
        ops_cost=i.ops_cost,
        remainder=i.premium - expected_claims - commission - i.prevention_rewards - i.ops_cost,
    )


def remainder(i: UnitInputs) -> float:
    """Insurer margin + platform fee."""
    return pnl(i).remainder
