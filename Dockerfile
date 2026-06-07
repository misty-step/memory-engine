FROM rust:1.94-bookworm AS builder

WORKDIR /src
COPY . .
RUN cargo build --release -p memory-engine-api

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/target/release/memory-engine-api /usr/local/bin/memory-engine-api

ENV HOST=0.0.0.0
ENV PORT=8080
EXPOSE 8080

CMD ["memory-engine-api"]
