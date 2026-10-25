//! UDPipe model resolution.

use super::pos;

/// Resolves the UDPipe model file for a language tag. Exposed for the
/// profile/model acceptance test, so a certified language without an
/// installed model fails loudly instead of degrading silently.
pub fn model_path(language: &str) -> Option<std::path::PathBuf> {
    pos::model_path(language)
}
