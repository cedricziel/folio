# syntax=docker/dockerfile:1

FROM rust:1-bookworm AS builder
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY folio-app ./folio-app
COPY migrations ./migrations
RUN cargo build --release --locked --bin folio-app

FROM debian:bookworm-slim

LABEL org.opencontainers.image.title="Folio" \
      org.opencontainers.image.description="AI-native document management system" \
      org.opencontainers.image.source="https://github.com/cedricziel/folio" \
      org.opencontainers.image.licenses="AGPL-3.0-only"

# pdftoppm and pdfinfo come from poppler-utils, convert from imagemagick.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        libssl3 \
        poppler-utils \
        imagemagick \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home-dir /app --shell /usr/sbin/nologin folio \
    && mkdir -p /app /data \
    && chown folio:folio /data

WORKDIR /app
COPY --from=builder /build/target/release/folio-app /usr/local/bin/folio-app
COPY config.toml ./config.toml
COPY static ./static

ENV FOLIO_SERVER_HOST=0.0.0.0 \
    FOLIO_SERVER_PORT=3000 \
    FOLIO_DATABASE_URL="sqlite:/data/folio.db?mode=rwc" \
    FOLIO_STORAGE_PATH=/data/documents

USER folio
VOLUME /data
EXPOSE 3000
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD ["/usr/local/bin/folio-app", "healthcheck"]
ENTRYPOINT ["folio-app"]
