set -euo pipefail
# The host repo is mounted read-only: copy it in so bun/cargo never
# write macOS-built node_modules or target/ back into it.
mkdir -p /build/repo
cd /src
tar -cf - --exclude=./target --exclude=node_modules --exclude=./.git . \
  | (cd /build/repo && tar -xf -)
cd /build/repo
echo "=== bun install ==="
bun install --frozen-lockfile 2>&1 | tail -3
echo "=== vite build ==="
cd crates/outl-desktop && bun run build 2>&1 | tail -4
echo "=== cargo build ==="
cd /build/repo
CARGO_TARGET_DIR=/build/target cargo build -p outl-desktop 2>&1 | tail -6
ls -la /build/target/debug/outl-desktop
