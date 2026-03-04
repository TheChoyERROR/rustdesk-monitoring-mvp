FROM node:20-bookworm AS web-build
WORKDIR /app/web-dashboard

COPY web-dashboard/package.json web-dashboard/package-lock.json ./
RUN npm ci

COPY web-dashboard/ ./
RUN npm run build


FROM rust:1-bookworm AS rust-build
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --bin monitoring-server


FROM debian:bookworm-slim AS runtime
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=rust-build /app/target/release/monitoring-server ./monitoring-server
COPY --from=web-build /app/web-dashboard/dist ./web-dashboard/dist
COPY server-config.railway.toml ./server-config.railway.toml

RUN mkdir -p /app/data

ENV RUST_LOG=info
ENV DASHBOARD_DIST_DIR=/app/web-dashboard/dist

EXPOSE 8080

CMD ["sh", "-c", "./monitoring-server --bind 0.0.0.0:${PORT:-8080} --database-path /app/data/outbox.db --config /app/server-config.railway.toml"]
