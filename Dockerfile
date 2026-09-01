# Static musl build in a scratch image (DECISIONS.md D12).
# TLS roots are compiled in via rustls/webpki-roots — no system CA bundle needed.

FROM rust:1-alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /build
COPY . .
RUN cargo build --release --locked -p mcp-atlassian --features http

FROM scratch
COPY --from=builder /build/target/release/mcp-atlassian /mcp-atlassian
ENTRYPOINT ["/mcp-atlassian"]
