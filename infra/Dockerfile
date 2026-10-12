# One image runs the API and the workers (different commands), for every market: the market is
# chosen at run time (`MARKET`, ADR-012). Build from the repository root:
#   docker build -f infra/Dockerfile -t microinsurance .
# The layout under /app is the repository's (`REPO_ROOT`): config/, crates/ai/prompts/, the built
# apps and the synthetic fixtures demo mode reads. The insurer's filed rating basis is market data,
# not code: the release adds it at /app/market-data/ (see infra/terraform/README.md).

FROM rust:1.99 AS rust
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p microinsurance-api -p microinsurance-workers

FROM node:24-bookworm-slim AS apps
WORKDIR /src
RUN corepack enable
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.base.json biome.json ./
COPY apps apps
RUN pnpm install --frozen-lockfile && pnpm -r --if-present build

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home /app microinsurance
WORKDIR /app
COPY --from=rust /src/target/release/microinsurance-api /usr/local/bin/
COPY --from=rust /src/target/release/microinsurance-workers /usr/local/bin/
COPY config config
COPY crates/ai/prompts crates/ai/prompts
COPY fixtures fixtures
COPY --from=apps /src/apps/customer-app/dist apps/customer-app/dist
COPY --from=apps /src/apps/embed-sdk/dist apps/embed-sdk/dist
COPY --from=apps /src/apps/partner-console/dist apps/partner-console/dist
COPY --from=apps /src/apps/ops-console/dist apps/ops-console/dist
ENV REPO_ROOT=/app PORT=8080
USER microinsurance
EXPOSE 8080
CMD ["microinsurance-api"]
