# Build the plugin for Linux and watch its editor actually draw.
#
# TASK-P12 was held back for a long time by a real objection: nobody on a
# Windows or macOS machine could tell whether a Linux editor opened a window or
# merely compiled. This image is the answer to that — it is the same thing the
# `plugin-editor-linux` job in `ci.yml` does, runnable locally, so a change to
# the X11/WebKitGTK path can be checked in a minute instead of a push.
#
#   docker build -f docker/linux-verify.Dockerfile -t freally-linux-verify docker
#   docker run --rm -v "$PWD":/work -v freally-linux-target:/target \
#     freally-linux-verify bash -c \
#     'cargo build -p freally-midi-master-plugin --release && \
#      xvfb-run -a --server-args="-screen 0 1600x1000x24" \
#        node scripts/screenshot-plugin.mjs /tmp/linux.png /target/release/standalone'
#
# ⚠ `dist/` must already be built on the host (`npm run build`) — it is compiled
# into the binary with `include_dir!`, so cargo cannot start without it.
#
# ⚠ This proves the *standalone* renders. It does not prove a Linux DAW loads
# the plugin; that needs a real host, which is the QEMU half of TASK-P12.
FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive

# libwebkit2gtk-4.1 is what wry 0.35's backend links; the xcb and xkbcommon set
# is what baseview's X11 backend needs; libjack arrives through nih-plug's
# `standalone` feature and the build fails without it; xvfb, xdotool and
# imagemagick are how a machine with no desktop sees a window at all.
RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential \
      ca-certificates \
      curl \
      pkg-config \
      libwebkit2gtk-4.1-dev \
      libgtk-3-dev \
      libasound2-dev \
      libjack-jackd2-dev \
      libx11-dev \
      libx11-xcb-dev \
      libxcb1-dev \
      libxcb-icccm4-dev \
      libxcb-util-dev \
      libxcursor-dev \
      libxkbcommon-dev \
      libxkbcommon-x11-dev \
      xvfb \
      x11-utils \
      xdotool \
      imagemagick \
      nodejs \
      git \
    && rm -rf /var/lib/apt/lists/*

# rustup rather than a pinned `rust:` tag, so the toolchain comes from the
# repo's own rust-toolchain.toml and cannot drift from the one CI uses.
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --no-modify-path --default-toolchain none
ENV PATH="/root/.cargo/bin:${PATH}"

# The host's target/ is built for another OS and sharing it would poison both.
# Linux artifacts go to a docker volume, which also keeps the cache warm.
ENV CARGO_TARGET_DIR=/target

WORKDIR /work
