//! Seeded randomness.
//!
//! Every random choice the engine makes comes from here, and every stream is
//! derived from one root seed. System entropy is never touched inside a
//! generator — that is what makes `(seed, styleId, session, version)` reproduce
//! byte-identical output (PRD § 7 Determinism).
//!
//! Streams are derived *per domain* rather than drawn from one shared
//! generator. That is the property rerolling depends on: regenerating the hook's
//! melody must not shift the verse's drums, which it would if every part pulled
//! from a single sequence in call order.

use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

/// SplitMix64 — the standard finalizer used to spread a counter into
/// well-distributed 64-bit values.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// FNV-1a over the domain label, so a stream's identity is its name rather than
/// a positional index nobody can audit.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

/// Derive a stable sub-seed for a named domain.
///
/// The same `(root, domain)` always yields the same value, and different
/// domains yield unrelated values.
pub fn derive_seed(root: u64, domain: &str) -> u64 {
    let mut state = root ^ fnv1a64(domain.as_bytes());
    // Two rounds: one to absorb the mix, one to finalize it.
    splitmix64(&mut state);
    splitmix64(&mut state)
}

/// The generator for a named domain, e.g. `"drums/kick"` or `"section:3/melody"`.
pub fn stream(root: u64, domain: &str) -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(derive_seed(root, domain))
}

/// The root generator. Prefer [`stream`] — anything drawn straight from the root
/// is order-dependent and will shift when unrelated code draws before it.
pub fn root_stream(seed: u64) -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(seed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::Rng;

    fn take(rng: &mut ChaCha8Rng, n: usize) -> Vec<u32> {
        (0..n).map(|_| rng.random::<u32>()).collect()
    }

    #[test]
    fn the_same_seed_reproduces_the_same_sequence() {
        let a = take(&mut root_stream(42), 16);
        let b = take(&mut root_stream(42), 16);
        assert_eq!(a, b);
    }

    #[test]
    fn a_different_seed_produces_a_different_sequence() {
        let a = take(&mut root_stream(42), 16);
        let b = take(&mut root_stream(43), 16);
        assert_ne!(a, b);
    }

    #[test]
    fn derived_domains_are_stable_and_independent() {
        let root = 0xDEAD_BEEF_CAFE_F00D;
        assert_eq!(derive_seed(root, "drums"), derive_seed(root, "drums"));
        assert_ne!(derive_seed(root, "drums"), derive_seed(root, "melody"));
        // One character apart must not correlate.
        assert_ne!(
            derive_seed(root, "section:1"),
            derive_seed(root, "section:2")
        );
    }

    #[test]
    fn a_domain_stream_does_not_move_when_another_is_drawn() {
        let root = 7;
        // Draw melody first in one ordering, and only drums in the other.
        let mut melody = stream(root, "melody");
        let _ = take(&mut melody, 100);
        let drums_after = take(&mut stream(root, "drums"), 8);
        let drums_alone = take(&mut stream(root, "drums"), 8);
        assert_eq!(
            drums_after, drums_alone,
            "a domain stream must not depend on what else was generated"
        );
    }

    #[test]
    fn derivation_survives_a_zero_root() {
        // A zero seed is a legitimate user-pasted value; it must not collapse.
        assert_ne!(derive_seed(0, "drums"), 0);
        assert_ne!(derive_seed(0, "drums"), derive_seed(0, "melody"));
    }
}
