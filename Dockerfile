FROM rust:1.68-alpine as builder
WORKDIR /usr/src/compose-apps-exporter
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN apk add --no-cache musl-dev
RUN cargo install --path . --root /usr/local/cargo

FROM alpine:3
COPY --from=builder /usr/local/cargo/bin/compose-apps-exporter /usr/local/bin/compose-apps-exporter
RUN apk add --no-cache ca-certificates tini
ENTRYPOINT ["/sbin/tini", "--", "compose-apps-exporter"]
