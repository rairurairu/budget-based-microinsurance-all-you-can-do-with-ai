import json
from datetime import date
from pathlib import Path

import microinsurance_py
from microinsurance_economics import (
    prevention_test,
    reward_economics,
    trigger_pricing,
    unit_economics,
)

CONFIG = Path(__file__).resolve().parents[3] / "config"


def test_r18_screen_example_passes():
    # 4% -> 2.5% on HK$800 saves HK$12 against a HK$3 reward
    assert round(prevention_test.claims_avoided(0.04, 0.025, 800), 2) == 12.0
    assert prevention_test.passes(3, 0.04, 0.025, 800)
    assert prevention_test.passes_engine(3, 0.04, 0.025, 800)
    assert microinsurance_py.claims_avoided_cents(400, 250, 80_000) == 1_200


def test_r18_model_and_engine_agree():
    for reward in (0.5, 3, 11.99, 12, 12.01, 30):
        for p_before, p_after in ((0.04, 0.025), (0.10, 0.02), (0.03, 0.03)):
            model = prevention_test.passes(reward, p_before, p_after, 800)
            assert prevention_test.passes_engine(reward, p_before, p_after, 800) == model


def test_r29_engine_rounds_half_up():
    assert microinsurance_py.mul_bps(5, 5_000) == 3
    assert microinsurance_py.mul_bps(-5, 5_000) == -3


def test_r32_horizon_average_shows_a_coming_peak():
    assert microinsurance_py.horizon_cost_ppm([10_000, 40_000, 10_000]) == (10_000, 20_000)


def test_reward_break_even():
    assert reward_economics.break_even_new_visit_share(5, 60, 0.25) == 5 / 15


def test_unit_economics_default_remainder():
    assert round(unit_economics.remainder(unit_economics.UnitInputs()), 2) == 15.0
    p = unit_economics.pnl(unit_economics.UnitInputs())
    assert (round(p.expected_claims), round(p.partner_commission)) == (44, 12)


def _engine() -> microinsurance_py.Engine:
    def texts(folder: str) -> list[str]:
        return [p.read_text(encoding="utf-8") for p in sorted((CONFIG / folder).glob("*.yaml"))]

    def text(name: str) -> str:
        return (CONFIG / name).read_text(encoding="utf-8")

    return microinsurance_py.Engine(
        text("insurer.example.yaml"),
        text("lines_of_business.yaml"),
        text("rating_factors.yaml"),
        text("event_types.yaml"),
        texts("triggers"),
        text("exposure_types.yaml"),
        text("data_types.yaml"),
        texts("catalogue"),
        text("prevention_measures.yaml"),
        text("suppliers.yaml"),
    )


def test_r40_reward_follows_claims_avoided_through_the_engine():
    engine = _engine()
    # The screen example at an average rating: HK$12 avoided, a quarter of it as the reward.
    assert prevention_test.reward_for_customer(engine, 0.04, 0.025, 800, 1.0, 8_000) == (3.0, 12.0)
    # Twice the rated risk: twice the reward. First loss: a HK$500 phone caps the claim cost.
    assert prevention_test.reward_for_customer(engine, 0.04, 0.025, 800, 2.0, 8_000) == (6.0, 24.0)
    assert prevention_test.reward_for_customer(engine, 0.04, 0.025, 800, 1.0, 500) == (1.87, 7.5)
    for relativity in (0.5, 1.0, 1.7, 3.0):
        reward, avoided = prevention_test.reward_for_customer(
            engine, 0.04, 0.025, 800, relativity, 8_000
        )
        assert reward < avoided  # R18


def test_r02_backtest_runs_the_production_optimiser():
    engine = _engine()
    months = [f"2026-{m:02d}-01" for m in range(10, 13)]
    basis = {
        "expected_cost": {
            cover: {month: ppm for month in months}
            for cover, ppm in (
                ("weather_income", 20_000),
                ("commute_disruption", 30_000),
                ("event_ticket", 5_000),
            )
        },
        "factor_tables": {
            "declared_work_hours": {
                "pecuniary.income_weather": {
                    "relativities": {"day": 10_000},
                    "undeclared": {"relativity": 10_000, "exposure_periods": 0},
                }
            }
        },
    }
    profile = {
        "customer_id": "synthetic-rider",
        "occupation": "delivery rider",
        "work_windows": [{"weekday": 1, "start_minute": 540, "end_minute": 1080}],
        "declared_items": [],
        "commute_routes": ["synthetic route"],
        "upcoming_events": [
            {
                "kind": "event.concert",
                "starts": "2026-10-20",
                "ends": None,
                "reference": None,
                "value": 80_000,
            }
        ],
        "typical_daily_earnings": 60_000,
        "consents": [],
    }
    inputs = {"factor_values": {"declared_work_hours": "day"}, "classes": {}}
    out = json.loads(
        engine.suggest(
            "starter", "2026-10-01", json.dumps(profile), json.dumps(inputs), json.dumps(basis)
        )
    )
    allocated = sum(a["premium"] for a in out["outcome"]["schedule"]["allocations"])
    assert allocated + out["outcome"]["unallocated"] == 4_000
    assert all(a["premium"] % 500 == 0 for a in out["outcome"]["schedule"]["allocations"])
    assert engine.risk_class(10_000) == "standard"
    assert engine.rate_ppm(engine.base_cost_ppm(40_000, 20_000, 0), 10_000) == 72_727


FIXTURES = Path(__file__).resolve().parents[3] / "fixtures"


def test_r31_trigger_pricing_replays_the_record_through_the_engine():
    engine = _engine()
    history = trigger_pricing.load_history(
        FIXTURES / "hko_warnings_history_synthetic.csv", "hko_warnings"
    )
    days = trigger_pricing.work_week(range(1, 6), 9 * 60, 18 * 60)
    costs = trigger_pricing.monthly_cost_ppm(engine, history, "weather_income", days, 2016, 2025)
    # The synthetic record has typhoons June-October and black rain April-September only.
    assert all(costs[m] == 0 for m in (1, 2, 3, 11, 12))
    assert sum(costs[m] for m in range(6, 11)) > 0
    # One working day in force costs the whole benefit amount (1,000,000 ppm) once.
    one_signal8 = [
        {
            "report_id": "x",
            "source": "hko_warnings",
            "code": "TC8NE",
            "starts": "2026-09-14T10:00:00+08:00",
            "ends": "2026-09-14T15:00:00+08:00",
        }
    ]
    assert engine.payout_by_month_ppm(
        json.dumps(one_signal8), "weather_income", json.dumps(days)
    ) == {"2026-09": 1_000_000}
    # Outside declared hours it pays nothing (R08), so a night worker costs differently.
    nights = trigger_pricing.work_week(range(1, 6), 22 * 60, 6 * 60)
    assert engine.payout_by_month_ppm(
        json.dumps(one_signal8), "weather_income", json.dumps(nights)
    ) == {"2026-09": 0}


def test_r32_trigger_pricing_curve_feeds_the_rating_horizon():
    costs = {m: (40_000 if m in (7, 8, 9) else 5_000) for m in range(1, 13)}
    curve = trigger_pricing.expected_cost_curve(costs, date(2026, 11, 1), 12)
    assert curve["2026-11-01"] == 5_000
    assert curve["2027-08-01"] == 40_000
    assert len(curve) == 12
    engine = _engine()
    # R31: rate = cost / target loss ratio 0.55, in the engine's integer arithmetic.
    assert trigger_pricing.rate_ppm(engine, 40_000) == 72_727
