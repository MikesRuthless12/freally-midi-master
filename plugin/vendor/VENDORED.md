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
- **The three remaining panics that could end the host's process are gone.**
  Release sets `panic = "abort"`, so none of them unwound and the DAW could not
  have caught any of them. Each now logs and degrades:
  - **`build()` on the webview.** Upstream's `unwrap_or_else(|e| panic!(...))`
    ran inside the closure `baseview::Window::open_parented` invokes **from the
    host's own editor-open callback** — so the crash was the host's. It is not a
    remote failure: no WebView2 Evergreen runtime, a read-only temp directory,
    or a `web_data_dir()` already held by another WebView2 environment with
    different options all land there, and that last one is the standalone opened
    alongside the DAW. `WindowHandler::webview` is therefore `Option<WebView>`
    off Linux, and a failure gives a **blank editor rather than a closed DAW**.
    Linux already behaved this way; the two branches now agree.
  - **The custom-protocol handler.** `handler(&request).unwrap()` ran from an
    `extern "C"` frame, where a panic cannot unwind at all. An `Err` becomes a
    500 the page can report. Latent while `plugin/src/editor.rs::serve` returned
    `Ok` on every path — and it did that by `unwrap`ping three fallible
    `Response::builder()` calls, which now use `?` and arrive here.
  - **`send_json`.** `evaluate_script(...).unwrap()`, called from the frame
    handler on the host's UI thread. A page mid-teardown is an ordinary reason
    for a script to fail.
- The webview's attributes moved into a `configure` function, and the webview
  itself is built through a `cfg` — inline on Windows and macOS, on the GTK
  thread on Linux. `WindowHandler::webview` is a `WebView` on the first two and
  a `linux::WebViewHandle` on the third, with the same method names, so nothing
  below the construction site has a `cfg` in it.
- **`own_message_queue` and the `windows_pump` module (new, TASK-P16).** A
  Windows message pump, **off unless the process opts in**, which fixes the blank
  standalone window diagnosed below. `plugin/src/bin/standalone.rs`
  is the only caller and a DAW never runs it, so the host's queue is never touched.
  ⛔ The pump **skips messages belonging to the editor window and its children** —
  see the doc comment on `windows_pump::drain`. Dispatching those re-enters
  baseview's window procedure while it is already holding a `RefCell` borrow, which
  panics inside an `extern "system"` frame and **aborts the process**. That was
  observed, not theorised: the first cut of this fix drained everything and died
  with `RefCell already borrowed` at `baseview/src/platform/win/window.rs:513` the
  moment the window was clicked.
- **The pump moved off `on_frame` onto a child window (TASK-063D).** It used to
  drain directly from `on_frame`. That works for rendering but puts *everything
  the drain dispatches* — WebView2's COM completions, and therefore
  `plugin/src/editor.rs`'s RPC handler — on a stack that is already inside
  baseview's window procedure with the `RefCell` borrow live. Harmless until
  something on that path runs a modal loop, and `DoDragDrop` is exactly that: it
  dispatched a `WM_TIMER` back into the procedure and **the standalone aborted on
  every drag**. So `on_frame` now calls `windows_pump::request`, which does
  nothing but `PostMessageW` to a 0×0 invisible `WS_CHILD` window of the editor;
  `baseview`'s own `open_blocking` loop retrieves that post — because
  `GetMessageW(&mut msg, hwnd, 0, 0)` returns messages for `hwnd` **and its
  children** — and dispatches it from *outside* the window procedure, where
  `drain` then runs with nothing borrowed.
  - ⚠ **A message-only (`HWND_MESSAGE`) window is the obvious shape and it is
    wrong**, which is worth knowing before "simplifying" this. It is not in the
    editor's subtree, so the filtered `GetMessageW` never retrieves it: the post
    would either sit unread forever or come back to `drain` — the very stack
    being escaped. The parenting is the mechanism, not a detail.
  - ⚠ `request` falls back to draining in place when the child window cannot be
    created. That reinstates the old abort risk, and it is the right trade: no
    pump at all is a blank window every time, where the abort needs the producer
    to start a drag.
  - ✅ This is what let `drag/windows.rs` set `STANDALONE_SAFE = true`, which is
    the one-line revert if a standalone drag ever aborts again.
- **The pump also revokes `baseview`'s OLE drop target (TASK-063D).** Moving the
  pump fixed the abort and revealed a *second*, unrelated crash underneath it:
  every standalone drag died with `STATUS_ACCESS_VIOLATION` — no panic, no
  message. The captured stack:

  ```text
  DragQueryFileW
  baseview::win::drop_target::DropTarget::parse_drop_data   drop_target.rs:140
  baseview::win::drop_target::DropTarget::drag_enter        drop_target.rs:209
  DoDragDrop
  freally_midi_master_plugin::drag::platform::drag
  ```

  A drag starts with the cursor over the window it came from, so `DoDragDrop`
  calls `IDropTarget::DragEnter` on **us** before anything else, and `baseview`
  registers a drop target on every window it opens (`win/window.rs:764`). Its
  parser does `*(*medium.u).hGlobal()` — dereferencing the `STGMEDIUM` union
  twice — which yields the *data pointer* of a movable block where
  `DragQueryFileW` needs the *handle*; `GlobalLock` of that is `NULL` and the
  shell faults. ⛔ **It is `baseview`'s bug and no allocation choice on the
  source side repairs it**: with `GMEM_FIXED` the same double-dereference reads
  `pFiles` and hands *that* to the shell instead. Our `DROPFILES` is well-formed
  and Ableton accepts it.
  - So `windows_pump::stop_being_a_drop_target` calls `RevokeDragDrop` once, on
    the window `on_frame` handed it. ⚠ **In the pump and not in the drag code:**
    an earlier cut revoked from `drag/windows.rs` using a handle fetched across
    the seam, it came back null, and the crash was unchanged. The pump has the
    real handle. It logs either way — a silent failure here is a crash later.
  - ⚠ Nothing consumes drops in this application (`DropData` appears once, as a
    `pub use`), so it is not restored afterwards — which also avoids needing the
    undocumented `OleDropTargetInterface` window property to get it back.
  - ⛔ Standalone only, because `request` returns early when the pump is off. A
    host's drop target must never be revoked.
- **`top_level` is now `host_frame`, and it is exactly ONE level: `GetParent`
  (2026-08-11).** This has been wrong **twice, in opposite directions**, and both
  belong in the record:
  - **A `GetParent` loop to "parent is null"** climbed out of the plugin
    entirely. `GetParent` answers *"parent **or owner**"*, and a host's floating
    plugin window is *owned* by its main window — so the loop returned **the
    DAW's own application frame**, and `fill_frame` bounded the webview to that.
    ▶ Mike on the Ableton VST3: *"it looks like the GUI size stretched and got
    bigger, but the actual size of the GUI's part did not, so it zoomed in."*
  - **`GetAncestor(GA_ROOT)`** fixed Ableton and was still wrong for **FL
    Studio**, which *docks* plugin editors inside its own window. There the root
    genuinely **is** FL's main frame, so the webview would be bounded to the
    whole DAW — the black square and torn arrangement view Mike screenshotted.
  - ▶ **`baseview::Window::open_parented` puts the editor directly inside the
    handle the host gave us**, so its immediate parent *is* that container in
    every case: the floating window in Ableton, the docked panel in FL,
    `nih_plug`'s wrapper frame in the standalone. There is no case where the
    right answer is further up, and every case where it is further up is one
    where we are measuring somebody else's window.
  - ⚠ It also makes the caption rename safe in FL for free: a docked panel has no
    title, `GetWindowTextW` returns nothing, and `retitle` bails before it can
    touch anything.
- ⛔⛔ **`fill_frame` MUST keep running in hosts, and gating it to the standalone
  blanked the Ableton VST3.** It was gated for exactly one build, on the
  reasoning that a host resizes through `IPlugView::onSize` → baseview →
  `Event::Window(Resized)` and therefore needs no polling. **That reasoning was
  wrong and Mike's screenshot settled it in one frame: a white, empty plugin
  window.** Whatever the theory says, `fill_frame` is what actually sizes the
  webview in Ableton — without it the webview kept its creation bounds, covered
  none of the window, and the host's own background showed through.
  - ⚠ **The lesson is the shape of the mistake, not the flag.** The defect was in
    `top_level`; the gate treated the symptom by deleting the feature, and it was
    reasoned about rather than observed because no DAW was available to check it
    in. Anything in this module that looks like standalone-only surgery should be
    *tested* in a host before it is fenced off from one.
- **`WebViewEditor::with_window_title` and `windows_pump::retitle` (new,
  2026-08-11).** Mike: *"can you replace the window's title bar after the vst3/
  clap file opens … so it just says it once?"* Ableton auto-names a fresh track
  after the instrument dropped on it and then builds the plugin window's caption
  from the device **and** the track, so a long name lands on both sides of a
  slash: `Freally MIDI Master By: Mike Weaver/1-Freally MIDI Master By: Mike
  Weaver`. How a host joins those is not ours to change; what the caption ends up
  saying is, so the frame loop overwrites it.
  - ⛔⛔ **THE GUARD IS THE WHOLE FEATURE: only a window whose caption ALREADY
    CONTAINS the plugin's name is renamed.** A plugin editor is not always in a
    window of its own — **FL Studio docks them**, and there `top_level` is FL's
    main application frame. Renaming that would retitle the whole of FL Studio
    from inside a plugin. A caption that already names us was built by the host
    *for this plugin*; `FL Studio 21 - project.flp` was not, and is left alone.
  - ⚠ Re-checked about twice a second rather than done once, because Ableton
    rewrites the caption whenever the track is renamed. It early-returns as soon
    as the caption is already right, so the steady state is one `WM_GETTEXT`.
  - ⚠ Safe from `on_frame`'s borrowed stack for the same reason `set_bounds` is:
    the messages go to the **host's** window procedure, never to baseview's.

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
   panicking on threads the host owns.
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

### ✅ FIXED 2026-07-29 — the diagnosis below stands, and the fix is `windows_pump`

`npm run plugin:standalone` renders. `node scripts/screenshot-plugin.mjs
screenshots/windows-plugin.png target/debug/standalone.exe` captures 1240x804 with
**332 distinct colours**, and that gate refuses a flat window, so the pass is real.

Two things the fix taught that the diagnosis did not predict:

1. **Draining the whole queue aborts the process.** `on_frame` runs *inside*
   baseview's window procedure, not between messages, so dispatching anything back
   to that same window re-enters a live `RefCell` borrow. The pump must skip the
   editor window and its children — which is also simply correct, because those are
   precisely the messages baseview's filtered `GetMessageW` *can* already retrieve.
2. **The `cpal`/WASAPI panic you will hit next is unrelated.** `Received 1056
   samples, while the configured buffer size is 512` comes from nih-plug's own
   standalone backend on the `cpal_wasapi_out` thread and has nothing to do with the
   editor. `--backend dummy` (what the screenshot gate uses) or a matching
   `--period-size` avoids it.

The diagnosis that got us here is kept below, because it is the reasoning that will
be needed again if a rebase moves any of this.

### ⛔ THE WINDOWS *STANDALONE* OPENED A BLANK WINDOW, AND IT WAS THE MESSAGE PUMP

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

*Established 2026-07-29 by `FREALLY_TRACE_EDITOR=1`, which is now the fastest way
back to this:* **not one WebView2 event fires.** With navigation, page-load and
document-title handlers all attached, the log is completely empty — no
`navigation requested`, no `page started`, nothing. Mike also reports it **ran a
few days earlier**, so this is a change in state or environment rather than in
this code.

Three handlers silent at once rules out the protocol, the URL and the page: those
would each fail *differently*. A webview whose every event is missing is a webview
whose events are not being delivered.

### ✅ ROOT CAUSE FOUND, 2026-07-29. IT IS A MESSAGE FILTER, NOT A MISSING PUMP.

**`baseview`'s `open_blocking` — `src/win/window.rs:615` — pumps with an `hwnd`
filter:**

```rust
let status = GetMessageW(&mut msg, hwnd, 0, 0);
//                                 ^^^^ not null_mut()
```

Win32: when `hWnd` is non-NULL, `GetMessage` retrieves **only messages for that
window and its children**, and **thread messages (`msg.hwnd == NULL`) are never
retrieved at all**. WebView2 is COM/STA, and an STA delivers cross-apartment
completions through a hidden message-only window COM owns
(`OleMainThreadWndClass`) plus posted thread messages. **That window is not our
`hwnd` and not a child of it.** So the loop runs forever, faithfully, and every
WebView2 callback sits in the queue unretrieved.

So the earlier note below is half right and half wrong, and the wrong half matters:
**the thread is pumping.** It is pumping the wrong subset. Anyone who adds "a pump"
without removing the filter will add a second loop to a thread that already has one
— which is exactly what the note below warned about, for the right reason.

**The proof, and it is a clean two-class experiment.** With
`FREALLY_TRACE_EDITOR=1` the adapter now counts `on_frame`:

```
[editor] on_frame #0 … #60 … #120 … #300      <- arriving, ~60fps
(no navigation / page / title events at all)  <- never arriving
```

`on_frame` is driven by a `WM_TIMER` **to the editor's own child HWND** — a child of
`hwnd`, so the filter passes it. The WebView2 events go to a window that is not.
**One class arrives and the other does not, on the same thread, in the same loop.**
That is a filter, and nothing else produces that split.

**Two `baseview` crates are in this build, which is worth knowing before fixing it:**

| rev | version | pulled in by | role here |
|-----|---------|--------------|-----------|
| `579130e` | 0.1.0 | `nih_plug` | owns the standalone's `open_blocking` loop — **the filtered one** |
| `91e3b4a` | 0.1.4 | this vendored adapter | creates the editor's child window |

They interoperate only through raw window handles, so this is not a type conflict —
but the loop and the window belong to *different crates*, which is why the bug reads
as nobody's fault from inside either one.

### ⛔ HOW TO FIX IT — AND THE TRAP IN THE OBVIOUS FIX

`on_frame` fires in the standalone (proven above), so **the pump can live in this
adapter** and no new fork is needed. Drain the queue with
`PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE)` — `null_mut()` is the whole
point — then `TranslateMessage` + `DispatchMessageW`.

**Two things will bite whoever writes it:**

1. **⛔ IT MUST NOT RUN INSIDE A DAW.** In Ableton or FL Studio the *host* owns that
   thread's queue. Draining it from our frame handler would steal the host's own
   messages and break it — the "takes the DAW down" failure this project has already
   been bitten by once. Gate it on being the standalone explicitly (the standalone
   binary is ours — have it say so), **not** on "no frame ticks arrive in Ableton",
   which is a quirk and not a guarantee.
2. **Guard against reentrancy.** `on_frame` is itself called from a dispatched
   message. Pumping inside it can dispatch another `WM_TIMER` and recurse into
   `on_frame`. Hold a thread-local "already pumping" flag and return early.

The alternative — forking `baseview` to change one argument at the true root — fixes
it for everyone and cannot touch the DAW path at all, since `open_blocking` is only
ever used by the standalone. It is the cleaner fix and the more honest one. It costs
a **third** fork to carry, which is the only reason it is not the recommendation.

---

*The original 2026-07-29 inference, kept because its warning was correct:* the cause
is the **Windows message loop** —
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
