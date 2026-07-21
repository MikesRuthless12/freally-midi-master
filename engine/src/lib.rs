//! Freally MIDI Master generation engine.
//!
//! Everything musical lives here: the style dataset, the five part generators,
//! the humanizer, the arrangement creator and the SMF writer. The crate is a
//! plain Rust library that speaks in serde data types only.
//!
//! Purity rules this crate must keep (PRD § 2):
//!
//! - no Tauri types, so `cargo test -p engine` runs headless and a future
//!   nih-plug VST can wrap the same code;
//! - no network and no AI/ML dependencies, ever;
//! - no `unsafe` (forbidden crate-wide in `Cargo.toml`);
//! - all randomness comes from an explicitly seeded RNG, never system entropy,
//!   so generation is reproducible from a seed.
