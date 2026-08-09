# Type-checking the macOS code from Windows

**What this buys you:** `plugin/src/drag/macos.rs` is the one file in this repo
that no local build ever compiles. It was written on a Windows machine against
objc2 bindings nobody could check, and the first version of it had **four hard
errors and two `-D warnings` failures** — every one of which would have been
found by CI's macOS runner, one push at a time.

You do not need an Apple machine to catch those. `cargo check` compiles Rust for
a target without **linking** it, and linking is the only step that needs Apple's
toolchain.

## ⛔ Why the obvious command does not work

```sh
cargo check -p freally-midi-master-plugin --target aarch64-apple-darwin   # fails
```

`clap-wrapper`'s build script compiles C++ for the target, and there is no
Darwin C++ cross-compiler on a Windows box. The build script runs before any of
our code is looked at, so this never gets far enough to type-check anything.

## ▶ What does work: check the file on its own

The objc2 surface is the whole risk, and it needs none of the plugin's other
dependencies. Point a throwaway crate at the real file with `#[path]`, give it
the same three objc2 crates the plugin declares, and check *that*.

```sh
mkdir -p /tmp/macoscheck/src && cd /tmp/macoscheck
```

`Cargo.toml` — note the empty `[workspace]`, which keeps it out of the real one:

```toml
[package]
name = "macoscheck"
version = "0.0.0"
edition = "2021"
publish = false

[workspace]

[dependencies]
objc2 = "0.6"
objc2-app-kit = "0.3"
objc2-foundation = "0.3"
```

`src/lib.rs` — the smallest possible stand-in for the parent module:

```rust
#![cfg(target_os = "macos")]

pub mod drag {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Dropped { Copied, Refused, Cancelled }

    #[derive(Debug, Clone)]
    pub struct Preview { width: u32, height: u32, rgba: Vec<u8> }

    impl Preview {
        pub(crate) fn parts(&self) -> (u32, u32, &[u8]) {
            (self.width, self.height, &self.rgba)
        }
    }

    pub(crate) const NO_DRAG_SOURCE: &str =
        "this platform has no drag source yet — use Export instead";

    // ⚠ `#[path]` and NOT `include!`: the real file opens with `//!` inner doc
    // comments, which are only legal at the top of a module's own file.
    #[path = r"C:\...\Freally MIDI Master\plugin\src\drag\macos.rs"]
    pub mod macos;
}
```

Then, **on the project's pinned toolchain** — `rust-toolchain.toml` only applies
inside the repo, so say it explicitly out here or you will check against
whatever `stable` happens to be and get `can't find crate for core`:

```sh
cargo +1.97.1 clippy --target aarch64-apple-darwin
```

⚠ `clippy`, not `check`. CI runs `cargo clippy --workspace --all-targets -- -D
warnings` with `RUSTFLAGS: -D warnings`, so a lint is as red as an error there.
`check` alone will not show you `unused_unsafe`, which accounted for two of the
six problems the first time this was run.

## What it will not tell you

**Nothing about behaviour.** It proves the file compiles and lints; it has never
spoken to a window server. Whether a drag actually lands in a DAW is still a
question only somebody with a Mac can answer — see `drag/macos.rs`'s own header
and `Live-To-Do.md`.

Three things it does catch, all of which it has:

- a `define_class!` missing `#[thread_kind = MainThreadOnly]` when the protocol
  it implements requires it — which also silently changes which `alloc` you get;
- an argument that needs `ProtocolObject::from_ref` rather than the concrete
  class (`initWithPasteboardWriter` takes `&ProtocolObject<dyn
  NSPasteboardWriting>`, not `&NSURL`);
- `unsafe` blocks around calls that are safe in these bindings, and the reverse —
  `NSDefaultRunLoopMode` is an extern static and *does* need one.

## First, read the bindings

They are on disk already, and reading them beats guessing:

```
~/.cargo/registry/src/index.crates.io-*/objc2-app-kit-0.3.2/src/generated/
```

A `pub fn` there is safe; a `pub unsafe fn` is not. That distinction is the one
this whole exercise exists to get right without a Mac.
