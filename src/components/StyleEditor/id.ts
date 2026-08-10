/**
 * The id a style's name becomes (TASK-040U).
 *
 * ⛔ **The same rule `plugin/src/models.rs::is_safe_stem` enforces, on purpose.**
 * The id *is* the filename there — which is what stops two names slugging to one
 * file, the failure the pattern library shipped and had to fix. Deriving it here
 * by the same rule means the editor cannot offer a name the store will then
 * refuse: a producer types "My Dark Trap", gets `my-dark-trap`, and the refusal
 * they never see is the one that was never needed.
 *
 * ⚠ ASCII only, and that is not an oversight: a name with no ASCII letters at
 * all slugs to nothing and the store refuses it by name, which is a message. The
 * fallback-to-a-hash trick `presets.rs` uses is right for a preset, whose id
 * nobody types — and wrong here, where the id is what `extends` refers to and
 * what a producer reads in an exported file.
 */
export function idFor(name: string): string {
  const out: string[] = [];
  for (const ch of name) {
    if (/[a-zA-Z0-9]/.test(ch)) out.push(ch.toLowerCase());
    else if (out.length > 0 && out[out.length - 1] !== '-') out.push('-');
  }
  // ⚠ Only the trailing trim, and only after the slice: the loop above appends
  // `-` solely when something precedes it, so a leading one is impossible — and
  // the slice is what can leave a dangling one behind.
  return out.join('').slice(0, 64).replace(/-+$/, '');
}
