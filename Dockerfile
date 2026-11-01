# syntax=docker/dockerfile:1

FROM rust:1-bookworm AS chef
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/* \
    && cargo install cargo-chef --locked --version 0.1.78
WORKDIR /build

# The recipe only changes when manifests change, so the cook layer below stays cached
# across source-only edits.
FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY folio-app ./folio-app
COPY xtask ./xtask
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /build/recipe.json recipe.json
RUN cargo chef cook --release --locked --recipe-path recipe.json --bin folio-app
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY folio-app ./folio-app
COPY migrations ./migrations
COPY xtask ./xtask
COPY static ./static
COPY design ./design
RUN cargo build --release --locked --bin folio-app

FROM debian:bookworm-slim

LABEL org.opencontainers.image.title="Folio" \
      org.opencontainers.image.description="AI-native document management system" \
      org.opencontainers.image.source="https://github.com/cedricziel/folio" \
      org.opencontainers.image.licenses="AGPL-3.0-only"

# pdftoppm, pdfinfo and pdftotext come from poppler-utils, convert from imagemagick,
# tesseract-ocr is the default local OCR engine. ghostscript is what ImageMagick shells out to
# for PDFs, used when pdftoppm cannot render a file.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        libssl3 \
        poppler-utils \
        imagemagick \
        ghostscript \
        tesseract-ocr \
        tesseract-ocr-eng \
        tesseract-ocr-deu \
    && rm -rf /var/lib/apt/lists/* \
    && sed -i 's|<policy domain="coder" rights="none" pattern="PDF" />|<policy domain="coder" rights="read\|write" pattern="PDF" />|' /etc/ImageMagick-6/policy.xml \
    && grep -q 'rights="read|write" pattern="PDF"' /etc/ImageMagick-6/policy.xml \
    && useradd --system --uid 10001 --home-dir /app --shell /usr/sbin/nologin folio \
    && mkdir -p /app /data \
    && chown folio:folio /data

WORKDIR /app
COPY --from=builder /build/target/release/folio-app /usr/local/bin/folio-app
COPY config.toml ./config.toml

ENV FOLIO_SERVER__HOST=0.0.0.0 \
    FOLIO_SERVER__PORT=3000 \
    FOLIO_DATABASE__URL="sqlite:/data/folio.db?mode=rwc" \
    FOLIO_STORAGE__PATH=/data/documents

USER folio
VOLUME /data
EXPOSE 3000
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD ["/usr/local/bin/folio-app", "healthcheck"]
ENTRYPOINT ["folio-app"]
