//! Compatibility facade for editorial text analysis.
//!
//! The implementation lives in `semantic-core` so the semantic worker can
//! consume it without depending on the media executor crate. This module
//! preserves the existing `pipelinegen_muscles::phrase_impact` API for the
//! legacy NDJSON worker and downstream callers during the staged cutover.

use crate::error::MuscleError;

pub use semantic_core::phrase_impact::{
    split_sentence_spans, split_sentences, BulletPoint, Chapter, ChapterBullet, ChapterManifest,
    ChapterOptions, Options, RankedSentence, Request, ResultDocument, SceneBulletSpan,
    SceneHighlight, SceneInput, ScenePhraseSpan, Segment, SemanticBulletSpan, StageTimings,
    SummaryLength, Timing,
};

#[cfg(test)]
pub(crate) use semantic_core::phrase_impact::extract_chart_values;

pub fn run(request: Request) -> Result<ResultDocument, MuscleError> {
    semantic_core::phrase_impact::run(request).map_err(MuscleError::generic)
}

pub fn run_with_scene_inputs(
    request: Request,
    inputs: &[SceneInput],
    options: &ChapterOptions,
) -> Result<ResultDocument, MuscleError> {
    semantic_core::phrase_impact::run_with_scene_inputs(request, inputs, options)
        .map_err(MuscleError::generic)
}

pub fn run_with_scene_context(
    request: Request,
    scenes: &[String],
    topics: &[String],
) -> Result<ResultDocument, MuscleError> {
    semantic_core::phrase_impact::run_with_scene_context(request, scenes, topics)
        .map_err(MuscleError::generic)
}

pub fn run_with_chapter_options(
    request: Request,
    scenes: &[String],
    topics: &[String],
    options: &ChapterOptions,
) -> Result<ResultDocument, MuscleError> {
    semantic_core::phrase_impact::run_with_chapter_options(request, scenes, topics, options)
        .map_err(MuscleError::generic)
}
