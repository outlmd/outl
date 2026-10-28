# UX lab for outl-desktop: the Linux (WebKitGTK) build under a headless
# X server, so the app can be driven without holding a real machine.
#
# What this reproduces: layout, CSS, keybindings, ordering — anything
# decided by the frontend or the Rust core.
# What it does NOT: macOS-specific window behaviour. The desktop uses
# WKWebView on macOS and WebKitGTK here, and first-responder / focus
# bugs live on that seam.
FROM debian:bookworm-slim

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential curl wget file pkg-config git ca-certificates unzip \
      libwebkit2gtk-4.1-dev libssl-dev libayatana-appindicator3-dev \
      librsvg2-dev libgtk-3-dev libsoup-3.0-dev \
      xvfb xdotool x11-utils x11-xserver-utils x11-xkb-utils xkb-data \
      xauth imagemagick matchbox-window-manager \
      fonts-dejavu-core fonts-noto-color-emoji \
      libgl1-mesa-dri libegl1 \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --default-toolchain 1.99.1 --profile minimal
ENV PATH=/root/.cargo/bin:$PATH

RUN curl -fsSL https://bun.sh/install | bash
ENV PATH=/root/.bun/bin:$PATH

# Xvfb needs a writable runtime dir; WebKitGTK needs the DMA-BUF
# renderer off under a software X server or it paints nothing.
ENV DISPLAY=:99 \
    WEBKIT_DISABLE_DMABUF_RENDERER=1 \
    WEBKIT_DISABLE_COMPOSITING_MODE=1 \
    XDG_RUNTIME_DIR=/tmp/xdg

WORKDIR /repo

# Analysis runs inside the container so the battery needs nothing on the
# host but docker. Appended as its own layer on purpose: editing the apt
# line above would rebuild rustup and bun for no reason.
RUN apt-get update && apt-get install -y --no-install-recommends \
      python3 python3-pil python3-numpy \
    && rm -rf /var/lib/apt/lists/*
