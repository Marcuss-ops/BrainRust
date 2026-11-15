use serde::Serialize;
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct LocalScoreCandidate<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub tags: &'a [String],
    pub source: &'a str,
    pub duration_ms: i64,
}

#[derive(Debug, Clone)]
pub struct LocalScoreQuery<'a> {
    pub text: &'a str,
    pub tags: &'a [String],
    pub source: &'a str,
    pub min_duration_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LocalScoreResult {
    pub id: String,
    pub score: f64,
}

/// Scores local catalog signals in one batch, avoiding per-row map allocation
/// in the Go search path.
pub fn score_local(
    query: &LocalScoreQuery<'_>,
    candidates: &[LocalScoreCandidate<'_>],
) -> Vec<LocalScoreResult> {
    candidates
        .iter()
        .map(|candidate| {
            let all_blank = candidate.title.is_empty()
                && candidate.tags.is_empty()
                && candidate.source.is_empty();
            let score = if all_blank {
                0.50
            } else {
                0.40 * title_match_score(candidate.title, query.text)
                    + 0.25 * tag_overlap_score(candidate.tags, query.tags)
                    + 0.10 * exact_match_score(candidate.source, query.source)
                    + 0.10 * duration_fit_score(candidate.duration_ms, query.min_duration_ms)
            };
            LocalScoreResult {
                id: candidate.id.to_string(),
                score,
            }
        })
        .collect()
}

fn title_match_score(title: &str, query: &str) -> f64 {
    if title.is_empty() || query.is_empty() {
        return 0.0;
    }
    let title = title.trim().to_lowercase();
    let query = query.trim().to_lowercase();
    if title == query {
        return 1.0;
    }
    if query.contains(&title) || title.contains(&query) {
        return 0.6;
    }
    let tokens: HashSet<&str> = title.split_whitespace().collect();
    if tokens.is_empty() {
        return 0.0;
    }
    let hits = tokens
        .iter()
        .filter(|token| token.len() >= 3 && query.contains(**token))
        .count();
    if hits as f64 / tokens.len() as f64 >= 0.60 {
        0.4
    } else {
        0.0
    }
}

fn tag_overlap_score(candidate_tags: &[String], query_tags: &[String]) -> f64 {
    let candidate: HashSet<String> = candidate_tags
        .iter()
        .map(|tag| tag.trim().to_lowercase())
        .filter(|tag| !tag.is_empty())
        .collect();
    let query: HashSet<String> = query_tags
        .iter()
        .map(|tag| tag.trim().to_lowercase())
        .filter(|tag| !tag.is_empty())
        .collect();
    if candidate.is_empty() || query.is_empty() {
        return 0.0;
    }
    let intersection = candidate.intersection(&query).count();
    let union = candidate.union(&query).count();
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

fn exact_match_score(a: &str, b: &str) -> f64 {
    if !a.is_empty() && !b.is_empty() && a.trim().eq_ignore_ascii_case(b.trim()) {
        1.0
    } else {
        0.0
    }
}

fn duration_fit_score(duration_ms: i64, min_duration_ms: i64) -> f64 {
    if min_duration_ms <= 0 || duration_ms >= min_duration_ms {
        1.0
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weighted_signals_match_the_retired_go_formula() {
        let query_tags = vec!["fruit".to_string(), "red".to_string()];
        let candidate_tags = query_tags.clone();
        let query = LocalScoreQuery {
            text: "Red Apple",
            tags: &query_tags,
            source: "artlist",
            min_duration_ms: 1_000,
        };
        let candidates = [LocalScoreCandidate {
            id: "apple",
            title: "red apple",
            tags: &candidate_tags,
            source: "artlist",
            duration_ms: 1_000,
        }];

        let scores = score_local(&query, &candidates);
        // Go's retired LocalScore used 40% title + 25% tag Jaccard +
        // 10% source + 10% duration for the fields this Postgres row exposes.
        assert_eq!(scores[0].score, 0.85);
    }

    #[test]
    fn empty_catalog_signals_keep_the_unranked_floor() {
        let query = LocalScoreQuery {
            text: "red apple",
            tags: &[],
            source: "",
            min_duration_ms: 0,
        };
        let candidates = [LocalScoreCandidate {
            id: "blank",
            title: "",
            tags: &[],
            source: "",
            duration_ms: 1_000,
        }];

        assert_eq!(score_local(&query, &candidates)[0].score, 0.50);
    }

    #[test]
    fn a_duration_shortfall_removes_only_the_duration_signal() {
        let query = LocalScoreQuery {
            text: "red apple",
            tags: &[],
            source: "",
            min_duration_ms: 1_000,
        };
        let candidates = [LocalScoreCandidate {
            id: "short",
            title: "red apple",
            tags: &[],
            source: "",
            duration_ms: 999,
        }];

        assert_eq!(score_local(&query, &candidates)[0].score, 0.40);
    }
}
