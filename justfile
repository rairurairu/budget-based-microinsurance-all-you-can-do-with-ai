set windows-shell := ["pwsh", "-NoLogo", "-Command"]

default: check

setup:
    rustup show
    uv sync --project python
    pnpm install
    just bindings

# Builds the engine's Python bindings into python/.venv (economic models call the engine).
bindings:
    uv run --project python maturin develop --uv -q -m crates/python-bindings/Cargo.toml

gen:
    cargo run -q -p microinsurance-contracts
    pnpm -r --if-present gen

check: bindings
    cargo fmt --all --check
    cargo clippy --all-targets -- -D warnings
    uv run --project python cargo clippy -p microinsurance-python-bindings -- -D warnings
    cargo test -q -p microinsurance-engine --test config_files
    uv run --project python ruff check python
    uv run --project python pyright python
    pnpm lint
    pnpm -r --if-present typecheck

test: bindings
    cargo test
    uv run --project python pytest python
    pnpm -r --if-present test

bench:
    cargo bench -p microinsurance-engine --bench engine

eval:
    uv run --project python pytest python/evals

# After changing a golden case's input: refresh the saved replies' keys.
eval-rekey:
    cd python/evals && uv run --project .. python -m microinsurance_evals.rekey

# With EVAL_LIVE=1: save the live model's replies for demo mode and offline evals.
eval-record:
    cd python/evals && uv run --project .. python -m microinsurance_evals.rekey --record

# The API in demo mode: offline, the synthetic book, saved AI replies, the apps at /app /embed
# /partner /ops (built first). Without DEMO_MODE=1 it needs the settings in crates/api/README.md.
# The international demonstration market (config/markets/global, US dollars) by default;
# `just dev hk` runs the base market, `just dev tw` Taiwan.
dev market="global":
    just _api "{{ if market == 'hk' { '' } else { market } }}" 0 "{{ if market == 'global' { '2026-11-18T14:00:00Z' } else { '' } }}"

# Demo mode with live AI: the same synthetic book, but every AI component calls the model. Needs
# ANTHROPIC_API_KEY in the environment; a failed call still gives the saved answer (R22).
dev-live market="global":
    just _api "{{ if market == 'hk' { '' } else { market } }}" 1 "{{ if market == 'global' { '2026-11-18T14:00:00Z' } else { '' } }}"

# The global market's demo clock is 14:00 UTC, in its riders' working day.
_api $MARKET $AI_LIVE $DEMO_START $DEMO_MODE="1":
    pnpm -r --if-present build
    cargo run -p microinsurance-api

# A load test of the non-AI routes against a running API (docs/performance.md): BASE_URL, and
# API_TOKEN_SECRET outside demo mode.
load:
    cargo run --release -p microinsurance-api --example load_test

# Demo mode for another market: `just demo-market tw` replays its feeds offline.
demo-market $MARKET $DEMO_MODE="1":
    cargo run -p microinsurance-workers

gen-check:
    cargo run -q -p microinsurance-contracts -- --check
    pnpm -r --if-present gen
    git diff --exit-code apps/api-client/src/generated

ci: gen-check check test
