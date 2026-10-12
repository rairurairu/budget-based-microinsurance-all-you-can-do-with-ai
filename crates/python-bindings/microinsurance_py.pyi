"""Type stubs for the engine bindings (crates/python-bindings/src/lib.rs)."""

def prevention_test_passes(
    reward_cost_cents: int, p_before_bps: int, p_after_bps: int, avg_claim_cost_cents: int
) -> bool: ...
def claims_avoided_cents(p_before_bps: int, p_after_bps: int, avg_claim_cost_cents: int) -> int: ...
def mul_bps(cents: int, bps: int) -> int: ...
def horizon_cost_ppm(costs_ppm: list[int]) -> tuple[int, int]: ...

class Engine:
    def __init__(
        self,
        insurer: str,
        lines_of_business: str,
        rating_factors: str,
        event_types: str,
        trigger_sources: list[str],
        exposure_types: str,
        data_types: str,
        catalogue: list[str],
        prevention_measures: str,
        suppliers: str,
    ) -> None: ...
    def base_cost_ppm(
        self, own_ppm: int, horizon_average_ppm: int, continuous_periods: int
    ) -> int: ...
    def rate_ppm(self, base_cost_ppm: int, relativity_bps: int) -> int: ...
    def risk_class(self, relativity_bps: int) -> str: ...
    def prevention_reward_cents(
        self,
        p_without_bps: int,
        p_with_bps: int,
        avg_claim_cost_cents: int,
        relativity_bps: int,
        sum_insured_cents: int,
    ) -> tuple[int, int]: ...
    def suggest(
        self,
        plan_id: str,
        period_start: str,
        profile_json: str,
        rating_inputs_json: str,
        rating_basis_json: str,
    ) -> str: ...
    def payout_by_month_ppm(
        self, reports_json: str, cover_id: str, work_windows_json: str
    ) -> dict[str, int]: ...
