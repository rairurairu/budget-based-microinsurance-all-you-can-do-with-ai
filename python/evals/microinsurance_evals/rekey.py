"""Keep saved replies in step with the golden cases.

``python -m microinsurance_evals.rekey`` runs every case and writes each one's replay key (and
its input, for people reading the file) into ``fixtures/ai_replies/<component>/<case>.json``.
Run it after changing a case's input; the reply text is left as it is.

``EVAL_LIVE=1 python -m microinsurance_evals.rekey --record`` instead saves the live model's
replies for cases that are not harness cases, so demo mode replays what the model said.
"""

from __future__ import annotations

import json
import sys
from typing import Any, cast

from microinsurance_evals.runner import REPLIES, live, load_cases, run


def main() -> int:
    record = "--record" in sys.argv
    if record and not live():
        print("--record needs EVAL_LIVE=1", file=sys.stderr)
        return 2
    cases = [c for c in load_cases() if not (record and c.harness_only)]
    for o in run(cases):
        path = REPLIES / o.case.component / f"{o.case.case}.json"
        if o.key is None:
            continue  # decided by a rule: no model call, no reply to save
        saved: dict[str, Any] = (
            cast(dict[str, Any], json.loads(path.read_text(encoding="utf-8")))
            if path.exists()
            else {}
        )
        if record:
            if o.reply is None:
                print(f"{o.case.case}: no reply ({o.reason})", file=sys.stderr)
                continue
            saved["reply"] = o.reply
        elif "reply" not in saved:
            continue
        sent = cast(Any, json.loads(o.sent)) if o.sent else None
        saved = {"case": o.case.case, "key": o.key, "input": sent, "reply": saved["reply"]}
        path.parent.mkdir(parents=True, exist_ok=True)
        text = json.dumps(saved, indent=2, ensure_ascii=False) + "\n"
        path.write_text(text, encoding="utf-8", newline="\n")
        print(f"{path.relative_to(REPLIES)}: {o.key}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
