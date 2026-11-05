//! Capture layer for `tattle` (Phase 0.2 / 1 of docs/BUILD_PLAN.md).
//!
//! Records what a Claude Code session did, without any AI:
//! - `hook`     records each hook event (cleaned) and takes code snapshots
//! - `export`   copies a recorded session into the test corpus
//! - `restore`  rebuilds an app folder from a recording (for corpus bases)
//! - `sandbox`  prepares a fresh folder for recording a test session
//! - `housekeeping` deletes old sessions (plugin mode)
//!
//! Supporting modules: `redact` (secret detection), `text` (redaction,
//! placeholders, trimming), `shadow` (snapshot repo), `store` (file layout).

pub mod export;
pub mod fsx;
pub mod hook;
pub mod housekeeping;
pub mod redact;
pub mod restore;
pub mod sandbox;
pub mod settings;
pub mod shadow;
pub mod store;
pub mod text;
