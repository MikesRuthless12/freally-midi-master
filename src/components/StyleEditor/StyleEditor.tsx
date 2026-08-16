import { useEffect, useMemo, useState } from 'react';

import { ChevronDown, Download, Upload, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { invoke } from '../../lib/ipc';
import { idFor } from './id';
import { useSession } from '../../state/session';
import { keptKey, useVariations } from '../../state/variations';
import type { Pattern, RosterEntry, Scale, SessionDefaults } from '../../lib/ipc-types';
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

type Draft = {
  name: string;
  basedOn: string;
  bpmMin: number;
  bpmMax: number;
  swing: number;
  hats: number;
  melodyMin: number;
  melodyMax: number;
  scales: Scale[];
};

const BLANK: Draft = {
  name: '',
  basedOn: 'trap',
  bpmMin: 130,
  bpmMax: 150,
  swing: 0.54,
  hats: 0.5,
  melodyMin: 3,
  melodyMax: 7,
  scales: [],
};

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
  const keptFiles = useVariations((s) => s.keptFiles);
  const keptCount = useMemo(
    () =>
      keptTakes.length +
      Object.values(keptFiles).reduce((sum, file) => sum + file.patterns.length, 0),
    [keptTakes, keptFiles],
  );

  const [draft, setDraft] = useState<Draft>(() => {
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
        kept.push(
          await invoke<Pattern>('generate_pattern', {
            request: {
              styleId: take.artistId,
              part: take.part,
              seed: take.seed,
              songSeed: take.songSeed,
              bars: take.bars,
              mood: take.mood ?? undefined,
            },
          }),
        );
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
                      scales: e.target.checked
                        ? [...draft.scales, scale]
                        : draft.scales.filter((s) => s !== scale),
                    })
                  }
                />
                {t(`scales.${scale}`, scale)}
              </label>
            ))}
            <small>{t('styles.scalesHint', { name: baseName })}</small>
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

  // ⚠ Omitted entirely when nothing is ticked, rather than written as an empty
  // list: an empty `scales` would *replace* the base's rather than inherit it,
  // and a style with no scale to generate in is a style that generates nothing.
  if (draft.scales.length > 0) {
    (model.session as Record<string, unknown>).scales = { values: draft.scales };
  }

  return model;
}

/** The draft a saved file reads back as. */
function draftFrom(model: Record<string, unknown>): Draft {
  const session = (model.session ?? {}) as Record<string, unknown>;
  const bpm = (session.bpm ?? {}) as Record<string, number>;
  const swing = (session.swing ?? {}) as Record<string, number>;
  const scales = (session.scales ?? {}) as { values?: Scale[] };
  const drums = (model.drums ?? {}) as { hihat?: { fillDensity?: number } };
  const melody = (model.melody ?? {}) as { densityPerBar?: number[] };

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
  };
}
