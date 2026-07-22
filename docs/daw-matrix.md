# DAW compatibility matrix

Where drag-out and SMF import actually work, recorded from real runs rather than
assumed. Filled in by following **Live-To-Do.md § 0.2**.

**Status: not yet run.** TASK-013 is the decision gate this table exists to settle.

## Drag-out (TASK-013, PRD § 15 Q1)

| OS | Session | DAW | Version | Drag lands? | Notes |
| --- | --- | --- | --- | --- | --- |
| Windows | — | FL Studio | | ☐ | |
| Windows | — | Ableton Live | | ☐ | |
| macOS | — | Logic Pro | | ☐ | |
| macOS | — | Ableton Live | | ☐ | |
| Linux | X11 | Reaper | | ☐ | |
| Linux | X11 | Bitwig | | ☐ | |
| Linux | **Wayland** | Reaper | | ☐ | **the open question** |
| Linux | **Wayland** | Bitwig | | ☐ | |

Record the *specific* failure, not just ❌. These are four different bugs:

- no drag cursor appears at all → the plugin never started the drag
- cursor appears, the DAW refuses the drop → the DAW rejects the payload type
- drop accepted, nothing appears → the file was empty or the path was wrong
- drop accepted, wrong content → the SMF is malformed

### Decision

*To be recorded once the table is filled.*

If Wayland fails: **Linux defaults to the Export flow**, the export chip relabels, and
native drag stays enabled on X11. The app already detects Wayland at runtime
(`drag_capability`) and leads with Export there, so this is a default change rather
than new code.

## SMF import behaviour (PRD § 15 Q2)

How each DAW handles a dropped or imported file. Decides the default export mode for
Song Mode.

| DAW | Type-0 clip | Type-1 multi-track | Notes |
| --- | --- | --- | --- |
| FL Studio | ☐ | ☐ | Expected: type-1 splits onto the playlist as channels |
| Ableton Live | ☐ | ☐ | Expected: imports to multiple tracks |
| Logic Pro | ☐ | ☐ | |
| Reaper | ☐ | ☐ | Expected: prompts on import |
| Bitwig | ☐ | ☐ | |

## Test file

The spike drags a real generated `.mid`, not a placeholder: 4 bars at 140 BPM, kick on
1 and the "and" of 3, snare on beat 3 only, straight 16th hats, drums on MIDI channel
10. Written by `engine::midi::pattern_to_smf` — the same writer that will export real
patterns, so a DAW quirk found here is a real one.
