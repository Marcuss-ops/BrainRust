//! Shared extractor fixtures for the extractor unit-test topics. Compiled
//! only under `cfg(test)`, like every module of this folder.

use super::super::*;
use crate::{ExtractOptions, VisualEntity};

pub(super) fn top3(text: &str) -> Vec<VisualEntity> {
    extract(
        text,
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 3,
        },
    )
    .unwrap()
}

pub(super) fn texts(entities: &[VisualEntity]) -> Vec<String> {
    entities.iter().map(|e| e.text.clone()).collect()
}
