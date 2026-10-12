# Fixtures

Saved feed responses and AI replies for demo mode and tests. Synthetic data only.

- `hko_warnsum_signal8.json`, `hko_warnsum_black_rain.json`, `hko_warnsum_all_clear.json`: HKO
  warning summary responses (Signal 8 issued, black rain, Signal 8 cancelled). Read by
  `DEMO_MODE=1 cargo run -p microinsurance-workers` and the adapter and worker tests.
- `hko_warnings_history_synthetic.csv`: a generated ten-year warnings record in the warnings
  database export format (`code,issued,cancelled`). Synthetic, for tests only: price only on
  the real HKO export.
- `mtr_line_status_suspended.json`, `mtr_line_status_normal.json`: the MTR line status file
  with the Tsuen Wan Line suspended (`red`) and the Kwun Tong Line delayed (`yellow`), then all
  lines back to normal.
- `td_trafficnews.xml`: Transport Department Special Traffic News (data dictionary v4.0) with a
  road closed, a lane closure, a closed incident, a planned closure and rail messages (not read
  from TD).
- `hkia_departures_delayed.json`: HKIA departures with delayed, estimated, cancelled, early and
  boarding flights, codeshares included.
- `organiser_cancellation.json`: a ticketing partner's notices (cancelled, postponed, and a status
  the engine does not register).
- `ai_replies/<component>/*.json`: saved model replies for demo mode and the golden sets
  (`python/evals`), keyed by the exact input. Some are deliberately bad, to test the fallbacks.
  `python -m microinsurance_evals.rekey` refreshes their keys after a case changes.
- `evidence/*.png`: synthetic images for the verifier's and the claim assessor's harness cases and
  demo mode (not photos of anything).
- `guardrails_synthetic.json`: a synthetic audit store: final decisions counted by audit group, for
  the fairness report (R19), and people's findings on audited decisions (R14). Not real customers.
- `prevention_basis_synthetic.json`: a synthetic prevention basis (claim probability without and
  with each registered measure, average claim cost). Replace with the insurer's claims data.
- `rating_basis_synthetic.json`: a synthetic rating basis (expected claim cost per cover and month,
  factor relativity tables) for the API's demo mode. Replace with `python/economics` output and the
  insurer's filed rates (`RATING_BASIS`).
- `reward_offers_synthetic.json`: partner reward offers as partners would send them (price, stock;
  one out of stock), for demo mode and the worker tests.
- `demo_script.yaml`: demo mode's synthetic book and the saved responses each source replays,
  with the time of each poll, for the base market (`crates/workers` `src/demo.rs`).
- `markets/tw/`: Taiwan's demo script and synthetic CAP 1.2 feeds (ADR-012): work suspensions
  announced the evening before for Taipei, New Taipei and Hualien, a classes-only suspension and
  an exercise (neither pays), an update replacing an alert, a typhoon sea and land warning, and an
  empty feed. Event codes and geocodes follow the config; confirm both against the real feed.
- `personas/*.yaml` (planned; for example a delivery rider; a fan with a concert in three months)
