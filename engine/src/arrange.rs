//! The Arrangement Creator — Song Mode (TASK-065, FR-008).
//!
//! Turns one model and one seed into a whole song: a structure sampled from
//! what the artist actually tends to write, sections given their bar counts,
//! and a pattern per part per section built by the same generators a four-bar
//! request uses.
//!
//! ## Three decisions this module makes, because they are not obvious
//!
//! **1. A section plays a looping clip; it is not through-composed.** A verse is
//! sixteen bars, but a trap verse is a four-bar loop played four times, and that
//! is what every DAW's arrangement view draws and what `rage`'s own
//! `drumLoopBars: 4` says out loud. So a pattern is generated at the model's
//! natural loop length ([`SessionContext::bars`]) and [`Section::bars`] says how
//! long it plays. Generating sixteen distinct bars instead would be four times
//! the work to produce something less like the genre, not more.
//!
//! **2. Sections of the same kind share one pattern.** Verse 1 and verse 2 are
//! the same beat — in these genres that is the rule, and the exception is
//! `switchUpProb`, which exists precisely because the default is repetition.
//! That is also why [`PatternRef`] is an id rather than an inline pattern: one
//! entry in [`Song::patterns`] serves every section that plays it.
//!
//! **3. A part that generates nothing is left out, not reported.** Most trap
//! artists author `drums.bass808.role = "bassline"`, so their `Bass` part is
//! deliberately silent (FR-007). A single-pattern request says so to the
//! producer, because they asked for that part by name; a song did not, so the
//! section simply has no bass row.
//!
//! ## What derives from what
//!
//! Every random choice runs on its own named stream, so re-rolling one thing
//! cannot move another. The domains are `arrange/structure`, `arrange/bars`, and
//! `section:{kind}/{part}` for the notes — the last of which
//! [`crate::rng`]'s own doc comment has named as the example since before this
//! module existed.

use std::collections::BTreeMap;

use rand::Rng;
use serde_json::Value;

use crate::context::SessionContext;
use crate::dataset::StyleModel;
use crate::generators::bass;
use crate::generators::chords::Chords;
use crate::generators::read;
use crate::parts;
use crate::pattern::{
    Lane, LaneTrack, Part, Pattern, PatternRef, Section, SectionKind, Song, PART_ORDER, PPQ,
};
use crate::rng;

/// The block a model authors its song forms under.
const ARRANGEMENT: &str = "arrangement";

/// Density parameters a section rule's multiplier scales.
///
/// `density` itself is deliberately absent: that is the *rule's own* key, and
/// scaling it would make a section's multiplier scale itself.
const DENSITY_KEYS: &[&str] = &["densityPerBar", "densityRatio", "fillDensity"];

/// The most sections a structure may produce.
///
/// Generation runs synchronously on the thread the host draws its window from,
/// and each section costs a full multi-part generation. A structure from a
/// community dataset listing four hundred sections is a frozen DAW rather than
/// a long song.
const MAX_SECTIONS: usize = 64;

/// Why a song could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArrangeError {
    /// The model authors no `arrangement` block at all.
    NoArrangement(String),
    /// It has one, but no `structures` to sample a form from.
    NoStructures(String),
    /// A structure names a section this engine has no kind for.
    UnknownSection { model: String, name: String },
    /// Every part of every section came out silent.
    Silent(String),
    /// A re-roll named a section the song does not have (TASK-067).
    NoSuchSection {
        song: String,
        index: usize,
        sections: usize,
    },
    /// The picker named a form the model does not author (TASK-070).
    NoSuchStructure {
        model: String,
        index: usize,
        structures: usize,
    },
}

impl std::fmt::Display for ArrangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArrangeError::NoArrangement(id) => write!(
                f,
                "`{id}` authors no `arrangement` block, so there is no song form \
                 to build — every shipped model inherits one from `_defaults`"
            ),
            ArrangeError::NoStructures(id) => write!(
                f,
                "`{id}`'s `arrangement` authors no `structures`, so there is no \
                 section order to sample"
            ),
            ArrangeError::UnknownSection { model, name } => write!(
                f,
                "`{model}` names a `{name}` section, which this engine has no \
                 kind for — it knows intro, verse, hook (or chorus), bridge and \
                 outro"
            ),
            ArrangeError::Silent(id) => write!(
                f,
                "every part of every section of `{id}` generated silence — the \
                 model has no parts authored that this form asks for"
            ),
            ArrangeError::NoSuchSection {
                song,
                index,
                sections,
            } => write!(
                f,
                "`{song}` has {sections} section(s), so there is no section \
                 {index} to re-roll"
            ),
            ArrangeError::NoSuchStructure {
                model,
                index,
                structures,
            } => write!(
                f,
                "`{model}` authors {structures} song form(s), so there is no \
                 form {index} to build"
            ),
        }
    }
}

impl std::error::Error for ArrangeError {}

/// One of the forms a model tends to write, and how often it writes it.
struct Structure {
    sections: Vec<String>,
    weight: f64,
}

/// Build a whole song.
///
/// `ctx` is the session every section shares — one key, one tempo, one feel, so
/// the arrangement is one record rather than eight unrelated loops. Its `bars`
/// is the *clip* length, not the song length; the structure decides how long the
/// song is.
pub fn generate(model: &StyleModel, ctx: &SessionContext, seed: u64) -> Result<Song, ArrangeError> {
    generate_with(model, ctx, seed, None)
}

/// The forms this model writes, in authored order (TASK-070).
///
/// What the structure picker offers. Section names are returned as the model
/// *authored* them rather than as [`SectionKind`]s, because that is what the
/// picker shows and because `chorus` and `hook` are one kind under two genres'
/// names for it — collapsing them would make pop's form read back in trap's
/// vocabulary.
pub fn structures_of(model: &StyleModel) -> Result<Vec<Vec<String>>, ArrangeError> {
    let block = model
        .blocks
        .get(ARRANGEMENT)
        .filter(|value| !value.is_null())
        .ok_or_else(|| ArrangeError::NoArrangement(model.id.clone()))?;
    let all = structures(block);
    if all.is_empty() {
        return Err(ArrangeError::NoStructures(model.id.clone()));
    }
    Ok(all.into_iter().map(|s| s.sections).collect())
}

/// Build a song, optionally forcing one of the model's authored forms.
///
/// ⛔ **`None` samples from the weights and is the only thing `generate` does.**
/// A forced index is the producer overriding the artist — "give me the one with
/// a bridge" — and it deliberately does not change any other stream, so the
/// same seed with a different form keeps the same beats. `arrange/structure` is
/// simply not consulted.
pub fn generate_with(
    model: &StyleModel,
    ctx: &SessionContext,
    seed: u64,
    structure: Option<usize>,
) -> Result<Song, ArrangeError> {
    let block = model
        .blocks
        .get(ARRANGEMENT)
        .filter(|value| !value.is_null())
        .ok_or_else(|| ArrangeError::NoArrangement(model.id.clone()))?;

    let names = match structure {
        Some(index) => {
            let mut all = structures(block);
            if all.is_empty() {
                return Err(ArrangeError::NoStructures(model.id.clone()));
            }
            if index >= all.len() {
                return Err(ArrangeError::NoSuchStructure {
                    model: model.id.clone(),
                    index,
                    structures: all.len(),
                });
            }
            let mut sections = all.swap_remove(index).sections;
            sections.truncate(MAX_SECTIONS);
            sections
        }
        None => pick_structure(model, block, seed)?,
    };

    let mut sections: Vec<Section> = Vec::new();
    let mut patterns: BTreeMap<String, Pattern> = BTreeMap::new();
    let mut start_bar = 0u32;

    let transitions = block.get("transitions");
    // The back half a switch-up applies to. Computed from the whole structure
    // before any section is built, because "the back half" is a fact about the
    // song rather than about the section being looked at.
    let switch_from = switch_up_point(transitions, &names, seed);

    for (index, name) in names.iter().enumerate() {
        let kind = section_kind(name).ok_or_else(|| ArrangeError::UnknownSection {
            model: model.id.clone(),
            name: name.clone(),
        })?;

        let bars = section_bars(block, name, seed, index);
        let rule = section_rule(block, name);

        // ⛔ A switch-up gives the back half its *own* patterns, and it does it
        // by changing the key they are stored under. Regenerating in place
        // would change the front half too, because both halves are looking at
        // one entry — which is the whole reason sharing is keyed by name.
        let variant = match switch_from {
            Some(from) if index >= from => format!("{name}:switchup"),
            _ => name.clone(),
        };

        // ⛔ **One seed per section, shared by every part in it — not one per
        // part.** `parts::render` writes a melody *against* a harmony it derives
        // internally from the seed it is handed, so a per-part seed makes that
        // internal harmony a different voicing from the Chords clip in the same
        // section: the two clips a producer hears together were never written
        // against each other. Both are individually in key, in register and
        // reproducible, which is why nothing else in the suite sees it. This is
        // the same rule the single-pattern path follows, where one seed serves
        // all five parts.
        //
        // It is also what lets the dependency renders be shared rather than
        // repeated — see `render_section`.
        let section_seed = rng::derive_seed(seed, &format!("section:{name}"));
        let switch_seed = rng::derive_seed(seed, &format!("section:{variant}"));

        // The model this section generates from, scaled once rather than once
        // per part: a serde-free clone, so the density walk cannot silently
        // fall back to full density on a round-trip failure.
        let scaled = with_density(model, rule.density);
        let section_model = scaled.as_ref().unwrap_or(model);

        let wanted = parts_for(&rule, model);
        let rendered = render_section(section_model, ctx, section_seed, &wanted, Carry::default());
        // A switch-up varies only the *melodic* parts, so the back half needs a
        // second render on its own seed. Skipped entirely when there is no
        // switch-up, which is roughly six songs in seven.
        //
        // ⛔ **The switch-up render is given the section's *own* kit.** Without
        // that it generated a second kit from `switch_seed` and wrote the
        // back-half melody around it — while the section went on storing the
        // drums from `section_seed`. The melody would have been written around a
        // kick pattern the section does not play, which is precisely the
        // incoherence the per-section seed fixed for the ordinary path.
        let switched = (switch_seed != section_seed).then(|| {
            render_section(
                section_model,
                ctx,
                switch_seed,
                &wanted,
                Carry {
                    kit: Some(&rendered.kit),
                    harmony: None,
                },
            )
        });

        let mut refs: BTreeMap<Part, PatternRef> = BTreeMap::new();
        for PartRequest { part, .. } in wanted {
            // ⛔ Keyed by the section *name*, not by its index: that is what
            // makes verse 1 and verse 2 the same beat. The store then holds one
            // pattern where a per-index key would hold two identical ones under
            // different names, and a producer would be looking at a repeat the
            // data claimed was a new part.
            //
            // Changing the drums in the back half is a different beat rather
            // than a switch-up, and the device these models author is the one
            // where the drums hold and the melody moves.
            let melodic = part != Part::Drums;
            let key = if melodic { &variant } else { name };
            let id = pattern_id(model, key, part);
            if !patterns.contains_key(&id) {
                let source = match (melodic, &switched) {
                    (true, Some(switched)) => switched,
                    _ => &rendered,
                };
                let Some(lanes) = source.lanes.get(&part) else {
                    continue;
                };
                let pattern_seed = if melodic && switched.is_some() {
                    switch_seed
                } else {
                    section_seed
                };
                patterns.insert(
                    id.clone(),
                    pattern_for(model, ctx, pattern_seed, part, &id, lanes.clone()),
                );
            }
            refs.insert(part, PatternRef { pattern_id: id });
        }

        sections.push(Section {
            kind,
            start_bar,
            bars,
            patterns: refs,
            // Filled in below: a drop-out is a property of the section *before*
            // the one it makes room for, so it cannot be decided here.
            drop_out_beats: 0,
            decay: rule.decay,
            markers: Vec::new(),
        });
        start_bar += u32::from(bars);
    }

    drop_out_before_hooks(&mut sections, transitions, seed, ctx.time_sig_num.max(1));

    if patterns.is_empty() {
        return Err(ArrangeError::Silent(model.id.clone()));
    }

    Ok(Song {
        id: format!("{}-song-{seed}", model.id),
        artist_id: model.id.clone(),
        seed,
        bpm: ctx.bpm,
        key_root: ctx.key_root,
        scale: ctx.scale,
        sections,
        time_sig_num: ctx.time_sig_num,
        time_sig_den: ctx.time_sig_den,
        patterns,
        ppq: PPQ,
    })
}

/// Re-generate one section of a song, leaving every other one byte-stable
/// (TASK-067).
///
/// `locked` names the parts the producer has pinned; they are kept exactly as
/// they are, notes and all. **That is the whole of "lock-respecting" as far as
/// the engine is concerned** — a lock is a UI state, and passing the resulting
/// part list keeps this module from having to know about cells, rows or
/// sections. An empty `locked` re-rolls the section whole.
///
/// ## ⛔ Why the re-rolled clips get their own ids
///
/// [`generate`] keys a pattern by *section name*, which is what makes verse 1
/// and verse 2 the same beat. Re-rolling in place would therefore change every
/// verse in the song — and "re-roll section 2 leaves the others byte-stable" is
/// this task's own acceptance criterion. So a re-rolled section is keyed by its
/// **index** instead and diverges from its siblings, exactly the way a switch-up
/// diverges by keying on `{name}:switchup`. Re-rolling the same section again
/// overwrites that entry rather than accumulating one per attempt.
///
/// ## ⛔ Why the locked parts are handed back in as dependencies
///
/// A melody is written *against* a harmony and a kit. Re-rolling the melody
/// alone while the chords stay put would write it against a harmony derived
/// from the new seed — a different voicing from the chord clip the section still
/// plays, which is precisely the incoherence the per-section seed exists to
/// prevent, one level down. So a locked Chords or Drums part is **re-derived
/// from its own stored `Pattern::seed`** and handed to the render as [`Carry`].
/// The stored clip is humanized and the generators want the raw one, which is
/// why this regenerates rather than reusing the notes in the song.
pub fn reroll_section(
    model: &StyleModel,
    ctx: &SessionContext,
    song: Song,
    index: usize,
    seed: u64,
    locked: &[Part],
) -> Result<Song, ArrangeError> {
    let section = song
        .sections
        .get(index)
        .ok_or_else(|| ArrangeError::NoSuchSection {
            song: song.id.clone(),
            index,
            sections: song.sections.len(),
        })?;

    let block = model
        .blocks
        .get(ARRANGEMENT)
        .filter(|value| !value.is_null())
        .ok_or_else(|| ArrangeError::NoArrangement(model.id.clone()))?;

    let kind_name = section_name(section.kind);
    let rule = section_rule(block, kind_name);

    // ⛔ **Which rows exist comes from the section, and the lane *narrowing*
    // comes from the rule.** The first half is deliberate — a re-roll varies a
    // section, it does not change which rows it has, so deriving the row list
    // from the rule would grow a bass row on a style whose 808 is the bass.
    //
    // ⛔ The second half was wrong, and hardcoded. `parts_for` sets
    // `only_low_end` when a section asks for the low end *without* asking for
    // drums on a style whose 808 is its bassline, and `render_section` then
    // keeps only the `Bass808` lane. `_defaults` authors exactly that for the
    // bridge — `"bridge": { "parts": ["chords", "bass808"] }` — and every trap
    // and drill model inherits it. Assuming `false` here meant re-rolling that
    // bridge handed back kick, snare, hats and perc where the model authored
    // chords over a bare 808, and fed that wrong kit to the re-rolled melody as
    // `Carry` besides. `parts_for`'s own comment says the narrowing exists "so
    // the bridge does not silently gain a full kit"; this is where it was.
    let narrowed: Vec<Part> = parts_for(&rule, model)
        .into_iter()
        .filter(|request| request.only_low_end)
        .map(|request| request.part)
        .collect();

    let wanted: Vec<PartRequest> = PART_ORDER
        .into_iter()
        .filter(|part| section.patterns.contains_key(part) && !locked.contains(part))
        .map(|part| PartRequest {
            part,
            only_low_end: narrowed.contains(&part),
        })
        .collect();
    if wanted.is_empty() {
        // Every part locked is a no-op, and returning the same song is what
        // makes it one — the UI's change detection compares by identity.
        return Ok(song.clone());
    }
    let scaled = with_density(model, rule.density);
    let section_model = scaled.as_ref().unwrap_or(model);

    // Keyed by index as well as name, so re-rolling verse 2 with the song's own
    // seed cannot land on verse 1's notes.
    let section_seed = rng::derive_seed(seed, &format!("section:{kind_name}@{index}"));

    // The dependencies a lock pins. Re-derived from the stored clip's own seed
    // rather than read out of it — see the doc comment.
    let locked_seed = |part: Part| {
        section
            .patterns
            .get(&part)
            .and_then(|reference| song.pattern(reference))
            .map(|pattern| pattern.seed)
    };
    let held_kit = locked
        .contains(&Part::Drums)
        .then(|| locked_seed(Part::Drums))
        .flatten()
        .map(|seed| crate::generators::drums::generate(section_model, ctx, seed));
    let held_harmony = locked
        .contains(&Part::Chords)
        .then(|| locked_seed(Part::Chords))
        .flatten()
        .map(|seed| crate::generators::chords::generate(section_model, ctx, seed));

    let rendered = render_section(
        section_model,
        ctx,
        section_seed,
        &wanted,
        Carry {
            kit: held_kit.as_deref(),
            harmony: held_harmony.as_ref(),
        },
    );

    // Taken before the move, because `section` borrows the song.
    let mut refs = section.patterns.clone();
    // ⛔ **Moved, not cloned.** A re-roll is bound to the bare `R` key, so it is
    // a rapid-fire gesture — and cloning copied every clip in the store to
    // replace at most five of them, on the thread the host draws its editor
    // from. The caller has already deserialized its own `Song` from the bridge.
    let mut next = song;
    for PartRequest { part, .. } in &wanted {
        let Some(lanes) = rendered.lanes.get(part) else {
            // A part that re-rolled to silence keeps the clip it had. Dropping
            // the row instead would make a re-roll a way to lose a part, and
            // there would be nothing on screen saying it had gone.
            continue;
        };
        // ⛔ **The re-roll seed is in the id, and that is what makes it
        // unique.** Keying on `{kind}@{index}` alone assumed no *other* section
        // could hold a ref to that string — but `cloneSection` inserts a copy
        // sharing the source's refs, and `pasteClips` writes an existing id into
        // another section. Re-roll a section, clone it, re-roll the original
        // again, and the second `insert` replaced the clip the clone was also
        // reading: the clone's notes changed with nothing on screen saying so,
        // which is exactly the byte-stability this task's own criterion states.
        // A fresh seed per re-roll cannot collide, and `prune_patterns` clears
        // the entry the section stopped pointing at.
        let id = pattern_id(model, &format!("{kind_name}@{index}:{seed}"), *part);
        next.patterns.insert(
            id.clone(),
            pattern_for(model, ctx, section_seed, *part, &id, lanes.clone()),
        );
        refs.insert(*part, PatternRef { pattern_id: id });
    }
    next.sections[index].patterns = refs;
    prune_patterns(&mut next);
    Ok(next)
}

/// Drop clips no section names any more.
///
/// ⛔ **Re-rolling repeatedly would otherwise grow the song without bound**, and
/// a `Song` crosses the bridge as JSON on every save — an arrangement carrying
/// forty abandoned takes is a project file forty times the size, and
/// `song_to_smf` would still be handed all of them.
fn prune_patterns(song: &mut Song) {
    let live: std::collections::BTreeSet<&str> = song
        .sections
        .iter()
        .flat_map(|section| section.patterns.values())
        .map(|reference| reference.pattern_id.as_str())
        .collect();
    let dead: Vec<String> = song
        .patterns
        .keys()
        .filter(|id| !live.contains(id.as_str()))
        .cloned()
        .collect();
    for id in dead {
        song.patterns.remove(&id);
    }
}

/// The authored name for a kind, for looking its rule back up.
///
/// The inverse of [`section_kind`], and it has to agree with it: `hook` is the
/// spelling `_defaults` authors, with `chorus` accepted as the alias on the way
/// in. `the_two_halves_of_the_section_vocabulary_agree` holds the pair together.
fn section_name(kind: SectionKind) -> &'static str {
    match kind {
        SectionKind::Intro => "intro",
        SectionKind::Verse => "verse",
        SectionKind::PreChorus => "prechorus",
        SectionKind::Hook => "hook",
        SectionKind::Bridge => "bridge",
        SectionKind::Outro => "outro",
    }
}

/// The structure names a model offers, in authored order, with their weights.
fn structures(block: &Value) -> Vec<Structure> {
    block
        .get("structures")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let sections: Vec<String> = entry
                        .get("sections")?
                        .as_array()?
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect();
                    if sections.is_empty() {
                        return None;
                    }
                    Some(Structure {
                        sections,
                        weight: entry
                            .get("weight")
                            .and_then(Value::as_f64)
                            .unwrap_or(1.0)
                            .max(0.0),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Sample one form, weighted, on its own stream.
///
/// The same shape as [`crate::dataset::modes::pick`], and for the same reason:
/// choosing the form must not move a single note, so a re-roll of the *notes*
/// with the same seed keeps the form and vice versa.
fn pick_structure(
    model: &StyleModel,
    block: &Value,
    seed: u64,
) -> Result<Vec<String>, ArrangeError> {
    let all = structures(block);
    let first = all
        .first()
        .ok_or_else(|| ArrangeError::NoStructures(model.id.clone()))?;

    let total: f64 = all.iter().map(|s| s.weight).sum();
    let chosen = if total <= 0.0 {
        // Every weight zero is an authoring mistake rather than "never pick a
        // form"; falling back to the first keeps the model generatable, exactly
        // as `modes::pick` does.
        first
    } else {
        let mut point = rng::stream(seed, "arrange/structure").random_range(0.0..total);
        all.iter()
            .find(|s| {
                point -= s.weight;
                point < 0.0
            })
            .unwrap_or(first)
    };

    let mut sections = chosen.sections.clone();
    if sections.len() > MAX_SECTIONS {
        sections.truncate(MAX_SECTIONS);
    }
    Ok(sections)
}

/// The engine's section vocabulary.
///
/// `chorus` is an alias for `hook` rather than a sixth kind: they are the same
/// section under two genre's names for it, and pop authoring calls it a chorus
/// while trap authoring calls it a hook.
fn section_kind(name: &str) -> Option<SectionKind> {
    match name.trim().to_ascii_lowercase().as_str() {
        "intro" => Some(SectionKind::Intro),
        "verse" => Some(SectionKind::Verse),
        "prechorus" | "pre-chorus" => Some(SectionKind::PreChorus),
        "hook" | "chorus" => Some(SectionKind::Hook),
        "bridge" => Some(SectionKind::Bridge),
        "outro" => Some(SectionKind::Outro),
        _ => None,
    }
}

/// How many bars this instance of the section runs for.
///
/// Per instance rather than per kind, on a stream keyed by position: a model
/// authoring `"intro": [4, 8]` means each intro is four *or* eight bars, and
/// deciding once for the whole song would throw away the variation it authored.
fn section_bars(block: &Value, name: &str, seed: u64, index: usize) -> u16 {
    let bars = read::number(
        block.get("sectionBars"),
        name,
        4.0,
        &mut rng::stream(seed, &format!("arrange/bars:{index}")),
    );
    // A section of zero bars occupies no time and draws as nothing, so it is a
    // hole in the song rather than a short section.
    (bars.round().max(1.0) as u32).min(u32::from(u16::MAX)) as u16
}

/// What a section rule says, with the fields this module reads.
#[derive(Default)]
struct Rule {
    /// `None` is `"all"` — every part the model can generate.
    parts: Option<Vec<String>>,
    add_layers: Vec<String>,
    density: f64,
    /// Whether this section fades across its length — `_defaults` sets it on
    /// the outro and nowhere else.
    decay: bool,
}

fn section_rule(block: &Value, name: &str) -> Rule {
    let rule = block.get("sectionRules").and_then(|rules| rules.get(name));

    let parts = match rule.and_then(|r| r.get("parts")) {
        // `"all"` is the authored spelling for "everything this model has".
        Some(Value::String(_)) | None => None,
        Some(Value::Array(items)) => Some(
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect(),
        ),
        Some(_) => None,
    };

    Rule {
        parts,
        add_layers: read::strings(rule, "addLayers"),
        density: rule
            .and_then(|r| r.get("density"))
            .and_then(Value::as_f64)
            .unwrap_or(1.0)
            .clamp(0.0, 4.0),
        decay: read::flag(rule, "decay", false),
    }
}

/// The index the back half starts at, if this song switches up (TASK-066).
///
/// `None` is the common case — `_defaults` authors `switchUpProb: 0.15`, so
/// roughly one song in seven varies its back half and the rest repeat, which is
/// the ratio these genres actually use. A switch-up on every song would be a
/// generator that cannot repeat a hook.
///
/// The point is the *midpoint of the section list*, so it always falls on a
/// section boundary. Half of the bar count would land inside a section and give
/// half a verse one melody and half another.
fn switch_up_point(transitions: Option<&Value>, names: &[String], seed: u64) -> Option<usize> {
    let probability = transitions
        .and_then(|t| t.get("switchUpProb"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    if probability <= 0.0 || names.len() < 4 {
        // Under four sections there is no "back half" worth the name: the
        // switch would land on the second section and the form would never
        // establish the thing it is varying from.
        return None;
    }
    if rng::stream(seed, "arrange/switchup").random::<f64>() >= probability {
        return None;
    }
    Some(names.len().div_ceil(2))
}

/// Silence the last beats of whatever runs into a hook (TASK-066).
///
/// ⛔ **Applied to the section *before* the hook, which is why it cannot be
/// decided while that section is being built** — at that point the loop does
/// not know what follows. The drop-out is the beat or two of nothing that makes
/// a hook land, and putting it on the hook itself would cut the top off the
/// thing it is announcing.
fn drop_out_before_hooks(
    sections: &mut [Section],
    transitions: Option<&Value>,
    seed: u64,
    beats_per_bar: u8,
) {
    let spec = transitions.and_then(|t| t.get("dropOutBeats"));
    let Some(spec) = spec else {
        return;
    };
    let probability = spec
        .get("prob")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let beats = spec.get("beats").and_then(Value::as_u64).unwrap_or(0);
    if probability <= 0.0 || beats == 0 {
        return;
    }

    let mut rng = rng::stream(seed, "arrange/dropout");
    for index in 1..sections.len() {
        if sections[index].kind != SectionKind::Hook {
            continue;
        }
        if rng.random::<f64>() >= probability {
            continue;
        }
        let previous = &mut sections[index - 1];
        // Never more than the section has. A four-beat drop-out on a one-bar
        // section is a section that does not play, which reads as a hole rather
        // than as a transition.
        //
        // ⛔ Measured in the *session's* beats per bar, not a hardcoded four.
        // The exporter derives the same span from `time_sig_num`, so a literal
        // 4 here made the engine and the file disagree in any other meter.
        let available = u32::from(previous.bars)
            .saturating_mul(u32::from(beats_per_bar))
            .saturating_sub(u32::from(beats_per_bar));
        previous.drop_out_beats = u8::try_from(beats.min(u64::from(available))).unwrap_or(0);
    }
}

/// What a section wants on one part: the part, and how much of it.
///
/// ⛔ **The lane narrowing rides with the part rather than being re-derived.**
/// It used to be two independent string tests over the same rule — one deciding
/// to pull the drums in, another deciding to strip them to the 808 — and two
/// tests of the same JSON can disagree. They did: a rule listing `drums` under
/// `addLayers` rather than `parts` satisfied the first and failed the second, so
/// a section that explicitly asked for a kit got an 808 and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PartRequest {
    part: Part,
    /// Keep only the 808 lane — set when the section asked for the low end and
    /// this style keeps it inside the drums.
    only_low_end: bool,
}

/// The parts this section asks for, deduplicated and in a stable order.
fn parts_for(rule: &Rule, model: &StyleModel) -> Vec<PartRequest> {
    let named =
        |names: &[String]| -> Vec<Part> { names.iter().filter_map(|n| part_named(n)).collect() };

    let mut wanted: Vec<Part> = match &rule.parts {
        None => PART_ORDER.to_vec(),
        Some(names) => named(names),
    };
    wanted.extend(named(&rule.add_layers));

    // ⛔ A style whose 808 *is* the bassline keeps its low end in the drums, so
    // a section asking for `bass808` without asking for `drums` would come out
    // with no low end at all — a trap bridge of chords over nothing. Asking for
    // the low end pulls the drums in, narrowed to the 808 lane so the bridge
    // does not silently gain a full kit.
    let explicitly = |name: &str| {
        rule.parts
            .iter()
            .chain(std::iter::once(&rule.add_layers))
            .flatten()
            .any(|n| n.eq_ignore_ascii_case(name))
    };
    let low_end_only =
        explicitly("bass808") && !explicitly("drums") && bass::eight_o_eight_is_the_bass(model);
    if low_end_only {
        wanted.push(Part::Drums);
    }

    PART_ORDER
        .into_iter()
        .filter(|part| wanted.contains(part))
        .map(|part| PartRequest {
            part,
            only_low_end: low_end_only && part == Part::Drums,
        })
        .collect()
}

/// The authored names for a part.
///
/// `bass808` maps to the bass because it names *the low end*, which a style
/// either plays with a bass part or with the 808 lane of its drums. Which of
/// the two it is, is the model's business rather than the arrangement's.
fn part_named(name: &str) -> Option<Part> {
    match name.trim().to_ascii_lowercase().as_str() {
        "drums" => Some(Part::Drums),
        "chords" => Some(Part::Chords),
        "melody" => Some(Part::Melody),
        "counter" | "countermelody" => Some(Part::Counter),
        "bass" | "bassline" | "bass808" => Some(Part::Bass),
        _ => None,
    }
}

/// Render every part a section wants, from one seed, sharing their inputs.
///
/// ⛔ **Sharing here is what makes the section coherent, not merely cheap.**
/// `parts::render` re-derives each part's dependencies internally, so calling it
/// five times generates the harmony four times over — and because all five share
/// this section's seed, every one of those is the same harmony. Building it once
/// and handing it down is the same notes and a third less work.
///
/// A part that comes out silent is simply absent from the map: a style whose 808
/// is its bassline authors no separate bass on purpose (FR-007).
fn render_section(
    model: &StyleModel,
    ctx: &SessionContext,
    seed: u64,
    wanted: &[PartRequest],
    carry: Carry<'_>,
) -> SectionRender {
    use crate::generators::{bass, chords, counter, drums, melody};
    use crate::humanize::humanize;

    let asked = |want: &[Part]| wanted.iter().any(|r| want.contains(&r.part));

    // The dependency order from `engine::parts`, run once. Only what this
    // section actually needs is built: a bridge of chords and 808 does not pay
    // for a countermelody nobody asked for.
    let needs_harmony = asked(&[Part::Chords, Part::Melody, Part::Counter, Part::Bass]);
    let needs_kit = asked(&[Part::Drums, Part::Melody, Part::Counter, Part::Bass]);
    let needs_lead = asked(&[Part::Melody, Part::Counter]);

    let harmony = match carry.harmony {
        Some(harmony) => Some(harmony.clone()),
        None => needs_harmony.then(|| chords::generate(model, ctx, seed)),
    };
    // `carry.kit` is the section's already-generated drums, handed in so a
    // switch-up's melody is written around the beat the section actually plays
    // rather than around a second kit nobody hears.
    let kit = match carry.kit {
        Some(kit) => kit.to_vec(),
        None if needs_kit => drums::generate(model, ctx, seed),
        None => Vec::new(),
    };
    let lead = match (needs_lead, &harmony) {
        (true, Some(harmony)) => Some(melody::generate(model, ctx, seed, harmony, &kit)),
        _ => None,
    };

    let mut out: BTreeMap<Part, Vec<LaneTrack>> = BTreeMap::new();
    for request in wanted {
        let mut lanes = match request.part {
            Part::Drums => kit.clone(),
            Part::Chords => harmony
                .as_ref()
                .map(|h| vec![h.track.clone()])
                .unwrap_or_default(),
            Part::Melody => lead.clone().map(|l| vec![l]).unwrap_or_default(),
            Part::Counter => match (&harmony, &lead) {
                (Some(harmony), Some(lead)) => {
                    vec![counter::generate(model, ctx, seed, harmony, lead)]
                }
                _ => Vec::new(),
            },
            Part::Bass => match &harmony {
                Some(harmony) => vec![bass::generate(model, ctx, seed, harmony, &kit)],
                None => Vec::new(),
            },
        };

        if request.only_low_end {
            lanes.retain(|lane| lane.lane == Lane::Sub);
        }

        // ⛔ Humanized here, once per part, rather than inside the shared renders
        // above: feel is applied to a lane exactly once, and `kit` is handed to
        // three generators before it becomes the drum clip.
        humanize(&mut lanes, ctx, seed);
        if !parts::is_silent(&lanes) {
            out.insert(request.part, lanes);
        }
    }
    SectionRender { lanes: out, kit }
}

/// What one section's render produced.
struct SectionRender {
    lanes: BTreeMap<Part, Vec<LaneTrack>>,
    /// The **un-humanized** kit the melodic parts were written against, so a
    /// switch-up can be written against the same one.
    kit: Vec<LaneTrack>,
}

/// Dependencies a render is *handed* rather than deriving from its own seed.
///
/// ⛔ **This is the coherence device the module header's rule needs an escape
/// hatch for.** Everything in a section derives from one seed precisely so the
/// clips a producer hears together were written against each other. Two callers
/// have to break that on purpose and must not break the invariant with it:
///
/// - A **switch-up** varies the melodic parts on a second seed while the drums
///   hold, so it is handed the section's own kit.
/// - A **re-roll of a subset** ([`reroll_section`]) leaves locked parts exactly
///   as they are, so whatever it does re-generate has to be written against the
///   clips that are staying — not against a fresh harmony and kit nobody keeps.
///
/// Both are `None` in the ordinary path, where the seed is the only input.
#[derive(Default, Clone, Copy)]
struct Carry<'a> {
    kit: Option<&'a [LaneTrack]>,
    harmony: Option<&'a Chords>,
}

/// Wrap a section's rendered lanes as the clip the store holds.
fn pattern_for(
    model: &StyleModel,
    ctx: &SessionContext,
    seed: u64,
    part: Part,
    id: &str,
    lanes: Vec<LaneTrack>,
) -> Pattern {
    Pattern {
        loop_region: None,
        clip_region: None,
        id: id.to_owned(),
        part,
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

/// A stable id for the pattern a section kind plays on a part.
fn pattern_id(model: &StyleModel, section: &str, part: Part) -> String {
    format!(
        "{}-{}-{}",
        model.id,
        section.trim().to_ascii_lowercase(),
        format!("{part:?}").to_ascii_lowercase()
    )
}

/// The model with every density parameter scaled, or `None` at 1.0.
///
/// ⛔ **`None` at 1.0 is load-bearing, not an optimisation.** A hook runs at full
/// density, and its notes must be byte-identical to what a plain request for that
/// part and seed returns.
///
/// The scaling walks `blocks` directly rather than round-tripping the model
/// through serde. Every density parameter — `densityPerBar`, `densityRatio`,
/// `fillDensity` — lives in that flattened JSON map and never in a typed field,
/// so the round trip was copying the whole model to reach values already in
/// hand. It was also measurably four times the cost of a clone, and it had two
/// silent `.ok()?` fallbacks that would have generated a section at *full*
/// density if it ever failed — a quiet wrong answer where there is now none.
fn with_density(model: &StyleModel, density: f64) -> Option<StyleModel> {
    if (density - 1.0).abs() < f64::EPSILON {
        return None;
    }
    let mut scaled = model.clone();
    // The arrangement block is skipped: its own `density` fields are the
    // multipliers being applied, not parameters to multiply.
    scaled.blocks.remove(ARRANGEMENT);
    for block in scaled.blocks.values_mut() {
        scale_densities(block, density);
    }
    Some(scaled)
}

fn scale_densities(value: &mut Value, factor: f64) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if DENSITY_KEYS.contains(&key.as_str()) {
                    // A count must not scale to nothing. A section that lists a
                    // part and then generates none of it is a hole in the
                    // arrangement rather than a quiet bar — the producer sees a
                    // row that is meant to be playing and is not.
                    let floor = (key.as_str() == "densityPerBar").then_some(1.0);
                    scale_numbers(child, factor, floor);
                } else {
                    scale_densities(child, factor);
                }
            }
        }
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| scale_densities(item, factor)),
        _ => {}
    }
}

/// Scale a parameter in any of the three authoring forms.
fn scale_numbers(value: &mut Value, factor: f64, floor: Option<f64>) {
    match value {
        Value::Number(number) => {
            if let Some(raw) = number.as_f64() {
                let mut scaled = raw * factor;
                if let Some(floor) = floor {
                    // Only where there was something to keep: an authored zero
                    // means *none of this*, and raising it to one would switch
                    // on a device the model deliberately switched off.
                    if raw >= floor {
                        scaled = scaled.max(floor);
                    }
                }
                if let Some(number) = serde_json::Number::from_f64(scaled) {
                    *value = Value::Number(number);
                }
            }
        }
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| scale_numbers(item, factor, floor)),
        Value::Object(map) => {
            // A weighted choice: the *values* scale, the weights do not. A
            // weight is how often this value is picked, not how much of it.
            if let Some(values) = map.get_mut("values") {
                scale_numbers(values, factor, floor);
            }
        }
        _ => {}
    }
}
