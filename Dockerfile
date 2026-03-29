# Stage 1: Build (musl for static linking)
FROM rust:1.83-alpine AS builder

RUN apk add --no-cache musl-dev openssl-dev openssl-libs-static

WORKDIR /app
COPY . .

RUN cargo build --release \
    --bin river-gateway \
    --bin river-discord \
    --bin river-orchestrator

# Stage 2: Runtime
FROM alpine:3.21

RUN apk add --no-cache ca-certificates

COPY --from=builder /app/target/release/river-gateway /usr/local/bin/
COPY --from=builder /app/target/release/river-discord /usr/local/bin/
COPY --from=builder /app/target/release/river-orchestrator /usr/local/bin/
COPY docker-entrypoint.sh /usr/local/bin/

RUN chmod +x /usr/local/bin/docker-entrypoint.sh

WORKDIR /app

ENTRYPOINT ["docker-entrypoint.sh"]
