# Vendored dependencies

Third-party source copied into this repository rather than fetched. Each one is
here for a stated reason, and each reason is a thing that should eventually stop
being true.

---

## `nih-plug-webview`

- **Upstream:** <https://github.com/httnn/nih-plug-webview>
- **Revision copied:** `745d79a32fb21374758812e49436cb2a21e217fd`
- **Copied on:** 2026-07-28
- **Licence:** ISC — see `nih-plug-webview/LICENSE`, which is preserved
  unmodified alongside the source.

### Why it is vendored

It does not compile against anything cargo will resolve for it.

Upstream declares `baseview = { git = "https://github.com/RustAudio/baseview" }`
with **no revision pin**, so cargo takes whatever HEAD is on the day. On
2026-06-19 baseview's `858113b` migrated to `raw-window-handle` 0.6, and
`cc28453` moved its size and position types to the `dpi` crate. Upstream's
source — written against `raw-window-handle` 0.5 and `baseview::Size` — now
fails with nine compile errors, all of them inside the dependency:

```
error[E0432]: unresolved import `baseview::Size`
error[E0277]: the trait bound `ParentWindowHandle: HasWindowHandle` is not satisfied
error[E0050]: method `on_frame` has 2 parameters but the declaration in trait has 1
...
```

An unpinned git dependency inside a dependency cannot be pinned from the
outside: cargo rejects a `[patch]` whose replacement points at the same source
URL. Vendoring the crate is what makes its `Cargo.toml` ours to pin, and it is
pinned to `91e3b4a` — baseview 0.1.4, the last revision before that migration
and the one this code was written against.

### What was changed

The manifest, and **one behaviour in `src/lib.rs`** — so a rebase is a diff
against two files rather than one.

`src/lib.rs`: the IPC handler used to `panic!` on JSON it could not parse. A
panic on the UI thread of someone else's DAW takes the host down with it, and
the message that caused it is a bug in the *page*, not grounds to kill Ableton.
It logs and returns instead.

The manifest:

- `baseview` pinned to `91e3b4a50f1db712355ba0d991c02d024050ef40`.
- `nih_plug` pinned to the same revision the rest of this workspace uses, so
  there is one `nih_plug` in the build rather than two whose types cannot meet.
- The `[workspace]` block and its `example`/`xtask` members removed, so this
  crate joins ours instead of declaring its own.
- `publish = false`.

### What has to happen for this to go away

Any one of:

1. Upstream pins `baseview`, updates to `raw-window-handle` 0.6, and stops
   panicking on malformed IPC.
2. The maintained framework fork (`nice-plug`) grows a webview adapter — it
   currently ships egui, iced, slint and vizia adapters and no webview, which
   is the whole reason this project is on the unmaintained `nih-plug`.
3. This project writes its own editor directly on `wry` + `baseview`. The
   surface is small — one `Editor` impl, one window handler, an IPC channel —
   and 280 lines of it are already sitting here to read.

**One upstream design decision this project had to route around, worth knowing
before anyone "simplifies" it back.** The adapter's IPC is one-way: messages
are queued and drained from `on_frame`, and replies go out via
`evaluate_script` from that same frame handler. **A plugin window parented into
Ableton Live never receives a frame tick**, so every command queued forever and
none was ever answered — the UI rendered perfectly and nothing worked. The
bridge therefore runs as an HTTP round trip over the custom protocol
(`POST /__rpc` in `plugin/src/editor.rs`), which the webview handles
synchronously and which depends on no tick at all. Do not move it back onto
`next_event`/`send_json`.

**Track this.** A vendored dependency nobody revisits is how a project ends up
maintaining a fork it never chose to own. See TASK-P05 and TASK-P11 in
`docs/product-roadmap.md`.
