use std::collections::HashSet;
use std::path::Path;

/// Read file content and return a set of word tokens (length >= 2)
pub fn content_tokens(path: &Path) -> HashSet<String> {
    let Ok(bytes) = std::fs::read(path) else {
        return HashSet::new();
    };
    let text = String::from_utf8_lossy(&bytes);
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() >= 2)
        .map(|s| s.to_lowercase())
        .collect()
}

/// Jaccard similarity between file contents using word tokens
pub fn content_similarity(a: &Path, b: &Path) -> f64 {
    let a_tokens = content_tokens(a);
    let b_tokens = content_tokens(b);
    if a_tokens.is_empty() && b_tokens.is_empty() {
        return 0.0;
    }
    let intersection = a_tokens.intersection(&b_tokens).count();
    let union = a_tokens.union(&b_tokens).count();
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}
