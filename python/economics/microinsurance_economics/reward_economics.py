"""Partner profit per coupon issued.

profit = redeemed × (new_visit_share × margin × spend − face_value)
break-even new-visit share = face_value / (margin × spend)
"""
from __future__ import annotations


def profit_per_coupon(face_value: float, spend: float, margin: float, redeemed: float,
                      new_visit_share: float) -> float:
    return redeemed * (new_visit_share * margin * spend - face_value)


def break_even_new_visit_share(face_value: float, spend: float, margin: float) -> float:
    return face_value / (margin * spend)


def from_gamed_missions(profit_per_1000: float, gamed_share: float) -> float:
    """Profit the partner still makes from gamed prevention tasks: a gamed prevention task still brings a visit."""
    return profit_per_1000 * gamed_share
