# The Linux build environment, as a container.
#
# ⛔ **Why this exists: the Linux drag source cannot be written blind.**
# `plugin/src/drag/unsupported.rs` refuses every drag on macOS and Linux, and
# `docs`/the handoff have recorded for months that the reason those two are
# unbuilt is not difficulty — it is that nobody could *verify* them. This is the
# Linux half of that gap closing: a container that compiles the workspace with
# the same system libraries CI installs, so the code can be type-checked and
# built against real GTK/WebKit headers from a Windows machine.
#
# ⚠ **The package list is copied from `.github/workflows/ci.yml`, deliberately
# and not abstracted.** Two lists that must agree are two lists that drift, and
# the failure is a build that works here and fails in CI — which is the
# situation this is meant to end. If CI's list changes, change this one.
#
# ⚠ **What this proves and what it does not.** It proves the Linux code
# compiles and links. It does NOT prove a drag works: that needs a desktop
# session and a DAW, which is what QEMU is for. `drag_supported` stays the gate
# — a handle that drops nothing is worse than no handle, and this project has
# written that down five times.
#
#   docker build -f docker/linux-build.Dockerfile -t freally-linux .
#   docker run --rm -v "${PWD}:/w" -w /w freally-linux cargo check --workspace
FROM rust:1.97.1-bookworm

# ⚠ `libwebkit2gtk-4.1-dev` is the one that matters for the drag work: it brings
# the GTK 3 headers the webview is built against, and a GTK drag has to be
# started from the same widget hierarchy the webview lives in.
RUN apt-get update && apt-get install -y --no-install-recommends \
      libwebkit2gtk-4.1-dev \
      libgtk-3-dev \
      libasound2-dev \
      libjack-jackd2-dev \
      libx11-dev \
      libx11-xcb-dev \
      libxcb1-dev \
      libxcb-icccm4-dev \
      libxcursor-dev \
      libxkbcommon-dev \
      pkg-config \
    && rm -rf /var/lib/apt/lists/*

# ⚠ **A separate target directory from the host's.** The repo is bind-mounted,
# and Windows and Linux artifacts in one `target/` invalidate each other on
# every switch — a full rebuild each way, which is the thing that makes people
# stop using the container.
ENV CARGO_TARGET_DIR=/target
