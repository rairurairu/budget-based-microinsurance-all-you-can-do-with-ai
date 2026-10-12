"""Golden-set evaluations for every AI component.

Each case states properties, not exact text, and the rules it protects. Offline (the default)
the model is the saved replies in ``fixtures/ai_replies/``: this checks the harness, the fixed
checks and the fallbacks end to end. With ``EVAL_LIVE=1`` the model is the configured provider:
this checks the prompts, and harness cases (deliberately bad saved replies) are skipped.
"""

from __future__ import annotations

import re
from typing import Any, cast

import pytest
from microinsurance_evals import language
from microinsurance_evals.runner import Case, Outcome, live, load_cases, run

PROTECTED_FIELDS = ("age", "gender", "ethnicity", "district", "religion", "nationality")

CASES = [c for c in load_cases() if not (live() and c.harness_only)]


@pytest.fixture(scope="module")
def outcomes() -> dict[str, Outcome]:
    return {o.case.case: o for o in run(CASES)}


def case_language(case: Case) -> str | None:
    """The customer's language setting the case runs with, if any."""
    profile = cast(dict[str, Any], case.input.get("profile") or {})
    return cast(str | None, case.input.get("language") or profile.get("language"))


def check_customer_text(text: str, case: Case) -> None:
    p = language.pack(case_language(case))
    assert language.one_sentence(text, p), f"R23 ({p.tag}): {text!r}"
    assert not language.states_money(text), f"R02: {text!r}"
    assert not language.mentions_protected(text), f"R19: {text!r}"


def customer_texts(o: Outcome) -> list[str]:
    v: Any = o.value
    c = o.case.component
    if c == "explain":
        return [str(e["sentence"]) for e in cast(list[dict[str, Any]], v)]
    if c == "explain_change":
        return [str(v)]
    if c == "prevention_tasks" and v is not None:
        return [str(cast(dict[str, Any], v)["action"])]
    if c == "verify":
        return [str(cast(dict[str, Any], v)["reason"])]
    if c == "assess":
        return [str(cast(dict[str, Any], v)["reason"])]
    if c == "understand_form":
        form = cast(dict[str, Any], v)
        return [str(q["text"]) for q in cast(list[dict[str, Any]], form["questions"])]
    return []


@pytest.mark.parametrize("case", CASES, ids=[f"{c.component}/{c.case}" for c in CASES])
def test_golden_case(case: Case, outcomes: dict[str, Outcome]) -> None:
    o = outcomes[case.case]
    assert o.error is None, o.error
    expect = case.expect

    # R22: in replay, every case lands where its saved reply sends it. Live, only fixed rules
    # are certain.
    want = expect.get("source")
    if want is not None and (not live() or want == "rule"):
        assert o.source == want, f"source {o.source} ({o.reason})"

    # R19: protected attributes never reach the model; nor do amounts the case lists.
    if o.sent is not None:
        for name in PROTECTED_FIELDS:
            assert f'"{name}":' not in o.sent, f"R19: {name} sent to the model"
        for text in cast(list[str], expect.get("not_sent", [])):
            assert text not in o.sent, f"{text!r} sent to the model"
    elif want != "rule":
        pytest.fail("no model request was made")

    for text in customer_texts(o):
        check_customer_text(text, case)

    CHECKS[case.component](o, expect)


def check_understand_form(o: Outcome, expect: dict[str, Any]) -> None:
    questions = cast(list[dict[str, Any]], o.value["questions"])
    assert 1 <= len(questions) <= int(expect["max_questions"])
    ids = [str(q["id"]) for q in questions]
    assert len(set(ids)) == len(ids)
    assert not any(part in PROTECTED_FIELDS for i in ids for part in i.split("_")), ids


def check_understand_facts(o: Outcome, expect: dict[str, Any]) -> None:
    v = cast(dict[str, Any], o.value)
    assert v["devices"] == expect["devices"]
    assert v["commute_routes"] == expect["commute_routes"]
    if v["occupation"] is not None:
        assert not language.mentions_protected(str(v["occupation"]))


def check_foresee(o: Outcome, expect: dict[str, Any]) -> None:
    events = cast(list[dict[str, Any]], o.value)
    # R02: the customer declares any value; the reading never does.
    assert all(e["value"] is None for e in events)
    got = {(str(e["kind"]), str(e["starts"])) for e in events}
    want = {(str(e["kind"]), str(e["starts"])) for e in cast(list[dict[str, Any]], expect["events"])}
    assert got == want


def check_explain(o: Outcome, expect: dict[str, Any]) -> None:
    covers = sorted(str(e["cover_id"]) for e in cast(list[dict[str, Any]], o.value))
    assert covers == sorted(cast(list[str], expect["covers"]))


def check_explain_change(o: Outcome, expect: dict[str, Any]) -> None:
    assert isinstance(o.value, str)


def check_prevention(o: Outcome, expect: dict[str, Any]) -> None:
    if "task" in expect and expect["task"] is None:
        assert o.value is None
        return
    task = cast(dict[str, Any], o.value)
    assert task is not None, o.reason
    assert task["evidence"] == expect["evidence"]
    assert re.fullmatch(r"[a-z][a-z0-9_]*", str(task["task_type"]))
    assert not re.search(r"\b(reward|points?|location)\b", str(task["action"]), re.IGNORECASE)


def check_verify(o: Outcome, expect: dict[str, Any]) -> None:
    v = cast(dict[str, Any], o.value)
    assert v["outcome"] == expect["outcome"]
    if v["outcome"] == "verdict":
        assert v["passed"] == expect["passed"]
        assert v["reuse_suspected"] == expect["reuse_suspected"]
        # R10: nothing suspected of reuse or editing ever passes.
        assert not (v["passed"] and v["reuse_suspected"])


def check_rewards(o: Outcome, expect: dict[str, Any]) -> None:
    ranked = cast(list[str], o.value)
    offers = [str(x["offer_id"]) for x in cast(list[dict[str, Any]], o.case.input["offers"])]
    # R16: every offer, cash included, is still there; the AI only orders them.
    assert sorted(ranked) == sorted(offers)
    if "first" in expect:
        assert ranked[0] == expect["first"]
    if "first_in" in expect:
        assert ranked[0] in cast(list[str], expect["first_in"])


def check_assess(o: Outcome, expect: dict[str, Any]) -> None:
    v = cast(dict[str, Any], o.value)
    assert v["outcome"] == expect["outcome"]
    if v["outcome"] == "refer_to_person":
        assert v["evidence_reused"] == expect.get("evidence_reused", False)
        return
    a = cast(dict[str, Any], v["assessment"])
    # R13: approve or refer; nothing else can come back.
    assert a["recommend"] in ("approve", "refer")
    if not live():
        assert a["recommend"] == expect["recommend"]
    elif expect["recommend"] == "refer":
        # Live, a case that needs a person must never be approved straight through.
        assert a["recommend"] == "refer"
    escalation = cast(dict[str, Any], a["escalation"])
    # R46: the AI may raise a case, so a case that needs it must be raised.
    for flag in ("urgent", "human_care"):
        if expect.get(flag):
            assert escalation[flag], flag


CHECKS = {
    "understand_form": check_understand_form,
    "understand_facts": check_understand_facts,
    "foresee": check_foresee,
    "explain": check_explain,
    "explain_change": check_explain_change,
    "prevention_tasks": check_prevention,
    "verify": check_verify,
    "assess": check_assess,
    "rewards": check_rewards,
}


def test_every_component_has_a_golden_set() -> None:
    covered = {c.component for c in load_cases()}
    assert covered == set(CHECKS)


def test_r22_every_component_has_a_fallback_case() -> None:
    fallback = {
        c.component for c in load_cases() if c.harness_only and c.expect.get("source") == "fallback"
    }
    assert fallback == set(CHECKS)


def test_case_names_are_unique() -> None:
    names = [c.case for c in load_cases()]
    assert len(names) == len(set(names)), "outcomes are keyed by case name"
