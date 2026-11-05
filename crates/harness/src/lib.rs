//! Test harness (Phase 0.5–0.6 of docs/BUILD_PLAN.md). No AI.
//!
//! - `synth`   synthetic recordings, made by driving the real recorder
//! - `mutate`  injects one known defect into a clean recording
//! - `score`   runs a checker over all fixtures and measures it
//! - `intent_score` measures prompt → intent-fact extraction (Phase 5)
//! - `tsedit`  text helpers used by the mutations

pub mod intent_score;
pub mod mutate;
pub mod score;
pub mod synth;
pub mod tsedit;
