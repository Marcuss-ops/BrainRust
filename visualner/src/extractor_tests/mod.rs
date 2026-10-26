//! Unit tests for the extractor, grouped by topic. Compiled only
//! under `cfg(test)`. Each topic file sees the production items
//! through `super::super::*` and the shared fixtures through
//! `super::common::*`.

mod common;
mod objects_values;
mod orgs_concepts;
mod persons_names;
mod places_locations;
mod scoring_guards;
mod tokenizer_pos;
