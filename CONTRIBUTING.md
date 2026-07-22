# Contributing to Freally MIDI Master

Contributions are welcome — especially **artist style research**, which is the part
of this project that genuinely benefits from people who live in the scene.

Read this first, because the licensing terms here are not the ones you are used to.

## The terms, up front

This project is **source-available, not open source**. The source is public so you
can read it, build it, verify the AI-free and offline claims, and improve it. It is
**not** public so it can be forked, rebranded, redistributed, or resold.

By opening a pull request you confirm that:

1. The contribution is **your own original work** and you have the right to submit it.
2. It contains **no copied MIDI, audio, or data** from any commercial recording,
   sample pack, MIDI pack, loop-channel file, or third-party product — this is the
   one rule that is absolutely non-negotiable, and it is why the product exists.
3. You grant the Owner a perpetual, worldwide, irrevocable, royalty-free,
   sublicensable license to use, modify, and distribute your contribution as part of
   the Software. You keep your own copyright.

You may modify your local copy solely to prepare a contribution. That permission does
not extend to publishing or distributing a modified build. Full terms in
[LICENSE](LICENSE).

## What's most useful

### Style research (the best way to help)

Style models live in `data/` as JSON. A model is **numbers and rules** — tempo ranges,
swing percentages, hat-roll subdivision grammars, 808 slide conventions, velocity
distributions, section templates. Never a transcription.

A good style PR:

- cites its sources — interviews, published analysis, documented statistics, or your
  own listening notes, stated as such;
- explains *why* a parameter has the value it does, in the model's own notes field;
- states a confidence level where the research is thin;
- passes `cargo run -p datasetc -- validate data/`.

**Do not** submit a model derived by transcribing a specific song, or by opening a
ripped MIDI file. That poisons the legal architecture the whole product rests on, and
it will be rejected.

If you know an artist's catalogue well and can describe what makes their drums *theirs*
in terms of measurable parameters, you are exactly the contributor this project wants.

### Code

Bug fixes, performance work, accessibility, and platform-specific fixes (especially
Linux/Wayland drag-out) are all welcome.

Architectural rules that PRs must respect:

- **The `engine` crate stays pure.** No Tauri types, no network, no AI/ML
  dependencies, no `unsafe` — it is `forbid(unsafe_code)` and must stay testable
  headless. All musical logic goes here.
- **No AI, ever.** No ML runtime, model, or inference API gets added to this project
  under any circumstances.
- **Determinism.** All randomness goes through the seeded `ChaCha8Rng`. Never system
  entropy inside a generator: the same seed must always reproduce the same output.
- **Style tokens only.** UI components use `var(--color-*)`; never a hardcoded hex,
  never a theme branch — both dark and light must keep working.

## Before you open a PR

Run the whole CI suite locally first:

```bash
npm run ci:local          # every gate CI runs, in CI's environment
npm run ci:local -- --fast  # skip the slow ones while iterating
```

This runs the same commands **with the same environment variables** CI sets. That
matters more than it sounds: a denylist check once passed locally and failed on CI
purely because CI sets `CARGO_TERM_COLOR=always`, which changed a crate name and
stopped it matching an allowlist entry. Running the same commands under a different
environment is not a rehearsal.

It cannot catch everything — a Linux-only link error still only appears on Linux —
but it catches everything that is merely a difference of shell.

Keep the diff small and focused; don't reformat files you didn't change.

## Reporting bugs

Use the in-app reporter — it assembles a scrubbed diagnostic report and opens a
pre-filled GitHub issue for you. Nothing is ever sent without your click, and you see
the exact text first. Otherwise, open an issue directly.

For anything security-related, email **mythodikalone@gmail.com** rather than filing a
public issue.

## Artists and rights-holders

If you are a named artist, producer, or rights-holder and would prefer your name not
appear in the roster, email **mythodikalone@gmail.com** and it will be removed. No
legal claim or explanation needed — just ask. See
[docs/legal/disclaimer.md](docs/legal/disclaimer.md).
