import { describe, expect, it } from 'vitest';

import { idFor } from './id';

/**
 * The id rule, held against the Rust one it mirrors.
 *
 * ⛔ These cases are not arbitrary — each is something `is_safe_stem` in
 * `plugin/src/models.rs` refuses, and the point of deriving the id here is that
 * a producer never meets that refusal. If this drifts, the editor starts
 * offering names the store rejects, and the message they get is about a rule
 * they were never shown.
 */
describe('idFor', () => {
  it('turns a name into something that can be a filename', () => {
    expect(idFor('My Dark Trap')).toBe('my-dark-trap');
    expect(idFor('808 Nights')).toBe('808-nights');
  });

  it('leaves nothing to traverse with', () => {
    // The id arrives from a text box in a webview and ends up joined to a
    // directory inside somebody else's DAW.
    expect(idFor('../../.ssh/authorized_keys')).toBe('ssh-authorized-keys');
    expect(idFor('C:\\Windows\\System32')).toBe('c-windows-system32');
    expect(idFor('a/b')).toBe('a-b');
  });

  it('never ends or begins with a hyphen, however the name is punctuated', () => {
    expect(idFor('  spaced  ')).toBe('spaced');
    expect(idFor('!!!shout!!!')).toBe('shout');
    expect(idFor('trap...')).toBe('trap');
  });

  it('answers empty for a name with no ascii in it at all', () => {
    // Refused by name upstream rather than hashed into something unreadable:
    // this id is what `extends` refers to and what a producer reads in an
    // exported file, so a random hex string would be worse than a message.
    expect(idFor('ダークトラップ')).toBe('');
    expect(idFor('...')).toBe('');
  });

  it('caps the length, so a pasted paragraph cannot become a filename', () => {
    expect(idFor('a'.repeat(200))).toHaveLength(64);
  });
});
