FROM rust:1.68-alpine as builder
WORKDIR /usr/src/compose-apps-exporter
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN apk add --no-cache musl-dev
RUN cargo install --path . --root /usr/local/cargo

FROM alpine:3
COPY --from=builder /usr/local/cargo/bin/compose-apps-exporter /usr/local/bin/compose-apps-exporter
RUN apk add --no-cache tini docker-cli docker-cli-compose
ENV COMPOSE_APPS_EXPORTER_ADDRESS="0.0.0.0"
EXPOSE 9179
HEALTHCHECK --interval=10s --timeout=3s --start-period=2s --retries=2 \
    CMD wget --no-verbose --tries=1 --spider http://localhost:9179/metrics || exit 1
ENTRYPOINT ["/sbin/tini", "--", "compose-apps-exporter"]
