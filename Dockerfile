# syntax=docker/dockerfile:1
FROM rust:1.91-slim-bookworm AS builder
WORKDIR /build

# Dependencies are vendored (see .cargo/config.toml) so the build needs no
# network — crates.io and the Debian mirrors are unreachable from some regions.
COPY .cargo/ .cargo/
COPY vendor/ vendor/
COPY Cargo.toml Cargo.lock ./

# Warm the dependency cache with a stub main so only our own sources recompile
# on subsequent builds.
RUN --mount=type=cache,target=/build/target \
    mkdir -p src && echo 'fn main(){}' > src/main.rs && \
    cargo build --release --offline

COPY src/ src/
RUN --mount=type=cache,target=/build/target \
    touch src/main.rs && \
    cargo build --release --offline && \
    cp target/release/proxy-server /proxy-server

FROM debian:bookworm-slim
COPY --from=builder /proxy-server /usr/local/bin/proxy-server
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/proxy-server"]
