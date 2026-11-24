# syntax=docker/dockerfile:1
# glibc (not musl/scratch): native extensions are dlopen()ed, which a static musl binary
# cannot do, and extension archives are unpacked into the runtime image's /tmp.
FROM rust:1-trixie AS builder
LABEL authors="junkydeveloper"

# .cargo/config.toml links with lld on gnu targets; aws-lc-sys may want cmake.
RUN apt-get update \
    && apt-get install -y --no-install-recommends lld cmake \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /steel
COPY . .

# rust-toolchain.toml pins the nightly; rustup installs it on first cargo call.
# Build downloads the vanilla server jar, so it needs network access.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/steel/target \
    cargo build --release --locked --features stand-alone \
    && install -D target/release/steel /out/steel

FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Server state is relative to the working directory: config/, saves/, extensions/ and .logs/.
# World-writable so it also works with `user:` overrides on bind mounts.
WORKDIR /data
RUN chmod 1777 /data
COPY --from=builder /out/steel /usr/local/bin/steel

EXPOSE 25565
VOLUME ["/data/config", "/data/saves", "/data/extensions", "/data/.logs"]

ENTRYPOINT ["/usr/local/bin/steel"]
