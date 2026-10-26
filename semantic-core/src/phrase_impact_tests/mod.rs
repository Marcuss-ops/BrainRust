//! Unit tests for `phrase_impact`, grouped by topic (compiled only under
//! `cfg(test)`). Each submodule sees the production items through
//! `super::super::*` and the shared helpers through `super::common::*`.

mod bullets;
mod chapters_behavior;
mod chapters_core;
mod common;
mod corpus_metrics;
mod rank;
mod scenes_titles;
mod splitting;
mod summary;
