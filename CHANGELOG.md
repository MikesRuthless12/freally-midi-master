# Changelog

All notable changes to Freally MIDI Master.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **The release job extracts the tagged version's section from this file** and
> uses it as the updater's release notes. Match the heading format exactly —
> `## [0.1.0] - YYYY-MM-DD`. A missing section means every user sees a generic
> note instead of what actually changed.

## [Unreleased]

### Added — the drum grid becomes an editor, 2026-08-11

- **Drag a box to select.** Shift-drag draws a rubber band and selects every cell
  inside it — the whole rectangle, not just the cells the pointer crossed.
- **Copy, paste, clone and delete a selection.** `Ctrl+C`, `Ctrl+V`, `Ctrl+D` and
  `Delete`, with `Ctrl+click` to add a single cell. A copied triplet pastes back
  as a triplet: the clipboard keeps the real note timing rather than snapping it
  to the grid.
- **Right-click and drag to wipe.** One gesture clears every cell it crosses, and
  one `Ctrl+Z` puts them all back. A right-click that does not travel still opens
  the roll palette.
- **The lane names are readable again.** The lock and the fill button now appear
  when you reach for a row, which gives the name back the space it needs — it had
  been rendering as a single letter. A locked padlock stays visible whatever the
  pointer is doing, because a lock you cannot see is a lock you forget you set.

### Added — the keyboard shortcuts panel, 2026-08-11

- **It has a button now**, next to Settings and About. It was reachable only by
  pressing `?` or `F1` — a panel whose entire job is telling you what the keys do,
  findable only if you already knew a key.
- **It fits on one screen.** Four columns across the window instead of one long
  scrolling list, and the drum grid's own gestures are documented for the first
  time, in all eighteen languages.

### Fixed — 2026-08-11

- **Every note is visible when a clip is generated, vertically as well as
  horizontally.** A melody spanning two octaves had notes above and below the
  visible rows; the roll now sizes its rows to the clip's own register and
  centres it, rather than pinning it to the top with the leftover space below.
- **The loop brace can be resized by its edges.** Two things were wrong: the grip
  took the *first* handle in range rather than the nearest, so a short loop's
  right edge could not be grabbed at all — and the whole ruler showed a resize
  cursor, which made the one band that resizes invisible. Missing it drew a new
  loop over the one you were aiming at.
- **The last ten native dropdowns are gone.** Their menus were drawn by the OS
  against the window rather than the field, which is why they appeared detached
  and at the wrong size.
- **Ten defects found by review of the above, each fixed with a test watched
  failing first.** Four were in the drum grid's paste, all from measuring it in
  ticks when the grid thinks in columns: a humanized-early hit vanished when
  pasted at the first cell, a sparse figure pasted over a dense region kept the
  dense one's extra hits, a note could land past the end of the grid where no
  cell draws it, and a paste could bring back a lane you had reassigned away.
  The rest: the stretch band stopped being exclusive, a shift-click whose Shift
  was released before the mouse button edited the pattern, `Ctrl+Shift+click`
  emptied the selection it was meant to add to, `Ctrl+C` on an empty selection
  destroyed the clipboard, and a selection outlived the pattern it was drawn on.

### Added — 32 new genres, 2026-08-10

- **The genre list goes from 20 to 52.** New this release: **dark plugg**,
  **Detroit bounce**, **Jersey club**, **ATL swag rap**, **UK underground**,
  **EDM rage**, **digicore**, **jump-up DnB**, **neurofunk**, **jungle**,
  **pop DnB**, **UK garage**, **house**, **dance pop**, **pop 2020s**,
  **country pop**, **country shuffle**, **neo-soul**, **funk**, **future bass**,
  **afrobeats**, **amapiano**, **dancehall**, **reggaeton**, **baile funk**,
  **afroswing**, **Memphis rap**, **G-funk**, **lo-fi hip hop**, **sexy drill**,
  **hyphy** and **crunk**.
- Each one writes its own drums, chords, melody, countermelody and bassline —
  none of them inherits a part wholesale — and each arrives with three moods, so
  picking a genre gives you three different records rather than one.
- Every genre is measured against the rest of the roster: no two produce the same
  beat, the same fill or the same melodic part more often than chance allows.

### Fixed — a note could sustain past the end of the pattern, 2026-08-10

- **Drum hits on the last 16th no longer run past the end of the loop.** A ghost
  snare nudged late, or a tambourine on the final subdivision, could carry its
  length beyond the pattern — so the clip you dragged out held a note the bar had
  no room for. The other four generators had always trimmed; the kit never did.

### Fixed — basslines could wander out of key, 2026-08-10

- **A chromatic bass note now lands somewhere.** Passing tones are written as an
  interval off the *chord*, and over an ordinary chord that interval can leave the
  key — so the bass would play a note that was reached from nowhere and led
  nowhere. It now approaches the note it precedes, the way a bass player walks
  into a change, and a figure that never asked to be chromatic stays in the key.
  Walked flat sevenths and blue notes are kept: those are the idiom, not the bug.
- **A bass no longer plays above the chords it is holding up** in the models where
  the two registers overlapped, and four models whose bass doubled their own 808
  note-for-note now play a real second part.

### Fixed — the plugin could ship a stale genre list, 2026-08-10

- **Editing a style model now rebuilds the plugin.** The dataset is compiled into
  the binary, but nothing told the build system to watch it — so a plugin built
  after a model changed could keep serving the roster it was compiled with, with
  nothing anywhere saying so.

### Fixed — the File Explorer could not open a subfolder, and it was breaking four other things too, 2026-08-10

- **You can browse into subfolders again — at any depth.** Opening a library
  folder worked; opening anything *inside* it was refused. The cause was one
  guard: Windows' own `canonicalize` returns paths beginning `\\?\`, and the
  check that refuses network paths treated that leading `\\` as a network path.
  **The same refusal was silently breaking the preview player, the waveform,
  `.mid` reading, and dropping a sample onto a drum pad** — every one of which
  goes through the same guard. Local drives are now recognised as local; genuine
  UNC paths are still refused.
- **The browser can use the whole rail.** It was capped at roughly half the
  height however deep your folders ran, and collapsing the roster gave it space
  it could not take — the panel between them never claimed any height, so the
  tree sat at its minimum. Both halves are fixed, and there is a button on the
  browser to hand it the whole rail and give it back.

### Security — a page-supplied path could make the plugin authenticate to a stranger, 2026-08-10

- **Starring a file checked the library *before* checking for a network path**,
  and the library check itself is what touches the disk. On Windows that is not a
  read — it is an outbound SMB session that hands over your Windows credentials.
  One message from the plugin's own web view was enough. The guard now lives
  inside the containment check itself, so no future command can reintroduce it,
  and the same inversion in the sample-drop path is closed with it.
- **A shared project file could point the sample library at your whole drive.**
  Library folders travel inside `.als`/preset files, and only *network* paths were
  refused — so a project could carry `C:\Users` and quietly grant everything that
  reads files inside your library. A root now has to be deep enough to plausibly
  be a sample folder; a drive or profile root is dropped.
- **A malformed MIDI file could kill your DAW.** A file declaring an extreme time
  signature at one tick per beat pushed the arrangement reader past the end of its
  own tick space; in a release build that is not an error you can dismiss, it takes
  the host down with your unsaved session. Reproduced and pinned by a test.

### Fixed — three ways your work could go missing, 2026-08-10

- **A project with more than eight sample folders lost the extra ones.** The new
  folder tabs cap what you can *add* at eight — but that cap was also applied when
  *loading*, and the trimmed list was then written straight back over your project.
  A bound on what you may add is not a licence to delete what you already had.
- **An imported song was never saved.** Dragging a MIDI file onto the Song tab
  filled the timeline, but nothing asked the project to store it — save, reopen,
  and it was gone.
- **Undo deleted an imported song.** An import belongs to no artist, and the guard
  that stops one artist's arrangement leaking into another's read that as a
  mismatch and dropped it from every undo step. One keystroke after importing,
  Ctrl+Z wiped it and Ctrl+Y could not bring it back.

### Added — a real file tree, folder tabs, starred favourites, and MIDI that knows what it is, 2026-08-10

- **The browser is a proper tree.** Your library folders sit at the top, their
  subfolders indent underneath, and the files sit below those. Folders show an
  open or shut icon, `Up` retracts the deepest branch, and the tree remembers
  what you left open when you switch between folders.
- **Up to eight library folders, as tabs.** Pick a tab, sift that folder. **Add
  folder is disabled once you have eight**, so the rule is visible before you
  spend the gesture rather than after.
- **Your library survives closing the app.** It used to be saved only inside the
  *project*, so it came back with a `.als` and never with the standalone. It is
  now kept per user as well, and both are merged on load so existing projects
  keep theirs.
- **Starred favourites.** A star to the left of every sample, one-shot and `.mid`
  — outline until you press it, then solid yellow. Starred files gather in a list
  under the tree; click one and the tree opens every folder between the root and
  that file to reveal it. **If its folder is no longer one of your eight open
  tabs, it opens the file's location in Windows Explorer or Finder instead**, so
  a favourite is reachable whatever your library has moved on to.
- **The keyboard walks the tree.** `↑`/`↓` move between rows, `→` opens a folder
  and `←` shuts it — and on a *file* those same two keys play it forwards and
  backwards. A folder has nothing to audition and a sample has nothing to expand,
  so which one you meant is never ambiguous.
- **`.mid` files are shown**, with their own icon, and are kept honestly apart
  from audio: a MIDI file has no waveform, and it cannot be dropped on a drum pad
  at all — the pad refuses it before you let go rather than erroring afterwards.
- **Put the selected sample on a pad without dragging.** Every pad grows a "use
  selected" button while a sample is selected. Both routes end in the same
  command, so a sample assigned either way is identical on the pad.
- **One list of formats.** What the tree shows, what the Open dialog filters on
  and what the decoder can read now come from a single place, so the browser can
  no longer offer a file the loader will refuse.

### Added — drop a MIDI file in and it works out what is in it, 2026-08-10

- **Drag a `.mid` onto a generator** and its notes land there.
- **Or drop it on the Song tab and take the parts you want.** The whole file
  arrives as an arrangement — sections across the top, parts down the side — and
  clicking any cell opens that clip in its generator, exactly as drilling into a
  generated song already does. Nothing overwrites what you have until you choose
  it. *(Generating a song is unchanged.)*
- **A layered file is separated into Bass, Melody, Counter, Chords and Drums**,
  and **every part says why it was routed where it was** — "on the GM drum
  channel", "chords, notes overlap", "lowest voice", "from the file name" — so a
  wrong guess is one click to redirect instead of something you discover later.
- **It was taught by real files, not by assumptions.** Four sample-pack exports
  broke the first version three times out of three, and each fix is a measurement
  of them: sample packs put drums on **channel 0**, not the General MIDI drum
  channel, so drums are recognised by their shape — a handful of pitches, struck
  many times, **short**. Held melodic notes overlap constantly, so chords are
  notes struck *together* rather than notes that merely overlap. A line with a low
  average pitch that ranges over three octaves is a melody, not a bassline. And
  **an 808's MIDI note says which key fires the sample, not how low it sounds** —
  so a bassline routinely sits in melody register, which is why the **file's own
  name** is trusted above any measurement of it.

### Added — drum pads on the stage, one box to find any artist, and a window that finally behaves, 2026-08-09 (evening)

- **Eight drum pads across the top of the stage.** Each carries everything on its
  face: the lane's name, what is on it, a **green or red dot** for whether you can
  hear it, **Play** in the top centre to audition it alone, a shuffle to re-roll
  it from the folder you are browsing, and a ✕ to put the built-in sound back.
  Press the pad to mute it; drag a sample onto it to assign one. Every pad's name
  is a picker over all thirty-seven lanes, **two pads may share a lane** so a
  snare can be layered, and the layout is remembered **per artist** — a style you
  built comes back exactly as you left it.
- **One combobox instead of a search box, a chip row and a five-hundred-row
  list.** Type to filter across artists, genres, aliases and typos; press the
  arrow for the whole list. Stop halfway and click away and it takes the best
  match, so you cannot end up with nothing chosen. The artist's details now sit
  **directly under the box** rather than below a list you had to scroll past to
  read them, and **"Original Workflow" is pinned to the top** whatever is
  selected, so building your own is never something you have to hunt for.
- **The generator mute switches moved onto the tabs**, as a dot in each tab's top
  right. They were a second row carrying the same six words as the tabs, so
  working out which one silenced Drums meant reading "Drums" twice.
- **The whole clip is visible when it is generated**, whatever its length. The
  roll opened at a fixed zoom, so anything past four bars ran off the edge and
  had to be zoomed out by hand after every generation.
- **Presets are a combobox too**, and the transport sits centred on its own row
  above the tabs, which now spread the full width of the stage.

### Fixed — 2026-08-09 (evening)

- ⛔ **The plugin ran DPI-unaware while WebView2 did not.** On a 150% display
  Windows stretched the window by half again while the page rendered at true
  pixels, so the UI covered two thirds of its window and left a dead margin — and
  the content did not follow the window when it was dragged. One mismatch, both
  symptoms.
- **The window can be dragged, resized and maximised**, and the UI scales with it
  rather than being cropped or leaving space. It will not shrink below the size
  the app was designed for.
- **A style's copied samples are loaded back.** The consent gate and the copy both
  worked; nothing ever read them, so the checkbox's promise that they *"still work
  if you move or delete the originals"* was false while the build shipped it.
- **"Single hit" no longer deletes the note it names** — it collapses a roll back
  to a single hit, which is the inverse of the roll it undoes.
- **Double-clicking the standalone works.** It supplies its own audio period size
  rather than depending on a command-line flag, and the release build no longer
  opens a console window behind it. Crashes are appended to a log in the app's
  data folder.

### Added — every artist gets moods, you can build and train a style of your own, and nothing copies your samples without asking, 2026-08-09

- **Every artist and genre now has moods.** Twelve genres had none — boom bap,
  Chicago drill, country, jerk, liquid DnB, NY drill, phonk, plugg, pluggnb,
  2000s pop, 2000s R&B and west coast club — and each now offers three: boom
  bap's *dusty / jazzy / hard*, phonk's *cowbell / memphis / brazilian*, country's
  *shuffle / ballad / barnburner*, and so on. Pressing Generate walks an artist's
  range instead of returning one sound with a different seed on it. **96
  (artist, mood) pairs across all thirty models**, up from 33 across nine, and
  84 of them reach a thousand different melodies in a thousand seeds.
- **Original Workflow — build a style of your own.** A pinned row above the
  roster opens a style editor: name it, base it on any artist or genre, set the
  tempo range, swing, hat density, melody density and scales, and save. It
  appears in the roster marked **Yours** and generates, locks, re-rolls and
  exports exactly like a shipped one, because it *is* one — the same format the
  shipped models use. It inherits everything you did not change, including
  improvements the base gets later. A new style opens seeded from the beat on
  screen rather than blank.
- **Train a workflow from the takes you keep.** A star on the take counter marks
  a generation to train on; at thirty the Train button fits a style to what you
  kept — density, register and phrase shape — and it will not save one that
  repeats itself. **There is no machine learning in it**: it measures your takes
  and writes the numbers back, and the app stays entirely offline.
- **Your own MIDI can train it too.** The plugin can now *read* Standard MIDI
  Files, not only write them, so a `.mid` you already have becomes a training
  source measured by the same code as a generation.
- **Nothing copies your samples without asking.** A style can keep the one-shots
  you assigned so they survive you moving or deleting the originals — and
  because that means a second copy on your drive, the editor tells you **how
  many files and how many megabytes** before anything happens, with the box
  unticked. Saving a style copies nothing on its own.
- **Your samples sound better.** A sample recorded at a different rate from the
  kit around it used to be stretched to fit on every single note; it is now
  converted once, properly filtered, when you load it.

### Fixed — 2026-08-09

- **The genre chips said they could not be clicked.** The Genres row in the left
  rail showed a "no entry" cursor over controls that worked perfectly, and had
  since the layout was first drawn. They also could not show which genre was
  selected. Both fixed.
- **Training could have frozen your DAW.** The check that a trained style does
  not repeat itself ran thousands of generations on the thread your host draws
  its window from, and would have hung it long enough for the request to time
  out and throw the work away.

### Added — the melody stops quoting other people, every genre gets its own voice, and the hats talk, 2026-08-08

- **⛔ A novelty guard, so a generated melody is not a hook somebody already
  owns.** Every melody and countermelody is screened against a bundled table of
  well-known contours before you ever hear it, and a take that matches is thrown
  away and drawn again — up to three times, then the screen loosens rather than
  refusing to give you notes. It costs under a millisecond and you will normally
  never know it ran. **Nothing in the table is a melody**: it holds one-way
  fingerprints of *contours* — where a line moved and how long it waited — and
  there is no way back from one to a note. Transposing a hook does not hide it,
  and playing it staccato does not either.
- **Every genre now has its own melody, countermelody and chords**, instead of
  five of them falling back on a shared default. NY drill's bells, phonk's
  cowbell lead with its minor-second menace, pluggnb's chord-tone arps and west
  coast's four-note plucky riff are each written from the research rather than
  inherited. Six genres that have no 808 — boom bap, country, liquid DnB, NY
  drill, 2000s pop and 2000s R&B — got a real bassline to go with it.
- **⛔ Hi-hat fills.** The hat is where trap, drill and plugg do their talking,
  and now it does: a phrase-end figure that breaks the stream and hands over to
  the next bar. Five figures — a roll, a stutter, a triplet burst, a rising ramp,
  or a **gap**, the hat stopping dead — landing where the genre says, every two,
  four or eight bars. And a one-press **add fill** on the hat lane in the grid,
  which writes the same figure in the same place the generator would have.
- **Click the piano roll's ruler to move the playhead.** The drum grid and the
  song timeline already did; the four melodic generators had no way to move the
  transport at all. Dragging on the ruler still sets the loop brace — a click is
  a seek, a drag is a brace, and a click no longer leaves a one-step loop behind
  it.
- **⛔ Solo, on every drum lane.** Beside the mute, and it does what solo does:
  everything else goes quiet while your notes still reach your DAW untouched. A
  lane you muted stays muted through it, so switching solo off cannot leave
  something audible you had deliberately silenced.
- **Click a lane's name to hear that drum on its own** — which pad am I about to
  edit — without soloing anything or pressing play.
- **A roll palette on every step.** Right-click any cell in the drum grid and
  pick 2, 3, 4, 6 or 8 hits inside it. The keyboard chords were already there;
  nothing on screen said so.

- **⛔ Lock a drum so a re-roll cannot touch it.** Click the padlock on any
  lane — or press `L` with the row focused — and Generate rebuilds everything
  else around it, note for note. `R` re-rolls, `G` generates, `Shift+G`
  generates every part, and `1`–`6` pick a generator.
- **⛔ A pattern library that outlives the project.** Name something you made
  and it is there next time, in any song and any DAW — **saved as notes, with no
  kit**, so you can load it and put whatever sounds you like under it. Each one
  is its own file, so a bad save costs one loop rather than the shelf.
- **⛔ Every take you have made this session, and a way back to any of them.**
  ◀ and ▶ walk your generations — per generator, counted from the first, with no
  cap — and stepping back brings the **whole setup**: the artist, the mood, the
  seed, the bars and the pins. It shows the tempo and key that were actually
  used, which is not always what you pinned.
- **⛔ Sixteen more drums.** Sub kick, ghost snare, pedal hat, ride bell, high
  and low toms, clave, conga, bongo, timbale, triangle, a second perc, an 808
  sub layer, and riser / impact / reverse. Every row now names its instrument
  and **can be switched to any drum the kit is not already using**.
- **The roster tells you what something is before you generate.** A badge says
  artist or genre, and a pane under the list gives the era, the genres and what
  that artist *tends to* — tempo, key, half-time, how many moods.

### Changed

- Three golden snapshots moved (`trap-7-4bar`, `trap-2024-8bar`,
  `uk-drill-7-4bar`): trap and UK drill now author a hi-hat fill, so their
  patterns changed on purpose. Diff read before regenerating.
- `ny-drill`, `liquid-dnb`, `country-train` and `boom-bap` play different
  basslines than they did, because they now author their own instead of
  inheriting one. `pop-smoke` gets its own on top of NY drill's.
- A fourth golden snapshot moved (`trap-2024-8bar`): one closed hat sat on top
  of an open hat and is gone, and every hat after it in the lane draws a
  different velocity because `humanize` walks a lane in note order. Diff read —
  the removed note is at tick 14880, where the open hat is.

### Fixed — what the review found, 2026-08-09

Fifteen findings survived independent verification. The ones a producer would
have hit, worst first:

- **⛔⛔ Opening the pattern library could take the whole DAW down.** The preview
  histogram divided by a clip length computed from the saved file's own `ppq`
  and meter, and those numbers arrive as whatever survived being backed up,
  synced or copied between machines — an unlucky pair divided by zero, and the
  plugin aborts rather than unwinds. One unreadable file now costs you that
  file, which is the entire reason each pattern is its own file.
- **⛔⛔ Saving a pattern could silently delete a different one.** "Take 1" and
  "Take-1" became the same filename, so the second save replaced the first with
  no warning and no way back. Saving over *your own* name still replaces it —
  that is the point — but two different names now keep two different files.
- **⛔⛔ A locked lane could vanish from the saved project.** Lock the hats you
  drew, press Generate, save, reopen: the clip was rebuilt from the seed and
  your hats were gone. A generation with a lock in it is no longer the seed's
  own output, so it is written into the project like any other edit.
- **⛔ A locked lane carried notes past the end of a shorter clip.** Lock the
  kick on eight bars, drag down to four, Generate — the grid drew four clean
  bars while the export played hits in bars five to eight that nothing on
  screen had ever shown you.
- **⛔ Soloing a drum row silenced the melody, countermelody, bass and chords.**
  Solo is offered on the drum grid; none of those four editors has a mute or a
  solo on it, so the preview went quiet with nothing to explain it and nothing
  to undo it. A solo silences what it is a solo among.
- **⛔ A hi-hat fill could put a closed hat on top of a sounding open hat.** One
  hi-hat cannot be open and shut at the same instant — the hat engine has always
  removed the closed hit underneath an open one, and both the roll and the fill
  wrote fresh hats over that decision afterwards. The rule now lives where the
  hat lane is finished rather than inside each thing that decorates it.
- **⛔ Song Mode never ran the novelty guard.** It was installed on the path the
  Melody tab uses, and Song Mode calls the generators directly — so every melody
  and countermelody in every arrangement went unscreened while the note above
  said otherwise. It is screened now, and a test asserts the wiring rather than
  the output, because with the shipped roster screened and unscreened output are
  identical and the obvious test could not fail.
- **⛔ A hand-added hi-hat fill landed 40 ticks early — ahead of the beat, every
  time.** "Add a fill" reached for the same window the grid uses to decide which
  cell a *humanized* note belongs to, and that window deliberately starts a
  little before the beat so a hit nudged early still reads as on it. Right for
  reading, wrong for writing.
- **And it cleared the wrong amount of the bar in any meter that is not x/4.**
  A beat is four sixteenths in 4/4 and two in 6/8; the fill always took four, so
  in 6/8 it destroyed a beat of hats nobody asked it to touch.
- **Stepping back through your takes gives you that take.** Recall regenerated
  through the ordinary Generate path, so it spliced in whatever was locked *now*
  and rebuilt at whatever tempo the session had drifted to — while the counter
  beside it displayed the take you asked for. It now restores the tempo, key and
  meter the take was actually written at, and reloads the artist's own defaults
  when the recall crosses artists.
- **The 808's sub layer exports on its own MIDI channel**, instead of sharing
  the 808's and arriving in the DAW as one merged instrument.
- **The lane picker no longer offers the two pitched 808 lanes as drum slots** —
  moving a perc row onto one sent its hits out as bass notes.
- **A locked lane survives Generate All.** The lock was consulted by Generate
  but not by Shift+G.
- **Auditioning a drum row plays that row.** Six lanes — including `sub` and
  `sub low`, which are two different rows on the grid — shared one General MIDI
  note, so clicking one could sound another.
- **`L` no longer toggles a lane lock while you are typing**, and neither do the
  variation arrows.
- **A saved pattern and a saved preset agree on what a file is called.** Their
  two name-to-filename functions carried identical comments and different
  constants.

Two gates were reporting success over less than they claimed, and are fixed
rather than merely noted: the TypeScript lane list had drifted to 21 of the
engine's 37, so the check that every lane has a name in all 18 languages had
been passing over sixteen it never saw; and the solo test *required* the
melodic parts to go silent, which is how that behaviour survived review. Both
now derive their list from the generated bindings or from the engine itself.

### Added — the sample browser you can actually use, and clips that look like clips, 2026-08-07

- **⛔ The sample browser is on screen.** Add your folders once and they come
  back with the project. Browse them, click a sample to see its waveform, and
  **drag it straight onto a drum lane**. The whole panel resizes — drag its edge
  and the arrangement shrinks to match, so you can read long file names without
  giving up the middle of the app. Everything behind this shipped last session
  with no way to reach it.
- **An audition player under the browser.** Play, pause and stop — stop goes
  back to the beginning, pause holds where it is. **Click anywhere in the
  waveform to play from there.** The part you have already heard is filled in
  behind the wave, a marker follows the playhead, the time reads out of the
  total, and there is a loop toggle. **Left arrow plays the sample backwards**,
  right arrow puts it forward again.
- **⛔ A clip in the arrangement looks like a clip.** Every one now draws its own
  notes — where they land and how high they sit — instead of a name over a
  shaded box. Two clips of the same part no longer look identical.
- **Each clip says what it can be handed over as**, MIDI or audio, and **can be
  dragged into your DAW on its own**. Before this you could drag a whole part or
  a whole arrangement, and the one clip you were looking at had no handle.
- **⛔ Clips resize.** Drag a clip's right-hand edge and that row loops on fewer
  bars inside its section — a four-bar idea played as a two-bar loop — without
  moving the section or any other part. What you hear and what you export agree.
- **⛔ The arrangement view is laid out like a DAW.** The track names, mute, solo
  and lock sit in a fixed column down the left, and only time scrolls past them.
  They used to float on top of the clips: at bar 1 the words "DRUMS" sat over
  the first clip of every row, and scrolling dragged them across whatever was
  underneath. The rows are taller, and the clips are solid enough to read as
  blocks laid out in time.

### Fixed — what three reviews found, 2026-08-07 (later still)

- **⛔ A resized clip's longer notes rang into its own next repeat**, so a DAW
  cut them dead against the note that had already re-struck — and the last
  repeat's tail could kill a note in the following section.
- **⛔ Hi-hats lost every ghost note outside 4/4.** In 6/8, 12/8 and any x/16
  meter the whole stream came out at one flat velocity, ignoring the quiet
  in-between hits every artist is written with.
- **⛔ Percussion set to "offbeat" went completely silent** in x/16 and x/32
  meters — no notes in the grid, in playback or in the exported stem.
- **A resized clip drew notes it does not play.** The timeline showed the whole
  pattern squeezed into every repeat.
- **The track names, mute, solo and lock stopped lining up with their rows**
  once the arrangement scrolled far enough to need it.
- **Re-rolling a section, or copying, pasting or dragging a clip, threw away the
  loop length you had set on it.**
- **Removing the folder you were browsing left the preview player still showing
  a sample it could no longer reach** (Windows only).
- **The bassline could still mirror the wrong kick** if you changed the bars,
  pinned a tempo or switched mood between generating the drums and the bass.
- **The selected language in Settings had no highlight**, and the shortcuts
  panel was partly unstyled.

### Fixed — the audition player, 2026-08-07 (later)

- **⛔ Auditioning a sample was silent whenever the Audio switch was off.** Play
  lit up, the player said it was playing, and the playhead sat frozen at 0:00
  with nothing saying why. Turning off audio means "send MIDI to my own sampler,
  don't double it" — it was never meant to stop the file browser playing the file
  you just clicked.
- **Clicking a paused waveform moved the playhead and then undid itself** half a
  second later.
- **Pressing Play left the playhead sitting still for up to half a second**,
  while the sample was already sounding. On a one-shot shorter than that the
  whole audition could finish before the marker moved at all.

### Fixed — 2026-08-07 (later)

- **⛔ The entire arrangement view was drawing unstyled**, and had been since it
  was written. Every colour it asked for was a name this app does not define, so
  the clips had no fill and no border, the section band had no background, the
  grid lines were invisible and the playhead was a transparent strip. It looked
  like a design nobody had finished.
- **The bassline landed on kicks the drums were not playing.** A bass that
  mirrors the kick — which is most of the trap and drill roster — was copying a
  kick pattern rebuilt from the wrong seed. On boom-bap 13 of 13 bass notes used
  to sit on a real kick; with the ordinary Generate-drums-then-Generate-bass
  workflow that had fallen to 9, and on UK drill to 1 of 14.
- **Percussion set to play "offbeat" played straight on the beat** in 6/8, 12/8
  and every other x/8 meter — the layer meant to sit between the pulse doubled
  it. UK drill, NY drill and Pop Smoke all ask for it.
- **Six drum voices shipped with no way to hear them.** Ride, crash, tom, shaker
  and cowbell were in the kit and in the lane list, and no artist had ever been
  written to play one; the off-snare had the same problem. Boom-bap and liquid
  drum & bass ride now, phonk and west-coast club have cowbells, the drill
  family answers its snare, and country and 2000s R&B have shakers.
- **The language you have selected in Settings had no highlight**, and its tick
  was invisible.

### Added — a real drum kit, a sample browser, and a kit per genre, 2026-08-07

- **⛔ Your drums have more than five lanes now.** Off-snare, ride, crash, tom,
  shaker, tambourine, cowbell and woodblock join the kick, snare, clap, hats,
  rim, snap, perc and 808 — sixteen percussion voices, and which of them an
  artist uses is part of that artist. Fifteen of the shipped models had already
  been *written* with a percussion block; nothing had ever read it, so UK drill
  had been asking for a woodblock since the day it was authored and getting
  silence.
- **⛔ Every genre gets its own kit.** Eleven families — trap, drill, rage,
  plugg, boom-bap, country, R&B, club, phonk, drum & bass and pop — each with
  its own tuning, top end and drive. Boom-bap's ceiling is 8 kHz because that is
  what an SP-1200 could sample, and that missing top *is* the sound. Drill's sub
  slides further; rage is distorted on purpose; country is open and undriven.
  Eight artists had been naming the kit they wanted for months and all of them
  were playing trap samples.
- **The 808 lane is called Sub now**, because it is the pitched bass that
  slides, not the bass drum — and you might load a 606, a 707 or a 909 into it.
  Your saved projects open exactly as before.
- **Finger snaps make a sound.** The lane has existed since one-shots shipped
  and never had a voice.
- **A keyboard-shortcuts panel**, on **?** or **F1**, grouped by what the
  shortcuts do. **Space plays and pauses** — which it did not before, anywhere.
- **⛔ Generate varies the take without changing the record.** Press Generate on
  the drums, switch to the melody, press it again: you get a different melody,
  written against the *same* key and chord progression as the drums. Before
  this, the second press quietly drew an unrelated harmony and the two parts had
  never been written against each other — they were each correct and did not
  belong together.
- **A sample browser** (the plugin's side of it). Add your sample folders once
  and they come back with the project; browse them, and drop a sample straight
  onto a drum lane. It reads the waveform for a preview player, and it will not
  browse outside the folders you added.

### Fixed — 2026-08-07

- **A loop brace you dragged was ignored whenever a second generator was
  switched on** — which is the default. It worked with one part playing and
  silently did nothing with two.
- **Play and Stop were lit or dark at the wrong times.** Generate drums, click
  the Melody tab, and Play went dark while the drums were armed and would have
  sounded. Switch every part off and Play stayed lit — pressing it reported
  playing forever with a marker that never moved.
- **Clicking a part switch on the Song tab silenced the whole arrangement**,
  with the timeline still on screen and Play still lit.
- **The position readout counted the wrong clip.** Four bars of drums beside
  eight of melody read 1.1 to 5.1 across eight bars of real time.
- **Loop came back on after reopening the plugin window**, whatever you had set
  it to.
- **The KIT and STEMS panels drew over each other in a short window** once the
  kit grew past thirteen rows.

### Added — play the generators together, and the 808s slide, 2026-08-06

- **Play several generators at once, or any one alone.** Each generated part
  gets a switch beside the tab strip; Play sounds every one that is on. A
  schedule holds a single clip, so the parts are merged into one before it —
  which is why this could not be done by pressing Play on each tab in turn.
- **⛔ The transport moved to the top of the app**, to the right of the
  generator tabs, beside the switches it acts on. The bottom bar keeps the
  position readout, the meter and the view controls.
- **⛔ The plugin plays without the DAW rolling.** Play, Stop and Pause now
  drive the plugin's *own* preview transport inside a host — auditioning a beat
  no longer means arming a track and starting the whole project. Starting the
  DAW's transport takes it straight back, so the two can never both play at
  once. Previously Play was disabled in a plugin window and said to press play
  in your DAW instead.
- **⛔ The Loop button works.** It was permanently pressed and disabled, with a
  tooltip claiming playback always looped — which was not even true unless a
  loop brace had been dragged on the roll. It toggles now: on repeats the 4 or 8
  bars, off runs to the end and stops. A brace you have dragged still wins. It
  lights up when it is on, rather than announcing its state only to a screen
  reader.
- **⛔808 slides are audible.** The generators have written slides for a long
  time and the exported MIDI has always carried them, but every rendered
  WAV — exported, dragged into a DAW, or heard in the plugin's own
  playback — played them as flat notes. The pitch now travels, holding the
  starting note for the first half and gliding across the second, which is
  exactly where the MIDI puts the change. The same code serves the preview, the
  export and the drag, so a stem sounds like the `.mid` beside it.

### Fixed — the drum lanes, the KIT panel, and a short right rail, 2026-08-06

- **⛔ "All Tracks" for drum audio was labelled the opposite of how it reads.**
  It gave one separate file per lane, while a second entry called "As one clip"
  gave the mix — so choosing "All Tracks" to get everything together produced
  the opposite. There is now one entry per drum lane, exactly as the MIDI menu
  has, and **"All Tracks" is every lane mixed into a single file**. MIDI still
  offers no whole-kit file, deliberately.
- **⛔ The KIT panel stopped drawing on top of the STEMS panel.** In a short
  window the KIT section collapsed to nothing while its contents kept their full
  height, so its text was painted straight over the panel below it as an
  unreadable smear.
- **The KIT panel is guarded against telling you there is no kit while one is
  playing** — the fault was fixed earlier, but nothing tested it, which is how it
  shipped the first time.

### Fixed — the editor window sizes itself correctly, 2026-08-06

- **⛔ The black band around the UI is gone.** A larger window left dead space
  around the app, in the standalone and in a hosted VST3 alike. The editor's own
  window was being sized from a display scale read at a different moment from the
  one the rest of the geometry used, so the app ended up two-thirds of the window
  that framed it and the frame's background painted the difference.
- **⛔ The size button offers two sizes, not four.** Pressing it grew the window
  *twice* before coming back, out of a list that has only ever held two entries.
  The display scale was read fresh on every press, and it changes underneath the
  editor: it reads 100% until the window system has been told the process is
  DPI-aware, which happens *after* the editor has already been sized. So each of
  the two presets quietly meant two different windows. The scale and the usable
  desktop are now each read once and pinned together for the session, so a preset
  is one window for as long as the plugin is open.
- **The editor opens larger, and the button makes it larger still.** The two
  sizes are now 1:1 and a step above it, so the first press grows the window and
  the second returns it — rather than the only choice being *smaller* than the
  default.
- **⛔ The Stems panel no longer disappears at the default window size.** The
  right rail opens at 1440px and the page lays out at exactly 1440px, so the two
  numbers were equal and the rail had no margin at all: the width arrives through
  a zoom round-trip that landed a single pixel short, and the whole rail — Stems,
  Kit, Session, Presets — was gone at every size except the one that happened to
  zoom by exactly 1. It now has room to spare, and neither size zooms the page.
- **⛔ The per-lane drag-out menu is no longer cut in half.** Opening MIDI on the
  Drums row showed a list that ran into the panel below it and stopped, so the
  lower drum lanes could not be dragged out at all. The menu was being clipped by
  the scrolling panel it lived in; it now escapes it, still opening downward, and
  lifts only if it would otherwise run off the bottom of the screen.

### Fixed — six defects Mike found running the plugin in Ableton, 2026-08-06

- **⛔ Generate really does generate now.** Pressing Generate repeatedly returned
  the *same beat* forever, which made the product look like it held one loop per
  artist. The engine's chosen seed was echoed back into the seed box and then
  re-sent on the next press. The box now distinguishes **a seed you chose** from
  **a seed the engine picked and is showing you** — typing pins it, clearing
  unpins it, and the padlock says which mode it is in. Unpinned, every press
  rolls a new one; **Generate all** draws one fresh seed and shares it across all
  five parts, because `parts.rs` only guarantees the parts agree on a shared one.
- **⛔ Dropped audio no longer plays at the wrong tempo.** A dragged `.wav`
  carried no tempo at all, so Ableton warped it by guess — a 140 BPM loop played
  at 96 in a 120 project. WAVs now carry an `acid` chunk with the tempo, the beat
  count and the meter, which is what loop libraries use and what both Ableton and
  FL read. MIDI was always fine.
- **The Stems panel opens itself** the first time a session generates anything.
  It remembered being collapsed across reloads, and it holds the only way to get
  a pattern out of the plugin — so collapsing it once hid the drag rows for good.
- **Two window sizes, not three.** The largest left dead black space around the
  UI. The deeper cause is fixed too: the window size and the page zoom are one
  number applied by two different routes, and the page now measures the window it
  actually got rather than trusting the one it asked for.
- **⛔ The controls stopped sitting on top of the velocity lane.** Found while
  fixing the above, and live for months: the Generate/seed/bars row was an
  absolutely-positioned column floating over the editor, so a velocity cap
  underneath it could not be dragged at all. It is below the editor now.
- **The piano roll's closing bar number is visible.** A four-bar clip is ruled
  1–5 and an eight-bar clip 1–9, the way a DAW counts. The line was always drawn;
  its *number* was painted three pixels past the edge of the canvas.

### Added

- **Drag each instrument out on its own.** Clicking a part's **MIDI** or
  **Audio** chip opens a menu of every instrument actually playing in it, each
  its own drag handle, with **All Tracks** last — every lane at once, as separate
  files, in one gesture. Mike, 2026-08-06: *"just dragging the hihats out like
  Drum Monkey"*. **All Parts** does the same for melody, bass, counter and chords
  together.
  - ⚠ **MIDI and Audio offer different lists, deliberately.** A lane that was
    written but that the kit cannot play drags as MIDI — the notes are real, and
    a producer routing them into Battery wants them — but not as audio, which
    would render silence.
  - **Ctrl decides the layout** on All Tracks: held, the clips stack; released,
    they land one after another. The modifier is read at the *drop*, so pressing
    it during the drag counts.
- **The whole arrangement drags out as audio**, which the plugin used to refuse
  outright. Rendering a record is seconds of work, so it now reports how far it
  has got and stops the moment the gesture is abandoned — that was the missing
  piece, not the rendering.
- **Clips can be dragged around the arrangement.** Every other DAW verb was
  already there — delete, copy, cut, paste, clone, resize — and rearranging meant
  copy, paste, then go back and delete the original.
- **The plugin window comes back after a drag.** Dragging into Ableton's
  Arrangement view made it disappear until you switched views and reopened it
  by hand.
- **Ctrl + ↑/↓ transposes a semitone, Shift + ↑/↓ an octave**, on one note or a
  whole selection.
- **A drag source for macOS and Linux.** Linux uses GTK with `text/uri-list`;
  macOS uses `NSDraggingSession`. ⚠ **Neither has been dropped into a real DAW
  yet** — Linux compiles and macOS has only ever been compiled by CI. They are
  switched on so they can be tested, not because they are proven.

- **Drag a part straight out of the plugin and onto a DAW track
  (TASK-063C / FMM-S03).** Mike, 2026-08-05: *"you need to be able to drag each
  generator's midi or audio from the generator to the DAW and ensure it shows a
  preview of what you are dragging"* … *"same with the song arrangement"*. Every
  generated part has a **MIDI** and an **Audio** handle in the right rail; with
  **Per lane** on, the drum part becomes one handle per lane, so just the hats
  or just the snares can go out on their own. The whole arrangement drags too.
  Files carry the name a producer would give them —
  `trap - Snare - 140 BPM - C# Minor` — and a picture of the clip's own notes
  rides on the cursor the whole way into the DAW.
  - ⚠ **This shipped Windows-only and no longer is** — see the macOS and Linux
    drag sources further up this release. Windows is the one a human has
    actually dropped into Ableton.
  - ⚠ **This shipped MIDI-only for arrangements and no longer is.** The reason
    given — that a song needs progress to watch and a cancel to press — was
    right, and it is now built rather than avoided.
  - ⚠ Dropped files are spooled to your temp folder. **MIDI is copied into your
    project by every DAW, so those are swept after a week — audio is
    *referenced* by path, so those are never deleted.** Use your DAW's Collect
    All and Save to keep a dropped loop for good.
- **The drum grid is an editor (TASK-131G).** It was read-only and its own
  header said so. Click a cell to place a hit or clear it; **Alt+click** clones
  the previous bar of that lane; **Ctrl+2 … Ctrl+9** turn a cell into a tuplet —
  a triplet, a quintuplet, whatever the digit says; **Delete** clears it. Edits
  go through the same path the piano roll uses, so undo, arming and saving with
  the project all come for free.
  ⚠ The edits work on **ticks, never on cells**: a cell has already thrown away
  where inside the 16th a hit sat, which is exactly what a tuplet is made of.
- **Export the generated parts on their own, as MIDI or audio (TASK-131F).**
  One file per part, or **one per lane** — so just the hats, or just the snares.
  Files are named the way a producer would name them:
  `trap - Snare - 140 BPM - C# Minor`.
  ⚠ This is the *export* half — writing the files into a folder you pick. The
  drag half landed alongside it (TASK-063C above) and shares the same naming
  and the same bytes, deliberately: a loop you drag and the same loop you
  export must be the same file.
- **Every genre has its own harmony (TASK-040).** Twelve genres inherited their
  chords from `_defaults` wholesale and all reached exactly 121 distinct chord
  parts in 200 seeds. Each now authors its own progression families, harmonic
  rhythms and voicings from the style research.


- **Your own one-shot on any part (TASK-131B).** Click a lane in the KIT panel,
  pick a sample, and that part plays it — drums lane by lane, plus melody,
  countermelody, bassline and chords. WAV, AIFF, FLAC, MP3, M4A and OGG, decoded
  by `symphonia`. The assignment is stored in the project as a **path**, the way
  every DAW stores a sample reference, and is reloaded when the project reopens;
  a file that has moved is reported rather than silently reverting.
  ⚠ A one-shot on a melodic part inherits the placeholder's root note, so it
  plays near its own pitch and moves by the melody's intervals rather than
  jumping octaves. Detecting a sample's real pitch is TASK-052.
  ⚠ `Lane::Snap` has never had a shipped pad, so it has always rendered silence;
  assigning a one-shot to it is now the only way to hear that lane.
- **Drum hits move in pitch (TASK-131D).** Rolls climb and fall in chromatic
  semitones, by a span each artist authors — rage travels eight, Drake two, and
  `country-train` none at all, because a train beat does not pitch its snare.
  The plugin's own sampler now transposes percussion to match, so what the grid
  shows is what you hear.

### Changed

- **Generation offers 4 or 8 bars.** Two is gone — there is not enough room in
  two bars for the fills and turnarounds the models author, so it made every
  artist sound the same. ⚠ A project saved at two bars still opens at two bars.
- **The piano roll wears a plain pointer**, not a `+`. A crosshair is what a
  drawing tool wears, and clicking empty grid selects rather than draws. Note
  edges now show which end will move rather than one two-headed arrow.

### Fixed — an xhigh code review of the above, 2026-08-06

Fifteen verified defects in the work described on this page, all but one closed.
Four of them would have shipped something broken to somebody.

- **⛔ Dragging a clip in the arrangement could delete another one.** Select two
  clips of the same part — two drums clips, or simply two whole sections, since
  selecting a section selects every part in it — drag them, and only one
  arrived: a section holds one clip per part, so the second overwrote the first
  after both had already been lifted. The clip was gone with nothing on screen
  saying so, and undo was the only way back. **A selection now keeps its shape**:
  the clip you grabbed lands where you dropped it and the rest move with it, the
  way a DAW does, clamped so nothing is pushed off the end.
- **⛔ The Audio drag chips disappeared if you had ever collapsed the KIT panel.**
  The panel was the only thing that loaded the kit, and a collapsed panel is not
  rendered — so with KIT closed the Stems panel decided nothing could be played
  and hid every Audio handle, permanently, while Export went on offering audio.
- **⛔ A drum part could no longer be dragged out as one file.** Adding the
  per-instrument menu turned the MIDI and Audio buttons into menu openers, and a
  menu opener cannot be dragged. **"As one clip" is the first entry in the menu
  now** — the whole kit on one DAW track, which is what those buttons used to do.
- **⛔ Dropped audio played at the wrong tempo in 6/8, 9/8, 12/8 and 7/8.** The
  tempo chunk counted a bar's beats with the numerator, but the audio is
  measured in quarter notes: four bars of 6/8 declared 24 beats for 12 beats of
  music, so Ableton warped the stem to half speed. That is the same defect the
  chunk was added to fix, arriving through the meter picker instead.
- **A trimmed clip dragged as "All Tracks" wrote silent files.** Every lane after
  the first was pushed past the clip's own trim marks, so its notes fell outside
  and the file arrived empty — eight files dropped into the DAW, seven of them
  containing nothing.
- **Pressing the padlock on a clip could move the clip.** With a few pixels of
  hand movement the press became a drag: it locked, threw away the rest of your
  selection, and could relocate the clip across a section boundary.
- **The right rail reopened by itself.** Collapsing it with **K** and then
  resizing the plugin window put it straight back, taking height off the
  velocity lane.
- **A second drag attempt was killed as "stopped making progress".** An
  abandoned render kept reporting into the next one's progress bar, so the real
  render's honest figures looked like no progress at all and the page cancelled
  it ten seconds later — every time.
- **A long render is refused everywhere, not just in one place.** Rendering past
  fifteen minutes of audio is refused with a message; before, only the
  arrangement drag said so and every other route quietly truncated.

### Verified

- **⛔ The macOS drag source is now checked by a compiler, from Windows.** It is
  the one file no local build compiles, and it had **four errors and two
  lint failures** — every one of which CI's macOS runner would have found one
  push at a time. `cargo check --target aarch64-apple-darwin` type-checks Rust
  for macOS without an Apple toolchain, because only *linking* needs one.
  `docs/runbooks/macos-typecheck.md` is how. ⚠ It still proves nothing about
  behaviour: no code here has spoken to a window server.
- **The macOS and Linux drags no longer freeze the host.** Both started a drag
  and then blocked the very event loop that had to run it — ten minutes of
  frozen DAW and no file dropped. Windows was never affected; its drag is
  genuinely modal, which is what made the mistake easy to make twice.
- **The plugin passes both automated plugin validators, for the first time.**
  `pluginval` at strictness level 5 (the maximum) against the VST3, and
  `clap-validator` against the CLAP — **33 passed, 10 skipped, 1 failed**. The
  skips are things this plugin genuinely does not implement (preset discovery,
  64-bit audio, automatable parameters); the one failure is the validator's own
  divide-by-zero, which it reports as "a bug in the validator".
- **Linux is verified locally**, in Docker: the full Rust suite passes and the UI
  suite is 155 of 156, the one failure being the first navigation against a
  cold Vite server rather than anything in the app.
- **Every feature is driven, asserted and photographed.** `npm run test:gallery`
  writes `screenshots/gallery/` with an image per screen, per language, and per
  feature — plus `FEATURES.md`, which lists what was proved and, just as
  importantly, the seven things a browser structurally cannot reach.
### Fixed

- **The KIT panel no longer lies (TASK-136).** It rendered eight hardcoded
  disabled buttons and a static "No kit yet" while a twelve-pad kit was loaded
  and audibly playing. It now draws a row per lane, read from the plugin: what
  plays it, whether that is your sample or the shipped one, and which lanes have
  no sound at all.
- **Every artist wrote the same snare roll.** The fill was built from a
  hardcoded `Roll::new(..).ramp(64, 120)` that read neither the model nor the
  seed, so it was a pure function of the fill's length. Measured across the
  roster: **six of the ten flagship trap artists produced a byte-identical
  roll**, and every model reached only one to four distinct rolls in forty
  seeds. Each artist now authors its own subdivisions, ramp range, jitter,
  descent and gap probability, and the generator samples inside them. Every
  model now clears twenty-five distinct fills, and no two artists collide on
  more than one seed in two hundred.
- **UK drill, NY drill and Pop Smoke wrote exactly one kick pattern, ever.** An
  explicit `fourBarGrammar` returned `grammar[bar % len]` and never touched the
  seed. A model may now author `grammarVariants` — several complete multi-bar
  forms, one chosen per pattern — so the signature still reproduces exactly and
  there is more than one of it. ⚠ Every variant stays inside the tresillo the
  research describes; `drills_kick_form_is_the_tresillo_it_is_described_as`
  sweeps two hundred seeds and holds that line.
- **An 808 could ring straight through a fill.** The mute list held only the
  backbeat, so a kick landing on the beat a fill starts on let the 808 sustain
  across the whole roll — the thing drill is defined against. It now stops at the
  first snare it reaches, fills included, while still only *skipping* the
  backbeat, so a roll does not shred the line. ⚠ The length clamp also used
  `find` over the lane's insertion order rather than `min` over time, so it could
  clamp to a later snare and leave an earlier one rung through.
- **rage and osamason wrote four distinct chord parts in two hundred seeds.**
  Both authored a single `harmonicRhythm` value and four mostly-one-chord
  families — which is not harmonically static, it is four. Widened within the
  style: rage now reaches 79, osamason 131.
- **Two sibling models were too close to tell apart.** `pop-smoke` extends
  `ny-drill` and differed from it by 0.05 on four numbers, producing an identical
  beat on 16 seeds of 200. Pulled apart on kick density, hat density and the open
  hat, along with `osamason`/`rage` and `metro-boomin`/`travis-scott`. Beat
  collisions across the whole roster are now **zero**.


- **An exported song no longer arrives as one instrument playing everything at
  once.** Every pitched part was written on MIDI channel 0 — melody,
  countermelody, bass and chords together — and while the file did carry one
  *track* per part, a great many hosts (FL Studio among them) split an imported
  SMF by **channel** rather than by track. Producers got four parts stacked on a
  single instrument, with each part's note-offs cutting the others' held notes on
  the same key. Each part now writes on its own channel, drums stay on channel 10
  where General MIDI puts percussion, and the 808 takes a pitched channel of its
  own rather than riding the drum channel — where its slides would have been read
  as unrelated drum voices.
  - ⚠ **The golden `.mid` snapshots moved and their JSON did not.** The note
    content is byte-for-byte what it was; only the channel nibble changed, which
    is why every file kept its exact length. Nothing about what the engine
    *generates* changed here — only where the notes are addressed.

- **Harmony no longer saturates.** Chord voicings were chosen by strict minimum
  cost, which made a voicing a pure function of the chord — so a model's entire
  reachable harmony was its progression families times its extension rolls, and
  `rage` produced **8 distinct progressions in 1,000 seeds** while its melody
  produced 823. Voicings are now sampled among the candidates within two
  semitones of the best, which keeps the top voice stepwise while letting the
  inversion and octave move. Every model that authors a real chord part now
  clears the 500-harmony floor; most roughly doubled.

### Added


- **Each generator keeps its own clip.** The five shared one slot, so generating a
  bassline destroyed the melody that was there. Every part now has its own, through
  the undo stack, the project file and the arm path — and a project saved before the
  change still opens with its hand-edited clip intact.
- **Generate all** fills every part from **one seed**, which is what makes the five
  a record rather than five loops in the same key: the engine guarantees they agree
  only when they share one. A part the style does not have — a bassline in a
  trap-family model, where the 808 *is* the bass — is skipped rather than failing
  the run.
- **Clear**, per generator and for all of them, undoable.
- **The melodic generators make a sound.** The preview kit carried percussion only,
  so melody, countermelody, bassline and chords rendered silence — which presents as
  a broken generator rather than a silent one. `kitgen` now synthesizes four pitched
  voices: a Karplus–Strong string, an FM bell, a filter-enveloped bass and an FM
  electric piano, each rooted at the centre of its part's authored register.
- **Clicking a selected note collapses the selection to it on mouse-up**, the way a
  DAW does — decided on whether the pointer moved, not on a delta the clamp may have
  pinned to zero.
- **The dataset protocol** (`docs/dataset-protocol.md`) and a generated roster
  ledger, plus five trap-family genre archetypes: dark trap, bouncy trap, trap soul,
  cloud rap and emo rap.

- **Song Mode: pick an artist, press Generate, and get a whole arrangement.**
  The engine samples one of the artist's own song forms, gives each section its
  bar count, and builds a clip per part per section — then the arrangement view
  draws it and lets you edit it before it goes anywhere.
  - **The Arrangement Creator** (TASK-065): weighted structure sampling, section
    part masks, per-section density, and one seed per section so a melody is
    written against the chords playing beside it rather than against a different
    voicing of them. Sections of the same kind share a clip, because verse 1 and
    verse 2 are the same beat.
  - **Transitions** (TASK-066): the drop-out beats before a hook, the back-half
    switch-up that varies the melody while the drums hold, and the outro's fade.
    Each is a property of *where a section sits* rather than of its notes,
    because the clip underneath it loops — and each is written into the exported
    file, not merely drawn.
  - **Genre song forms** (TASK-064): pop's verse–pre-chorus–chorus form, plugg's
    chords-only intro and kick-and-bass bridge, west-coast club's instant hook,
    phonk's 16-bar chorus, country's fills on the eight, drill's 16-bar verses,
    and liquid's mute-the-turnaround. Taken from the research and only from the
    research — the two genres it says nothing structural about keep the shared
    default rather than a form nobody wrote down. `SectionKind` gained a
    pre-chorus, because pop's form cannot be spelled honestly without one.
  - **One multi-track MIDI file for the whole song**: a conductor track carrying
    the tempo map and a marker per section, then a track per part named so you
    can see which rows came from here. A section's clip is tiled across its
    length, so a sixteen-bar verse over a four-bar loop exports as four bars
    played four times rather than one bar and three of silence.
  - **The arrangement view** (TASK-063A/B): a ruler with bar numbers and
    timestamps, gridlines that get finer as you zoom rather than staying fixed,
    and clips you can select, resize from either edge, clone, delete and
    copy/cut/paste on the ordinary shortcuts. Deleting a part out of a section
    leaves the section standing.
  - **A song plays** (TASK-072), which is what finally made the rest of this
    list possible. The arrangement is flattened into one clip and handed to the
    transport, so the marker in the arrangement view is a position through the
    *record* rather than through whichever four bars happened to be armed —
    with click-to-seek, a loop toggle on any section, and per-part mute and solo
    for auditioning. The tiling that lays a song out in time now lives in one
    place that both the player and the exporter read, so **what you hear and
    what you export come out of the same arithmetic**.
  - **Re-roll one section** (TASK-067) without touching the rest of the song,
    keeping any clips you have locked. A re-rolled section gets its own clips
    rather than rewriting the one its twin is also playing, so re-rolling verse
    2 leaves verse 1 alone. A locked Chords or Drums part is handed back to the
    generator as the harmony and kit to write against, so the new melody sits on
    the chords the section actually plays.
  - **An edited arrangement is saved with the project** (TASK-067). It was not,
    while a single edited clip was — so arranging a whole song and reopening
    lost all of it. An *unedited* arrangement is still stored as nothing but its
    inputs, because pressing Generate reproduces it exactly.
  - **Ctrl+Z works on the Song tab.** It used to do nothing, deliberately,
    because the arrangement had no undo stack and stepping the session back
    instead was worse. The arrangement is part of the same snapshot now: one
    stack, one shortcut, and no question about which document a keypress is
    about.
  - **The timeline reads at a glance** (TASK-070): a note-density sketch inside
    every clip, locks on any cell, row or section, a chip row naming the form
    the song has, and a picker offering the forms *the artist writes* — never a
    shape nobody researched. Forcing a form moves no notes: the same seed in
    another shape keeps the same beats.
  - **Audition, re-roll and drill-in** (TASK-071): hear one cell on its own,
    press `R` to re-roll the section you are in, and double-click a clip to open
    it in its own editor — where your edits write back into the arrangement.
  - **Export** (TASK-073/TASK-069): write the whole arranged song as one
    multi-track `.mid`, or one `.mid` per part into a folder, through your
    platform's own Save As. The generation animation cascades across the
    sections as they are built, with the same reduced-motion path everything
    else uses.
  - ⚠ **Stems are MIDI, not audio.** The preview kit is a drum kit, so the four
    melodic parts have no voice to render through yet — writing four silent
    `.wav`s and calling them stems would be worse than not offering them. The
    audio half arrives with the pitched instrument voices.
  - ⚠ **Dragging a song out to the desktop is not built.** An HTML5 drag inside
    a plugin's webview is not an operating-system file drag; that needs a native
    drag source per platform, which is its own piece of work. Export does the
    same job in the meantime and the button says "Export" rather than "Drag".

### Fixed

Song Mode went through four reviews before it shipped, and they found things
worth naming — every one with a test that was watched failing first.

- **Undo now reaches the project file.** Undoing an arrangement edit changed
  the screen and not the saved project, so closing and reopening brought the
  edit back.
- **Undo across an artist change no longer resurrects the previous artist's
  record** under the new artist's name — and a preset load no longer leaves a
  redo step that can bring back the arrangement it just cleared.
- **Locks, the loop and an audition follow their section when you clone.**
  Cloning renumbers everything after the insert, so a padlock drew on the wrong
  section and a re-roll regenerated the very clips it said were pinned.
- **Re-rolling honours the record it belongs to.** A re-rolled section keeps
  the mode the song was generated in, keeps a pinned swing, and keeps a
  section the style authors as 808-only from coming back with a full kit.
- **Re-rolling a section leaves every other one alone**, including a clone of
  it, and no longer strands a cut clip or an open editor.
- **The transport plays what the visible tab shows.** Leaving Song Mode hands
  it back the clip on screen, an undo taken on a part tab does not put the
  whole record on it, and muting or soloing a part no longer restarts the song
  from bar 1.
- **Clicking the empty grid works** — it clears the selection and moves the
  playhead. It had never worked: the guard could not pass.
- **Generating a fresh song clears what belonged to the old one** — the
  clipboard, a solo on a part the new form does not play, and an audition.
- **An export dialog left open for a long time still reports where the file
  went**, instead of the page giving up and refusing the next export.
- **Every melodic stem carries its own track name**, rather than the drum
  track's, and the multi-track file and the stems agree note for note.

- **A piano roll, and the four melodic parts are visible at last.** Melody,
  Countermelody, Bass and Chords generated through the bridge and landed on the
  host's track without ever appearing on screen; now they are drawn, and
  editable, to the standard Ableton and FL set. Canvas rather than DOM because a
  roll is 128 rows deep and the same approach the drum grid uses would be ~15,000
  elements before a single note — with the notes published as a visually-hidden
  list beside it, so the editor is reachable by a screen reader and assertable by
  a test rather than only pokeable by coordinate.
  - **Selection and note editing** (TASK-041A): marquee, `Shift`/`Ctrl`-click,
    `Ctrl+A`, `Esc`, drag either edge to resize, `Del`, `Shift+D` and `Ctrl+D` to
    duplicate, cut/copy/paste at the playhead, arrow-key transpose and nudge with
    `Shift` widening both to an octave and a bar, and `Ctrl`-drag to copy. Every
    gesture commits once, so a drag across forty notes is one undo step.
  - **Scale awareness** (TASK-041B): in-key rows tinted, out-of-key dimmed, the
    root marked, `Fold to scale` and Ableton's note `Fold`, and a scale picker in
    the header that writes through to the session chip rather than holding a
    second opinion about the key. A row holding a note is never folded away, so a
    chromatic passing tone stays visible, audible and exported.
  - **The full scale set** (TASK-041C): `Scale` goes from 12 to 41 — the modes,
    the pentatonics and blues, the minor and major variants and their modes, the
    symmetric scales, nine world scales and Messiaen modes 3–7 — each with a
    Dark/Neutral/Bright character, and the picker offers the *model's* own scales
    narrowed by the mood's character rather than all forty-one.
  - **A velocity lane** (TASK-041V), under the roll and the drum grid: one stem
    per note with a round cap, drag a cap to set it, drag sideways to paint every
    slider at the pointer's height, `Shift` for a straight ramp, a selection
    drags relatively so its accents survive, and right-click or double-click puts
    a note back to the velocity the *model* wrote — which is now kept on the note
    rather than recomputed, because `humanize` has already spread it by then.
  - **Transforms on a selection** (TASK-041D): invert, reverse, stretch and
    compress (with `*` and `/`, and handles on the selection's outer edges),
    legato, quantize with a strength slider, humanize, and transpose to scale.
    One undo step each.
  - **Clip timing** (TASK-041E): a bar/beat ruler drawn from the clip's own
    meter, a loop brace with draggable ends, clip start/end markers independent
    of it, and a per-clip time signature that writes through to the chips *and*
    to the exported file's meta event. The transport honours the loop region,
    rendering each block in segments split at the turnover rather than wrapping
    at the block boundary — which would put every note after the wrap up to a
    32nd note out of place at 140 BPM.
  - **A visual design pass** (TASK-041F): velocity-mapped note fill, a hover
    affordance that appears under the pointer instead of a grip on every note,
    grid and playhead snapped to whole device pixels so they stay crisp at any
    `devicePixelRatio`, and a note outline chosen per theme for contrast — the
    fixed one landed at 2.6:1 on the light theme's note colour, where "this note
    is selected" quietly stopped being visible.
  - **An edited clip is saved with the project.** The plugin stores the
    *request* — artist, seed, pins — because the engine is deterministic, which
    is what keeps a project file a few hundred bytes. The moment a producer moves
    a note that stops being true, so from then on the clip itself is stored. An
    unedited session still carries no notes at all.
- **The plugin makes a sound.** The sampler and preview kit are ported out of
  `src-tauri` and into the plugin, and the generated pattern is rendered into the
  host's output — so a producer hears the beat on insert instead of having to
  wire an instrument up first. The kit is compiled into the binary (a plugin has
  no resource directory it can trust) and rendered in segments split at each
  note-on, so a hit lands on its exact sample rather than quantised to the block
  boundary — up to 11 ms at 512 samples, which is a 32nd note at 140 BPM. It
  sounds only while the DAW's transport is running, so pressing Generate with
  the project stopped no longer plays the pattern at you unasked. ⚠ **The
  preview kit is a drum kit**, so Melody, Countermelody, Bass and Chords still
  render silence — pitched instrument voices are FMM-N15/N16, and playing a
  melody through the kick pad would be worse than hearing nothing.
- **MIDI-only is one click away, and per-lane audio mute keeps the notes
  flowing.** The plugin can be silenced entirely so the DAW's own instruments
  play the notes — which is what it did before it had a sampler, and what a
  producer routing into Battery needs. Muting a single lane's *audio* leaves its
  MIDI going to the host, so the plugin can play the hats while your own sampler
  takes the snare. Both save with the project, and both step back with undo.
- **A playhead that runs with the tempo, and a timeline you can click.** The
  marker moves with the BPM because it is derived from the same clock that
  decides which notes have been emitted — the two cannot drift apart. Click
  anywhere on the grid to play from there; **Pause holds the marker where it is
  and Stop returns it to the beginning**, keeping the pattern armed rather than
  discarding it — including across a DAW transport stop, which used to throw the
  generated pattern away and leave the next play silent until Generate was
  pressed again. ⚠ Drums is the only part with an editor today, so that is where
  it is drawn; the transport underneath is what the piano roll and the
  arrangement view will use.
- **All five generators are reachable in the plugin.** Melody, countermelody and
  bassline were in the engine the whole time; `plugin/src/bridge.rs` refused
  them with "not implemented yet", so the refusal was in the bridge rather than
  in what it reported on. They generate in their real dependency order — a
  melody against the harmony and around the drums, a countermelody answering the
  melody, a bassline following the harmony and locking to the kick — which is
  what makes five parts cohere instead of being one part played five times.
  ⚠ Only the Drums tab has an editor, so the other four are reachable through
  the bridge and not yet through the UI; the piano roll is what surfaces them.
- **Asking Trap for a bassline now says why it has none.** Its 808 *is* the
  bassline, so a separate bass lane would double it — that used to report as
  "has no Bass part authored", which reads as a hole in the style rather than a
  deliberate one.
- **Genre and artist cross-filtering in the roster.** Picking a genre shows only
  the artists who work in it; picking an artist shows only the genres they work
  in, and the genre chips become that artist's own. Both directions come from a
  new curated `relatedGenres` field on each artist — the relation did not exist
  in the dataset before and could not be inferred from it: `extends` carries
  exactly one parent per artist, and the `genres` field is free-text tags in a
  vocabulary of its own where `rap` sits on almost every model. Each artist's
  list is sourced from its own `notes`, and every id is checked to name a real
  genre, so `datasetc validate` fails on a typo rather than shipping a filter
  that silently matches nobody. A narrowed list says what narrowed it and offers
  Show all; a genre nobody works in says so instead of rendering empty.

- **The per-lane audio mute has a control now.** Every lane header in the drum
  grid carries one, and the row dims rather than emptying: the notes still go to
  the host's track, so the pattern has not changed — only what the plugin plays
  of it. The label says "preview" for that reason; "Mute kick" would promise
  something this does not do. The Rust half had shipped complete with nothing
  able to set it.
- **Play, Pause and Stop work in the standalone.** nih-plug's audio backend reports
  its transport as permanently running, so a generated pattern looped from the
  moment it landed, Stop rewound to the start and kept playing, there was no
  pause at all, and the transport bar told you to "press play in your DAW" when
  there was no DAW. The standalone now owns its own transport, stopped until you
  press Play; inside a host nothing changes, because there the DAW's transport is
  the only one and a second Play button of ours could not move it. **Pause holds
  the marker and Stop returns it to the beginning**, so Stop stays reachable
  *from* a pause — which is the whole difference between the two.

### Changed

- **The desktop application's crate is gone.** `src-tauri/` — the Tauri shell,
  its tray, its updater, its crash reporter, its settings file and its audio
  path — is removed from the tree. The product was retired on 2026-07-29; the
  crate outlived it for two specific reasons, and both have now expired: it
  generated `ipc-audio-types.ts`, which the frontend imported, and it held the
  only sampler this project had. The sampler was ported into the plugin first,
  which is what makes this safe rather than lossy.

  What went with it: the updater and its prompt, the crash reporter, the system
  tray and its three settings, the native title bar and window controls, the
  export and drag-out chips, and the desktop screenshot CI job. Theme, language
  and reduce-motion now persist to the WebView's own storage — `settings.json`
  was their durable half and it went with the shell.

  Two things this fixed on the way past, both of which had been broken in the
  shipping plugin rather than merely dead: **Settings and About were
  unreachable**, because the title bar was their only entry point and it never
  rendered in a plugin — they now live in the transport bar. And the **About
  pane showed an em dash for the version and platform**, because it asked only
  when running under Tauri, while the plugin's bridge had been answering
  `app_info` all along.
- **The plugin makes no outbound connections at all**, and the README, the EULA
  and the documentation site now say so instead of describing the update check
  and the crash reporter that used to exist. This is not a wording change: the
  supply-chain gate's allowlist held `reqwest`, `hyper`, `hyper_rustls` and
  `hyper_util`, every one of them justified by `tauri-plugin-updater`, and it is
  now **empty** — 187 linked crates with no HTTP client among them. Six RUSTSEC
  advisory exemptions stopped matching anything and were deleted too; an ignore
  that matches nothing is a standing permission for an advisory to come back
  unnoticed.

### Fixed

- **A machine without WebView2 no longer takes the DAW down with it.** Opening
  the editor on a machine with no WebView2 Evergreen runtime — or with a
  read-only temp directory, or with the plugin's browser profile already held by
  another WebView2 environment, which is what the standalone opened alongside a
  DAW does — panicked inside the host's own editor-open callback. Release builds
  abort on panic, so the host could not catch it and the session went with it.
  The editor now opens blank and says why on stderr. Two related panics went the
  same way: the custom-protocol handler, which ran from a frame a panic cannot
  even unwind out of, now answers 500, and `send_json` no longer treats a page
  that is being torn down as fatal. Full write-up in
  `plugin/vendor/VENDORED.md`.

## [0.4.0] - 2026-07-29

### Added — unlimited undo/redo, and the licence gate

- **Unlimited undo/redo** (FMM-U01). Every session change — artist, seed, bars,
  pins, auto-sync and each generation — steps back with `Ctrl`/`Cmd`+`Z` and
  forward with `Ctrl`/`Cmd`+`Shift`+`Z` or `Ctrl`+`Y`. No depth limit: an entry
  is a handful of scalars plus a *shared* `Pattern` reference, because a pattern
  is derived from its seed rather than stored, so a hundred steps across one
  generation cost one pattern. A run of edits to one control inside 600 ms
  collapses into a single step; two generations never merge, however fast the
  reroll. Recorded by a store subscription rather than per-action calls, so a
  future action cannot forget to register — the same argument the session save
  already made. Armed *after* the project restore, so `Ctrl`+`Z` cannot step
  behind the session the host handed back onto an empty plugin.
- **First-run licence gate.** The agreement is compiled in from
  `EULA.md` and shown before anything else; nothing generates, plays, exports or
  saves until it is accepted. **Agree stays disabled until the text has been
  scrolled to the end**, because "you cannot use it until you read it" is the
  requirement and a live button asks nobody to read anything. ⛔ Enforced at the
  plugin's RPC boundary, not in the UI — a page that was reloaded, bypassed or
  driven from devtools still cannot generate — and by an *allowlist*, so a new
  command has to be added deliberately to be reachable before acceptance.
  **Decline leaves the plugin inert rather than closing the DAW**, which a plugin
  must never do; the agreement can be reopened and accepted at any time, and
  everything works immediately after. Acceptance is stored per user, beside the
  presets — never in the project file, which would ask a collaborator to
  re-accept because they opened your song.
- **The standalone carries the app icon on Windows**, with product, company and
  copyright strings, via a new `plugin/build.rs`. A missing resource compiler
  warns rather than failing the build.
- **A documentation site** under `docs/`: what the
  plugin does, how it follows the host, seeds, presets, shortcuts, privacy and
  building from source, with search and a short changelog.

### Fixed

- **The Windows standalone opened a blank window** (TASK-P16). `baseview`'s
  `open_blocking` pumps with `GetMessageW(&mut msg, hwnd, …)` — a non-NULL
  `hwnd`, which retrieves messages only for that window and its children and
  **never retrieves thread messages at all**. WebView2 is COM/STA and delivers
  its completions as exactly those, through a COM-owned message-only window, so
  the custom-protocol handler was never dispatched, navigation never completed
  and the page stayed on `about:blank`. The vendored adapter now drains the queue
  from `on_frame`, **off unless the process opts in** — `plugin/src/bin/standalone.rs`
  is the only caller and a DAW never runs it, so a host's queue is never touched.
  ⛔ It skips messages belonging to the editor window and its children:
  dispatching those re-enters baseview's window procedure while it already holds
  a `RefCell` borrow, which panics inside an `extern "system"` frame and aborts
  the process. Verified by photographing the window, not by a green build.

### Changed

- **No downloads before `v1.0.0`.** The `v0.1.0` and `v0.2.0` desktop releases
  were withdrawn from GitHub (converted to drafts; assets intact), and the
  documentation site offers no download link. Building from source is the only
  supported way to run it until 1.0.

## [0.3.0] - 2026-07-29

### Added — melodic generation, and moods that multiply it

- **Melody generator** (TASK-035, FR-005). Phrase structures (riff loop,
  question/answer, call/response, long arc), chord-tone bias on strong beats,
  colour tones, interval and contour distributions, octave jumps, end variation,
  and the per-genre devices: rage's two-to-three-note staccato motif, drill's
  snare-mirrored onsets and doubled voicing, straight-eighth bars and deliberate
  silence. Pitches are chosen as **scale degrees** and only then made into MIDI
  notes, so staying in the key is structural rather than filtered for.
- **Countermelody generator** (TASK-036, FR-006). Octave echo, bell echo,
  arpeggio, answer lick and sustained pad, placed in the melody's gaps by
  construction rather than by filtering afterwards.
- **Moods** (TASK-040V, engine half). A model may author named `modes` — trap
  ships dark, bounce, melodic and minimal — each a partial override merged into
  the model *before* generation, so every generator honours a mood without
  knowing moods exist. Moods inherit through `extends`, so an artist offers only
  the moods its own lineage does.
- **Presets the plugin owns** (TASK-P13, session half). Six factory presets
  compiled into the binary, user presets in the platform's per-user data
  directory, and a panel in the right rail. No factory preset pins a tempo — that
  would override your DAW on load.
- **The Linux editor** (TASK-P12). The plugin's window now opens on Linux over
  X11 and WebKitGTK, verified by photographing it under Xvfb rather than by a
  green build.

### Fixed

- **A repeated melody no longer clashes with the chords it repeats over.** A riff
  now follows the progression, keeping its own contour and rhythm.
- **The countermelody is no longer silent about half the time.** An octave echo
  is delayed, because an octave copy at zero delay is a doubling.
- **Sustained pads voice more than the chord root**, and pick their octave.
- **`echoOffset: "1/8"` is read.** The note-value parser knew `"8th"` and `"16T"`
  and silently ignored the third spelling the dataset uses.
- **The seed box shows a whole seed.** It was 12 characters wide against a
  20-digit `u64`, so a long seed was cut off — and a seed you cannot read is one
  you cannot type back in.

### Changed — Freally MIDI Master is becoming a plugin

Decided 2026-07-28. Not for the format's sake: a plugin is handed the host's
tempo, time signature and playhead, so a generated pattern lands in the song you
are actually writing rather than at whatever tempo the artist is authored at.
`docs/product-roadmap.md` carries the decision, what survived and what did not.

- **New `plugin/` crate** on `nih-plug`, exporting **CLAP**. VST3 and AU are
  projected from it by `clap-wrapper` at packaging time.
- **The `engine` crate is unchanged**, which was the point of keeping it free of
  shell types. No FFI and no C++.
- **Host tempo sync**, with precedence **user pin > host > model**. Trap
  authored at 140 generates at 92 inside a 92 BPM project; a pinned tempo beats
  the host; a host that has not reported yet leaves the model its own value.
- **Notes are emitted onto the host's track**, replacing drag-out.
- **The session is saved with the project.** Artist, seed, pins, bars and the
  window size go through the host's own state calls — there is no settings file
  and no path to find, and a session belongs to a *song* rather than to a
  machine. The notes are not saved; the inputs that make them are, because the
  engine is deterministic and a project file should not carry regenerable notes.
- **The window has three scales** (small, medium, large) on a button in the
  transport bar. The layout stays 1440x900 at all of them — the window is drawn
  smaller, rather than shown less of — so the kit and session panels never
  disappear just because the window shrank.
- **Verified in Ableton Live 12 (VST3) and FL Studio (CLAP).** FL is the first
  host to load the `.clap` itself rather than the projection.
- **Releases now carry the plugin**, as a per-platform zip holding both the
  `.clap` and the `.vst3`, validated by `clap-validator` before the draft is
  published and refused by `verify-downloads.yml` if either format is missing.

### Removed — the desktop app is retired

Freally MIDI Master ships as a **plugin** now. Releases from here on carry the
CLAP and the VST3 and nothing else; the Windows, macOS and Linux installers are
no longer built.

**If you are on v0.2.0:** nothing you have installed stops working, and nothing
is uninstalled. Your copy will simply stop finding updates, because there will
not be any — the update channel goes quiet rather than breaking. To carry on,
install the plugin from this release and load it in your DAW, which is where
the tempo sync, the host key and the notes-on-the-track live. That is the whole
reason for the move: the desktop app could not know what song you were writing.

### Known issue

- **A corrupt project file can abort the host.** `nih-plug`'s CLAP state loader
  reads a length prefix straight into an allocation with no sanity check, so
  malformed state aborts the process rather than failing to load. It is upstream's
  bug, the maintained fork carries it identically, and it needs a patched fork to
  fix. `clap-validator`'s `state-invalid-random` is excluded by name until then.
- **The UI carried across.** `src/lib/ipc.ts` was always the one seam and gained
  a third branch; the React app, the 18 locale catalogs and the design tokens
  are the same ones the desktop app shipped.

### Added

- **Session chips** — BPM, key, scale and swing, editable in the right rail.
  Empty means the artist decides; a value means you do. When running in a host
  the tempo chip follows the DAW and says so.
- **The chords generator** (FR-004): progression families, diatonic
  third-stacking so every pitch is in the key by construction, borrowed chords,
  sus and the drill middle-note drop, close and open voicings, and syncopated
  3–5 beat cells.
- **`scripts/assert-plugin-bundled.mjs`** — refuses a plugin binary whose UI or
  dataset failed to embed, because that failure otherwise presents as a blank
  window with no error.
- **`npm run plugin:standalone`** — the plugin in its own window, no DAW needed.
  **`npm run plugin:install`** symlinks it into the CLAP folder so a rebuild is
  live without copying.

### Fixed

- **`vst3-sys` is GPLv3** and nih-plug's VST3 export links it — which would have
  put this proprietary product in breach. Caught by `cargo deny`. VST3 now comes
  from `clap-wrapper` (MIT) instead. Steinberg's own VST3 SDK went MIT in
  November 2025; nih-plug does not use it.
- **The generation error message has never been visible.** `.stage__error` had
  no CSS rule and sat behind the FX layer, which is `position: absolute;
  inset: 0` over the whole stage.
- **A pinned tempo is clamped** to Ableton Live's 20–999 at the IPC edge, and
  the BPM box accepts digits only — `<input type="number">` accepts `e`, `E`,
  `+` and `-`, so "1e5" was a legal tempo.
- `.github/workflows/{ci,release}.yml` were failing `format:check` on `main`.

## [0.2.0] - 2026-07-25

Phase 1: the app makes beats. Search an artist, press Generate, hear it, and
drag it into a DAW.

### Added

- The style dataset is bundled with the app and loaded at startup: every model
  is parsed, inheritance-resolved and validated before the first frame, and the
  roster is served to the UI by the new `roster_summary` and `resolve_model`
  commands. An invalid model is skipped and reported rather than taken as a
  reason to refuse to start.
- The humanizer: MPC swing (50% straight to 66% triplet), velocity tiers for
  accents, main hits and ghost notes, per-lane timing jitter in milliseconds,
  and a quantize strength that decides how much of that jitter survives. Swing
  warps the whole timeline, so rolls written at finer resolutions travel with
  the beat they belong to.
- The drum generator core: the kick grammar (anchors, density, syncopation,
  tresillo lean, the gap before the snare, explicit multi-bar forms) and snare
  placement — half-time on 3, the 2-and-4 backbeat, drill's two-bar 3-then-4,
  and the country train beat — with ghost snares and a layered clap. Trap comes
  out with its snare on beat 3; UK drill's authored two-bar kick form
  reproduces exactly on every seed.
- The hat engine: base subdivision (8ths, 16ths or a tresillo grouping), fill
  density, open hats that close the hat underneath them, a pitch-bent second
  layer and the swell across a loop. Beats and offbeat 8ths carry the accent;
  the 16ths between them fill in quietly.
- The roll vocabulary: subdivision-switch hat rolls (16th through 64th,
  including the triplet grids), rolls placed at phrase ends, before the snare
  and before the downbeat, velocity ramps in both directions, bursts, gaps and
  offset clusters — plus snare-roll ladders with build-and-stop and dual-layer
  variants, the 8-bar riser and the stutter cluster.
- The 808 lane: it rides the kick at the share the model locks them to, sustains
  legato from one note to the next, takes its root from the session key, and
  slides by the intervals the model lists — written as the overlapping notes a
  sampler reads as portamento. UK drill's 808 stops under the snare; trap's
  rings through it.
- Fills at phrase boundaries: a small variation every two bars, a bigger one
  every eight, and a fill on the last bar so a loop leads somewhere instead of
  stopping dead. Fills take the end of their bar and leave the backbeat — and
  the ghost notes — intact.
- Twelve more genre archetypes: Chicago and NY drill, plugg and pluggnb, jerk,
  phonk, west-coast club, boom bap, 2000s R&B, liquid drum & bass, the country
  train beat and 2000s pop — fifteen in all, each with a test asserting the
  grammar that makes it that genre. Models can now say their 808 is staccato
  rather than legato, that they have no 808 at all, and that their fills turn
  over on the clap.

- Golden determinism snapshots: a fixed seed, model and session now produce
  byte-identical pattern JSON and MIDI, pinned by committed snapshots. This is
  what makes the seed chip's promise — paste a seed, get the same beat — a
  guarantee rather than an intention.
- Generation and export are reachable from the app: `generate_pattern` runs a
  style model through the drum generator and the humanizer and hands back the
  pattern, and `export_midi` writes it into the session directory as a type-0
  MIDI file. A request may pin the tempo, key, scale, swing, bar count or
  half-time feel; anything it leaves alone is the model's own choice, and
  omitting the seed picks a fresh one that reproduces the result exactly.
- Ten flagship artist models over the genre bases — Metro Boomin, Southside,
  Pierre Bourne, OsamaSon, Nettspend, Summrs, Pop Smoke, Travis Scott, Future
  and Drake — each with aliases to search by, and a test that every one of them
  generates something its parent genre does not.
- **The app makes beats.** Search an artist, press Generate, and the pattern
  appears in the drum grid; press Play and you hear it; drag it into a DAW or
  export it to a folder. Search is fuzzy and forgiving — "osa" finds OsamaSon,
  "drizzy" finds Drake, and a typo still lands — with the keyboard alone:
  ↑↓ to move, Enter to take, Esc to close.
- Playback: a real-time audio engine with a synthesized preview kit, sample-
  accurate sequencing, looping, a playhead that follows along, and a limiter so
  a dense pattern never cracks the output. A machine with no sound card still
  generates and exports, and says why playback is unavailable rather than
  failing silently on click.
- The seed is shown after every generation and can be pasted back to get the
  same beat again.
- A generation ripple sweeps the grid while a pattern is built, so a beat
  arrives rather than blinking into place, igniting brightest where the notes
  actually land. Turning on the system's reduced-motion setting replaces it with
  a short crossfade — immediately, without a restart.
- Settings now says when a style model was skipped at startup, with the file
  and the reason. A skipped model is a missing artist, and until now only the
  console ever mentioned it.
- Unplugging an audio interface mid-session no longer leaves the app silently
  deaf. It says the device is gone, reopens one by itself as soon as there is
  one to open — retrying for as long as it takes — and says when it is back.
  Playback does not restart on its own; pressing play works again, which was
  the thing that stopped working.
- A **Reduce motion** setting under Appearance, for machines whose system
  offers no such preference, or anyone who wants everything else animated and
  this one thing still. It takes effect the moment it is ticked.

### Changed

- Exported MIDI now carries a key signature. It was the one session field the
  file did not describe, so a clip landed in a project without saying what key
  its 808 was in. A mode is written as its parallel major or minor, which is as
  much as the format can say. **The golden `.mid` snapshots were regenerated for
  this**: six bytes per file, the new meta event and nothing else — the pattern
  JSON is untouched.

- An 808 slide may now reach an octave above the note it starts from rather
  than being folded back inside the model's register. An octave glide — the
  phonk signature — previously landed on its own root and was discarded.
- Inheritance resolution no longer copies the whole accumulated model at each
  step of a chain, which brought a 1,000-model load from 330 ms to 219 ms —
  inside the 300 ms startup budget.

## [0.1.0] - 2026-07-22

First tagged build: the Phase 0 foundation. The Studio shell, the pure
generation engine, the style-model dataset and the full CI spine are in place;
the generators themselves arrive in Phase 1, so the transport and Generate are
deliberately disabled rather than pretending to work.

### Added

- Tauri v2 + React + TypeScript shell on a Cargo workspace, with the pure
  `engine` crate (no Tauri types, no network, no `unsafe`).
- Studio layout: left rail, six generator tabs, grid stage, right rail and
  transport, with every panel independently collapsible and the state persisted.
- Dark and light themes, contrast-verified against WCAG 2.1 AA in both.
- Engine core: `Pattern`/`Note`/`Lane`/`Song`, `SessionContext`, and seeded
  ChaCha8 RNG with per-domain stream derivation so rerolling one part leaves
  every other part byte-identical.
- Style dataset: JSON Schema, inheritance deep-merge with cycle detection,
  semantic lints, and the first three genre archetypes — trap, uk-drill, rage.
- `datasetc` CLI — validate, lint, stats, coverage.
- Crash reporter per the Havoc standard: opt-in, scrubbed, never transmitted
  without a click.
- Three-OS CI, supply-chain gates, and the AI/network dependency denylist.
- Playwright E2E against `vite dev` with IPC mocked at a single seam.
- Borderless window with its own minimise / maximise / close controls, a centred
  title, and drag-to-resize on all eight edges.
- Settings and About, reachable from the title bar, with a system-tray option
  (minimise-to-tray and close-to-tray, both off by default).
- Bug reporter and the Havoc-standard updater.
- **Eighteen languages** — English plus Arabic, Chinese (Simplified), Dutch,
  French, German, Hindi, Indonesian, Italian, Japanese, Korean, Polish,
  Portuguese (Brazil), Russian, Spanish, Turkish, Ukrainian and Vietnamese.
  Switching is instant, persists, and Arabic mirrors the whole layout.
- **Noto throughout**, bundled: 546 faces covering CJK, Arabic, Hebrew, the
  Indic scripts, Thai, Khmer, Georgian, Armenian, Ethiopic and more, so no
  language falls back to whatever the machine happens to have. Nothing is
  fetched at runtime — the app still makes no network request except the
  update check.
- CI captures the running app on all three OSes, and the Settings modal in every
  language, as downloadable artifacts. The macOS capture is partial and the job
  says so (see Live-To-Do).

### Known limitations

- The generators, playback and audio export are not implemented yet; their
  controls are disabled rather than inert.
- Native drag-out is built but **unverified against real DAWs** — that is the
  Phase 0 decision gate and it needs a human.
- The tray menu (Show / Quit) is not translated.
- Installers are unsigned: expect SmartScreen on Windows and Gatekeeper on
  macOS. See the release notes for the per-platform steps.

[Unreleased]: https://github.com/MikesRuthless12/freally-midi-master/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/MikesRuthless12/freally-midi-master/releases/tag/v0.1.0
