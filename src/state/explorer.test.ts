import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  RAIL_DEFAULT_WIDTH,
  RAIL_MAX_WIDTH,
  RAIL_MIN_WIDTH,
  clampRailWidth,
  flattenTree,
  formatSeconds,
  innermostExpanded,
  isInside,
  samePath,
  useExplorer,
} from './explorer';
import { outlineOf } from '../components/Explorer/waveform';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('../lib/ipc', () => ({ invoke }));

beforeEach(() => {
  invoke.mockReset();
  window.localStorage.clear();
  useExplorer.setState({
    roots: [],
    folder: null,
    children: {},
    expanded: [],
    truncatedIn: [],
    missingRoots: [],
    activeRoot: null,
    favourites: [],
    starred: new Set(),
    midiSplit: null,
    midiAudition: null,
    loaded: false,
    selected: null,
    selectedKind: null,
    waveform: null,
    position: {
      playing: false,
      seconds: 0,
      total: 0,
      looping: false,
      reverse: false,
      gainDb: 0,
      raw: false,
    },
    error: null,
  });
});

describe('the browser rail width', () => {
  it('refuses to go absurdly wide, or narrow enough to lose the handle', () => {
    // ⛔ Mike asked for the ceiling by name, 2026-08-07: *"don't let it get
    // absurdly wide."* The floor is the other half — dragged past it there is
    // no handle left to grab, so the gesture that shrank the rail could not
    // undo itself.
    expect(clampRailWidth(10_000)).toBe(RAIL_MAX_WIDTH);
    expect(clampRailWidth(0)).toBe(RAIL_MIN_WIDTH);
    expect(clampRailWidth(-40)).toBe(RAIL_MIN_WIDTH);
    expect(clampRailWidth(320)).toBe(320);
  });

  it('falls back to the default width for a value that is not a number', () => {
    // ⚠ **The default, not the ceiling**, and the distinction is deliberate:
    // localStorage is a string store shared with everything else in the WebView
    // profile, so `NaN` and `Infinity` mean *corrupt*, not "as wide as
    // possible". Clamping them to the maximum would open the rail to 560px on
    // launch because something unrelated wrote over the key.
    expect(clampRailWidth(Number.NaN)).toBe(RAIL_DEFAULT_WIDTH);
    expect(clampRailWidth(Number.POSITIVE_INFINITY)).toBe(RAIL_DEFAULT_WIDTH);
    expect(RAIL_DEFAULT_WIDTH).toBeGreaterThanOrEqual(RAIL_MIN_WIDTH);
    expect(RAIL_DEFAULT_WIDTH).toBeLessThanOrEqual(RAIL_MAX_WIDTH);
  });

  it('persists what was dragged', () => {
    useExplorer.getState().setRailWidth(400);
    expect(useExplorer.getState().railWidth).toBe(400);
    expect(window.localStorage.getItem('freally.browserWidth')).toBe('400');
  });
});

describe('the time readout', () => {
  it('reads as minutes and seconds out of the total', () => {
    expect(formatSeconds(0)).toBe('0:00.0');
    expect(formatSeconds(1.25)).toBe('0:01.3');
    expect(formatSeconds(83.4)).toBe('1:23.4');
  });

  it('never renders a negative or a NaN at the producer', () => {
    // The position is polled from an atomic the audio thread writes; a dropped
    // or half-initialised read must show `0:00.0` rather than `NaN:NaN`.
    expect(formatSeconds(-1)).toBe('0:00.0');
    expect(formatSeconds(Number.NaN)).toBe('0:00.0');
  });
});

describe('the waveform outline', () => {
  it('draws both bounds, so a sample does not read as DC-offset', () => {
    // ⛔ The same claim `explorer::waveform`'s own test makes on the Rust side.
    // Drawing only the maxima produces a half-waveform that looks like a broken
    // recording — and a renderer that ignored the minima would make that test
    // prove nothing.
    const path = outlineOf([
      [-1, 1],
      [-0.5, 0.5],
    ]);
    // Mid-line is 50, amplitude 48: +1 is 2, -1 is 98.
    expect(path).toContain('2.00');
    expect(path).toContain('98.00');
    expect(path.startsWith('M')).toBe(true);
    expect(path.endsWith('Z')).toBe(true);
  });

  it('is empty for a sample with no peaks rather than a broken path', () => {
    expect(outlineOf([])).toBe('');
  });

  it('clamps a peak that arrives outside -1..1', () => {
    // The peaks come from a decoder, and a clipped or float-format file can
    // exceed full scale. Unclamped that draws outside the box and the SVG
    // silently crops the loudest part of the sample flat.
    const path = outlineOf([
      [-4, 4],
      [-4, 4],
    ]);
    expect(path).not.toContain('-');
    expect(path).toContain('2.00');
    expect(path).toContain('98.00');
  });
});

describe('selecting a sample', () => {
  it('asks for the peaks and loads the audition voice from one click', () => {
    invoke.mockResolvedValue({
      path: '/lib/kick.wav',
      name: 'kick.wav',
      peaks: [],
      seconds: 1,
    });
    void useExplorer.getState().select('/lib/kick.wav');

    expect(invoke).toHaveBeenCalledWith('explorer_waveform', { path: '/lib/kick.wav' });
    expect(invoke).toHaveBeenCalledWith('preview_load', { path: '/lib/kick.wav' });
  });

  it('drops a reply that arrives after a newer selection', async () => {
    // ⛔ Clicking down a folder faster than the decodes come back would
    // otherwise leave whichever finished *last* on screen, which is not
    // necessarily the one selected. The waveform carries its own path so this
    // is answerable at all.
    invoke.mockResolvedValue({ path: '/lib/old.wav', name: 'old.wav', peaks: [], seconds: 1 });
    const stale = useExplorer.getState().select('/lib/old.wav');
    useExplorer.setState({ selected: '/lib/new.wav' });
    await stale;

    expect(useExplorer.getState().waveform).toBeNull();
    expect(useExplorer.getState().selected).toBe('/lib/new.wav');
  });

  it('shows the sample it is about to draw before the decode returns', () => {
    invoke.mockReturnValue(new Promise(() => {}));
    void useExplorer.getState().select('/lib/kick.wav');

    // ⚠ Selected immediately, waveform cleared: a slow decode must not leave
    // the previous sample's shape on screen looking like the one just clicked.
    expect(useExplorer.getState().selected).toBe('/lib/kick.wav');
    expect(useExplorer.getState().waveform).toBeNull();
  });

  /**
   * ⛔⛔ **LANDING ON A FILE PLAYS IT** — Mike, 2026-08-11: *"the files need to
   * play as you go up and down in the list with the up/down arrow or by clicking
   * on them."*
   *
   * ⚠ Pinned in the **store**, because that is the point of putting it there: a
   * click, an arrow key and a starred file are three different callers, and the
   * bug this prevents is one of them staying silent.
   */
  it('plays what it just landed on', async () => {
    invoke.mockResolvedValue({
      path: '/lib/kick.wav',
      name: 'kick.wav',
      peaks: [],
      seconds: 1,
    });
    await useExplorer.getState().select('/lib/kick.wav');

    expect(invoke).toHaveBeenCalledWith('preview_play');
    expect(useExplorer.getState().position.playing).toBe(true);
  });

  it('does not sound a file the walk has already moved off', async () => {
    // ⛔ Holding ↓ starts a load per row and they resolve out of order. Without
    // the guard the *previous* row's reply starts the *current* row's sample a
    // second time — which reads as the browser stuttering on every step.
    invoke.mockResolvedValue({ path: '/lib/old.wav', name: 'old.wav', peaks: [], seconds: 1 });
    const stale = useExplorer.getState().select('/lib/old.wav');
    useExplorer.setState({ selected: '/lib/new.wav' });
    await stale;

    expect(invoke).not.toHaveBeenCalledWith('preview_play');
  });

  it('says nothing when there is no audio thread to play it with', async () => {
    // ⚠ **Silent, unlike the transport buttons.** An audition is feedback on a
    // gesture that already happened, and every browser session has no preview
    // voice at all — so a failure here would put an error on screen for a click
    // that did nothing wrong. Same rule `DrumGrid/audition.ts` follows.
    invoke.mockImplementation((command: string) =>
      command === 'preview_play'
        ? Promise.reject(new Error('no audio thread'))
        : Promise.resolve({ path: '/lib/kick.wav', name: 'kick.wav', peaks: [], seconds: 1 }),
    );
    await useExplorer.getState().select('/lib/kick.wav');

    expect(useExplorer.getState().error).toBeNull();
    expect(useExplorer.getState().waveform).not.toBeNull();
  });

  it('leaves a MIDI file alone, because there is nothing to sound', async () => {
    // ⚠ The two-kinds rule at the one place that would otherwise treat them
    // alike: a `.mid` has no PCM until something renders one, so selecting it
    // must not reach for the audition voice. ⛔ **Landing on it does not audition
    // it either** (TASK-160): rendering a file is the slow half, and walking a
    // folder with ↓ would render every `.mid` stepped past.
    invoke.mockResolvedValue([]);
    await useExplorer.getState().select('/lib/loop.mid');

    expect(invoke).not.toHaveBeenCalledWith('preview_play');
    expect(invoke).not.toHaveBeenCalledWith('explorer_midi_audition', expect.anything());
  });
});

/**
 * Hearing a `.mid` (TASK-160).
 *
 * ⛔⛔ Mike: *".mid files … have its own sound like Ableton does that can play
 * the .mid file"*. The roadmap put the cost at *"its own note scheduler"* on the
 * audio thread; `midi_audition::render` produces the same `Vec<f32>` a decoded
 * `.wav` arrives as, so the audition voice plays it with the code that already
 * exists. What this pins is the page's half of that: which command, and that the
 * transport can never end up describing the wrong file.
 */
describe('auditioning a MIDI file', () => {
  it('renders it into the audition voice, and does not start it', async () => {
    invoke.mockResolvedValue({ seconds: 4, clipped: false });
    useExplorer.setState({ selected: '/lib/loop.mid', selectedKind: 'midi' });

    await useExplorer.getState().auditionMidi();

    expect(invoke).toHaveBeenCalledWith('explorer_midi_audition', { path: '/lib/loop.mid' });
    // ⛔ Loading because a producer pressed Play is one thing; making a noise
    // from the load itself is another. `Preview::load` states the same rule for
    // a sample, and the press is what plays.
    expect(invoke).not.toHaveBeenCalledWith('preview_play');
    expect(useExplorer.getState().midiAudition).toEqual({ clipped: false });
  });

  it('refuses to render anything that is not the selected MIDI file', async () => {
    // ⚠ The transport is drawn from `midiAudition`, so a render asked for while
    // an audio file is selected would put a MIDI transport over a waveform.
    invoke.mockResolvedValue({ seconds: 4, clipped: false });
    useExplorer.setState({ selected: '/lib/kick.wav', selectedKind: 'audio' });

    await useExplorer.getState().auditionMidi();

    expect(invoke).not.toHaveBeenCalledWith('explorer_midi_audition', expect.anything());
  });

  it('drops a render that finished after the producer moved on', async () => {
    // ⛔ **The audition voice is shared with sample playback.** A late reply that
    // set `midiAudition` for a file no longer selected would draw a transport
    // over the *next* file and sound the previous one on the next Play — the same
    // failure the waveform's own path guards, arriving through a slower command.
    invoke.mockResolvedValue({ seconds: 4, clipped: false });
    useExplorer.setState({ selected: '/lib/old.mid', selectedKind: 'midi' });
    const stale = useExplorer.getState().auditionMidi();
    useExplorer.setState({ selected: '/lib/new.mid' });
    await stale;

    expect(useExplorer.getState().midiAudition).toBeNull();
  });

  it('forgets the render when the selection changes', async () => {
    // ⚠ Otherwise the transport goes on describing the previous `.mid` under the
    // new file's name, and Play sounds the old one.
    invoke.mockResolvedValue({ seconds: 4, clipped: false });
    useExplorer.setState({ selected: '/lib/loop.mid', selectedKind: 'midi' });
    await useExplorer.getState().auditionMidi();
    expect(useExplorer.getState().midiAudition).not.toBeNull();

    invoke.mockResolvedValue([]);
    await useExplorer.getState().select('/lib/other.mid');
    expect(useExplorer.getState().midiAudition).toBeNull();
  });

  it('says when a long file was cut rather than cutting it quietly', async () => {
    // ⚠ The rule the truncated folder listing follows: a producer who does not
    // hear the end and is told nothing concludes the audition is broken.
    invoke.mockResolvedValue({ seconds: 120, clipped: true });
    useExplorer.setState({ selected: '/lib/song.mid', selectedKind: 'midi' });

    await useExplorer.getState().auditionMidi();

    expect(useExplorer.getState().midiAudition?.clipped).toBe(true);
  });
});

describe('the transport', () => {
  it('rewinds on stop and holds on pause', async () => {
    // ⛔ Mike named both in one sentence, and they are two behaviours. A stop
    // that only paused would make the second press of Play resume from the
    // middle of a one-shot.
    invoke.mockResolvedValue(undefined);
    useExplorer.setState({
      position: {
        playing: true,
        seconds: 0.8,
        total: 1,
        looping: false,
        reverse: false,
        gainDb: 0,
        raw: false,
      },
    });

    await useExplorer.getState().pause();
    expect(useExplorer.getState().position.seconds).toBe(0.8);

    await useExplorer.getState().stop();
    expect(useExplorer.getState().position.seconds).toBe(0);
    expect(useExplorer.getState().position.playing).toBe(false);
  });

  it('writes a seek through before the poll answers', async () => {
    invoke.mockResolvedValue(undefined);
    await useExplorer.getState().seek(0.4);

    // Otherwise the playhead sits where it was for up to a frame after the
    // click, which reads as the click having missed.
    expect(useExplorer.getState().position.seconds).toBe(0.4);
    expect(invoke).toHaveBeenCalledWith('preview_seek', { seconds: 0.4 });
  });

  it('reports a refused command rather than swallowing it', async () => {
    invoke.mockRejectedValue(new Error('no output device'));
    await useExplorer.getState().play();
    expect(useExplorer.getState().error).toContain('no output device');
  });

  it('sends every transport gesture the plugin needs to hear', async () => {
    // ⛔ **The plugin holds the authority for all of these.** The store's
    // optimistic write is only so the button moves on the frame it was pressed;
    // a gesture that updated the store and never reached the audio thread would
    // look right and sound like nothing, which is the failure this whole panel
    // was written after.
    invoke.mockResolvedValue(undefined);
    const store = useExplorer.getState();

    await store.play();
    await store.pause();
    await store.stop();
    await store.toggleLoop();
    await store.setReverse(true);

    const sent = invoke.mock.calls.map(([command]) => command);
    expect(sent).toEqual([
      'preview_play',
      'preview_pause',
      'preview_stop',
      'preview_loop',
      'preview_reverse',
    ]);
    // The two that carry an argument carry the right one.
    expect(invoke).toHaveBeenCalledWith('preview_loop', { on: true });
    expect(invoke).toHaveBeenCalledWith('preview_reverse', { on: true });
  });

  it('toggles loop back off rather than only on', async () => {
    invoke.mockResolvedValue(undefined);
    await useExplorer.getState().toggleLoop();
    expect(useExplorer.getState().position.looping).toBe(true);

    await useExplorer.getState().toggleLoop();
    expect(useExplorer.getState().position.looping).toBe(false);
    expect(invoke).toHaveBeenLastCalledWith('preview_loop', { on: false });
  });
});

describe('dropping a sample on a lane', () => {
  it('routes through the tested no-dialog loader', async () => {
    // ⛔ `explorer_drop` is a name for `OneShots::restore`, which is the path a
    // reopened project already uses. A second loader would be a second set of
    // the same rules — remote refusal, decode, missing file, kit rebuild — to
    // keep in agreement.
    invoke.mockResolvedValue(undefined);
    await useExplorer.getState().dropOn('kick', '/lib/kick.wav');
    expect(invoke).toHaveBeenCalledWith('explorer_drop', {
      lane: 'kick',
      path: '/lib/kick.wav',
      // ⚠ **Forwards unless asked**, so every route that predates `Ctrl`+arrow —
      // the drag onto a pad, the KIT row drop, the `↵` button — keeps behaving
      // exactly as it did.
      reversed: false,
    });
  });

  it('asks for a backwards one-shot when Ctrl+← was the gesture', async () => {
    // ⛔⛔ Mike, 2026-08-11: *"'Ctrl + left arrow' … should add the sample to
    // that selected drum pad lane **in reverse**."* The flag has to reach the
    // plugin, which flips the buffer at decode time and writes the choice into
    // the project — `oneshot::load` and `PluginSession::one_shots_reversed`
    // carry the two halves of why.
    invoke.mockResolvedValue(undefined);
    await useExplorer.getState().dropOn('kick', '/lib/kick.wav', true);
    expect(invoke).toHaveBeenCalledWith('explorer_drop', {
      lane: 'kick',
      path: '/lib/kick.wav',
      reversed: true,
    });
  });

  it('says why a drop was refused', async () => {
    invoke.mockRejectedValue(new Error('that sample is not in your sample library'));
    await useExplorer.getState().dropOn('kick', '/etc/passwd');
    expect(useExplorer.getState().error).toContain('sample library');
  });
});

describe('removing a library folder', () => {
  it('lets go of a sample that was inside it', async () => {
    // ⛔⛔ **The selected path is deliberately NOT the one being removed.** The
    // plugin stores the browse location canonically — on Windows `\\?\C:\…` —
    // while a root is kept as it was added, so the `startsWith` this replaced
    // never matched there and the preview player kept drawing a sample the
    // browser could no longer reach. What decides is the plugin clearing its own
    // browse location, which it reports as a null folder.
    invoke.mockResolvedValue({
      roots: [],
      folder: null,
      parent: null,
      entries: [],
      truncated: false,
      picking: false,
    });
    useExplorer.setState({
      selected: '\\\\?\\C:\\lib\\Samples\\kick.wav',
      waveform: {
        path: '\\\\?\\C:\\lib\\Samples\\kick.wav',
        name: 'kick.wav',
        peaks: [],
        seconds: 1,
      },
    });

    await useExplorer.getState().removeFolder('C:\\lib\\Samples');
    expect(useExplorer.getState().selected).toBeNull();
    expect(useExplorer.getState().waveform).toBeNull();
  });

  it('leaves a sample alone when some other root is removed', async () => {
    // The plugin only clears the browse location when the root it was *inside*
    // goes, so a non-null folder after the refresh means the selection is still
    // reachable — clearing it would be a second bug in the other direction.
    invoke.mockResolvedValue({
      roots: [{ name: 'Samples', path: 'C:\\lib\\Samples', isDir: true }],
      folder: 'C:\\lib\\Samples',
      parent: null,
      entries: [],
      truncated: false,
      picking: false,
    });
    useExplorer.setState({
      selected: 'C:\\lib\\Samples\\kick.wav',
      waveform: { path: 'C:\\lib\\Samples\\kick.wav', name: 'kick.wav', peaks: [], seconds: 1 },
    });

    await useExplorer.getState().removeFolder('C:\\other\\Loops');
    expect(useExplorer.getState().selected).toBe('C:\\lib\\Samples\\kick.wav');
    expect(useExplorer.getState().waveform).not.toBeNull();
  });
});

describe('the tree', () => {
  /**
   * ⛔⛔ **The bug this whole tree was rebuilt around.**
   *
   * Mike, 2026-08-10: *"you can get to the subfolders list, but you cannot go
   * into those subfolders."* The cause was in Rust — `canonicalize` answers a
   * verbatim path on Windows and `refuse_remote` read its leading slashes as a
   * network path — but the page had its own half of it: every path comparison
   * here was `===` or `startsWith` between a root spelled as the producer added
   * it and a folder the plugin had canonicalised.
   *
   * ⚠ `String.raw` throughout, so these read as the paths they are rather than
   * as doubled escapes. A Windows path written with `\\` in a test is a path
   * nobody can check against the thing it is meant to mirror.
   */
  it('treats the canonical and raw spellings of a path as one folder', () => {
    expect(samePath(String.raw`\\?\C:\lib\Samples`, String.raw`C:\lib\Samples`)).toBe(true);
    // ⚠ Concatenated rather than raw: a raw literal cannot *end* in a
    // backslash, because it escapes the closing backtick.
    expect(samePath(`${String.raw`C:\lib\Samples`}\\`, String.raw`C:\lib\Samples`)).toBe(true);
    expect(samePath(String.raw`c:\lib\samples`, String.raw`C:\lib\Samples`)).toBe(true);
    expect(samePath(String.raw`C:\lib\Other`, String.raw`C:\lib\Samples`)).toBe(false);

    expect(isInside(String.raw`\\?\C:\lib\Samples\Kicks`, String.raw`C:\lib\Samples`)).toBe(
      true,
    );
    // ⛔ A separator is required, or `Samples-old` counts as a child of `Samples`.
    expect(isInside(String.raw`C:\lib\Samples-old\a.wav`, String.raw`C:\lib\Samples`)).toBe(
      false,
    );
    expect(isInside(String.raw`C:\lib\Samples`, String.raw`C:\lib\Samples`)).toBe(false);
  });

  it('reads a folder when it is expanded, without navigating away from it', async () => {
    invoke.mockImplementation((command: string) => {
      if (command === 'explorer_list') {
        return Promise.resolve({
          roots: [],
          folder: String.raw`C:\lib\Samples`,
          parent: null,
          entries: [
            { name: 'kick.wav', path: String.raw`C:\lib\Samples\kick.wav`, isDir: false },
          ],
          truncated: false,
          picking: false,
        });
      }
      return Promise.resolve({
        roots: [],
        folder: null,
        parent: null,
        entries: [],
        truncated: false,
        picking: false,
      });
    });

    await useExplorer.getState().toggleFolder(String.raw`C:\lib\Samples`);

    expect(useExplorer.getState().expanded).toContain(String.raw`C:\lib\Samples`);
    expect(useExplorer.getState().children[String.raw`C:\lib\Samples`]).toHaveLength(1);
    // ⛔ **Both commands.** `explorer_list` draws the rows; `explorer_open` is
    // what makes the folder the one a per-pad randomise draws from. Sending only
    // the first would leave the dice pointed at wherever the producer was last.
    expect(invoke).toHaveBeenCalledWith('explorer_list', { path: String.raw`C:\lib\Samples` });
    expect(invoke).toHaveBeenCalledWith('explorer_open', { path: String.raw`C:\lib\Samples` });
  });

  it('shuts a folder and every branch under it', () => {
    useExplorer.setState({
      expanded: [
        String.raw`C:\lib\Samples`,
        String.raw`C:\lib\Samples\Kicks`,
        String.raw`\\?\C:\lib\Samples\Kicks\Vinyl`,
        String.raw`C:\lib\Loops`,
      ],
    });

    useExplorer.getState().collapse(String.raw`C:\lib\Samples`);

    // ⛔ Descendants go too, or re-opening the folder silently regrows every
    // branch the producer just tidied away — including the one two levels down
    // that the plugin had spelled canonically.
    expect(useExplorer.getState().expanded).toEqual([String.raw`C:\lib\Loops`]);
  });

  it('shuts again when the folder could not be read', async () => {
    invoke.mockImplementation((command: string) =>
      command === 'explorer_list'
        ? Promise.reject(new Error('that is not a folder in your sample library'))
        : Promise.resolve({
            roots: [],
            folder: null,
            parent: null,
            entries: [],
            truncated: false,
            picking: false,
          }),
    );

    await useExplorer.getState().toggleFolder(String.raw`C:\lib\Nope`);

    // ⛔ An expanded twisty over no rows reads as "this folder is empty", which
    // is a claim the page has not earned — the read is what failed.
    expect(useExplorer.getState().expanded).not.toContain(String.raw`C:\lib\Nope`);
    expect(useExplorer.getState().error).toContain('sample library');
  });

  it('retracts the deepest branch, not the one clicked most recently', () => {
    // ⚠ Opening a sibling after a child leaves the child open and more deeply
    // nested, so "most recent" would shut the wrong one — and `Up` would appear
    // to do nothing while a branch stayed open below it.
    expect(
      innermostExpanded([
        String.raw`C:\lib\Samples`,
        String.raw`C:\lib\Samples\Kicks\Vinyl`,
        String.raw`C:\lib\Samples\Kicks`,
      ]),
    ).toBe(String.raw`C:\lib\Samples\Kicks\Vinyl`);
    expect(innermostExpanded([])).toBeNull();
  });

  it('forgets a removed root’s branch', async () => {
    invoke.mockResolvedValue({
      roots: [],
      folder: null,
      parent: null,
      entries: [],
      truncated: false,
      picking: false,
    });
    useExplorer.setState({
      expanded: [String.raw`C:\lib\Samples`, String.raw`C:\lib\Samples\Kicks`],
      children: {
        [String.raw`C:\lib\Samples`]: [
          {
            name: 'Kicks',
            path: String.raw`C:\lib\Samples\Kicks`,
            isDir: true,
            kind: 'dir' as const,
          },
        ],
        [String.raw`C:\lib\Samples\Kicks`]: [],
      },
    });

    await useExplorer.getState().removeFolder(String.raw`C:\lib\Samples`);

    // Left behind, the rows would reappear the moment a folder of the same name
    // was added back — a listing nothing had re-read.
    expect(useExplorer.getState().expanded).toEqual([]);
    expect(useExplorer.getState().children).toEqual({});
  });
});

/**
 * The flattening the virtualized tree draws from (TASK-058).
 *
 * ⛔⛔ **Tested here rather than through the component**, because two things read
 * it: `FileTree` draws a window of these rows and `ExplorerPanel`'s ↑/↓ walk
 * steps through them. The walk used to read `.tree__row` out of the DOM, which is
 * correct only while every row is mounted — so the order and the membership of
 * this list are now load-bearing in a way no render test would pin.
 */
describe('flattening the tree', () => {
  const dir = (name: string, path: string) => ({
    name,
    path,
    isDir: true,
    kind: 'dir' as const,
  });
  const file = (name: string, path: string) => ({
    name,
    path,
    isDir: false,
    kind: 'audio' as const,
  });

  const root = dir('Samples', '/lib/Samples');
  const library = {
    '/lib/Samples': [
      dir('Kicks', '/lib/Samples/Kicks'),
      file('clap.wav', '/lib/Samples/clap.wav'),
    ],
    '/lib/Samples/Kicks': [file('kick-808.wav', '/lib/Samples/Kicks/kick-808.wav')],
  };

  it('draws only the branches that are open, in the tree’s own order', () => {
    const shut = flattenTree(root, {
      expanded: [],
      children: library,
      truncatedIn: [],
      query: '',
    });
    expect(shut.map((row) => row.key)).toEqual(['/lib/Samples']);

    const open = flattenTree(root, {
      expanded: ['/lib/Samples', '/lib/Samples/Kicks'],
      children: library,
      truncatedIn: [],
      query: '',
    });
    // ⛔ Depth-first and folders-before-files, which is `explorer::list`'s
    // ordering rather than this function's — one answer to "what order are the
    // rows in", in the place that reads the directory.
    expect(open.map((row) => row.key)).toEqual([
      '/lib/Samples',
      '/lib/Samples/Kicks',
      '/lib/Samples/Kicks/kick-808.wav',
      '/lib/Samples/clap.wav',
    ]);
    // The indent the row draws with, and the level the treeitem announces.
    expect(open.map((row) => row.depth)).toEqual([0, 1, 2, 1]);
  });

  it('keeps the folders that lead to a match, and everything under one that matched', () => {
    const expanded = ['/lib/Samples', '/lib/Samples/Kicks'];

    // A file matches: its folders survive so there is a path to it on screen.
    expect(
      flattenTree(root, { expanded, children: library, truncatedIn: [], query: '808' }).map(
        (row) => row.key,
      ),
    ).toEqual(['/lib/Samples', '/lib/Samples/Kicks', '/lib/Samples/Kicks/kick-808.wav']);

    // ⛔ **A matching FOLDER brings its whole subtree**, the way every file
    // manager's filter behaves: typing "Kicks" is how you get to the kicks, not
    // how you make them disappear.
    expect(
      flattenTree(root, { expanded, children: library, truncatedIn: [], query: 'kicks' }).map(
        (row) => row.key,
      ),
    ).toEqual(['/lib/Samples', '/lib/Samples/Kicks', '/lib/Samples/Kicks/kick-808.wav']);
  });

  it('⛔ a tag filter narrows to files, and a folder name cannot widen it back', () => {
    // TASK-058C's third bullet: *"filter the tree by tag and by favourite,
    // composable with the existing type-to-filter."* Composable means both
    // constraints apply — and a tag filter is a statement about FILES, so a
    // folder called `Kicks` must not re-admit the untagged files inside it the
    // way a typed query legitimately does.
    const expanded = ['/lib/Samples', '/lib/Samples/Kicks'];
    const only = new Set(['/lib/Samples/clap.wav']);

    expect(
      flattenTree(root, {
        expanded,
        children: library,
        truncatedIn: [],
        query: '',
        only,
      }).map((row) => row.key),
    ).toEqual(['/lib/Samples', '/lib/Samples/clap.wav']);

    // ⛔ The half that would have shipped broken. `query: 'kicks'` forces the
    // whole `Kicks` subtree when the tag filter is off — asserted just above —
    // and it must NOT while one is on, because `kick-808.wav` does not carry
    // the tag. The folder survives only if something under it did, and here
    // nothing does.
    expect(
      flattenTree(root, {
        expanded,
        children: library,
        truncatedIn: [],
        query: 'kicks',
        only,
      }).map((row) => row.key),
      // Nothing survives, so `flattenTree` falls back to the root alone — which
      // is what the no-matches line beside the box is for.
    ).toEqual(['/lib/Samples']);

    // And the two really do compose rather than one winning: a query that names
    // the tagged file keeps it.
    expect(
      flattenTree(root, {
        expanded,
        children: library,
        truncatedIn: [],
        query: 'clap',
        only,
      }).map((row) => row.key),
    ).toEqual(['/lib/Samples', '/lib/Samples/clap.wav']);
  });

  it('leaves the root on screen when nothing matches, rather than going blank', () => {
    // ⚠ An empty panel reads as the library having gone. The root is the tab the
    // producer is standing on, and the no-matches line beside the box is what
    // says the query was too narrow.
    const rows = flattenTree(root, {
      expanded: ['/lib/Samples', '/lib/Samples/Kicks'],
      children: library,
      truncatedIn: [],
      query: 'nothing-is-called-this',
    });
    expect(rows.map((row) => row.key)).toEqual(['/lib/Samples']);
  });

  it('does not put “no samples in this folder” under a folder the filter emptied', () => {
    // ⛔ The status line means the folder *is* empty. Drawn for a folder whose
    // rows the query hid, it would be the readout-that-lies failure — and it
    // would also keep every filtered-out folder on screen, which is the opposite
    // of filtering.
    const rows = flattenTree(root, {
      expanded: ['/lib/Samples', '/lib/Samples/Kicks'],
      children: library,
      truncatedIn: [],
      query: 'clap',
    });
    expect(rows.some((row) => row.note !== null)).toBe(false);
    expect(rows.map((row) => row.key)).toEqual(['/lib/Samples', '/lib/Samples/clap.wav']);
  });

  it('carries the states a branch can be in that are not rows', () => {
    const reading = flattenTree(root, {
      expanded: ['/lib/Samples'],
      children: {},
      truncatedIn: [],
      query: '',
    });
    expect(reading[1]?.note).toBe('explorer.decoding');

    const capped = flattenTree(root, {
      expanded: ['/lib/Samples'],
      children: { '/lib/Samples': [] },
      truncatedIn: ['/lib/Samples'],
      query: '',
    });
    expect(capped.map((row) => row.note)).toEqual([
      null,
      'explorer.empty',
      'explorer.truncated',
    ]);
  });
});

describe('a library folder that is not there', () => {
  it('is reported rather than drawn as a folder that refuses to open', async () => {
    // ⛔⛔ `explorer::merge_folders` keeps a root whose disk is unplugged on
    // purpose — a producer who unplugged their sample drive has not left the
    // library — but until the plugin said which ones those were, such a root
    // drew as an ordinary folder, failed to expand with the one refusal message
    // every failure shares, and shut its own twisty. Indistinguishable from an
    // empty folder.
    invoke.mockResolvedValue({
      roots: [{ name: 'Samples', path: '/lib/Samples', isDir: true, kind: 'dir' }],
      folder: null,
      parent: null,
      entries: [],
      truncated: false,
      picking: false,
      missing: ['/lib/Samples'],
    });

    await useExplorer.getState().refresh();

    expect(useExplorer.getState().roots).toHaveLength(1);
    expect(useExplorer.getState().missingRoots).toEqual(['/lib/Samples']);
  });

  it('reads an absent list as nothing missing', async () => {
    // ⚠ The reply also comes from `ipc-mock` and from a plugin older than the
    // field. "Nothing is known to be missing" is the safe reading of silence —
    // the alternative would strike out every tab in the browser build.
    invoke.mockResolvedValue({
      roots: [],
      folder: null,
      parent: null,
      entries: [],
      truncated: false,
      picking: false,
    });

    await useExplorer.getState().refresh();

    expect(useExplorer.getState().missingRoots).toEqual([]);
  });
});

/**
 * The audition level and the `Raw` bypass (TASK-058B).
 *
 * ⛔ What these are for is that `Raw` is a **bypass, not a level**. The two
 * controls are separate on purpose, and the failure they guard against is a
 * producer A/Bing a sample against the file and coming back to 0 dB instead of
 * to the level they had dialled in.
 */
it('keeps the level a producer set while Raw is bypassing it', async () => {
  invoke.mockResolvedValue(undefined);

  await useExplorer.getState().setPreviewGain(-6);
  await useExplorer.getState().setRaw(true);
  expect(useExplorer.getState().position.gainDb).toBe(-6);
  expect(useExplorer.getState().position.raw).toBe(true);

  await useExplorer.getState().setRaw(false);
  expect(useExplorer.getState().position.gainDb).toBe(-6);
  expect(invoke).toHaveBeenCalledWith('preview_raw', { on: false });
});

it('says so when the plugin refuses a level change', async () => {
  // ⚠ The same rule the rest of this store follows: a control that moved and
  // changed no sound has to leave a reason on screen.
  invoke.mockRejectedValueOnce(new Error('nothing is loaded'));
  await useExplorer.getState().setPreviewGain(3);
  expect(useExplorer.getState().error).toBe('nothing is loaded');
});

/**
 * Reading a sample's notes (TASK-058D / TASK-058F).
 *
 * ⛔⛔ **The command does not answer the question it is asked.** Reading a
 * forty-second stem takes about two seconds, and the bridge runs on the DAW's
 * editor thread — so `explorer_audio_split` *starts* a detached read and the page
 * polls `explorer_audio_status` for it. Everything below is about that loop,
 * which is the part no Rust test can see and the part a producer's cursor is
 * waiting on.
 */
describe('reading the notes out of a sample', () => {
  const GRID = { bpm: 140, timeSigNum: 4, timeSigDen: 4 };

  /**
   * A finished read of `path`.
   *
   * ⚠ **The part is derived from the path**, so a reply about the wrong file is
   * distinguishable from the right one. The first cut of this helper returned the
   * same `drums` either way, which made the "somebody else's answer" test below
   * pass whether the guard existed or not.
   */
  const done = (path: string) => ({
    state: 'done',
    path,
    split: {
      parts: [{ part: path.includes('other') ? 'chords' : 'drums', notes: 3 }],
      bpm: 140,
      vocalLeftAlone: false,
    },
  });

  beforeEach(() => {
    useExplorer.setState({ audioSplit: null, extracting: null, error: null });
  });

  it('polls until the read lands, rather than expecting one answer', async () => {
    // ⚠ Two `running` replies before the result, because that is what the
    // plugin really does — a mock that answered at once would leave the loop
    // itself untested.
    invoke.mockImplementation((command: string) => {
      if (command === 'explorer_audio_split') return Promise.resolve(null);
      if (command !== 'explorer_audio_status') return Promise.resolve(null);
      const calls = invoke.mock.calls.filter(
        ([name]: unknown[]) => name === 'explorer_audio_status',
      ).length;
      return Promise.resolve(
        calls < 3 ? { state: 'running', path: '/lib/loop.wav' } : done('/lib/loop.wav'),
      );
    });

    const split = await useExplorer.getState().extractNotes('/lib/loop.wav', GRID);
    expect(split?.parts).toHaveLength(1);
    expect(useExplorer.getState().extracting).toBeNull();
    expect(useExplorer.getState().audioSplit?.found.bpm).toBe(140);
  });

  it('a result for another file is not taken as the answer to this one', async () => {
    // ⛔ **The mailbox is one slot per plugin instance.** A `done` naming a file
    // this call did not ask about is somebody else's answer, and taking it would
    // put one sample's notes on screen under another's name.
    invoke.mockImplementation((command: string) => {
      if (command === 'explorer_audio_split') return Promise.resolve(null);
      if (command !== 'explorer_audio_status') return Promise.resolve(null);
      const calls = invoke.mock.calls.filter(
        ([name]: unknown[]) => name === 'explorer_audio_status',
      ).length;
      return Promise.resolve(calls < 3 ? done('/lib/other.wav') : done('/lib/loop.wav'));
    });

    const split = await useExplorer.getState().extractNotes('/lib/loop.wav', GRID);
    expect(split).not.toBeNull();
    expect(useExplorer.getState().audioSplit?.found.parts[0]?.part).toBe('drums');
  });

  it('a failure names its reason rather than leaving a spinner', async () => {
    invoke.mockImplementation((command: string) =>
      Promise.resolve(
        command === 'explorer_audio_status'
          ? { state: 'failed', path: '/lib/loop.wav', reason: 'that file is silent' }
          : null,
      ),
    );
    expect(await useExplorer.getState().extractNotes('/lib/loop.wav', GRID)).toBeNull();
    expect(useExplorer.getState().error).toBe('that file is silent');
    expect(useExplorer.getState().extracting).toBeNull();
  });

  it('a refused path stops before it starts polling', async () => {
    // ⚠ `checked_audio` refuses on the caller's thread, so the *start* rejects
    // and there is nothing in flight to poll for.
    invoke.mockRejectedValueOnce(new Error('that sample is not in your sample library'));
    expect(await useExplorer.getState().extractNotes('//evil/share/a.wav', GRID)).toBeNull();
    expect(useExplorer.getState().error).toBe('that sample is not in your sample library');
    expect(
      invoke.mock.calls.some(([name]: unknown[]) => name === 'explorer_audio_status'),
    ).toBe(false);
  });

  it('cancelling stops the wait even if the plugin never answers the cancel', async () => {
    // ⛔ `extracting` is cleared *before* the round trip, so the poll returns on
    // its next tick whatever the plugin does with the message.
    invoke.mockImplementation((command: string) =>
      command === 'explorer_audio_cancel'
        ? new Promise(() => {})
        : Promise.resolve(
            command === 'explorer_audio_status'
              ? { state: 'running', path: '/lib/loop.wav' }
              : null,
          ),
    );
    const reading = useExplorer.getState().extractNotes('/lib/loop.wav', GRID);
    await vi.waitFor(() => expect(useExplorer.getState().extracting).toBe('/lib/loop.wav'));
    useExplorer.getState().cancelExtract();
    expect(await reading).toBeNull();
    expect(useExplorer.getState().extracting).toBeNull();
  });

  it('walking to another file clears what the last one was found to contain', async () => {
    // ⛔ The readout-that-lies failure in miniature: the panel going on showing
    // the previous sample's parts under this file's name.
    invoke.mockImplementation(() => Promise.resolve(null));
    useExplorer.setState({
      audioSplit: {
        path: '/lib/first.wav',
        found: { parts: [], bpm: 90, vocalLeftAlone: true },
      },
      selected: '/lib/first.wav',
      selectedKind: 'audio',
    });
    await useExplorer.getState().select('/lib/second.wav');
    expect(useExplorer.getState().audioSplit).toBeNull();
  });
});
