//! The command bridge between the webview UI and the plugin.
//!
//! The desktop app called Tauri's `invoke`; `src/lib/ipc.ts` was always the one
//! seam, and this is what sits behind it now. Same command names, same
//! payloads, so the React app does not have to know which shell it is in.
//!
//! One rule carried over from `ipc-mock`, and it matters more here: **an
//! unknown command is a loud failure, never a silent `undefined`.** A bridge
//! that quietly answers everything hides exactly the wiring bugs this layer
//! exists to expose.

use engine::context::{SessionDefaults, SessionOverrides};
use engine::generators::{chords, drums};
use engine::humanize::humanize;
use engine::pattern::{Part, Pattern, PPQ};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::dataset;
use crate::host::HostSession;
use crate::presets;
use crate::state::{self, PluginSession, SessionStore};

/// A call from the webview.
#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    /// Correlates the reply. The UI awaits a promise keyed by this.
    pub id: u64,
    pub command: String,
    #[serde(default)]
    pub args: Value,
}

/// The longest pattern the plugin will generate.
///
/// Well above the 8 the UI offers, so it never binds in normal use; it exists
/// so a value from a file, a preset or devtools cannot ask for a pattern that
/// takes minutes to build on the thread the host draws its window from.
const MAX_BARS: u16 = 128;

/// What the UI asks for when the user presses Generate.
///
/// The same shape as the desktop app's `GenerateRequest`, because it is the
/// same frontend sending it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerateArgs {
    style_id: String,
    #[serde(default)]
    part: Option<Part>,
    #[serde(default)]
    session: Option<SessionOverrides>,
    #[serde(default)]
    bars: Option<u16>,
    #[serde(default)]
    seed: Option<String>,
}

/// Answer one call.
///
/// Returns the value the promise resolves with, or the message it rejects
/// with. `host` is what the DAW last reported, so a generation started from
/// the UI is placed in the project's own tempo and meter without the UI ever
/// having to know the tempo. `session` is what the DAW will save with the
/// project — see [`crate::state`].
pub fn dispatch(
    request: &Request,
    host: &HostSession,
    session: &SessionStore,
) -> Result<Value, String> {
    match request.command.as_str() {
        // ---- The licence gate. See [`crate::eula`] ------------------------
        "eula_status" => serde_json::to_value(crate::eula::status()).map_err(|e| e.to_string()),

        "eula_accept" => crate::eula::accept().map(|()| Value::Null),

        "eula_decline" => crate::eula::decline().map(|()| Value::Null),

        "roster_summary" => {
            serde_json::to_value(&dataset::loaded().summary).map_err(|e| e.to_string())
        }

        "resolve_model" => {
            let id = request.args["id"].as_str().unwrap_or_default();
            serde_json::to_value(dataset::model(id)?).map_err(|e| e.to_string())
        }

        "session_defaults" => {
            let id = request.args["styleId"].as_str().unwrap_or_default();
            let mut defaults = SessionDefaults::of(&dataset::model(id)?);
            // The chip must show what pressing Generate *would* do, and inside
            // a host that is the host's tempo rather than the model's. Showing
            // the authored 140 next to a beat that will come out at 92 is the
            // readout-that-lies failure TASK-033 exists to prevent.
            //
            // ⛔ Which is exactly why this now asks the toggle (TASK-P15). With
            // auto-sync off the beat comes out at the *model's* tempo, so showing
            // the host's would be the same lie in the opposite direction.
            let auto_sync = state::with(session, |s| s.auto_sync).unwrap_or(true);
            if auto_sync {
                if let Some(tempo) = host.tempo() {
                    defaults.bpm = tempo as f32;
                }
            }
            serde_json::to_value(defaults).map_err(|e| e.to_string())
        }

        "generate_pattern" => {
            let args: GenerateArgs = serde_json::from_value(request.args["request"].clone())
                .map_err(|e| format!("bad generate request: {e}"))?;
            // Auto-sync is a *session* setting, so it is read from the store
            // rather than sent with the request: the page already saves it there
            // and two copies of one switch is how they start disagreeing.
            let auto_sync = state::with(session, |s| s.auto_sync).unwrap_or(true);
            serde_json::to_value(generate(&args, host, auto_sync)?).map_err(|e| e.to_string())
        }

        // The host reports these; the UI shows them. Its own command so the
        // chips can refresh on a tempo change without regenerating.
        "host_session" => Ok(json!({
            "tempo": host.tempo(),
            "timeSigNum": host.time_signature().0,
            "timeSigDen": host.time_signature().1,
            "playing": host.playing(),
        })),

        // ---- Session state, saved with the project by the host -------------
        //
        // The UI asks for this once when the editor opens and writes it back
        // whenever the user changes something. There is no file and no path:
        // the value lives in the plugin's persisted params, and the DAW
        // decides when to write it out.
        // Serialized in place rather than out of a clone: `to_value` only needs
        // a reference, and the clone would be dropped on the next line.
        "session_state" => state::with(session, |s| serde_json::to_value(s))
            .unwrap_or_else(|| serde_json::to_value(PluginSession::default()))
            .map_err(|e| e.to_string()),

        // Deliberately replaces the whole session rather than patching a field.
        // A partial update needs the two sides to agree on which fields were
        // *meant* to be absent, and `SessionOverrides` already uses absence to
        // mean "the artist chooses" — so a patch protocol would make "unpin
        // the tempo" and "do not mention the tempo" the same message.
        "save_session_state" => {
            let mut next: PluginSession = serde_json::from_value(request.args["session"].clone())
                .map_err(|e| format!("bad session state: {e}"))?;

            // `window_size` is the *editor's*, not the UI's — `set_editor_size`
            // is what sets it. A whole-session write that did not carry it
            // would silently reset the window every time the user changed an
            // artist, and the project would reopen at the wrong size with
            // nothing to explain why.
            if next.window_size.is_none() {
                next.window_size = state::read(session).window_size;
            }

            state::write(session, next);
            Ok(Value::Null)
        }

        // Presets: the same session, named, kept outside any one project.
        // See [`crate::presets`] — the plugin owns these, with its own UI;
        // CLAP's preset-discovery factory is deliberately not used.
        "presets_list" => serde_json::to_value(presets::list()).map_err(|e| e.to_string()),

        // Saves what is stored, not what the args carry. The session store is
        // already what the host persists, so a preset is a copy of it — and
        // taking it from the args instead would let the two disagree about what
        // "the current session" is.
        "preset_save" => {
            let name = request.args["name"].as_str().unwrap_or_default();
            serde_json::to_value(presets::save(name, state::read(session))?)
                .map_err(|e| e.to_string())
        }

        // Answers with the session rather than applying it. The UI owns the
        // session store and the generate flow; handing the value back lets it
        // apply the preset through the same path a user typing into the chips
        // takes, instead of a second one that could drift from it.
        "preset_load" => {
            let id = request.args["id"].as_str().unwrap_or_default();
            serde_json::to_value(presets::load(id)?).map_err(|e| e.to_string())
        }

        "preset_delete" => {
            let id = request.args["id"].as_str().unwrap_or_default();
            presets::delete(id).map(|()| Value::Null)
        }

        "app_info" => Ok(json!({
            "version": env!("CARGO_PKG_VERSION"),
            "platform": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        })),

        // Playback belongs to the host now. Answering with a reason rather
        // than an error keeps the transport honestly disabled, which is the
        // shape the UI already handles (`playbackFailure`).
        "playback_status" => Ok(Value::String(
            "Press play in your DAW — the plugin puts the notes on the track.".into(),
        )),

        other => Err(format!(
            "the plugin has no handler for `{other}`. Add one in \
             plugin/src/bridge.rs — answering silently would hide the wiring \
             bug this layer exists to expose."
        )),
    }
}

/// Build a pattern for the UI's request, in the host's session.
///
/// The same three calls in the same order as the desktop app's `render` and
/// `engine/tests/golden.rs`: generate on the grid, humanize, then hand back.
/// A change here that is not a change there is a change nobody meant.
fn generate(args: &GenerateArgs, host: &HostSession, auto_sync: bool) -> Result<Pattern, String> {
    let part = args.part.unwrap_or(Part::Drums);
    if !matches!(part, Part::Drums | Part::Chords) {
        return Err(format!(
            "the {part:?} generator is not implemented yet — only drums and chords are"
        ));
    }

    let seed = match &args.seed {
        Some(text) if !text.is_empty() => text
            .parse::<u64>()
            .map_err(|_| format!("`{text}` is not a seed"))?,
        // The one place system entropy is allowed: the *choice* of seed is not
        // part of generation, and everything downstream of it is reproducible
        // from the value.
        _ => fresh_seed(),
    };

    let model = dataset::model(&args.style_id)?;

    let mut overrides = args.session.clone().unwrap_or_default();
    if let Some(bars) = args.bars {
        overrides.bars = Some(bars.clamp(1, MAX_BARS));
    }
    // ⛔ Also clamp what arrived inside `session`. Both paths reach the engine,
    // and generation runs synchronously on the host's UI thread — an unbounded
    // `bars` from a preset, a restored project or devtools is a multi-minute
    // freeze of somebody's DAW, not a big pattern.
    if let Some(bars) = overrides.bars {
        overrides.bars = Some(bars.clamp(1, MAX_BARS));
    }

    let ctx = host.session_for(&model, &overrides, seed, auto_sync);
    let mut lanes = match part {
        Part::Chords => vec![chords::generate(&model, &ctx, seed).track],
        _ => drums::generate(&model, &ctx, seed),
    };
    humanize(&mut lanes, &ctx, seed);

    if lanes.iter().all(|lane| lane.notes.is_empty()) {
        return Err(format!(
            "{} has no {part:?} part authored — nothing to generate",
            model.name
        ));
    }

    Ok(Pattern {
        id: format!("{}-{seed}", model.id),
        part,
        artist_id: model.id.clone(),
        seed,
        bars: ctx.bars,
        bpm: ctx.bpm,
        time_sig_num: ctx.time_sig_num,
        time_sig_den: ctx.time_sig_den,
        key_root: ctx.key_root,
        scale: ctx.scale,
        lanes,
        ppq: PPQ,
    })
}

/// A seed with no dependency on the engine's own randomness.
///
/// The engine forbids system entropy inside a generator, and rightly — but a
/// *fresh seed* is the one thing that must not be reproducible, or every user
/// pressing Generate for the first time would get the same beat.
fn fresh_seed() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Guarantees distinctness *within* a process, which neither of the other
    /// two ingredients does.
    ///
    /// The first version of this used only the clock and a heap address, and
    /// `fresh_seeds_do_not_collide` caught it immediately: 64 calls produced
    /// 61 distinct values. Windows' clock granularity is around 100 ns, so a
    /// loop repeats it, and an allocator hands back the address it just freed
    /// — so the two "independent" ingredients collide together.
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15);

    // The clock and the address separate two *processes* started at the same
    // moment — which two plugin instances on one track genuinely are.
    let boxed = Box::new(0u8);
    let address = Box::into_raw(boxed) as u64;
    // Hand the allocation straight back; it was only ever wanted for its
    // address.
    drop(unsafe { Box::from_raw(address as *mut u8) });

    let counter = COUNTER
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15);

    // SplitMix64's finalizer, so neighbouring counter values do not produce
    // neighbouring seeds — a seed is shown to the user and copied, and
    // consecutive generations reading 1, 2, 3 would look broken.
    let mut z = nanos ^ address.rotate_left(17) ^ counter;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(command: &str, args: Value) -> Request {
        Request {
            id: 1,
            command: command.into(),
            args,
        }
    }

    fn host() -> HostSession {
        HostSession::observed_for_test(Some(92.0), 4, 4)
    }

    /// The cases below predate session state and say nothing about it, so they
    /// get a throwaway store. The session commands use [`super::dispatch`]
    /// directly with one they can inspect.
    fn dispatch(request: &Request, host: &HostSession) -> Result<Value, String> {
        super::dispatch(request, host, &SessionStore::default())
    }

    #[test]
    fn the_roster_reaches_the_ui() {
        let value = dispatch(&request("roster_summary", json!({})), &host()).unwrap();
        let entries = value["entries"].as_array().unwrap();
        assert!(entries.iter().any(|e| e["id"] == "trap"));
        assert_eq!(value["problems"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn an_unknown_command_fails_loudly() {
        // The rule `ipc-mock` set and this inherits: a bridge that silently
        // answers everything hides the wiring bugs it exists to expose.
        let err = dispatch(&request("no_such_command", json!({})), &host()).unwrap_err();
        assert!(err.contains("no_such_command"), "{err}");
        assert!(err.contains("bridge.rs"), "{err}");
    }

    #[test]
    fn generating_uses_the_hosts_tempo() {
        // The whole pivot, through the path the UI actually takes.
        let value = dispatch(
            &request(
                "generate_pattern",
                json!({ "request": { "styleId": "trap", "bars": 4, "seed": "7" } }),
            ),
            &host(),
        )
        .unwrap();

        assert_eq!(value["bpm"], 92.0);
        assert_eq!(value["artistId"], "trap");
        assert_eq!(value["seed"], "7");
    }

    #[test]
    fn the_defaults_chip_shows_the_hosts_tempo_not_the_models() {
        // trap authors 140. Inside a 92 BPM project the chip must say 92, or
        // it is promising a beat the plugin will not produce.
        let value = dispatch(
            &request("session_defaults", json!({ "styleId": "trap" })),
            &host(),
        )
        .unwrap();
        assert_eq!(value["bpm"], 92.0);

        // ...and with no host tempo yet, the model's own value stands.
        let silent = HostSession::observed_for_test(None, 4, 4);
        let value = dispatch(
            &request("session_defaults", json!({ "styleId": "trap" })),
            &silent,
        )
        .unwrap();
        assert_eq!(value["bpm"], 140.0);
    }

    #[test]
    fn a_pinned_tempo_survives_the_bridge() {
        let value = dispatch(
            &request(
                "generate_pattern",
                json!({ "request": {
                    "styleId": "trap",
                    "seed": "7",
                    "session": { "bpm": 150.0, "keyRoot": null, "scale": null, "swing": null, "bars": null, "halfTime": null }
                }}),
            ),
            &host(),
        )
        .unwrap();
        assert_eq!(value["bpm"], 150.0, "the user's pin must beat the host");
    }

    #[test]
    fn the_same_seed_reproduces_the_same_pattern_through_the_bridge() {
        let call = || {
            dispatch(
                &request(
                    "generate_pattern",
                    json!({ "request": { "styleId": "trap", "seed": "2024" } }),
                ),
                &host(),
            )
            .unwrap()
        };
        assert_eq!(call(), call());
    }

    #[test]
    fn an_absent_seed_is_fresh_each_time() {
        let call = || {
            dispatch(
                &request(
                    "generate_pattern",
                    json!({ "request": { "styleId": "trap" } }),
                ),
                &host(),
            )
            .unwrap()["seed"]
                .clone()
        };
        assert_ne!(call(), call(), "a fresh generation should be fresh");
    }

    #[test]
    fn a_bad_seed_is_refused_rather_than_silently_replaced() {
        let err = dispatch(
            &request(
                "generate_pattern",
                json!({ "request": { "styleId": "trap", "seed": "not-a-number" } }),
            ),
            &host(),
        )
        .unwrap_err();
        assert!(err.contains("not a seed"), "{err}");
    }

    #[test]
    fn the_host_session_is_readable_on_its_own() {
        // So a tempo change can refresh the chips without regenerating.
        let value = dispatch(&request("host_session", json!({})), &host()).unwrap();
        assert_eq!(value["tempo"], 92.0);
        assert_eq!(value["timeSigNum"], 4);
    }

    #[test]
    fn playback_says_the_host_owns_it_rather_than_failing() {
        let value = dispatch(&request("playback_status", json!({})), &host()).unwrap();
        assert!(value.as_str().unwrap().contains("DAW"));
    }

    #[test]
    fn a_saved_session_reads_back_through_the_bridge() {
        // The round trip a reopened project makes: the UI writes what the user
        // chose, the host persists the store behind it, and the UI asks for it
        // again when the editor next opens.
        let store = SessionStore::default();

        let saved = super::dispatch(
            &request(
                "save_session_state",
                json!({ "session": {
                    "selectedId": "uk-drill",
                    "seed": "2024",
                    "bars": 8,
                    "pins": { "bpm": 150.0, "keyRoot": 3, "scale": null, "swing": null }
                }}),
            ),
            &host(),
            &store,
        );
        assert!(saved.is_ok(), "{saved:?}");

        let value = super::dispatch(&request("session_state", json!({})), &host(), &store).unwrap();

        assert_eq!(value["selectedId"], "uk-drill");
        assert_eq!(value["seed"], "2024");
        assert_eq!(value["bars"], 8);
        assert_eq!(value["pins"]["bpm"], 150.0);
    }

    #[test]
    fn an_unsaved_session_reads_as_empty_rather_than_failing() {
        // A plugin inserted on a fresh track has no state, and that is not an
        // error — the UI has to be able to tell "nothing saved" from "the
        // bridge is broken".
        let value = dispatch(&request("session_state", json!({})), &host()).unwrap();
        assert_eq!(value["selectedId"], Value::Null);
        assert_eq!(value["seed"], "");
    }

    #[test]
    fn a_malformed_session_is_refused_rather_than_silently_emptied() {
        // Writing junk must not clear a good session. Refusing loudly is what
        // lets a wiring bug in the UI be found, rather than presenting as a
        // project that quietly forgets what the user picked.
        let store = SessionStore::default();
        state::write(
            &store,
            PluginSession {
                selected_id: Some("trap".into()),
                ..PluginSession::default()
            },
        );

        let err = super::dispatch(
            &request(
                "save_session_state",
                json!({ "session": { "bars": "eight" } }),
            ),
            &host(),
            &store,
        )
        .unwrap_err();
        assert!(err.contains("bad session state"), "{err}");

        assert_eq!(
            state::read(&store).selected_id.as_deref(),
            Some("trap"),
            "a rejected write must leave the stored session alone"
        );
    }

    #[test]
    fn saving_the_session_does_not_forget_the_window_size() {
        // The UI writes the whole session on every change and knows nothing
        // about the window. Without the carry-over, picking an artist would
        // reset the size, and the project would reopen wrong with nothing to
        // explain it.
        let store = SessionStore::default();
        state::write(
            &store,
            PluginSession {
                window_size: Some("small".into()),
                ..PluginSession::default()
            },
        );

        super::dispatch(
            &request(
                "save_session_state",
                json!({ "session": { "selectedId": "trap", "seed": "7" } }),
            ),
            &host(),
            &store,
        )
        .unwrap();

        let saved = state::read(&store);
        assert_eq!(saved.selected_id.as_deref(), Some("trap"));
        assert_eq!(saved.window_size.as_deref(), Some("small"));
    }

    #[test]
    fn an_explicit_window_size_still_wins() {
        // The carry-over must not make the field unwritable.
        let store = SessionStore::default();
        state::write(
            &store,
            PluginSession {
                window_size: Some("small".into()),
                ..PluginSession::default()
            },
        );

        super::dispatch(
            &request(
                "save_session_state",
                json!({ "session": { "windowSize": "large" } }),
            ),
            &host(),
            &store,
        )
        .unwrap();

        assert_eq!(state::read(&store).window_size.as_deref(), Some("large"));
    }

    #[test]
    fn the_saved_seed_regenerates_the_pattern_it_was_saved_with() {
        // Why persisting the *inputs* is enough: restore is a regeneration, and
        // the engine is deterministic. If this ever stops holding, a reopened
        // project comes back on a different beat.
        let store = SessionStore::default();
        super::dispatch(
            &request(
                "save_session_state",
                json!({ "session": { "selectedId": "trap", "seed": "2024" } }),
            ),
            &host(),
            &store,
        )
        .unwrap();

        let restored = state::read(&store);
        let regenerate = || {
            super::dispatch(
                &request(
                    "generate_pattern",
                    json!({ "request": {
                        "styleId": restored.selected_id.clone().unwrap(),
                        "seed": restored.seed,
                    }}),
                ),
                &host(),
                &store,
            )
            .unwrap()
        };

        assert_eq!(regenerate(), regenerate());
        assert_eq!(regenerate()["seed"], "2024");
    }

    #[test]
    fn fresh_seeds_do_not_collide() {
        // 4096 in a tight loop, well past what the clock alone can separate.
        // The first version of `fresh_seed` used only the clock and a heap
        // address and produced 61 distinct values out of 64.
        const N: usize = 4096;
        let seeds: std::collections::BTreeSet<u64> = (0..N).map(|_| fresh_seed()).collect();
        assert_eq!(
            seeds.len(),
            N,
            "two generations would produce the same beat"
        );
    }
}
