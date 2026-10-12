"""The fixed text checks, read from the language packs in ``config/languages/`` as the Rust
harness reads them (``crates/ai/src/language.rs``): nothing about a language is in this code."""

from __future__ import annotations

import re
from dataclasses import dataclass
from typing import Any, cast

import yaml

from microinsurance_evals.runner import ROOT

CONFIG = ROOT / "config"
WORDLIKE = re.compile(r"^[A-Za-z0-9\- ]+$")


@dataclass
class Pack:
    tag: str
    terminators: list[str]
    max_words: int | None
    max_chars: int | None


def _load() -> tuple[dict[str, Pack], list[str], list[str], str]:
    packs: dict[str, Pack] = {}
    currency: list[str] = []
    protected: list[str] = []
    for path in sorted((CONFIG / "languages").glob("*.yaml")):
        raw = cast(dict[str, Any], yaml.safe_load(path.read_text(encoding="utf-8")))
        sentence = cast(dict[str, Any], raw["sentence"])
        checks = cast(dict[str, list[str]], raw["checks"])
        packs[str(raw["language"])] = Pack(
            tag=str(raw["language"]),
            terminators=[str(t) for t in cast(list[Any], sentence["terminators"])],
            max_words=cast(int | None, sentence.get("max_words")),
            max_chars=cast(int | None, sentence.get("max_chars")),
        )
        currency += checks["currency"]
        protected += checks["protected"]
    insurer = cast(
        dict[str, Any], yaml.safe_load((CONFIG / "insurer.example.yaml").read_text(encoding="utf-8"))
    )
    default = str(cast(dict[str, Any], insurer["languages"])["default"])
    return packs, currency, protected, default


PACKS, CURRENCY, PROTECTED, DEFAULT = _load()


def _found(terms: list[str], text: str) -> bool:
    lower = text.lower()
    words = [w for w in re.split(r"[^\w\-]+", lower) if w]
    for term in terms:
        t = term.strip().lower()
        if not t:
            continue
        if WORDLIKE.match(t):
            phrase = t.split()
            if any(words[i : i + len(phrase)] == phrase for i in range(len(words))):
                return True
        elif t in lower:
            return True
    return False


def pack(tag: str | None) -> Pack:
    return PACKS.get(tag or DEFAULT, PACKS[DEFAULT])


def one_sentence(text: str, p: Pack) -> bool:
    """R23 in the pack's language."""
    t = text.strip()
    if not t or "\n" in t or t[-1] not in p.terminators or t.endswith(".."):
        return False
    if p.max_words is not None and len(t.split()) > p.max_words:
        return False
    if p.max_chars is not None and len(t) > p.max_chars:
        return False
    inner = t[:-1]
    for i, c in enumerate(inner):
        if c in p.terminators and (not c.isascii() or inner[i + 1 : i + 2].isspace()):
            return False
    return True


def states_money(text: str) -> bool:
    return _found(CURRENCY, text)


def mentions_protected(text: str) -> bool:
    return _found(PROTECTED, text)
