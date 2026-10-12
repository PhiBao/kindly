# Kindly API — multi-stage build.
#
#   docker build -t kindly-api .
#   docker run -p 8787:8787 -e KINDLY_DATA_DIR=/data -v kindly-data:/data kindly-api
#
# The runtime image carries the binary, the web assets it serves and nothing
# else. SQLite lives on a mounted volume when one is provided; otherwise the
# demo reseeds itself on every boot, which is what the hosted demo does.

FROM rust:1-slim AS builder
WORKDIR /build

# Dependencies first so the layer caches across source changes.
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release -p kindly-server --locked

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Run as a non-root user; the data directory is the only writable path.
RUN useradd --system --create-home --uid 10001 kindly \
    && mkdir -p /data \
    && chown kindly:kindly /data

COPY --from=builder /build/target/release/kindly /usr/local/bin/kindly

USER kindly
WORKDIR /home/kindly

ENV KINDLY_DATA_DIR=/data \
    KINDLY_PORT=8787 \
    RUST_LOG=info

EXPOSE 8787

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s \
    CMD /usr/local/bin/kindly --help > /dev/null 2>&1 || exit 1

ENTRYPOINT ["/usr/local/bin/kindly"]
CMD ["serve"]
