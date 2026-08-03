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
use engine::dataset::modes;
use engine::generators::bass;
use engine::parts;
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
    /// The mood to generate in. Absent is "Any" — see [`generate`].
    #[serde(default)]
    mood: Option<String>,
    /// Which of the model's authored song forms to build (TASK-070).
    ///
    /// Absent is "the artist chooses", sampled from the weights — the same
    /// meaning absence carries everywhere else here. Read only by
    /// `generate_song`; a single pattern has no form.
    #[serde(default)]
    structure: Option<usize>,
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
    exports: &crate::export::Exports,
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
            // ⛔ **Through the pinned mood, because `generate` applies the mode
            // before it builds the context** — and a mode retunes the session
            // block. Trap is 140, its `dark` mode 136 and `bounce` 148, so
            // reading the bare model made the chip name a tempo the engine was
            // never going to use, which is the readout-that-lies failure the
            // chip exists to prevent.
            //
            // ⚠ Only a *pinned* mood can be resolved here. On "Any" the mode is
            // picked from the seed at generation time, and an empty seed box
            // means the seed does not exist yet — so the chip shows the model's
            // own tempo, which is the honest answer to "what will it be" when
            // the thing that decides has not happened.
            let model = dataset::model(id)?;
            let model = match state::with(session, |s| s.mood.clone()).flatten() {
                Some(mood) => modes::apply(&model, &mood).unwrap_or(model),
                None => model,
            };
            let mut defaults = SessionDefaults::of(&model);
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

        // Which semitones a scale contains, for the roll's row tinting and
        // folding (TASK-041B).
        //
        // ⛔ **Asked rather than duplicated.** The alternative was a second
        // interval table in TypeScript, and there are forty-one scales — a copy
        // that drifts from `theory::scale_semitones` would tint the wrong rows
        // and fold away notes that are in the key, which looks like the *engine*
        // generating out of key. The page caches the answer per scale, so this
        // is one round trip per scale change and never one per frame.
        "scale_pitches" => {
            let scale: engine::pattern::Scale =
                engine::pattern::Scale::deserialize(&request.args["scale"])
                    .map_err(|e| format!("unknown scale: {e}"))?;
            Ok(json!(engine::theory::scale_semitones(scale)))
        }

        // An edited clip, on its way back to the audio thread (TASK-041).
        //
        // ⛔ **The reply is the pattern, and that is what does the work.**
        // `editor.rs` arms whatever a command answers with when it deserializes
        // as a `Pattern`, so this command's whole job is to *validate* — a set
        // of notes that does not round-trip through the engine's own type is
        // refused here rather than reaching `Schedule::arm`.
        //
        // ⛔ **The plugin does not store it.** `state.rs` persists a session's
        // inputs, not its notes, and the page owns the edited document until the
        // materialisation decision is built (see the roadmap, above Phase 4).
        // Arming is a *playback* concern and is all this needs to do.
        "arm_pattern" => {
            let pattern: Pattern = Pattern::deserialize(&request.args["pattern"])
                .map_err(|e| format!("bad pattern: {e}"))?;
            serde_json::to_value(pattern).map_err(|e| e.to_string())
        }

        "generate_pattern" => {
            let args: GenerateArgs = Deserialize::deserialize(&request.args["request"])
                .map_err(|e| format!("bad generate request: {e}"))?;
            // Auto-sync is a *session* setting, so it is read from the store
            // rather than sent with the request: the page already saves it there
            // and two copies of one switch is how they start disagreeing.
            let auto_sync = state::with(session, |s| s.auto_sync).unwrap_or(true);
            serde_json::to_value(generate(&args, host, auto_sync)?).map_err(|e| e.to_string())
        }

        // ---- Song Mode (TASK-065) ------------------------------------------
        //
        // The same request shape as `generate_pattern` minus `part`, because a
        // song is every part at once. Answers with the whole `Song` — sections,
        // their geometry, and the pattern store the refs resolve into — so the
        // timeline can draw it without a second round trip per clip.
        "generate_song" => {
            let args: GenerateArgs = Deserialize::deserialize(&request.args["request"])
                .map_err(|e| format!("bad generate request: {e}"))?;
            let auto_sync = state::with(session, |s| s.auto_sync).unwrap_or(true);
            serde_json::to_value(generate_song(&args, host, auto_sync)?).map_err(|e| e.to_string())
        }

        // Write the whole arranged song to a file the producer picks (TASK-073).
        //
        // ⛔ **Starts an export and returns; it does not wait for the dialog.**
        // A native Save As is modal, and this handler runs on a frame the page
        // is waiting on — inside a host, the DAW's own editor thread. See
        // `crate::export` for the whole reasoning; it is the same
        // "takes the DAW down" shape this codebase has been bitten by twice.
        "export_song" => {
            let song: engine::pattern::Song =
                engine::pattern::Song::deserialize(&request.args["song"])
                    .map_err(|e| format!("bad song: {e}"))?;
            check_refs(&song)?;
            // The same trust boundary `song_smf` documents, and it has to be
            // here too: this path reaches `song_to_smf` with a `Song` that came
            // from the webview.
            check_song(&song)?;
            let suggested = format!("{}-{}.mid", song.artist_id, song.seed);
            exports
                .start_song_midi(song, &suggested)
                .map(|()| Value::Null)
        }

        // One file per part, into a folder the producer picks (TASK-069).
        //
        // ⚠ **MIDI stems, not audio stems, and `export.rs` explains why in
        // full**: the preview kit is a drum kit, so the four melodic parts have
        // no voice to render through yet (FMM-N15/FMM-N16). Writing four silent
        // wavs and calling them stems would be worse than not offering them.
        "export_stems" => {
            let song: engine::pattern::Song =
                engine::pattern::Song::deserialize(&request.args["song"])
                    .map_err(|e| format!("bad song: {e}"))?;
            // ⛔ **The same refusal `export_song` makes, and leaving it out was
            // an inconsistency rather than a shortcut.** A dangling reference
            // draws as an empty row and writes as silence — so without this, a
            // stem folder came back looking complete with one part quietly
            // empty, which is exactly the failure the check exists to name.
            check_refs(&song)?;
            check_song(&song)?;
            let folder = format!("{}-{}-stems", song.artist_id, song.seed);
            exports
                .start_song_stems(song, &folder)
                .map(|()| Value::Null)
        }

        // How the export that is running ended, if it has.
        //
        // ⚠ Polled, and the outcome is *taken* — see `export::take_status`. A
        // terminal status left in place would re-announce the same successful
        // export on every tick.
        "export_status" => serde_json::to_value(exports.take_status()).map_err(|e| e.to_string()),

        // The forms this artist writes, for the structure picker (TASK-070).
        //
        // ⚠ Its own command rather than a field on `session_defaults`, because
        // the two answer different questions and are asked at different times:
        // the chips need defaults the moment an artist is picked, and the forms
        // are only wanted once somebody opens the Song tab. A model with no
        // `arrangement` block answers with an error rather than an empty list —
        // "this artist writes no songs" and "this artist writes one unnamed
        // song" are not the same thing, and the picker says so.
        "song_structures" => {
            let id = request.args["styleId"].as_str().unwrap_or_default();
            let model = dataset::model(id)?;
            let forms =
                engine::arrange::structures_of(&model).map_err(|error| error.to_string())?;
            Ok(json!({ "structures": forms }))
        }

        // The arrangement, on its way to the audio thread (TASK-072).
        //
        // ⛔ **Answers with the flattened `Pattern`, and that reply *is* the
        // work.** `editor.rs` arms whatever a command answers with when it
        // deserializes as a `Pattern` — keyed on the shape rather than the
        // command's name, precisely so a fourth caller does not have to be
        // remembered — so a song reaches the transport through the same door an
        // edited clip does. The plugin had no notion of playing a `Song` at all
        // before this, which is why TASK-063B's playhead was left unbuilt: a
        // marker fed from one pattern's clock would sit at bar 3 of a 56-bar
        // arrangement and stay there.
        "arm_song" => {
            let args: ArmSongArgs = Deserialize::deserialize(&request.args["request"])
                .map_err(|e| format!("bad arm request: {e}"))?;
            // The same trust boundary the export and the re-roll use. Flattening
            // allocates one note per tile, so an unbounded song is an unbounded
            // allocation on the thread the host draws its editor from.
            check_song(&args.song)?;
            let mut pattern = args.song.flatten_parts(args.parts.as_deref());
            // ⚠ Set after flattening rather than inside it: a loop is a
            // transport instruction about the clip, and `flatten` answers the
            // different question of what the clip contains.
            pattern.loop_region = args
                .loop_section
                .and_then(|index| args.song.section_span(index));
            serde_json::to_value(pattern).map_err(|e| e.to_string())
        }

        // Re-roll one section of an arrangement (TASK-067).
        //
        // ⛔ **Takes the `Song` rather than regenerating from its seed**, for
        // the same reason `song_smf` does: once a clip has been edited or a
        // section moved, the seed no longer describes what is on screen, and a
        // re-roll that re-derived the whole song would silently discard every
        // other edit the producer had made.
        "reroll_section" => {
            let args: RerollArgs = Deserialize::deserialize(&request.args["request"])
                .map_err(|e| format!("bad re-roll request: {e}"))?;
            serde_json::to_value(reroll_section(args, host)?).map_err(|e| e.to_string())
        }

        // The bytes of a whole arranged song, for the drag-out and for Save As.
        //
        // ⛔ Takes the `Song` from the page rather than regenerating from the
        // seed. That is the edited-clip decision applied to the export: once a
        // clip has been edited or a section moved, the seed no longer describes
        // what is on screen, and re-deriving here would hand the producer the
        // song they started with instead of the one they arranged.
        "song_smf" => {
            let song: engine::pattern::Song =
                engine::pattern::Song::deserialize(&request.args["song"])
                    .map_err(|e| format!("bad song: {e}"))?;
            check_refs(&song)?;
            // ⛔ **Bounded before the engine sees it, because this is a trust
            // boundary.** A `Song` arrives here as JSON from the webview — a
            // project file somebody else saved, or devtools — and every field is
            // whatever that JSON said. Deserializing proves the *shape*; it
            // proves nothing about the numbers, and this command was the first
            // to hand unbounded ones to the tick arithmetic in `engine::midi`.
            // `generate_pattern` has clamped `bars` for this reason since the
            // meter work; a song has three more axes that multiply with it.
            check_song(&song)?;
            Ok(json!({ "bytes": engine::midi::song_to_smf(&song) }))
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
            let mut next: PluginSession = PluginSession::deserialize(&request.args["session"])
                .map_err(|e| format!("bad session state: {e}"))?;

            // `window_size` is the *editor's*, not the UI's — `set_editor_size`
            // is what sets it. A whole-session write that did not carry it
            // would silently reset the window every time the user changed an
            // artist, and the project would reopen at the wrong size with
            // nothing to explain why.
            if next.window_size.is_none() {
                next.window_size = state::read(session).window_size;
            }

            // ⛔ **`muted_lanes` is NOT carried over the way `window_size` is,
            // and the difference is that empty is a legitimate value here.**
            // Filling an empty list in from the store made "unmute the last
            // muted lane" impossible to express — the message that clears the
            // final lane is indistinguishable from the page not mentioning the
            // field, so the mute would come straight back and the UI would
            // insist the lane was on while the preview stayed silent. The page
            // sends the field on every save instead (`send()` in session.ts).

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

    // ⛔ **The mode is applied before anything reads the model, and that
    // ordering is the design** — a mode is a partial override of its own model,
    // including the `session` block it may retune the tempo or swing in. Apply
    // it after the context is built and the generators get a mode the session
    // never heard about.
    //
    // An absent `mood` means "Any", which is a *pick* rather than "no mode":
    // the whole point of TASK-040V is that one artist writes more than one kind
    // of record, so a reroll should be able to land on a different one. Picking
    // runs on its own seeded stream, so it cannot move the notes of a part that
    // was not re-rolled.
    let (model, mood) = match args.mood.as_deref().map(str::trim) {
        Some(name) if !name.is_empty() => (modes::apply(&model, name)?, Some(name.to_owned())),
        _ => match modes::pick(&model, seed) {
            Some(name) => (modes::apply(&model, &name)?, Some(name)),
            // The common case today: eleven of the shipped genres author no
            // modes at all, and a model with none generates exactly as before.
            None => (model, None),
        },
    };

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

    // The dependency order between the five parts lives in `engine::parts` —
    // Song Mode builds every section through the same function, so there is one
    // copy of it rather than two that can drift.
    let lanes = parts::render(&model, &ctx, seed, part);

    if parts::is_silent(&lanes) {
        // A style whose 808 *is* the bassline authors no separate bass lane on
        // purpose — doubling it is a production mistake rather than a fuller
        // sound (FR-007). "Has no Bass part authored" would read as a gap in
        // the style, and now that Bass is reachable it is a message producers
        // will actually meet.
        if part == Part::Bass && bass::eight_o_eight_is_the_bass(&model) {
            return Err(format!(
                "{}'s 808 is the bassline — it plays in the drums, so there is \
                 no separate bass part to generate",
                model.name
            ));
        }
        return Err(format!(
            "{} has no {part:?} part authored — nothing to generate",
            model.name
        ));
    }

    Ok(Pattern {
        loop_region: None,
        clip_region: None,
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
        mood,
    })
}

/// What the UI asks for when a song should start playing (TASK-072).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArmSongArgs {
    song: engine::pattern::Song,
    /// The section to loop, or `None` to play the record through.
    #[serde(default)]
    loop_section: Option<usize>,
    /// The parts to play. `None` is all of them — see `Song::flatten_parts`.
    #[serde(default)]
    parts: Option<Vec<Part>>,
}

/// What the UI asks for when the producer re-rolls one section (TASK-067).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RerollArgs {
    song: engine::pattern::Song,
    index: usize,
    /// Absent means "pick one" — the same rule the seed box follows.
    #[serde(default)]
    seed: Option<String>,
    /// The parts the producer has locked. Empty re-rolls the section whole.
    #[serde(default)]
    locked: Vec<Part>,
    #[serde(default)]
    mood: Option<String>,
    /// The pins the producer set. Only  and  are read — every
    /// other session value is carried by the  itself.
    #[serde(default)]
    session: Option<SessionOverrides>,
}

/// Re-roll one section, in the song's own key, tempo and meter.
///
/// ⛔ **The context is built from the `Song`, not from the host or the session
/// chips.** Every other section was written in the key and tempo stored on the
/// song; deriving this one from the current session instead would put a single
/// verse in a different key from the record around it — in tune with nothing,
/// and reproducible, so it would look deliberate.
fn reroll_section(args: RerollArgs, host: &HostSession) -> Result<engine::pattern::Song, String> {
    // The same trust boundary `song_smf` documents: a `Song` arrives here as
    // JSON from the webview, and re-rolling clones it and generates against its
    // numbers. Deserializing proves the shape and nothing about the values.
    check_song(&args.song)?;

    let seed = match &args.seed {
        Some(text) if !text.is_empty() => text
            .parse::<u64>()
            .map_err(|_| format!("`{text}` is not a seed"))?,
        _ => fresh_seed(),
    };

    let model = dataset::model(&args.song.artist_id)?;
    // Applied before the context is built, as everywhere else — a mode retunes
    // the `session` block. Pinned only: on "Any" the mode was picked from the
    // *song's* seed when it was generated, and re-picking from the re-roll seed
    // would let one section land in a different mode from the record.
    let model = match args.mood.as_deref().map(str::trim) {
        Some(name) if !name.is_empty() => modes::apply(&model, name)?,
        _ => model,
    };

    // The clip length this section already plays at. A re-roll varies the notes;
    // it does not resize the clip, and taking `bars` from the session chips
    // would make a re-rolled 4-bar loop come back 8 bars long.
    let bars = args
        .song
        .sections
        .get(args.index)
        .and_then(|section| section.patterns.values().next())
        .and_then(|reference| args.song.pattern(reference))
        .map(|pattern| pattern.bars.clamp(1, MAX_BARS));

    let overrides = SessionOverrides {
        bpm: Some(args.song.bpm),
        key_root: Some(args.song.key_root),
        scale: Some(args.song.scale),
        time_sig_num: Some(args.song.time_sig_num),
        time_sig_den: Some(args.song.time_sig_den),
        bars,
        // ⛔ **From the producer's pins, not from nothing.** These two are the
        // only session values a `Song` does not carry, and assuming they were
        // never pinned was wrong: `generate_song` takes `overrides` straight
        // from the page's `pins`, which includes `swing` — so a song generated
        // at a pinned 0.62 came back re-rolled at the model's authored 0.54, and
        // the re-rolled section's hats sat ~17 ms ahead of every other
        // section's, reproducibly, so it read as deliberate. That is the same
        // failure this function's own doc comment exists to prevent for the key
        // and the tempo, one field over.
        //
        // Absent still means "the artist chooses", which is what `SessionPins`
        // uses absence for everywhere else.
        swing: args.session.as_ref().and_then(|s| s.swing),
        half_time: args.session.as_ref().and_then(|s| s.half_time),
    };

    // ⛔ `auto_sync` is **false**, and it is not a shortcut. Every field the
    // host could contribute is pinned above from the song itself, so the flag
    // cannot change the answer — passing the session's value would imply the
    // host still has a say in what key this section comes back in, and it does
    // not.
    let ctx = host.session_for(&model, &overrides, seed, false);

    engine::arrange::reroll_section(&model, &ctx, args.song, args.index, seed, &args.locked)
        .map_err(|error| error.to_string())
}

/// The longest song the plugin will export, in bars.
///
/// Generous — an eight-minute record at 140 BPM is under 300 bars — so it never
/// binds on anything a producer arranged. It exists so a *file* cannot describe
/// a song whose length only makes sense as an attack.
const MAX_SONG_BARS: u32 = 4_096;

/// Refuse a song that names a clip it does not carry.
///
/// ⛔ **Every path that turns a `Song` into a file asks this**, and it was
/// written out at each of them until a third export arrived without it. A
/// dangling reference draws as an empty row and writes as *silence* — so the
/// producer gets a file that opens, imports, and is quietly missing a part,
/// with nothing anywhere saying why. `arrange` cannot produce one; a restored
/// project file or a hand-edited one can.
fn check_refs(song: &engine::pattern::Song) -> Result<(), String> {
    let dangling = song.dangling_refs();
    if dangling.is_empty() {
        return Ok(());
    }
    Err(format!(
        "this song names {} pattern(s) it does not carry ({}) — exporting it \
         would write silence where they play",
        dangling.len(),
        dangling.join(", ")
    ))
}

/// Refuse a song whose numbers cannot describe real music.
///
/// ⛔ **Refusing rather than clamping, and that is the deliberate half.** A
/// clamp would silently export something other than what the file said, which
/// on this path means handing the producer a `.mid` that is not their song and
/// saying nothing. Every bound below is far outside anything the UI can produce,
/// so a song that trips one did not come from arranging.
fn check_song(song: &engine::pattern::Song) -> Result<(), String> {
    if song.sections.len() > MAX_SECTIONS {
        return Err(format!(
            "this song has {} sections; the most that can be exported is \
             {MAX_SECTIONS}",
            song.sections.len()
        ));
    }
    // A denominator a MIDI file cannot express, or a numerator past the meters
    // the engine will generate, makes the tick arithmetic and the file's own
    // time-signature event disagree about how long a bar is.
    if !matches!(song.time_sig_den, 1 | 2 | 4 | 8 | 16 | 32) || song.time_sig_num == 0 {
        return Err(format!(
            "{}/{} is not a meter this can be written in",
            song.time_sig_num, song.time_sig_den
        ));
    }
    if u32::from(song.time_sig_num) > MAX_BEATS_PER_BAR {
        return Err(format!(
            "{} beats to the bar is past the {MAX_BEATS_PER_BAR} this can write",
            song.time_sig_num
        ));
    }

    for (index, section) in song.sections.iter().enumerate() {
        if section.bars == 0 || u32::from(section.bars) > MAX_SONG_BARS {
            return Err(format!(
                "section {index} is {} bars; sections run from 1 to \
                 {MAX_SONG_BARS}",
                section.bars
            ));
        }
        if section.start_bar > MAX_SONG_BARS {
            return Err(format!(
                "section {index} starts at bar {}, past the {MAX_SONG_BARS}-bar \
                 limit",
                section.start_bar
            ));
        }
    }

    for (id, pattern) in &song.patterns {
        if pattern.bars == 0 || u32::from(pattern.bars) > u32::from(MAX_BARS) {
            return Err(format!(
                "clip `{id}` is {} bars; a clip runs from 1 to {MAX_BARS}",
                pattern.bars
            ));
        }
    }

    // ⛔ **The two bounds that are on the *product*, and the reason the ones
    // above are not enough.** Every check so far reads one axis at a time, and
    // an axis-at-a-time bound cannot see what happens when the legal maxima are
    // combined:
    //
    // - **Notes.** A 1-bar clip holding 200,000 notes and one 4,096-bar section
    //   playing it passes every per-axis test — and then tiles to roughly 1.6
    //   billion events in a single `Vec`, which pins the thread the host draws
    //   its editor from until the process is killed. The counted loop stopped
    //   the *infinite* spin; it does not bound the work.
    // - **Ticks.** 32 beats to the bar over a `/1` denominator puts a bar at
    //   122,880 ticks, so a section starting at bar 4,096 lands at 503 million —
    //   past the 28 bits an SMF delta can hold. `midly` *masks* rather than
    //   refusing, so the export would succeed and place every marker and note in
    //   that section at a completely different bar than the arrangement showed.
    let ticks_per_bar = song.ticks_per_bar();
    let mut events: u64 = 0;
    for (index, section) in song.sections.iter().enumerate() {
        let end = u64::from(section.start_bar.saturating_add(u32::from(section.bars)))
            * u64::from(ticks_per_bar);
        if end > u64::from(MAX_TICK) {
            return Err(format!(
                "section {index} ends at tick {end}, past the {MAX_TICK} a MIDI \
                 file can address — the meter and the bar count are each legal \
                 but their product is not"
            ));
        }
        let repeats = u64::from(section.bars);
        for reference in section.patterns.values() {
            let notes = song
                .patterns
                .get(&reference.pattern_id)
                .map_or(0, |p| p.note_count());
            events = events.saturating_add(repeats.saturating_mul(notes as u64));
        }
    }
    if events > MAX_SONG_EVENTS {
        return Err(format!(
            "this song would write about {events} notes; the most that can be \
             exported is {MAX_SONG_EVENTS}"
        ));
    }

    Ok(())
}

/// The widest meter the export will write.
const MAX_BEATS_PER_BAR: u32 = 32;

/// Mirrors `engine::arrange`'s own cap, which is what generation can produce.
const MAX_SECTIONS: usize = 64;

/// The largest tick an SMF delta can address: `u28::MAX`.
///
/// ⛔ **`midly` masks a larger value rather than refusing it**, so this is the
/// only place that can catch it. Past this the file still parses and every event
/// in the offending section sits at the wrong bar.
const MAX_TICK: u32 = (1 << 28) - 1;

/// The most note events a song may write.
///
/// Roughly two orders of magnitude above anything the generators produce — a
/// dense 64-section song is a few tens of thousands — so it never binds on a
/// real arrangement. It exists because the *product* of legal per-axis maxima is
/// not itself legal.
const MAX_SONG_EVENTS: u64 = 2_000_000;

/// Build a whole arranged song for the UI's request.
///
/// Shares [`generate`]'s front half — seed, model, mode, session overrides — so
/// a song is placed in the host's tempo and meter and honours the pinned mood
/// exactly as a single pattern does. What it does *not* share is `bars`: in a
/// song that is the **clip** length, and the structure decides how long the
/// song runs.
fn generate_song(
    args: &GenerateArgs,
    host: &HostSession,
    auto_sync: bool,
) -> Result<engine::pattern::Song, String> {
    let seed = match &args.seed {
        Some(text) if !text.is_empty() => text
            .parse::<u64>()
            .map_err(|_| format!("`{text}` is not a seed"))?,
        _ => fresh_seed(),
    };

    let model = dataset::model(&args.style_id)?;
    // The mood is applied and not reported back, unlike a pattern's: a `Song`
    // has no `mood` field to carry it, so naming it here would be a value with
    // nowhere to go. See the note in `generate` for why the mode is applied
    // before the context is built.
    let model = match args.mood.as_deref().map(str::trim) {
        Some(name) if !name.is_empty() => modes::apply(&model, name)?,
        _ => match modes::pick(&model, seed) {
            Some(name) => modes::apply(&model, &name)?,
            None => model,
        },
    };

    let mut overrides = args.session.clone().unwrap_or_default();
    if let Some(bars) = args.bars {
        overrides.bars = Some(bars);
    }
    // ⛔ Clamped once, and on `overrides` rather than on `args`, so the bound
    // covers what arrived inside `session` as well. Generation runs
    // synchronously on the thread the host draws its window from — an unbounded
    // `bars` from a preset, a restored project or devtools is a multi-minute
    // freeze of somebody's DAW rather than a big pattern, and in a song it is
    // that cost once per section.
    if let Some(bars) = overrides.bars {
        overrides.bars = Some(bars.clamp(1, MAX_BARS));
    }

    let ctx = host.session_for(&model, &overrides, seed, auto_sync);
    // ⛔ **A forced form does not move a single note (TASK-070).** The structure
    // is sampled on its own stream, so overriding it leaves every other stream
    // untouched — the same seed with a different form keeps the same beats,
    // which is what makes the picker a way to hear one arrangement's ideas in
    // another's shape rather than a second reroll button.
    engine::arrange::generate_with(&model, &ctx, seed, args.structure)
        .map_err(|error| error.to_string())
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
        super::dispatch(
            request,
            host,
            &SessionStore::default(),
            &crate::export::Exports::default(),
        )
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

    /// Generate one part of one style, or say why it refused.
    fn generate_part(style: &str, part: &str) -> Result<Value, String> {
        dispatch(
            &request(
                "generate_pattern",
                json!({ "request": {
                    "styleId": style, "part": part, "bars": 4, "seed": "7"
                }}),
            ),
            &host(),
        )
    }

    fn note_count(pattern: &Value) -> usize {
        pattern["lanes"]
            .as_array()
            .expect("a pattern has lanes")
            .iter()
            .map(|lane| lane["notes"].as_array().map_or(0, Vec::len))
            .sum()
    }

    #[test]
    fn all_five_generators_are_reachable_through_the_bridge() {
        // ⛔ Three of these answered "the Melody generator is not implemented
        // yet" until the parts were wired through, and the engine had had them
        // all along — so the refusal was in this file rather than in the thing
        // it reported on. The gate is that a part answers with *notes*, not
        // merely that it answers.
        //
        // `uk-drill` because it authors all five and its 808 is a
        // `counter_riff`, so the bassline is a lane of its own. See the next
        // test for the styles where that is deliberately not true.
        for part in ["drums", "melody", "counter", "bass", "chords"] {
            let value = generate_part("uk-drill", part).unwrap_or_else(|e| panic!("{part}: {e}"));
            assert_eq!(value["part"], part);
            assert!(note_count(&value) > 0, "{part} generated no notes");
        }
    }

    #[test]
    fn a_bass_request_defers_to_the_808_rather_than_doubling_it() {
        // Trap's 808 *is* the bassline (`drums.bass808.role`), so a second bass
        // lane would double it — a production mistake rather than a fuller
        // sound. The refusal has to say that, or it reads as a hole in the
        // style: the part is authored, and it is deliberately silent.
        let err = generate_part("trap", "bass").unwrap_err();
        assert!(err.contains("808 is the bassline"), "{err}");
        assert!(!err.contains("no Bass part authored"), "{err}");
    }

    // ---- Song Mode (TASK-065) --------------------------------------

    /// A whole song for a style, or why it refused.
    fn generate_song(style: &str, seed: &str) -> Result<Value, String> {
        dispatch(
            &request(
                "generate_song",
                json!({ "request": { "styleId": style, "seed": seed } }),
            ),
            &host(),
        )
    }

    #[test]
    fn a_song_reaches_the_ui_with_its_patterns_attached() {
        // ⛔ The sections carry *references*, so a song whose store did not come
        // with it draws as a timeline of empty rows. The UI has no second call
        // to fetch them and should not need one.
        let song = generate_song("trap", "7").unwrap();
        let sections = song["sections"].as_array().expect("sections");
        assert!(!sections.is_empty(), "{song}");

        let store = song["patterns"].as_object().expect("a pattern store");
        for section in sections {
            for reference in section["patterns"].as_object().expect("refs").values() {
                let id = reference["patternId"].as_str().expect("an id");
                assert!(
                    store.contains_key(id),
                    "a section names `{id}` and the store does not hold it"
                );
            }
        }
    }

    // ---- Playing a whole song (TASK-072) ----------------------------------

    fn arm_song(song: &Value, extra: Value) -> Result<Value, String> {
        let mut request_args = json!({ "song": song });
        if let (Some(target), Some(source)) = (request_args.as_object_mut(), extra.as_object()) {
            for (key, value) in source {
                target.insert(key.clone(), value.clone());
            }
        }
        dispatch(
            &request("arm_song", json!({ "request": request_args })),
            &host(),
        )
    }

    #[test]
    fn arming_a_song_answers_with_a_pattern_so_the_editor_arms_it() {
        // ⛔ **This is the whole mechanism.** `editor.rs` arms from the *shape*
        // of a reply, not the command's name — so a reply that stopped
        // deserializing as a `Pattern` would leave Play doing nothing at all,
        // silently, with every other assertion here still passing.
        let song = generate_song("trap", "7").unwrap();
        let armed = arm_song(&song, json!({})).unwrap();

        let pattern: Pattern = serde_json::from_value(armed.clone())
            .expect("the reply must deserialize as a Pattern or nothing will arm it");
        assert!(pattern.note_count() > 0, "{armed}");
        // The whole song, so the playhead is a position through the arrangement
        // rather than through whichever clip happens to be playing.
        assert_eq!(
            u32::from(pattern.bars),
            song["sections"]
                .as_array()
                .expect("sections")
                .iter()
                .map(|s| s["startBar"].as_u64().unwrap_or(0) + s["bars"].as_u64().unwrap_or(0))
                .max()
                .unwrap_or(0) as u32
        );
    }

    #[test]
    fn looping_a_section_arms_that_sections_span_and_nothing_wider() {
        let song = generate_song("trap", "7").unwrap();
        let armed = arm_song(&song, json!({ "loopSection": 1 })).unwrap();

        let bar = 960 * 4; // 4/4 at the engine's PPQ
        let start = song["sections"][1]["startBar"].as_u64().expect("startBar") as u32;
        let bars = song["sections"][1]["bars"].as_u64().expect("bars") as u32;
        assert_eq!(armed["loopRegion"]["fromTick"], start * bar);
        assert_eq!(armed["loopRegion"]["toTick"], (start + bars) * bar);
    }

    #[test]
    fn playing_the_record_through_arms_no_loop() {
        let song = generate_song("trap", "7").unwrap();
        let armed = arm_song(&song, json!({})).unwrap();
        assert!(armed["loopRegion"].is_null(), "{}", armed["loopRegion"]);
    }

    #[test]
    fn soloing_a_part_leaves_the_others_out_of_what_is_armed() {
        // ⚠ An audition filter, not an edit: the song is unchanged and so is the
        // export. This asserts only that the armed clip narrows.
        let song = generate_song("trap", "7").unwrap();
        let all = arm_song(&song, json!({})).unwrap();
        let drums = arm_song(&song, json!({ "parts": ["drums"] })).unwrap();

        let count = |value: &Value| {
            serde_json::from_value::<Pattern>(value.clone())
                .expect("a pattern")
                .note_count()
        };
        assert!(count(&drums) > 0, "soloing the drums silenced everything");
        assert!(
            count(&drums) < count(&all),
            "soloing one part armed as many notes as the whole song"
        );
    }

    #[test]
    fn arming_a_crafted_song_is_refused_at_the_bridge() {
        // ⛔ Flattening allocates one note per tile, so this is the same
        // unbounded-work boundary `song_smf` documents — on the command that
        // runs while a producer is pressing Play.
        let mut song = generate_song("trap", "7").unwrap();
        song["timeSigDen"] = json!(3);

        let err = arm_song(&song, json!({})).unwrap_err();
        assert!(err.contains("not a meter"), "{err}");
    }

    #[test]
    fn a_loop_naming_no_section_plays_the_record_through() {
        // Out of range is "no loop" rather than an error: the index comes from
        // the page's own selection, and a stale one arriving after a section was
        // deleted must not stop playback.
        let song = generate_song("trap", "7").unwrap();
        let armed = arm_song(&song, json!({ "loopSection": 9_999 })).unwrap();
        assert!(armed["loopRegion"].is_null());
    }

    // ---- Re-rolling one section (TASK-067) --------------------------------

    /// Re-roll `index` of `song`, with the given parts locked.
    fn reroll(song: &Value, index: usize, seed: &str, locked: &[&str]) -> Result<Value, String> {
        dispatch(
            &request(
                "reroll_section",
                json!({ "request": {
                    "song": song,
                    "index": index,
                    "seed": seed,
                    "locked": locked,
                } }),
            ),
            &host(),
        )
    }

    /// A section of `song` playing more than one part.
    fn busy_section(song: &Value) -> usize {
        song["sections"]
            .as_array()
            .expect("sections")
            .iter()
            .position(|s| s["patterns"].as_object().map(|p| p.len()).unwrap_or(0) > 1)
            .expect("no section plays more than one part")
    }

    #[test]
    fn a_rerolled_section_comes_back_in_the_songs_own_key_and_tempo() {
        // ⛔ The failure this is here to catch is silent and sounds terrible: the
        // context is built from the *song*, not from the host or the session
        // chips, so a re-rolled verse cannot come back in a different key from
        // the record around it. The host here reports 92 and the song was
        // generated at 92, so the test moves the song's own tempo away from both
        // to prove which one the re-roll follows.
        let mut song = generate_song("trap", "7").unwrap();
        song["bpm"] = json!(155.0);
        song["keyRoot"] = json!(9);

        let index = busy_section(&song);
        let rerolled = reroll(&song, index, "4242", &[]).unwrap();

        assert_eq!(rerolled["bpm"], 155.0);
        assert_eq!(rerolled["keyRoot"], 9);

        // ⚠ **The re-rolled section's clips only.** Every other section keeps
        // the clip it already had — that is the byte-stability the whole task
        // rests on — so those still carry the tempo they were generated at, and
        // asserting over the whole store would be asserting that a re-roll
        // rewrites the sections it was told not to touch.
        let store = rerolled["patterns"].as_object().expect("store");
        let section = &rerolled["sections"][index]["patterns"];
        let refs = section.as_object().expect("refs");
        assert!(!refs.is_empty());
        for reference in refs.values() {
            let id = reference["patternId"].as_str().expect("an id");
            let clip = &store[id];
            assert_eq!(clip["bpm"], 155.0, "`{id}` kept the host's tempo");
            assert_eq!(clip["keyRoot"], 9, "`{id}` kept the host's key");
        }
    }

    #[test]
    fn a_reroll_keeps_the_clip_length_the_section_already_plays() {
        // A re-roll varies the notes; it does not resize the clip. Taking `bars`
        // from the session chips instead would hand back a 4-bar loop as 8.
        let song = generate_song("trap", "7").unwrap();
        let index = busy_section(&song);
        let before: Vec<u64> = song["patterns"]
            .as_object()
            .expect("store")
            .values()
            .map(|c| c["bars"].as_u64().expect("bars"))
            .collect();

        let rerolled = reroll(&song, index, "88", &[]).unwrap();
        let store = rerolled["patterns"].as_object().expect("store");
        for reference in rerolled["sections"][index]["patterns"]
            .as_object()
            .expect("refs")
            .values()
        {
            let id = reference["patternId"].as_str().expect("an id");
            assert!(
                before.contains(&store[id]["bars"].as_u64().expect("bars")),
                "re-rolled clip `{id}` changed length"
            );
        }
    }

    #[test]
    fn a_reroll_of_a_crafted_song_is_refused_at_the_bridge() {
        // ⛔ **The same trust boundary `song_smf` documents, and it has to be on
        // this command too.** A `Song` arrives here as JSON from the webview — a
        // project file somebody else saved, or devtools — and re-rolling clones
        // it and generates against its numbers. Without this the bound would
        // exist on the export and not on the command next to it.
        let mut song = generate_song("trap", "7").unwrap();
        song["sections"][0]["bars"] = json!(0);

        let err = reroll(&song, 0, "1", &[]).unwrap_err();
        assert!(err.contains("sections run from"), "{err}");
    }

    #[test]
    fn a_reroll_naming_no_section_says_so_rather_than_panicking() {
        let song = generate_song("trap", "7").unwrap();
        let count = song["sections"].as_array().expect("sections").len();

        let err = reroll(&song, count, "1", &[]).unwrap_err();
        assert!(err.contains("re-roll"), "{err}");
    }

    #[test]
    fn a_reroll_is_reproducible_from_its_own_seed() {
        let song = generate_song("trap", "7").unwrap();
        let index = busy_section(&song);
        assert_eq!(
            reroll(&song, index, "31337", &[]),
            reroll(&song, index, "31337", &[])
        );
        assert_ne!(
            reroll(&song, index, "31337", &[]),
            reroll(&song, index, "31338", &[])
        );
    }

    #[test]
    fn a_rerolled_song_still_exports() {
        // The two commands sit next to each other and the second is handed what
        // the first returns, so a re-roll that left a dangling reference or an
        // out-of-bound number would only show up here.
        let song = generate_song("trap", "7").unwrap();
        let index = busy_section(&song);
        let rerolled = reroll(&song, index, "5", &[]).unwrap();

        dispatch(&request("song_smf", json!({ "song": rerolled })), &host())
            .expect("a re-rolled song must still be exportable");
    }

    #[test]
    fn a_song_is_placed_in_the_hosts_tempo_the_way_a_pattern_is() {
        // The host reports 92; the model asks for its own. Song Mode must not
        // be the one path that ignores the DAW's tempo.
        let song = generate_song("trap", "7").unwrap();
        assert_eq!(song["bpm"], 92.0);
    }

    #[test]
    fn the_same_seed_returns_the_same_song() {
        assert_eq!(generate_song("trap", "11"), generate_song("trap", "11"));
        assert_ne!(generate_song("trap", "11"), generate_song("trap", "12"));
    }

    #[test]
    fn a_song_exports_bytes_that_start_with_the_smf_header() {
        let song = generate_song("trap", "7").unwrap();
        let out = dispatch(&request("song_smf", json!({ "song": song })), &host()).unwrap();
        let bytes: Vec<u64> = out["bytes"]
            .as_array()
            .expect("bytes")
            .iter()
            .map(|b| b.as_u64().unwrap())
            .collect();
        // "MThd" — every SMF begins with it.
        assert_eq!(&bytes[..4], &[0x4d, 0x54, 0x68, 0x64], "not an SMF");
    }

    #[test]
    fn exporting_a_song_that_lost_its_patterns_is_refused_rather_than_written() {
        // ⛔ Writing it anyway produces a file that is silent exactly where the
        // missing clips play, which a producer discovers in the DAW rather than
        // here. The refusal names the ids so it is actionable.
        let mut song = generate_song("trap", "7").unwrap();
        song["patterns"] = json!({});
        let err = dispatch(&request("song_smf", json!({ "song": song })), &host()).unwrap_err();
        assert!(err.contains("does not carry"), "{err}");
    }

    #[test]
    fn a_song_whose_numbers_could_hang_the_host_is_refused() {
        // ⛔ **The exploit /security-review found, refused by its own values.**
        // `song_events_for` tiled a clip across a section with
        // `offset += clip_len`. Release builds have no overflow checks, so for
        // a long enough section over a long enough clip `offset` wrapped past
        // the end and cycled forever — a 69/4 meter, a 64,824-bar section and a
        // 16,384-bar clip spin at 100% on the thread the host draws its editor
        // from, with no crash and no way out but killing the DAW.
        //
        // The loop is a counted one now, so it cannot hang whatever arrives;
        // this asserts the *other* half — that a song like this is refused at
        // the bridge instead of being quietly exported as something else.
        let mut song = generate_song("trap", "7").unwrap();
        song["timeSigNum"] = json!(69);
        song["sections"][0]["bars"] = json!(64824);
        let err = dispatch(&request("song_smf", json!({ "song": song })), &host()).unwrap_err();
        assert!(err.contains("bars") || err.contains("beats"), "{err}");
    }

    #[test]
    fn a_song_in_a_meter_no_midi_file_can_write_is_refused() {
        // A denominator of 3 divided the tick arithmetic while the file's own
        // meta event said /4 — the two then disagreed about how long a bar is.
        let mut song = generate_song("trap", "7").unwrap();
        song["timeSigDen"] = json!(3);
        let err = dispatch(&request("song_smf", json!({ "song": song })), &host()).unwrap_err();
        assert!(err.contains("meter"), "{err}");
    }

    #[test]
    fn a_song_whose_legal_axes_multiply_past_a_midi_tick_is_refused() {
        // ⛔ **Every axis is legal and their product is not.** 32 beats to the
        // bar over a `/1` denominator puts a bar at 122,880 ticks, so a section
        // starting at bar 4,096 — exactly the limit — lands at 503 million,
        // past the 28 bits an SMF delta can hold. `midly` *masks* rather than
        // refusing, so without this the export succeeded and placed every
        // marker and note in that section at a completely different bar than
        // the arrangement showed, with nothing reporting anything.
        let mut song = generate_song("trap", "7").unwrap();
        song["timeSigNum"] = json!(32);
        song["timeSigDen"] = json!(1);
        song["sections"][0]["startBar"] = json!(4096);
        let err = dispatch(&request("song_smf", json!({ "song": song })), &host()).unwrap_err();
        assert!(err.contains("past the"), "{err}");
    }

    #[test]
    fn a_song_whose_clips_multiply_to_billions_of_notes_is_refused() {
        // ⛔ The other product the per-axis checks cannot see. A 1-bar clip and
        // a 4,096-bar section are each inside their own limit; together they
        // tile to more events than fit in memory, on the thread the host draws
        // its editor from. The counted loop stopped the *infinite* spin; it
        // does not bound the work.
        let mut song = generate_song("trap", "7").unwrap();
        song["sections"] = json!([{
            "type": "verse",
            "startBar": 0,
            "bars": 4096,
            "patterns": { "drums": { "patternId": "wide" } },
            "dropOutBeats": 0,
            "decay": false,
            "markers": []
        }]);

        // One bar, packed with notes. 4,096 repeats of it is tens of millions.
        let notes: Vec<Value> = (0..2_000)
            .map(|i| json!({ "startTick": i, "lenTicks": 8, "pitch": 60, "vel": 100 }))
            .collect();
        song["patterns"] = json!({
            "wide": {
                "id": "wide", "part": "drums", "artistId": "trap", "seed": "1",
                "bars": 1, "bpm": 140.0, "timeSigNum": 4, "timeSigDen": 4,
                "keyRoot": 0, "scale": "natural_minor", "ppq": 960,
                "lanes": [{ "lane": "kick", "notes": notes }]
            }
        });

        let err = dispatch(&request("song_smf", json!({ "song": song })), &host()).unwrap_err();
        assert!(err.contains("notes"), "{err}");
    }

    #[test]
    fn an_ordinary_song_still_exports() {
        // The bounds must not bind on anything the app itself produces.
        let song = generate_song("trap", "7").unwrap();
        assert!(dispatch(&request("song_smf", json!({ "song": song })), &host()).is_ok());
    }

    #[test]
    fn a_pinned_mood_is_the_one_generated_in() {
        let value = dispatch(
            &request(
                "generate_pattern",
                json!({ "request": { "styleId": "trap", "seed": "7", "mood": "melodic" } }),
            ),
            &host(),
        )
        .unwrap();

        assert_eq!(value["mood"], "melodic");
    }

    #[test]
    fn any_picks_a_mood_from_the_seed_rather_than_generating_without_one() {
        // ⛔ "Any" is a pick, not an absence — the whole point of TASK-040V is
        // that one artist writes more than one kind of record, so a reroll has
        // to be able to land on a different one. A pattern with no mood here
        // would mean the modes were silently skipped.
        let picked = |seed: &str| {
            dispatch(
                &request(
                    "generate_pattern",
                    json!({ "request": { "styleId": "trap", "seed": seed } }),
                ),
                &host(),
            )
            .unwrap()["mood"]
                .clone()
        };

        let offered = ["dark", "bounce", "melodic", "minimal"];
        assert!(offered.contains(&picked("7").as_str().unwrap()));
        // Same seed, same mood — picking runs on its own seeded stream.
        assert_eq!(picked("7"), picked("7"));
    }

    #[test]
    fn a_style_with_no_modes_generates_exactly_as_before() {
        // Eleven of the shipped genres author none, so this is the common path.
        let value = dispatch(
            &request(
                "generate_pattern",
                json!({ "request": { "styleId": "phonk", "seed": "7" } }),
            ),
            &host(),
        )
        .unwrap();

        assert!(value.get("mood").is_none(), "{value}");
    }

    #[test]
    fn a_mood_the_style_does_not_offer_is_refused_by_name() {
        let err = dispatch(
            &request(
                "generate_pattern",
                json!({ "request": { "styleId": "trap", "seed": "7", "mood": "nope" } }),
            ),
            &host(),
        )
        .unwrap_err();

        assert!(err.contains("nope"), "{err}");
    }

    #[test]
    fn a_melodic_part_is_reproducible_from_its_seed_like_every_other() {
        // The melodic parts regenerate their own inputs — the harmony, and the
        // kit or the lead they sit with — so determinism has to survive a chain
        // of generators rather than one. If it stops holding, a reopened
        // project comes back on a different melody.
        assert_eq!(
            generate_part("uk-drill", "counter"),
            generate_part("uk-drill", "counter")
        );
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
    fn playback_status_is_not_this_layer_s_to_answer() {
        // It moved to `editor::window_command`, which has the `Shared` it needs
        // to say which shell this is — see the note there. What this layer must
        // do is refuse it loudly rather than answer it with something plausible,
        // which is the rule every unknown command follows.
        let error = dispatch(&request("playback_status", json!({})), &host()).unwrap_err();
        assert!(error.contains("no handler"), "{error}");
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
            &crate::export::Exports::default(),
        );
        assert!(saved.is_ok(), "{saved:?}");

        let value = super::dispatch(
            &request("session_state", json!({})),
            &host(),
            &store,
            &crate::export::Exports::default(),
        )
        .unwrap();

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
            &crate::export::Exports::default(),
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
            &crate::export::Exports::default(),
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
            &crate::export::Exports::default(),
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
            &crate::export::Exports::default(),
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
                &crate::export::Exports::default(),
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
