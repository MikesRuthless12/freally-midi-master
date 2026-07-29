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

The manifest, two behaviours in `src/lib.rs`, and **one file that is entirely
ours** — `src/linux.rs`, which upstream has no equivalent of. A rebase is
therefore a diff against the manifest and `lib.rs`, plus a module that can be
carried across whole.

`src/lib.rs`:

- The IPC handler used to `panic!` on JSON it could not parse. A panic on the UI
  thread of someone else's DAW takes the host down with it, and the message that
  caused it is a bug in the *page*, not grounds to kill Ableton. It logs and
  returns instead.
- The webview's attributes moved into a `configure` function, and the webview
  itself is built through a `cfg` — inline on Windows and macOS, on the GTK
  thread on Linux. `WindowHandler::webview` is a `WebView` on the first two and
  a `linux::WebViewHandle` on the third, with the same method names, so nothing
  below the construction site has a `cfg` in it.

`src/linux.rs` (**new, TASK-P12**): the X11 + WebKitGTK editor. Upstream is
macOS/Windows only and this is the whole of the difference.

### ⛔ The Linux editor runs GTK on one thread, and that is not a style choice

wry's WebKitGTK backend asks for `gdk::Display::default()`, which is `None`
until somebody has called `gtk_init`. A plugin cannot require that of its host —
Reaper and Bitwig are not GTK applications — so the editor has to start GTK
itself.

The obvious place is baseview's window thread, and it is **wrong**. baseview
spawns a fresh thread per editor, and gtk-rs panics on the second `init()` from
a different thread:

```text
panicked at 'Attempted to initialize GTK from two different threads.'
```

A panic in a plugin takes the DAW down, and this is not an exotic path: closing
the editor and reopening it is a new baseview thread, and so is a second
instance on a second track. It would have passed a single-window screenshot test
and destroyed a real session.

So GTK is initialised **once, on a thread this crate owns**, every webview lives
there, and other threads address one by id. Two things follow that are easy to
undo by accident:

1. **`on_gtk` waits for that thread before posting anything.**
   `g_main_context_invoke` does not merely queue — if the calling thread can
   *acquire* the context it runs the job inline, right there. Posting before the
   GTK thread has claimed the context therefore runs webview construction on
   baseview's thread and aborts the process with *"GTK has not been initialized"*.
   That is what the first Xvfb run did, and the two symptoms pointed away from
   the cause. Once `gtk::init()` returns Ok the GTK thread holds the default
   context permanently (gtk-rs acquires and deliberately leaks it), so every
   later `invoke` genuinely queues.
2. **When GTK is unavailable the job is dropped, not run**, for the same reason.

**Known limit, unverified rather than solved:** if the host's own main thread
already owns the default main context — a host that is itself a GTK application
— `gtk::init()` fails, and the plugin logs and opens no editor rather than
crashing. The proper answer is to run the webview on the host's loop, which
nih-plug's `Editor` API does not expose. Verified under Xvfb and in the
standalone; **no Linux DAW has loaded this yet**, which is the rest of TASK-P12.

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

### ⛔ THE WINDOWS *STANDALONE* OPENS A BLANK WINDOW, AND IT IS THE MESSAGE PUMP

**Diagnosed 2026-07-29. The plugin is fine; `npm run plugin:standalone` is not,
on Windows.** Do not go looking for this in the engine, the embedded UI or the
custom protocol — all three were ruled out, in this order:

1. `assert-plugin-bundled` confirms the UI and dataset are in the binary.
2. Instrumenting the adapter showed the webview is built correctly:
   `bounds 1224x765, protocol=Some("freally"), devtools=true`,
   `with_url(freally://localhost/index.html)`, `build ok = true`.
3. `serve` is **never called** — not a 404, no request at all.
4. Devtools (on by default in release) reports the page as **`about:blank`**,
   with a real `<html><body>` sized 1208x749. So the webview exists and never
   navigated.
5. **The decisive test.** In the devtools console:
   ```js
   fetch('http://freally.localhost/index.html')
     .then(r => console.log('STATUS', r.status))
     .catch(e => console.log('FAILED', e.message))
   ```
   The promise stays **`pending` forever** — it neither resolves nor rejects. A
   rejection would mean wry's `http://freally.*` resource filter did not match. A
   *hang* means WebView2 intercepted the request and **the handler was never
   dispatched**.

**What is established, and what is still inference. Keep the two apart.**

*Established by the evidence above:* WebView2 accepted the request and **never
dispatched our handler**, and navigation never completed. A hung fetch cannot be
a filter miss, a 404, or a missing asset.

*Inference, not yet proven:* that the cause is the **Windows message loop** —
WebView2 delivers `WebResourceRequested` and completes navigation through the
message loop of the thread that created it, so an unpumped loop would produce
exactly this. **But do not treat that as diagnosed.** There is a specific reason
to doubt it: baseview's Windows backend creates a *parented* window on the
**calling** thread, not a new one, and nih-plug's standalone runs a blocking loop
on that thread — which ought to be pumping already. Whoever picks this up should
confirm where the messages are going before writing a pump, or they will add a
second loop to a thread that already has one.

Worth checking first, in rough order of likelihood: whether the WebView2
environment is being created against a user data folder that another environment
already holds with different options (the adapter passes the bare
`std::env::temp_dir()`, which is shared and is a poor choice regardless); whether
`open_parented` on Windows really is same-thread here; and whether the handler
closure is being dropped before the request arrives.

**Why the DAWs work.** Ableton and FL Studio pump their own message loop, so the
events fire and the editor renders — which is what TASK-P08 verified. **Why Linux
works.** `src/linux.rs` runs a real `gtk::main()` on a thread it owns.

Two things that are *not* the cause, both tried, so nobody repeats them:

- **The URL scheme.** Asking for `http://freally.localhost/index.html` directly
  changes nothing; wry already rewrites the custom scheme to exactly that.
- **`with_web_context` ordering.** It must go on before the custom protocol —
  that is real and is now commented in `configure` — but fixing it did not fix
  the blank window, and reverting the whole refactor did not either. **The blank
  standalone predates this session's changes.**

**What a fix would look like:** pump the Windows message queue for the webview's
thread from the editor's frame handler, the same shape `src/linux.rs` uses for
GLib. Tracked as TASK-P16, and it is what the Windows and macOS screenshot legs
of CI are waiting on.

**Track this.** A vendored dependency nobody revisits is how a project ends up
maintaining a fork it never chose to own. See TASK-P05 and TASK-P11 in
`docs/product-roadmap.md`.
