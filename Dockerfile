# syntax=docker/dockerfile:1.7
#
# A headless outl peer: `outl serve`, holding this device's iroh endpoint so
# your other devices converge whether or not any two of them are awake at the
# same time. It is a full replica — the op log is the source of truth and this
# box has all of it — not a "cloud" that owns your notes.
#
# Read docs/self-hosting.md before running it. Two things there are not
# guessable from this file: the workspace has to JOIN your graph (never host
# the pairing), and the two persistent volumes are not optional.

########################################
# Build
########################################
FROM rust:1.95.0-slim-bookworm AS builder

# aws-lc-sys — rustls' crypto backend, pulled in by iroh — is the only
# dependency in the tree that needs a C toolchain: cmake drives its build and
# it compiles C plus assembly. Everything else is pure Rust, which is why
# there is no libssl-dev here and no runtime library to match in the final
# stage.
RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential cmake perl \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY . .

# `-p outl-cli --bin outl` keeps the Tauri crates out of the build graph.
# They are workspace members, so cargo reads their manifests, but their build
# scripts want a bundled frontend that has no business in a server image.
#
# The cache mounts make a rebuild incremental across `docker build` runs. They
# are BuildKit-only; on a builder without it the build still works, just cold.
#
# The `strip` is load-bearing and belongs here rather than in `[profile.release]`.
# RFC 0377 §5 took `strip = "symbols"` out of that profile so a crash report
# arrives with function names instead of bare addresses, and the *release
# pipeline* strips each artifact after uploading symbols. This image is not
# that pipeline: it builds without `--features sentry`, so it has no reporter
# and no use for symbols, and without this line it would carry ~12 MB of debug
# info into a server image that `docker.yml` pushes on every non-PR push.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/build/target,sharing=locked \
    cargo build --release -p outl-cli --bin outl \
 && install -Dm755 target/release/outl /out/outl \
 && strip /out/outl

########################################
# Runtime
########################################
FROM debian:bookworm-slim AS runtime

# ca-certificates is for the iroh relay's TLS. Nothing else is linked.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*

RUN groupadd --gid 1000 outl \
 && useradd --uid 1000 --gid 1000 --home-dir /home/outl --create-home outl

COPY --from=builder /out/outl /usr/local/bin/outl
COPY docker/entrypoint.sh /usr/local/bin/outl-entrypoint

# `$HOME` locates the iroh identity (`~/.outl/identity.key`, which IS this
# device's node id); `$XDG_CONFIG_HOME` locates the device store
# (`~/.config/outl`, which owns the actor bindings). Both live under
# /home/outl, and both must outlive the container — see the VOLUME below.
#
# Deliberately NOT $OUTL_DEVICE_DIR: that variable means "throwaway actor" and
# moves the iroh identity with it, so exporting it here would rotate the node
# id and break every pairing. See `outl_sync_iroh::default_device_dir`.
ENV HOME=/home/outl \
    XDG_CONFIG_HOME=/home/outl/.config \
    OUTL_WORKSPACE=/data \
    RUST_LOG=info

RUN mkdir -p /data /home/outl/.outl /home/outl/.config/outl \
 && chown -R outl:outl /data /home/outl

# Two volumes, and they are two for a reason the project treats as an
# invariant: the device store has to sit OUTSIDE the workspace. /data is
# replicated to every peer; /home/outl is what makes this container one
# device rather than a new one on every restart.
VOLUME ["/data", "/home/outl"]

# Non-root, and never root — not even for a chown-then-drop entrypoint.
# `docker exec` bypasses the entrypoint, so a drop would put the documented
# `docker compose exec outl outl peer pair` on the root side of it, writing
# `identity.key` and `peers.json` as root and locking the daemon out of its
# own identity. A bind mount therefore needs one `chown -R 1000:1000` on the
# host; named volumes inherit the ownership set above and need nothing.
USER outl

WORKDIR /data
STOPSIGNAL SIGTERM

# No HEALTHCHECK on purpose. The obvious probes are both harmful here:
# `outl workspace info` takes the per-actor write lock, loses it to the
# running daemon, and mints a fresh ephemeral `ops-<ulid>.jsonl` on every
# probe; `outl peer status` binds an endpoint and fights the daemon for the
# device lease. Liveness is the process, and the daemon exits non-zero when
# its watcher dies — let the restart policy do the work.

ENTRYPOINT ["/usr/local/bin/outl-entrypoint"]
CMD ["serve"]
