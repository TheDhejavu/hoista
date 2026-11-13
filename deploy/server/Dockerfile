# Hoista on a server, on any host that builds a Dockerfile. Build from the repository root:
#   docker build -f deploy/server/Dockerfile .
# Runs hoistd with its state on the volume at /data. Set HOISTA_PUBLIC_URL
# and HOISTA_SEALED_KEY; see https://docs.hoista.dev/guides/self-host.

FROM rust:1.97.0-slim-bookworm AS build
WORKDIR /src
# libdbus-1-dev: the keyring crate links D-Bus on Linux (the desktop's secret
# service). A server keeps tokens with its sealed key instead, but the code is built in.
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config ca-certificates libdbus-1-dev && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY bin bin
COPY crates crates
COPY docs/backend/network-hosts.txt docs/backend/network-hosts.txt
COPY docs/backend/api docs/backend/api
COPY skills skills
COPY .sqlx .sqlx
ENV SQLX_OFFLINE=true
RUN cargo build --release --locked -p hoistd -p hoist

FROM debian:bookworm-slim
# libdbus-1-3: hoistd links it (see the build stage).
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates git libdbus-1-3 && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/hoistd /usr/local/bin/hoistd
COPY --from=build /src/target/release/hoist /usr/local/bin/hoist
# Hosts mount volumes owned by root, so hoistd starts as root, as a small
# supervisor: it runs the credential keeper as hoista-keeper (the only user
# that can read the sealed tokens) and the rest of Hoista as hoista, without
# the key. See https://docs.hoista.dev/security/remote-runtime.
RUN useradd --system --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin hoista \
 && useradd --system --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin hoista-keeper
VOLUME /data
ENV PORT=8080
EXPOSE 8080
CMD ["hoistd", "--state-dir", "/data"]
