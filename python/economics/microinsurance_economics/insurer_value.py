"""value = claims_avoided + retention_value + new_customer_value
         − reward_cost − verification_cost − platform_cost"""
from __future__ import annotations


def value(claims_avoided: float, retention_value: float, new_customer_value: float,
          reward_cost: float, verification_cost: float, platform_cost: float) -> float:
    return (claims_avoided + retention_value + new_customer_value
            - reward_cost - verification_cost - platform_cost)
