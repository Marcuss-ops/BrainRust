//! Label tokenization and Jaccard overlap used by the diversity stage.

/// Lowercase alphanumeric word tokens of a label, in order, deduplicated.
pub(crate) fn label_tokens(label: &str) -> std::collections::BTreeSet<String> {
    label
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().count() > 1)
        .map(|word| word.to_lowercase())
        .collect()
}

pub(crate) trait Jaccard {
    fn jaccard(&self, other: &Self) -> f32;
}

impl Jaccard for std::collections::BTreeSet<String> {
    fn jaccard(&self, other: &Self) -> f32 {
        if self.is_empty() || other.is_empty() {
            return 0.0;
        }
        let intersection = self.intersection(other).count();
        let union = self.union(other).count();
        intersection as f32 / union as f32
    }
}
