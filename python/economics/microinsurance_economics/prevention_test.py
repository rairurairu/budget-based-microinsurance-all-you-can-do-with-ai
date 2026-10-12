"""reward_cost < claim_probability_reduction × average_claim_cost  (R18)

Example: 4% → 2.5% monthly screen claims on a HK$800 repair saves HK$12 against a HK$3 reward.
Admission decisions use the engine (`passes_engine`, via `microinsurance_py`), so the model and
the live system cannot disagree; the float versions are for exploration only.

Per customer (R40): the claim probabilities scale with the customer's rated relativity, the claim
cost is at most their sum insured (first loss, R48), and the reward is
`reward_share_of_claims_avoided` of what is avoided (`reward_for_customer`, through the engine).
"""
from __future__ import annotations

from typing import Protocol

import microinsurance_py


class RewardEngine(Protocol):
    def prevention_reward_cents(
        self,
        p_without_bps: int,
        p_with_bps: int,
        avg_claim_cost_cents: int,
        relativity_bps: int,
        sum_insured_cents: int,
    ) -> tuple[int, int]: ...


def claims_avoided(p_before: float, p_after: float, avg_claim_cost: float) -> float:
    return (p_before - p_after) * avg_claim_cost


def passes(reward_cost: float, p_before: float, p_after: float, avg_claim_cost: float) -> bool:
    return reward_cost < claims_avoided(p_before, p_after, avg_claim_cost)


def passes_engine(
    reward_cost: float, p_before: float, p_after: float, avg_claim_cost: float
) -> bool:
    """The engine's decision: probabilities to basis points, HK$ to cents, integer arithmetic."""
    return microinsurance_py.prevention_test_passes(
        round(reward_cost * 100),
        round(p_before * 10_000),
        round(p_after * 10_000),
        round(avg_claim_cost * 100),
    )


def reward_for_customer(
    engine: RewardEngine,
    p_without: float,
    p_with: float,
    avg_claim_cost: float,
    relativity: float,
    sum_insured: float,
) -> tuple[float, float]:
    """R40: (reward, claims avoided) in HK$ for one customer, decided by the engine."""
    reward, avoided = engine.prevention_reward_cents(
        round(p_without * 10_000),
        round(p_with * 10_000),
        round(avg_claim_cost * 100),
        round(relativity * 10_000),
        round(sum_insured * 100),
    )
    return reward / 100, avoided / 100
