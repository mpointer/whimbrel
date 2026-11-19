# The Whimbrel cloud routine runner: one short job per account, run on a schedule by the host.
#
# Build context is the core/ directory (the Cargo workspace root), not the repo root:
#
#   docker build -f core/cloud-runner/Dockerfile -t whimbrel-cloud-runner core
#
# The runner keeps nothing between runs and holds no files, so the image declares no volume. Run it
# with a read-only root filesystem and no TZ set; the runner sets TZ itself from the synced zone.
# See docs/self-hosting-runner.md.

# ---- Build stage ----------------------------------------------------------------------------
# A C compiler is needed for the bundled SQLite, sqlite-vec, libsql and the static bzip2. The
# image ships gcc, make and pkg-config. TLS is rustls, so there is no OpenSSL to install.
FROM rust:1-slim-bookworm AS build
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential pkg-config \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
RUN cargo build -p cloud-runner --release --locked \
    && strip target/release/whimbrel-cloud-runner

# ---- Runtime stage --------------------------------------------------------------------------
FROM debian:bookworm-slim
# ca-certificates: libsql panics without native roots. tzdata: the runner looks up the user's
# synced zone under /usr/share/zoneinfo.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tzdata \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin runner
COPY --from=build /src/target/release/whimbrel-cloud-runner /usr/local/bin/whimbrel-cloud-runner
USER 10001:10001
# No VOLUME, no EXPOSE, no ENV TZ. The scratch store is in memory.
ENTRYPOINT ["/usr/local/bin/whimbrel-cloud-runner"]
