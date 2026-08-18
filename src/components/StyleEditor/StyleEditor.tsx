import { useEffect, useMemo, useState } from 'react';

import { ChevronDown, Download, Upload, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { invoke } from '../../lib/ipc';
import { BLANK, saysNothingOfItsOwn, type Draft } from './draft';
import { idFor } from './id';
import { useSession } from '../../state/session';
import { keptKey, patternsIn, useVariations } from '../../state/variations';
import type {
  Bass808Role,
  Generated,
  Pattern,
  RosterEntry,
  Scale,
  SessionDefaults,
  SnarePlacement,
} from '../../lib/ipc-types';
import './StyleEditor.css';

/**
 * The fewest kept takes a fit will run on.
 *
 * ⛔ **Mirrors `engine::fit::MIN_KEPT`, and the engine is the one that enforces
 * it.** This copy exists so the panel can say `18 / 30` rather than sending a
 * request it knows will be refused; if the two ever disagree the engine wins and
 * the producer sees its message, which is the right way round.
 */
const MIN_KEPT = 30;

/**
 * Build a style of your own, save it, and generate from it (TASK-040U).
 *
 * ⛔ **A user model is a `StyleModel` in the same schema `datasetc` validates**,
 * so there is one format and one validator rather than a second-class one for
 * users. This screen writes that JSON; `plugin/src/models.rs` refuses anything
 * that would not load, *before* it writes — so a mistake here is a message
 * while the producer is still looking at the form, not a style that quietly
 * never appears in the roster.
 *
 * ⛔ **`extends` does the hard part and is why this form is short.** Everything
 * not overridden is inherited, so a style based on `trap` already has trap's
 * drums, its harmony, its fills and its moods — and gains whatever the shipped
 * `trap` learns later. A form that asked for all of it would be a worse model
 * *and* a worse form.
 *
 * ⚠ **The scale list is narrowed to the base's own, and that is a rule rather
 * than a convenience.** Mike's constraint is that an artist generates in its own
 * genre, in a mood it records in, in a scale that genre uses; offering a scale
 * the lineage never uses would make "artist-accurate" untrue at the one moment
 * a producer is deciding what their style *is*.
 */

/**
 * Where the snare lands, as `drums.rs` reads it (TASK-040U).
 *
 * ⚠ **Labelled with numbers rather than words**, which is both how a producer
 * reads a backbeat and why this needs no strings in eighteen catalogs: "2 & 4"
 * and "1 & 3" mean the same thing in every one of them.
 *
 * ⛔⛔ **Typed as `Record<SnarePlacement, string>` since TASK-167, and that is
 * the whole point of the type.** This used to be a `{ value: string }[]` and its
 * own doc said the hazard out loud — *"adding a sixth there without adding it
 * here is a placement no user model can reach"* — which is exactly what happened
 * when TASK-158F added `downbeat_1_3` for `lowend` in two separate hand edits.
 * `SnarePlacement` is now exported from the engine by ts-rs, so a sixth variant
 * that nobody labels here is a **typecheck failure**, not a feature a producer
 * silently cannot reach.
 */
const PLACEMENTS: Record<SnarePlacement, string> = {
  halftime_3: '3',
  backbeat_24: '2 & 4',
  drill_3_4: '3 → 4',
  train_16ths: '16ths',
  downbeat_1_3: '1 & 3',
};

/**
 * The 808's role, by the label a producer reads for it.
 *
 * ⛔ **`Record<Bass808Role, ...>` for the same reason as [`PLACEMENTS`]**
 * (TASK-167): the two radios were hand-written JSX, so a third role added to the
 * engine's enum would have been a role no user model could reach and nothing
 * would have said so. Keyed by the exported union, a new variant fails the
 * typecheck here.
 */
const BASS_ROLES: Record<Bass808Role, string> = {
  bassline: 'styles.bassIsTheBass',
  counter_riff: 'styles.bassCounterRiff',
};

/**
 * A stored value read back as one of a closed set, or unset.
 *
 * ⛔⛔ **Narrowing `snare` and `bassRole` to the exported unions found this**
 * (TASK-167): both were read straight out of a saved style's JSON as plain
 * strings and assigned without a check, so a file written by an older build — or
 * hand-edited — could put a value in the draft that `SnarePlacement::parse` and
 * `Bass808Role::parse` both refuse. Opening and saving such a style would then
 * author a placement the engine cannot read, silently.
 *
 * ⚠ Membership is asked of the label maps rather than a second list, so the
 * check cannot drift from the unions the radios are built from.
 *
 * ⚠ **What this costs, stated because a review raised it and the answer is a
 * judgement rather than a fix:** a style authored by a *newer* build has its
 * unrecognised placement or role read back as inherit, and saving then drops it
 * — and because the write side refuses `slideProb` without a role, an 808 slide
 * would go with it. Dropping is still the better half of the trade: the
 * alternative is carrying a value into the draft that both `parse`s refuse, and
 * saving *that* authors a style the engine cannot read at all. A build that
 * needs to round-trip forward-authored styles wants the write side to refuse or
 * warn, which is a feature rather than a validation rule.
 */
function storedChoice<K extends string>(labels: Record<K, string>, value: unknown): K | '' {
  // ⚠ `in`, not `Object.hasOwn`: this repo sets no `build.target`, so Vite's
  // default baseline still includes Safari 14 and does not polyfill built-ins —
  // `Object.hasOwn` needs 15.4 and would throw on open. Same semantics here,
  // because `labels` is a literal with no prototype chain to inherit from.
  return typeof value === 'string' && value in labels ? (value as K) : '';
}

/** The roll subdivisions `grid::note_value_ticks` resolves, as producers write them. */
const ROLLS = ['16', '32', '16T', '8T'];

/**
 * One value added to or removed from a checkbox group's list.
 *
 * ⚠ The three groups — scales, rolls, progression families — each spelled this
 * ternary out inline, so the array-toggle was written three times and differed
 * only in the field it read. Generic over the member type so `scales: Scale[]`
 * keeps its narrower type rather than widening to `string[]`.
 */
const withToggled = <T extends string>(list: T[], value: T, on: boolean): T[] =>
  on ? [...list, value] : list.filter((v) => v !== value);

/**
 * The progression families offered, as roman numerals.
 *
 * ⛔ **Written at equal weight, and that is the honest simplification.** A
 * shipped model weights its families because its research says which one the
 * artist reaches for most; a producer ticking boxes has said only "these ones".
 * Inventing weights they did not choose would put numbers in their model that
 * nobody measured.
 *
 * ⚠ Roman numerals need no translation — they are the same in every catalog,
 * which is why these carry no `t()` call.
 */
const FAMILIES = [
  'i',
  'i-VI',
  'i-VII',
  'i-iv',
  'i-VI-VII',
  'i-VII-VI',
  'I-V-vi-IV',
  'vi-IV-I-V',
];

export function StyleEditor({
  editing,
  onClose,
}: {
  editing: string | null;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const roster = useSession((s) => s.roster);
  const refreshRoster = useSession((s) => s.refreshRoster);
  const select = useSession((s) => s.select);

  // ⛔ **Start from what you liked, not from a blank form** (TASK-040U). A new
  // style opens seeded with the generation on screen — its artist, its tempo,
  // its scale — because "make a style from this" is the difference between an
  // editor a producer will use and one they will open once. There is no button
  // for it: the way in is already open in front of them, and a blank form
  // beside a beat they just made would be asking them to retype it.
  //
  // ⚠ The **pattern**, not the pins: the pattern is what actually came out, and
  // an unpinned session has no pins to read. Falling back to the selection's own
  // defaults covers the case where nothing has been generated yet.
  // ⚠ **Derived from the two slices it actually depends on, and memoised on
  // them.** The dialog re-renders on every keystroke in the name field and every
  // tick of a slider, and the generation log is deliberately never capped — so
  // re-flattening it each time would be a copy of the producer's whole session
  // allocated and thrown away per character.
  const kept = useVariations((s) => s.kept);
  const entries = useVariations((s) => s.entries);
  const keptTakes = useMemo(
    () =>
      Object.values(entries)
        .flat()
        .filter((entry) => kept[keptKey(entry.part, entry.seed)]),
    [entries, kept],
  );
  // ⛔ **A kept file counts as the patterns it holds, not as one file**
  // (TASK-040T). `engine::fit::MIN_KEPT` is a floor on the number of *patterns*
  // measured — its own doc says thirty is where "a single kept outlier sets a
  // range's edge" stops being true — so counting a four-part `.mid` as one would
  // put a number in front of the producer that the engine does not use.
  //
  // ⚠ **Counted through `patternsIn`, which is what `keptFilePatterns()` — the
  // set actually sent to `user_model_train` — is built from.** Re-deriving the
  // sum here meant the number in front of the producer and the training set were
  // two expressions: any future change to one, deduping identical patterns
  // across files say, would have made the readout lie. Passing the subscribed
  // map rather than reading `getState()` inside keeps the memo's dependency real
  // as well as its answer.
  const keptFiles = useVariations((s) => s.keptFiles);
  const keptCount = useMemo(
    () => keptTakes.length + patternsIn(keptFiles).length,
    [keptTakes, keptFiles],
  );

  /**
   * The draft as the dialog opened it — seeded from the session, then frozen.
   *
   * ⛔ **Kept as its own state, because it is the baseline `saysNothingOfItsOwn`
   * measures against.** `BLANK` is not that baseline: `swing` and the BPM range
   * below are read from the selected model's defaults, so an untouched draft
   * over `trap` already differs from `BLANK` and the rule would never fire.
   */
  const [opened] = useState<Draft>(() => {
    // ⚠ Read once, through `getState`, rather than subscribed: these three are
    // only ever wanted as they were when the dialog opened, and subscribing
    // would re-render it on every generation behind it for nothing.
    const { patterns, selectedId, defaults } = useSession.getState();
    const pattern = Object.values(patterns)[0] ?? null;
    const base = pattern?.artistId ?? selectedId ?? BLANK.basedOn;
    const bpm = Math.round(pattern?.bpm ?? defaults?.bpm ?? 0);
    return {
      ...BLANK,
      basedOn: base,
      ...(bpm > 0 ? { bpmMin: bpm - 10, bpmMax: bpm + 10 } : {}),
      swing: defaults?.swing.amount ?? BLANK.swing,
    };
  });
  const [draft, setDraft] = useState<Draft>(opened);
  /**
   * Whether there was a beat on screen when this dialog opened.
   *
   * ⚠ **Read once and frozen**, for the same reason the draft's own seeding is:
   * it is a fact about the moment the producer pressed the button. It is the
   * other half of Mike's rule — a draft with nothing of its own is still worth
   * saving when it was seeded from a take, and worth nothing when it was not.
   */
  const [openedOverATake] = useState(
    () => Object.keys(useSession.getState().patterns).length > 0,
  );
  const [offered, setOffered] = useState<Scale[]>([]);
  // ⛔ **What copying this style's samples would cost, and whether the producer
  // said yes.** Mike, 2026-08-09: *"ensure that the end user knows that creating
  // their own original artist adds copies of samples … and ensure that they want
  // to do that before allowing the app to copy the samples."* Default off, and
  // the size is shown rather than described — "some samples" is not consent.
  const [cost, setCost] = useState<SampleCost | null>(null);
  const [copySamples, setCopySamples] = useState(false);
  // The base picker is a list this dialog draws — see the note where it renders.
  const [basesOpen, setBasesOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);

  const bases = useMemo(() => {
    // Your own styles are not offered as a base. Extending one is legitimate and
    // the resolver handles it, but a chain of user models is also the easiest
    // way to build a cycle, and this is the first release of the feature.
    const own = roster.filter((entry) => !entry.mine);
    return [
      ...own.filter((entry) => entry.type === 'genre'),
      ...own.filter((entry) => entry.type === 'artist'),
    ];
  }, [roster]);

  // The base's own scales, which is what the picker may offer.
  useEffect(() => {
    let live = true;
    void invoke<SessionDefaults>('session_defaults', { styleId: draft.basedOn })
      .then((defaults) => {
        if (!live) return;
        setOffered(defaults.scales);
        // Anything chosen that this base does not record in is dropped rather
        // than carried silently — the constraint has to survive changing base.
        setDraft((current) => ({
          ...current,
          scales: current.scales.filter((scale) => defaults.scales.includes(scale)),
        }));
      })
      .catch(() => {
        if (live) setOffered([]);
      });
    return () => {
      live = false;
    };
  }, [draft.basedOn]);

  // The one-shots this session has assigned, and what copying them would take.
  // ⚠ Asked once when the dialog opens: the answer is a `stat` per file, and it
  // is the number the producer is being asked to agree to.
  // ⛔ **The plugin sources its own paths.** This used to read `kit_state` and
  // hand the paths back over the bridge, which a security review correctly
  // called an existence-and-size oracle over the whole disk: the page could name
  // *any* file and read the answer. The plugin already holds the assignments, so
  // nothing about them crosses the boundary now.
  useEffect(() => {
    let live = true;
    void invoke<SampleCost>('user_model_sample_cost')
      .then((answer) => {
        if (live) setCost(answer.count > 0 ? answer : null);
      })
      .catch(() => {
        if (live) setCost(null);
      });
    return () => {
      live = false;
    };
  }, []);

  // Editing an existing style reads back the file that was written, so what is
  // on screen is what is on disk rather than a second reconstruction of it.
  useEffect(() => {
    if (editing === null) return;
    void invoke<string>('user_model_export', { id: editing })
      .then((text) => setDraft(draftFrom(JSON.parse(text) as Record<string, unknown>)))
      .catch((reason: unknown) => setError(String(reason)));
  }, [editing]);

  const id = idFor(draft.name);

  async function save() {
    setError(null);
    if (id === '') {
      setError(t('styles.nameNeeded'));
      return;
    }
    // ⛔ **Refused here rather than by greying the button out.** An unedited
    // song is deliberately not saved either (`song.ts`), and this is the same
    // rule: the app does not write a file for a change nobody made. It *says*
    // so, because a control that quietly greys out is a rule the producer has
    // to guess — which is the failure TASK-040T's own note names.
    if (!openedOverATake && saysNothingOfItsOwn(draft, opened)) {
      setError(t('styles.nothingToSave'));
      return;
    }

    try {
      const entry = await invoke<RosterEntry>('user_model_save', {
        model: modelFrom(draft, id, bases),
      });

      // ⛔ **Only here, and only if they ticked it.** Saving a style copies
      // nothing on its own — the plugin has two separate commands precisely so
      // that a save can never take somebody's disk space by accident.
      let copied = 0;
      if (copySamples && cost !== null) {
        const landed = await invoke<string[]>('user_model_copy_samples', { id });
        copied = landed.length;
      }

      // ⛔ **A rename leaves no second copy behind.** The id is derived from the
      // name, so renaming a saved style writes a *new* file — and without this
      // the old one stayed in the roster too, turning one rename into two
      // styles with nothing saying which was which.
      if (editing !== null && editing !== entry.id) {
        await invoke('user_model_delete', { id: editing });
      }

      await refreshRoster();
      select(entry.id);
      setSaved(
        copied > 0
          ? `${entry.name} — ${t('styles.samplesCopied', { count: copied })}`
          : entry.name,
      );
    } catch (reason: unknown) {
      setError(String(reason));
    }
  }

  /**
   * Fit a model to the takes the producer kept (TASK-040T).
   *
   * ⛔ **The kept takes are regenerated before they are sent.** The variation
   * log stores seeds rather than notes — the engine is deterministic, so an
   * entry regenerates its own beat exactly — and the fit measures `Pattern`s.
   * Regenerating here is what keeps that one measurement path: a `.mid` dragged
   * in from the browser will arrive as a `Pattern` too, so a model fitted from
   * files cannot drift from one fitted from generations.
   */
  async function trainFromKept() {
    setError(null);
    if (id === '') {
      setError(t('styles.nameNeeded'));
      return;
    }

    try {
      // ⛔ **One at a time, not `Promise.all`.** The plugin answers the webview
      // *synchronously*, so N requests do not overlap — they queue on one
      // thread. Firing them together started N copies of the same 15-second
      // fetch deadline at the same instant, so with thirty takes the tail
      // aborted and took every completed generation with it. In sequence each
      // call gets its own deadline, and a generation is milliseconds.
      const kept: Pattern[] = [];
      for (const take of keptTakes) {
        // ⛔ **The clip, not the reply.** `generate_pattern` hands back the take
        // AND the parts it was written against (TASK-166), and `invoke<Pattern>`
        // is an explicit generic — so pushing the reply straight in typechecked
        // clean while sending `{pattern, upstream}` objects to
        // `user_model_train`, whose Rust side reads `Vec<Pattern>` and would
        // have failed on a missing `lanes` for every model trained from kept
        // takes. Caught by review, not by the compiler.
        const { pattern: clip } = await invoke<Generated>('generate_pattern', {
          request: {
            styleId: take.artistId,
            part: take.part,
            seed: take.seed,
            songSeed: take.songSeed,
            bars: take.bars,
            mood: take.mood ?? undefined,
          },
        });
        kept.push(clip);
      }

      // ⛔ **The kept files go in beside them** (TASK-040T). They are already
      // `Pattern`s — the explorer keeps a `.mid`'s own split — so they need no
      // regeneration and, unlike a take, could not be regenerated at all: there
      // is no seed that rebuilds somebody else's file.
      kept.push(...useVariations.getState().keptFilePatterns());

      // ⚠ **Only the takes carry a mood.** A file was not generated in one, and
      // recording a mood it does not have would be provenance nobody could act
      // on. An all-file training set therefore trains in no named mood, which is
      // the honest answer rather than a missing one.
      const moods = [...new Set(keptTakes.map((take) => take.mood).filter((m) => m !== null))];
      const entry = await invoke<RosterEntry>('user_model_train', {
        id,
        name: draft.name.trim(),
        base: draft.basedOn,
        moods,
        kept,
      });
      await refreshRoster();
      select(entry.id);
      setSaved(entry.name);
    } catch (reason: unknown) {
      setError(String(reason));
    }
  }

  async function remove() {
    if (editing === null) return;
    try {
      await invoke('user_model_delete', { id: editing });
      await refreshRoster();
      onClose();
    } catch (reason: unknown) {
      setError(String(reason));
    }
  }

  async function exportOne() {
    if (id === '') return;
    try {
      const text = await invoke<string>('user_model_export', { id });
      await navigator.clipboard.writeText(text);
      setSaved(draft.name);
    } catch (reason: unknown) {
      setError(String(reason));
    }
  }

  async function importOne() {
    try {
      const text = await navigator.clipboard.readText();
      const entry = await invoke<RosterEntry>('user_model_import', { text });
      await refreshRoster();
      select(entry.id);
      setSaved(entry.name);
    } catch (reason: unknown) {
      setError(String(reason));
    }
  }

  const baseName = bases.find((entry) => entry.id === draft.basedOn)?.name ?? draft.basedOn;

  return (
    <div className="styleeditor" role="dialog" aria-modal="true" aria-label={t('styles.title')}>
      <div className="styleeditor__panel">
        <header className="styleeditor__head">
          <h2>{t('styles.title')}</h2>
          <button
            type="button"
            className="btn-ghost"
            onClick={onClose}
            aria-label={t('common.close')}
          >
            <X size={16} aria-hidden />
          </button>
        </header>

        <div className="styleeditor__body">
          <label className="styleeditor__field">
            <span>{t('styles.name')}</span>
            <input
              type="text"
              value={draft.name}
              placeholder={t('styles.namePlaceholder')}
              onChange={(e) => setDraft({ ...draft, name: e.target.value })}
            />
          </label>

          {/* ⛔ **Not a native `<select>`, and that was a real defect** (Mike,
              2026-08-09, with a screenshot). A `<select>` popup inside WebView2
              is drawn by the OS, at OS scale, positioned against the *window*
              rather than the field — with thirty models in it, it filled most of
              the screen and floated well away from the control it belonged to.
              Nothing about a native popup is styleable, so the only fix is to
              own it: a button and a list this dialog draws, anchored under the
              field, the same width as the field, and bounded in height. */}
          <div className="styleeditor__field styleeditor__combo">
            <span id="styles-basedon">{t('styles.basedOn')}</span>
            <button
              type="button"
              className="styleeditor__combo-button"
              aria-labelledby="styles-basedon"
              aria-expanded={basesOpen}
              onClick={() => setBasesOpen((open) => !open)}
            >
              {baseName}
              <ChevronDown size={14} aria-hidden />
            </button>

            {basesOpen && (
              <ul className="styleeditor__menu" role="listbox" aria-labelledby="styles-basedon">
                {bases.map((entry) => (
                  <li key={entry.id}>
                    <button
                      type="button"
                      role="option"
                      aria-selected={entry.id === draft.basedOn}
                      onClick={() => {
                        setDraft({ ...draft, basedOn: entry.id });
                        setBasesOpen(false);
                      }}
                    >
                      {entry.name}
                    </button>
                  </li>
                ))}
              </ul>
            )}
            <small>{t('styles.basedOnHint')}</small>
          </div>

          <fieldset className="styleeditor__field">
            <legend>{t('styles.tempo')}</legend>
            <input
              type="number"
              aria-label={t('styles.tempo')}
              min={40}
              max={300}
              value={draft.bpmMin}
              onChange={(e) => setDraft({ ...draft, bpmMin: Number(e.target.value) })}
            />
            <input
              type="number"
              aria-label={t('styles.tempo')}
              min={40}
              max={300}
              value={draft.bpmMax}
              onChange={(e) => setDraft({ ...draft, bpmMax: Number(e.target.value) })}
            />
          </fieldset>

          <label className="styleeditor__field">
            <span>{t('styles.swing')}</span>
            <input
              type="range"
              min={0.5}
              max={0.7}
              step={0.01}
              value={draft.swing}
              onChange={(e) => setDraft({ ...draft, swing: Number(e.target.value) })}
            />
          </label>

          <label className="styleeditor__field">
            <span>{t('styles.hats')}</span>
            <input
              type="range"
              min={0}
              max={1}
              step={0.01}
              value={draft.hats}
              onChange={(e) => setDraft({ ...draft, hats: Number(e.target.value) })}
            />
          </label>

          <fieldset className="styleeditor__field">
            <legend>{t('styles.melody')}</legend>
            <input
              type="number"
              aria-label={t('styles.melody')}
              min={0}
              max={16}
              value={draft.melodyMin}
              onChange={(e) => setDraft({ ...draft, melodyMin: Number(e.target.value) })}
            />
            <input
              type="number"
              aria-label={t('styles.melody')}
              min={0}
              max={16}
              value={draft.melodyMax}
              onChange={(e) => setDraft({ ...draft, melodyMax: Number(e.target.value) })}
            />
          </fieldset>

          <fieldset className="styleeditor__field styleeditor__scales">
            <legend>{t('styles.scales')}</legend>
            {offered.map((scale) => (
              <label key={scale}>
                <input
                  type="checkbox"
                  checked={draft.scales.includes(scale)}
                  onChange={(e) =>
                    setDraft({
                      ...draft,
                      scales: withToggled(draft.scales, scale, e.target.checked),
                    })
                  }
                />
                {t(`scales.${scale}`, scale)}
              </label>
            ))}
            <small>{t('styles.scalesHint', { name: baseName })}</small>
          </fieldset>

          {/* ⛔⛔ **The four blocks TASK-040U's entry named**: *"they cannot yet
              reach roll vocabulary, snare placement, 808 behaviour or progression
              families"*. Those four are most of what separates one authored style
              from another, and a producer building a vibe by hand could reach
              none of them.

              ⚠ **Every group leads with "from the base" and starts there.** An
              authored value *replaces* the parent's (`inherit::deep_merge`), so a
              control with no unset state would make opening the dialog enough to
              overwrite the thing the style is based on. */}
          <fieldset className="styleeditor__field styleeditor__choice">
            <legend>{t('styles.snare')}</legend>
            <label>
              <input
                type="radio"
                name="styles-snare"
                checked={draft.snare === ''}
                onChange={() => setDraft({ ...draft, snare: '' })}
              />
              {t('styles.inherit')}
            </label>
            {(Object.entries(PLACEMENTS) as [SnarePlacement, string][]).map(
              ([placement, label]) => (
                <label key={placement}>
                  <input
                    type="radio"
                    name="styles-snare"
                    checked={draft.snare === placement}
                    onChange={() => setDraft({ ...draft, snare: placement })}
                  />
                  {label}
                </label>
              ),
            )}
            <small>{t('styles.snareHint')}</small>
          </fieldset>

          <fieldset className="styleeditor__field styleeditor__choice">
            <legend>{t('styles.rolls')}</legend>
            {ROLLS.map((roll) => (
              <label key={roll}>
                <input
                  type="checkbox"
                  checked={draft.rolls.includes(roll)}
                  onChange={(e) =>
                    setDraft({
                      ...draft,
                      rolls: withToggled(draft.rolls, roll, e.target.checked),
                    })
                  }
                />
                {roll}
              </label>
            ))}
            <small>{t('styles.rollsHint', { name: baseName })}</small>
          </fieldset>

          <fieldset className="styleeditor__field styleeditor__choice">
            <legend>{t('styles.bass808')}</legend>
            <label>
              <input
                type="radio"
                name="styles-bass"
                checked={draft.bassRole === ''}
                onChange={() => setDraft({ ...draft, bassRole: '' })}
              />
              {t('styles.inherit')}
            </label>
            {(Object.entries(BASS_ROLES) as [Bass808Role, string][]).map(([role, label]) => (
              <label key={role}>
                <input
                  type="radio"
                  name="styles-bass"
                  checked={draft.bassRole === role}
                  onChange={() => setDraft({ ...draft, bassRole: role })}
                />
                {t(label)}
              </label>
            ))}
            {/* ⚠ Disabled until a role is chosen, because a slide probability
                with no role is half an 808 — and the disabled state is what says
                so without a sentence. */}
            <label className="styleeditor__slide">
              <span>{t('styles.slide')}</span>
              <input
                type="range"
                min={0}
                max={1}
                step={0.01}
                disabled={draft.bassRole === ''}
                value={draft.slide}
                onChange={(e) => setDraft({ ...draft, slide: Number(e.target.value) })}
              />
            </label>
            <small>{t('styles.bass808Hint')}</small>
          </fieldset>

          <fieldset className="styleeditor__field styleeditor__choice">
            <legend>{t('styles.progressions')}</legend>
            {FAMILIES.map((family) => (
              <label key={family}>
                <input
                  type="checkbox"
                  checked={draft.progressions.includes(family)}
                  onChange={(e) =>
                    setDraft({
                      ...draft,
                      progressions: withToggled(draft.progressions, family, e.target.checked),
                    })
                  }
                />
                {family.split('-').join('–')}
              </label>
            ))}
            <small>{t('styles.progressionsHint', { name: baseName })}</small>
          </fieldset>
        </div>

        {error !== null && <p className="styleeditor__error">{error}</p>}
        {error === null && saved !== null && (
          <p className="styleeditor__saved">{t('styles.saved', { name: saved })}</p>
        )}

        {/* ⛔ **The consent, and it is a real one.** Shown only when there is
            something to copy, stating the count and the size rather than
            describing them — "some samples" is not consent — and unticked by
            default so the producer has to reach for it. Nothing on the Rust side
            copies without this. */}
        {cost !== null && cost.count > 0 && (
          <div className="styleeditor__samples">
            <strong>{t('styles.samplesHeading')}</strong>
            <label>
              <input
                type="checkbox"
                checked={copySamples}
                onChange={(e) => setCopySamples(e.target.checked)}
              />
              {t('styles.samplesCopy', { count: cost.count, size: megabytes(cost.bytes) })}
            </label>
            <small>{t('styles.samplesNote')}</small>
          </div>
        )}

        {/* ⛔ **Training sits beside the form rather than in a panel of its
            own** (TASK-040T). It writes the same object the form does — a fit
            *is* a set of authored numbers — so a producer who has kept thirty
            takes should be able to press one button here instead of learning a
            second place where styles come from. The count is shown rather than
            the button being greyed out: `18 / 30 kept` is something you can act
            on and a dead control is not. */}
        <p className="styleeditor__kept">
          {t('styles.kept', { count: keptCount, needed: MIN_KEPT })}
        </p>

        <footer className="styleeditor__foot">
          <button type="button" className="btn-ghost" onClick={() => void importOne()}>
            <Upload size={14} aria-hidden /> {t('styles.import')}
          </button>
          <button type="button" className="btn-ghost" onClick={() => void exportOne()}>
            <Download size={14} aria-hidden /> {t('styles.export')}
          </button>
          {editing !== null && (
            <button type="button" className="btn-ghost" onClick={() => void remove()}>
              {t('styles.delete')}
            </button>
          )}
          <button
            type="button"
            className="btn-ghost"
            disabled={keptCount < MIN_KEPT}
            onClick={() => void trainFromKept()}
          >
            {t('styles.train')}
          </button>
          <button type="button" className="btn-generate" onClick={() => void save()}>
            {t('styles.save')}
          </button>
        </footer>
      </div>
    </div>
  );
}

/** What copying a style's samples would take, as the bridge answers it. */
type SampleCost = { count: number; bytes: number };

/// A byte count a producer can judge. Deliberately coarse — the decision is
/// "is that a lot of my disk", not an audit.
function megabytes(bytes: number): string {
  const mb = bytes / (1024 * 1024);
  return mb < 0.1 ? '< 0.1 MB' : `${mb.toFixed(1)} MB`;
}

/** The authored JSON a draft becomes. */
function modelFrom(draft: Draft, id: string, bases: RosterEntry[]): Record<string, unknown> {
  const base = bases.find((entry) => entry.id === draft.basedOn);

  const model: Record<string, unknown> = {
    id,
    type: 'artist',
    name: draft.name.trim(),
    extends: [draft.basedOn],
    // ⛔ The base's own tags and relations, so a saved style is reachable by the
    // same searches and the same cross-filter as the thing it came from. An
    // empty `genres` would make it findable only by its exact name.
    genres: base?.genres ?? [],
    relatedGenres: base?.type === 'genre' ? [draft.basedOn] : (base?.relatedGenres ?? []),
    session: {
      bpm: {
        min: Math.min(draft.bpmMin, draft.bpmMax),
        max: Math.max(draft.bpmMin, draft.bpmMax),
        mode: Math.round((draft.bpmMin + draft.bpmMax) / 2),
      },
      swing: { grid: '16th', amount: draft.swing },
    },
    drums: { hihat: { fillDensity: draft.hats } },
    melody: {
      densityPerBar: [
        Math.min(draft.melodyMin, draft.melodyMax),
        Math.max(draft.melodyMin, draft.melodyMax),
      ],
    },
  };

  // ⛔ **Written only when the producer set them, for the reason `scales` gives
  // below: `inherit::deep_merge` has an authored value *replace* the base's, so
  // a field written unasked is the base quietly overruled.** A partial block is
  // safe — objects merge key by key, so `snare: { placement }` keeps the parent's
  // ghosts and its layering — but an array replaces outright, which is exactly
  // what "these families and no others" should mean.
  const drums = model.drums as Record<string, unknown>;
  if (draft.snare !== '') {
    drums.snare = { placement: draft.snare };
  }
  if (draft.rolls.length > 0) {
    // ⛔⛔ **`weights` must be written beside `values`, and omitting it made the
    // control unusable.** `inherit::deep_merge` replaces the `values` array
    // whole but merges `vocab` key by key — so the producer's list arrived
    // against the *parent's* weights, and 566 of the 600 shipped models author
    // that pair. Tick one subdivision over a base with five and
    // `validate::check_weighted` refuses the save with `5 weights for 1 values`,
    // a JSON pointer to a field they never touched. Flat weights are the honest
    // reading of a checkbox list: the producer said which subdivisions, not how
    // often each.
    (drums.hihat as Record<string, unknown>).rolls = {
      vocab: { values: draft.rolls, weights: draft.rolls.map(() => 1) },
    };
  }
  if (draft.bassRole !== '') {
    // ⚠ The slide rides with the role and never alone — see `Draft`.
    drums.bass808 = { role: draft.bassRole, slideProb: draft.slide };
  }
  if (draft.progressions.length > 0) {
    model.chords = {
      progressionFamilies: draft.progressions.map((roman) => ({
        roman: roman.split('-'),
        weight: 1,
      })),
    };
  }

  // ⚠ Omitted entirely when nothing is ticked, rather than written as an empty
  // list: an empty `scales` would *replace* the base's rather than inherit it,
  // and a style with no scale to generate in is a style that generates nothing.
  //
  // ⛔⛔ **`weights` must be written beside `values`, and this control shipped
  // without it long before TASK-040U copied the shape.** `deep_merge` replaces
  // the `values` array whole but merges the object around it key by key, so the
  // producer's list arrived against the base's `weights` — and **599 of the 620
  // model files author that pair**. Unless the number of scales ticked happened
  // to equal the base's weight count, `validate::check_weighted` refused the
  // save with `3 weights for 1 values`, naming a field nobody touched. The Rolls
  // control was written by copying this block and inherited the same defect;
  // `engine/tests/user_partials.rs` is what found both, by merging every
  // fragment this function writes over every shipped base and running the real
  // lint. Flat weights are the honest reading of a checkbox list — the producer
  // said which scales, not how often each.
  if (draft.scales.length > 0) {
    (model.session as Record<string, unknown>).scales = {
      values: draft.scales,
      weights: draft.scales.map(() => 1),
    };
  }

  return model;
}

/** The draft a saved file reads back as. */
function draftFrom(model: Record<string, unknown>): Draft {
  const session = (model.session ?? {}) as Record<string, unknown>;
  const bpm = (session.bpm ?? {}) as Record<string, number>;
  const swing = (session.swing ?? {}) as Record<string, number>;
  const scales = (session.scales ?? {}) as { values?: Scale[] };
  const drums = (model.drums ?? {}) as {
    hihat?: { fillDensity?: number; rolls?: { vocab?: { values?: string[] } } };
    snare?: { placement?: string };
    bass808?: { role?: string; slideProb?: number };
  };
  const melody = (model.melody ?? {}) as { densityPerBar?: number[] };
  const chords = (model.chords ?? {}) as { progressionFamilies?: { roman?: string[] }[] };

  return {
    name: typeof model.name === 'string' ? model.name : '',
    basedOn: Array.isArray(model.extends) ? String(model.extends[0] ?? 'trap') : 'trap',
    bpmMin: bpm.min ?? BLANK.bpmMin,
    bpmMax: bpm.max ?? BLANK.bpmMax,
    swing: swing.amount ?? BLANK.swing,
    hats: drums.hihat?.fillDensity ?? BLANK.hats,
    melodyMin: melody.densityPerBar?.[0] ?? BLANK.melodyMin,
    melodyMax: melody.densityPerBar?.[1] ?? BLANK.melodyMax,
    scales: scales.values ?? [],
    // ⚠ **Absent reads back as unset, not as a default** — the same distinction
    // the write side keeps. A saved style that never chose a snare placement must
    // reopen still inheriting one, or opening and saving would author it.
    snare: storedChoice(PLACEMENTS, drums.snare?.placement),
    rolls: drums.hihat?.rolls?.vocab?.values ?? [],
    bassRole: storedChoice(BASS_ROLES, drums.bass808?.role),
    slide: drums.bass808?.slideProb ?? BLANK.slide,
    progressions: (chords.progressionFamilies ?? [])
      .map((family) => (family.roman ?? []).join('-'))
      .filter((roman) => roman !== ''),
  };
}
