# syntax=docker/dockerfile:1
# SPDX-FileCopyrightText: 2026 Axle Duggan (axlecoffee) <contact@axle.coffee>
# SPDX-License-Identifier: AGPL-3.0-only
FROM rust:1.99.0-alpine3.24 AS tools
RUN apk add --no-cache build-base cmake ca-certificates mold
RUN rustup target add wasm32-unknown-unknown \
    && cargo install trunk --version 0.21.14 --locked
WORKDIR /build

FROM tools AS source
COPY Cargo.toml Cargo.lock README.md LICENSE.md THIRD_PARTY_NOTICES REUSE.toml Dockerfile docker-compose.yml .dockerignore .gitignore ./
COPY LICENSES/ LICENSES/
COPY .env.example ./
COPY server/ server/
COPY web/ web/
COPY .github/workflows/ .github/workflows/
# keep offered source separate from generated assets and compiler output
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    mkdir -p /source/.cargo \
    && cp -a . /source/ \
    && cd /source \
    && cargo vendor --locked vendor > .cargo/config.toml \
    && tar -czf /source.tar.gz .

# target and registry caches ride along across builds, mold shortens the link
FROM source AS frontend
WORKDIR /build/web
RUN --mount=type=cache,target=/build/target \
    --mount=type=cache,target=/usr/local/cargo/registry \
    trunk build --release --locked --public-url /assets/ --dist /build/web/dist

FROM source AS backend
ENV RUSTFLAGS="-C link-arg=-fuse-ld=mold"
RUN --mount=type=cache,target=/build/target \
    --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --locked --release -p haste-server \
    && cp /build/target/release/haste-server /build/haste-server

FROM alpine:3.24 AS runtime
RUN apk add --no-cache ca-certificates
WORKDIR /app
COPY --from=backend /build/haste-server /usr/local/bin/haste-server
COPY --from=frontend /build/web/dist/ /app/assets/
COPY --from=source /source.tar.gz /app/source.tar.gz
COPY LICENSES/ /app/LICENSES/
COPY LICENSE.md THIRD_PARTY_NOTICES /app/
ENV PORT=8292 ASSET_DIR=/app/assets SOURCE_ARCHIVE=/app/source.tar.gz
USER 1000:1000
EXPOSE 8292
ENTRYPOINT ["/usr/local/bin/haste-server"]
