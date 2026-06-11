FROM rust:1.94-bookworm AS builder

WORKDIR /src
COPY . .
RUN cargo build --release -p memory-engine-api

FROM debian:bookworm-slim AS runtime

# curl is required by the magic-link mailer script.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/target/release/memory-engine-api /usr/local/bin/memory-engine-api
COPY --from=builder /src/bin/send-magic-link /usr/local/bin/send-magic-link

ENV HOST=0.0.0.0
ENV PORT=8080
EXPOSE 8080

CMD ["memory-engine-api"]
