//! Internal split of phrase_impact (pure move, no behavior change).

use super::*;

pub(super) fn cosine(a: &[f64], b: &[f64]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| x * y)
        .sum::<f64>()
        .clamp(0.0, 1.0)
}

pub(super) fn cosine_matrix(vectors: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let n = vectors.len();
    let mut matrix = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in i + 1..n {
            let value = cosine(&vectors[i], &vectors[j]);
            matrix[i][j] = value;
            matrix[j][i] = value;
        }
    }
    matrix
}

pub(super) fn top_k_graph(similarities: &[Vec<f64>]) -> Vec<Vec<Neighbor>> {
    let n = similarities.len();
    let mut graph = vec![Vec::new(); n];
    for i in 0..n {
        let mut candidates: Vec<_> = similarities[i]
            .iter()
            .copied()
            .enumerate()
            .filter(|(j, w)| *j != i && *w > 0.0)
            .collect();
        candidates.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        for (j, weight) in candidates.into_iter().take(GRAPH_K) {
            if !graph[i].iter().any(|edge: &Neighbor| edge.index == j) {
                graph[i].push(Neighbor { index: j, weight });
            }
            if !graph[j].iter().any(|edge: &Neighbor| edge.index == i) {
                graph[j].push(Neighbor { index: i, weight });
            }
        }
    }
    for row in &mut graph {
        row.sort_by_key(|edge| edge.index);
    }
    graph
}

pub(super) fn page_rank(graph: &[Vec<Neighbor>]) -> Vec<f64> {
    let n = graph.len();
    if n == 0 {
        return Vec::new();
    }
    let mut rank = vec![1.0 / n as f64; n];
    let degree: Vec<f64> = graph
        .iter()
        .map(|row| row.iter().map(|edge| edge.weight).sum())
        .collect();
    for _ in 0..50 {
        let mut next = vec![(1.0 - DAMPING) / n as f64; n];
        let mut dangling = 0.0;
        for i in 0..n {
            if degree[i] == 0.0 {
                dangling += rank[i];
                continue;
            }
            for edge in &graph[i] {
                next[edge.index] += DAMPING * rank[i] * edge.weight / degree[i];
            }
        }
        let share = DAMPING * dangling / n as f64;
        for value in &mut next {
            *value += share;
        }
        let delta: f64 = next.iter().zip(&rank).map(|(a, b)| (a - b).abs()).sum();
        rank = next;
        if delta < 1e-8 {
            break;
        }
    }
    rank
}

pub(super) fn percentile_scale(values: &[f64]) -> Vec<f64> {
    if values.is_empty() {
        return Vec::new();
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let low = sorted[((sorted.len() - 1) as f64 * 0.10) as usize];
    let high = sorted[((sorted.len() - 1) as f64 * 0.90).ceil() as usize];
    if high - low < 1e-9 {
        return vec![0.0; values.len()];
    }
    values
        .iter()
        .map(|value| ((value - low) / (high - low)).clamp(0.0, 1.0))
        .collect()
}


/// Creates a deterministic sparse lexical vector for offline ranking.
/// Hashing bounds memory use regardless of transcript vocabulary size.
/// The stop-word set is the caller's per-language input: the crate owns none.
pub(super) fn lexical_vector(text: &str, stopwords: &HashSet<String>) -> Vec<f64> {
    const DIMENSIONS: usize = 512;
    let mut vector = vec![0.0; DIMENSIONS];
    for token in text
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| token.chars().count() > 1)
    {
        let lowered = token.to_lowercase();
        if stopwords.contains(&lowered) {
            continue;
        }
        let mut hash = 2_166_136_261_u32;
        for byte in lowered.bytes() {
            hash ^= byte as u32;
            hash = hash.wrapping_mul(16_777_619);
        }
        vector[hash as usize % DIMENSIONS] += 1.0;
    }
    let norm = vector.iter().map(|value| value * value).sum::<f64>().sqrt();
    if norm > f64::EPSILON {
        for value in &mut vector {
            *value /= norm;
        }
    }
    vector
}


