//! The pivot, end to end, against the data that ships.
//!
//! One claim, and it is the reason this project stopped being a desktop app:
//! **a pattern generated inside a host lands at the host's tempo, in the
//! host's meter, on the host's timeline** — with no protocol, no MIDI cable
//! and nothing for the user to type.
//!
//! The unit tests in `host.rs` and `voice.rs` prove each half. This runs the
//! whole path the plugin runs: shipped model → host session → engine →
//! schedule.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

use engine::context::SessionOverrides;
use engine::generators::drums;
use engine::humanize::humanize;
use engine::pattern::{Part, Pattern, PPQ};
use engine::StyleModel;
use freally_midi_master_plugin::{HostSession, Schedule};

/// Every shipped model, read and resolved **once per test binary**.
///
/// ⛔⛔ **This used to re-read and re-resolve the whole dataset on every call**,
/// and [`shipped`] below is called once per *generation* across seven tests —
/// so `read_dir` over 609 JSON files plus `resolve_all()` over 590 models ran
/// hundreds of times to answer for one id. Measured at **1,116 seconds of CPU on
/// this one test binary**, which was a large share of why the `quality` job took
/// ~2.5 hours.
///
/// ⚠ **The same shape `engine/tests/genre_invariants.rs` already uses**, and its
/// doc records the same lesson one crate over: a helper that looks like a cheap
/// lookup and is a full dataset load is the most expensive kind of convenient.
fn registry() -> &'static BTreeMap<String, StyleModel> {
    static MODELS: OnceLock<BTreeMap<String, StyleModel>> = OnceLock::new();
    MODELS.get_or_init(|| {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("data");
        let scan = engine::dataset::files::scan(&dir).expect("data/ must be readable");
        let (models, errors) = engine::dataset::registry_from(scan.files).resolve_all();
        assert!(errors.is_empty(), "the dataset must resolve: {errors:#?}");
        models
    })
}

fn shipped(id: &str) -> StyleModel {
    registry().get(id).cloned().expect("no such model")
}

/// What the plugin does when the user presses Generate, minus the UI.
fn generate_in_host(host: &HostSession, id: &str, seed: u64) -> Pattern {
    let model = shipped(id);
    let ctx = host.session_for(&model, &SessionOverrides::default(), seed, true);

    // The same three calls, in the same order, as the desktop app's `render`
    // and `engine/tests/golden.rs`. Generation writes on the grid; feel is
    // applied after.
    let mut lanes = drums::generate(&model, &ctx, seed);
    humanize(&mut lanes, &ctx, seed);

    Pattern {
        loop_region: None,
        clip_region: None,
        id: format!("{id}-{seed}"),
        part: Part::Drums,
        artist_id: model.id.clone(),
        seed,
        song_seed: seed,
        bars: ctx.bars,
        bpm: ctx.bpm,
        time_sig_num: ctx.time_sig_num,
        time_sig_den: ctx.time_sig_den,
        key_root: ctx.key_root,
        scale: ctx.scale,
        lanes,
        ppq: PPQ,
        mood: None,
    }
}

/// A host session reporting this tempo and meter.
fn host_at(tempo: f64, num: u8, den: u8) -> HostSession {
    HostSession::observed_for_test(Some(tempo), num, den)
}

#[test]
fn a_pattern_generated_in_a_project_carries_that_projects_tempo() {
    // trap authors 130–170 with a mode of 140. Dropped into a 92 BPM session,
    // it must come out at 92 — that is the entire pivot in one assertion.
    let pattern = generate_in_host(&host_at(92.0, 4, 4), "trap", 7);

    assert_eq!(pattern.bpm, 92.0);
    assert!(pattern.note_count() > 0, "a tempo change must not empty it");
}

#[test]
fn the_same_model_follows_whatever_tempo_the_host_reports() {
    for tempo in [70.0, 92.0, 128.0, 174.0] {
        let pattern = generate_in_host(&host_at(tempo, 4, 4), "trap", 3);
        assert_eq!(pattern.bpm, tempo as f32, "at {tempo} BPM");
    }
}

#[test]
fn a_three_four_project_generates_three_four_bars() {
    // Not cosmetic: `ticks_per_bar` decides where every position in the
    // grammar lands, so a 4/4 pattern dropped in a 3/4 project would place
    // every backbeat in the wrong place *and* export the wrong meta event.
    let pattern = generate_in_host(&host_at(120.0, 3, 4), "trap", 5);

    assert_eq!((pattern.time_sig_num, pattern.time_sig_den), (3, 4));

    let bar = PPQ * 3;
    let last = pattern
        .lanes
        .iter()
        .flat_map(|lane| &lane.notes)
        .map(|note| note.start_tick)
        .max()
        .expect("something should have generated");
    assert!(
        last < bar * u32::from(pattern.bars),
        "a note escaped the pattern: {last} past {}",
        bar * u32::from(pattern.bars)
    );
}

#[test]
fn the_notes_land_where_the_hosts_tempo_says_they_should() {
    // The other half, through the real scheduler: the engine generated at the
    // host's tempo, and `Schedule` places those ticks in samples at that same
    // tempo. If the two ever disagreed, a clip would be generated correctly
    // and played wrong — which is the failure nobody would attribute to the
    // right file.
    let sample_rate = 48_000.0_f32;
    let pattern = generate_in_host(&host_at(120.0, 4, 4), "trap", 11);

    let mut schedule = Schedule::default();
    schedule.arm(&pattern, sample_rate);
    let (count, last_sample) = schedule.placement();

    assert_eq!(
        count,
        pattern.note_count(),
        "every note should be scheduled"
    );

    // At 120 BPM a 4/4 bar is two seconds, so four bars is exactly eight —
    // 384,000 samples at 48 kHz. Every note, including its release, is inside.
    let pattern_samples = 8.0 * sample_rate;
    assert!(
        f32::from(u16::try_from(last_sample / 1000).unwrap_or(u16::MAX)) * 1000.0
            <= pattern_samples + 1000.0,
        "the last note ends at {last_sample} samples, past the {pattern_samples}-sample loop"
    );
}

#[test]
fn halving_the_hosts_tempo_doubles_the_clip_in_real_time() {
    // The clearest statement of the sync: same model, same seed, half the
    // tempo — the notes are the same and they take twice as long.
    let fast = generate_in_host(&host_at(140.0, 4, 4), "trap", 4);
    let slow = generate_in_host(&host_at(70.0, 4, 4), "trap", 4);

    // Same seed, same grammar: the same notes, in the same order.
    //
    // The *ticks* are deliberately not compared, and that is worth stating
    // rather than working around. Feel is authored in **milliseconds** —
    // `timingJitterMs` and drill's `offGridMs` — because a hat that sits 8 ms
    // late sits 8 ms late whatever the tempo. Converting that to ticks
    // therefore has to give a different number at 70 BPM than at 140, and it
    // does. A future change that made the humanized ticks identical across
    // tempos would have quietly turned the feel into a fraction of a beat,
    // which is the one thing it is not.
    let pitches = |p: &Pattern| -> Vec<(engine::pattern::Lane, u8)> {
        p.lanes
            .iter()
            .flat_map(|lane| lane.notes.iter().map(move |note| (lane.lane, note.pitch)))
            .collect()
    };
    assert_eq!(
        pitches(&fast),
        pitches(&slow),
        "the tempo must not change what was generated, only how long it lasts"
    );

    let mut a = Schedule::default();
    a.arm(&fast, 48_000.0);
    let mut b = Schedule::default();
    b.arm(&slow, 48_000.0);

    let (_, fast_end) = a.placement();
    let (_, slow_end) = b.placement();
    // Within a millisecond at 48 kHz. Not sample-exact, because the humanized
    // positions above genuinely differ by a few ticks between the two tempos.
    assert!(
        slow_end.abs_diff(fast_end * 2) < 48,
        "half the tempo should take twice as long: {slow_end} vs {}",
        fast_end * 2
    );
}

#[test]
fn the_host_tempo_changes_when_notes_land_and_never_how_many() {
    // The precise version of the claim above, and the one worth guarding.
    //
    // Tick positions legitimately move with the tempo — twice, in two places.
    // `humanize` converts `timingJitterMs` to ticks, and `drums.rs` applies
    // `offGridMs` (drill's nudged snare) in the grammar itself, because a
    // genre made of an 11 ms displacement is made of 11 milliseconds and not
    // of a fraction of a beat.
    //
    // What must *not* move is how much is played. A grammar that thinned out
    // at 174 BPM or doubled up at 70 would be a different pattern wearing the
    // same seed, and nothing else in the suite would notice.
    for (id, model) in registry().iter().filter(|(id, _)| !id.starts_with('_')) {
        let counts = |tempo: f64| -> Vec<(engine::pattern::Lane, usize)> {
            generate_in_host(&host_at(tempo, 4, 4), id, 4)
                .lanes
                .iter()
                .map(|lane| (lane.lane, lane.notes.len()))
                .collect()
        };

        // ⛔⛔ **Two authored keys are allowed to break this, and both are NAMED
        // here rather than tolerated by loosening the assertion.** Each buys
        // exactly one lane, and the claim for a model carrying one is the same
        // claim minus the lane it bought: **that lane may change and nothing
        // else may.** That is stronger than exempting the model, and both times
        // it has confirmed nothing else moved when the key fired.
        //
        // - `snare.fullTimeAtHighTempoProb` means "play full time when the
        //   session is fast" — a rule about *how many*, asked for by the
        //   producer, and `rage` authors it at 0.15. It buys the **snare**.
        // - `snare.rimshotBelowBpm` means "lay a rimshot under the backbeat when
        //   the record is slow", and `rnb-2000s` authors it at 80 — so `112` and
        //   everything else inheriting it plays a rim at 70 and none at 140. It
        //   buys the **rim**.
        //
        // ⚠ **An empty list is the exact comparison this test has always made**,
        // so a model authoring neither key is held to the original claim with no
        // filter at all.
        let snare = model
            .blocks
            .get("drums")
            .and_then(|drums| drums.get("snare"));
        let mut bought: Vec<engine::pattern::Lane> = Vec::new();
        if snare
            .and_then(|s| s.get("fullTimeAtHighTempoProb"))
            .is_some()
        {
            bought.push(engine::pattern::Lane::Snare);
        }
        if snare.and_then(|s| s.get("rimshotBelowBpm")).is_some() {
            bought.push(engine::pattern::Lane::Rim);
        }

        // ⛔⛔ **`fullTimeAtHighTempoProb` buys a THIRD lane, transitively — and
        // only for the models whose grammar asks it to.** The key moves the
        // snare from half time onto the backbeat, and a hi-hat roll positioned
        // `pre_snare` is *defined* by where the snare is: one snare a bar
        // becomes two, so the roll is offered a second window and the closed-hat
        // count moves with it. `RollPosition::parse` makes `pre_snare` the only
        // snare-relative position there is, so this is the whole of the
        // transitivity rather than the first case of an open set.
        //
        // ⚠ **Latent since the key was written, and hidden by its own
        // probability.** `rage` authors it at 0.15 and five of its children roll
        // `pre_snare` hats; `sheck-wes` and `trippie-redd` have been passing
        // this gate for months because their draw came up false, and
        // `nine-vicious` is simply the first whose came up true. The gate was
        // never sound for those five — it was lucky.
        //
        // ⚠ Conditioned on the *combination*, so a model with the key and no
        // snare-relative roll, or a roll and no key, is still held to the exact
        // claim on every lane but the snare.
        let rolls_follow_the_snare = model
            .blocks
            .get("drums")
            .and_then(|drums| drums.get("hihat"))
            .and_then(|hihat| hihat.get("rolls"))
            .and_then(|rolls| rolls.get("positions"))
            .and_then(serde_json::Value::as_array)
            .is_some_and(|positions| {
                positions.iter().any(|at| at.as_str() == Some("pre_snare"))
            });
        if bought.contains(&engine::pattern::Lane::Snare) && rolls_follow_the_snare {
            bought.push(engine::pattern::Lane::ClosedHat);
        }

        let without_bought = |tempo: f64| -> Vec<(engine::pattern::Lane, usize)> {
            counts(tempo)
                .into_iter()
                .filter(|(lane, _)| !bought.contains(lane))
                .collect()
        };
        assert_eq!(
            without_bought(70.0),
            without_bought(140.0),
            "{id} changed a lane no tempo key bought, at 70 (bought: {bought:?})"
        );

        // ⚠ 140 and 174 are both above `FULL_TIME_BPM`, so this half of the
        // claim is untouched by the switch and stays exact for every model.
        assert_eq!(
            counts(140.0),
            counts(174.0),
            "{id} plays differently at 174"
        );
    }
}

#[test]
fn every_shipped_style_generates_inside_a_host_session() {
    // The desktop app asserted this against the model's own tempo. The plugin
    // has to hold it at the *host's*, which is a stronger claim: a grammar
    // that only works at its authored tempo would fail here.
    for tempo in [80.0, 140.0] {
        let host = host_at(tempo, 4, 4);
        for id in registry().keys().filter(|id| !id.starts_with('_')) {
            let pattern = generate_in_host(&host, id, 21);
            assert_eq!(pattern.bpm, tempo as f32, "{id} ignored the host");
            assert!(
                pattern.note_count() > 0,
                "{id} generated nothing at {tempo}"
            );
        }
    }
}
