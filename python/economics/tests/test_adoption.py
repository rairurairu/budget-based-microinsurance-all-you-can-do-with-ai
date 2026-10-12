"""Phase 7 models (ADR-012): the pilot's gates, the partner's value and the data loop's
indications for the actuary."""

from microinsurance_economics import experience, partner_value, pilot_evaluation
from microinsurance_economics.pilot_evaluation import Gate

GATES = [
    Gate("conversion_rate", "scale", at_least=0.05),
    Gate("claims_avoided_vs_control", "scale", at_least=0.10),
    Gate("satisfaction", "scale", at_least=0.70),
    Gate("loss_ratio", "stop", at_most=0.75),
    Gate("fairness_min_parity", "stop", at_least=0.80),
]


def test_r59_pilot_scales_only_when_every_scale_gate_passes_measured():
    good = {
        "conversion_rate": 0.08,
        "claims_avoided_vs_control": 0.15,
        "satisfaction": 0.80,
        "loss_ratio": 0.55,
        "fairness_min_parity": 0.92,
    }
    assert pilot_evaluation.evaluate(good, GATES).decision == "scale"
    # Not measured yet: hold, never a pass by missing data.
    held = pilot_evaluation.evaluate({**good, "claims_avoided_vs_control": None}, GATES)
    assert held.decision == "hold"
    assert [r.status for r in held.gates][1] == "not_measured"
    # A scale gate missed: hold.
    assert pilot_evaluation.evaluate({**good, "conversion_rate": 0.02}, GATES).decision == "hold"
    # Any stop gate breached stops the pilot, whatever else passes.
    stopped = pilot_evaluation.evaluate({**good, "fairness_min_parity": 0.70}, GATES)
    assert stopped.decision == "stop"
    # A stop gate not yet measured does not stop it, but it is shown as not measured.
    missing = pilot_evaluation.evaluate({**good, "loss_ratio": None}, GATES)
    assert missing.decision == "scale"
    assert [r.status for r in missing.gates][3] == "not_measured"


def test_r59_the_apis_report_is_read_in_the_gates_terms():
    report = {
        "metrics": [
            {"metric": "conversion_rate", "unit": "bps", "value": 800, "sample": 500},
            {"metric": "claims_avoided_vs_control", "unit": "bps", "value": None, "sample": 0},
            {"metric": "claim_payment_hours_p95", "unit": "hours", "value": 30, "sample": 40},
            {"metric": "loss_ratio", "unit": "bps", "value": 5_500, "sample": 500},
            {"metric": "fairness_min_parity", "unit": "bps", "value": 9_200, "sample": 300},
        ],
        "gates": [
            {"metric": g.metric, "kind": g.kind, "at_least": g.at_least, "at_most": g.at_most}
            for g in GATES
        ]
        + [{"metric": "claim_payment_hours_p95", "kind": "scale", "at_most": 48}],
    }
    metrics = pilot_evaluation.metrics_from_report(report)
    assert metrics["conversion_rate"] == 0.08
    assert metrics["claim_payment_hours_p95"] == 30.0
    # Claims avoided are not measured yet; satisfaction comes from the survey.
    out = pilot_evaluation.evaluate_report(report, {"satisfaction": 0.8})
    assert out.decision == "hold"
    out = pilot_evaluation.evaluate_report(
        report, {"satisfaction": 0.8, "claims_avoided_vs_control": 0.12}
    )
    assert out.decision == "scale"


def test_partner_value_is_the_sum_of_its_parts():
    capacity = partner_value.spare_capacity_value(100, 20.0, 6.0)
    assert capacity == 1_400.0
    assert partner_value.value(500.0, 1_200.0, 300.0, capacity) == 3_400.0
    assert partner_value.of(partner_value.PartnerInputs(0, 0, 0, -50.0)) == -50.0


def study(claims: int, expected: int, incurred: int) -> dict:
    return {
        "as_of": "2026-12",
        "full_credibility_claims": 1_082,
        "classes": [
            {
                "class_of_business": "pecuniary.income_weather",
                "period": "2026-11",
                "claims": claims,
                "expected_minor": expected,
                "incurred_minor": incurred,
            }
        ],
        "prevention": [
            {
                "class_of_business": "property.device",
                "measure": "screen_protector_fitted",
                "frequency_with_bps": 200,
                "frequency_without_bps": 500,
                "credibility_bps": 5_000,
            }
        ],
    }


def test_r31_experience_moves_the_rate_only_as_far_as_it_is_credible():
    # Twice the expected cost on few claims: a small move.
    few = experience.class_indications(study(10, 100_000, 200_000))[0]
    assert few.actual_to_expected == 2.0
    assert 1.0 < few.indicated_factor < 1.2
    # On full credibility the experience stands on its own.
    full = experience.class_indications(study(1_082, 100_000, 200_000))[0]
    assert full.indicated_factor == 2.0
    basis = {"expected_cost": {"weather_income": {"2026-12-01": 150_000, "2027-01-01": 150_000}}}
    proposed = experience.propose_rating_basis(
        basis,
        study(1_082, 100_000, 200_000),
        {"weather_income": "pecuniary.income_weather"},
        "2027-01-01",
    )
    assert proposed["expected_cost"]["weather_income"] == {
        "2026-12-01": 150_000,
        "2027-01-01": 300_000,
    }
    assert basis["expected_cost"]["weather_income"]["2027-01-01"] == 150_000, "a proposal"


def test_r18_prevention_probabilities_blend_the_book_with_the_basis():
    basis = {
        "measures": {
            "screen_protector_fitted": {
                "claim_prob_without": 400,
                "claim_prob_with": 250,
                "avg_claim_cost": 80_000,
            }
        }
    }
    [m] = experience.prevention_indications(study(10, 1, 1), basis)
    assert (m.claim_prob_without, m.claim_prob_with) == (450, 225)
