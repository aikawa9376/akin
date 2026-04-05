use std::collections::HashSet;
use std::path::Path;
use std::time::SystemTime;

use strsim::{jaro_winkler, levenshtein};

use crate::content::{content_similarity, link_bonus};
use crate::tokenizer::{domain_tokens, normalize, primary_stem, NOISE_TOKENS};

/// Compute token weight (lower for noise tokens)
pub fn token_weight(token: &str) -> f64 {
    if NOISE_TOKENS.contains(&token) { 0.2 } else { 1.0 }
}

/// Jaccard similarity on token sets, with noise weighting
pub fn jaccard_weighted(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let mut intersection_weight = 0.0;
    let mut union_weight = 0.0;

    let all_tokens: HashSet<&String> = a.iter().chain(b.iter()).collect();

    for token in &all_tokens {
        let w = token_weight(token);
        let in_a = a.contains(token);
        let in_b = b.contains(token);
        if in_a && in_b {
            intersection_weight += w;
        }
        union_weight += w;
    }
    if union_weight == 0.0 { 0.0 } else { intersection_weight / union_weight }
}

/// Score based on the primary stem (first segment before any dot).
/// This correctly equates "UserService.ts" with "UserService.spec.ts".
pub fn stem_similarity(target: &Path, candidate: &Path) -> f64 {
    let t = primary_stem(target);
    let c = primary_stem(candidate);
    if t.is_empty() || c.is_empty() {
        return 0.0;
    }
    let jw = jaro_winkler(&t, &c);
    let boost = if c.contains(t.as_str()) || t.contains(c.as_str()) { 0.15 } else { 0.0 };
    (jw + boost).min(1.0)
}

/// Fraction of shared directory components (proximity in tree).
/// e.g. target="src/controllers/user.ts", candidate="src/controllers/user.spec.ts" → 1.0
///      target="src/controllers/user.ts", candidate="src/views/user.html"           → 0.5
pub fn dir_proximity(target: &Path, candidate: &Path) -> f64 {
    let t_dirs: Vec<_> = target.parent()
        .map(|p| p.components().map(|c| c.as_os_str().to_owned()).collect())
        .unwrap_or_default();
    let c_dirs: Vec<_> = candidate.parent()
        .map(|p| p.components().map(|c| c.as_os_str().to_owned()).collect())
        .unwrap_or_default();

    let common = t_dirs.iter().zip(c_dirs.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let max_depth = t_dirs.len().max(c_dirs.len());
    if max_depth == 0 { 1.0 } else { common as f64 / max_depth as f64 }
}

/// Recency score: 1.0 for just-modified, decaying exponentially.
/// Half-life ≈ 48 hours (score ~0.5 at 2 days, ~0.1 at ~5 days).
pub fn recency_score(path: &Path) -> f64 {
    let Ok(meta) = std::fs::metadata(path) else { return 0.0 };
    let Ok(modified) = meta.modified() else { return 0.0 };
    let Ok(elapsed) = SystemTime::now().duration_since(modified) else { return 0.0 };
    let hours = elapsed.as_secs_f64() / 3600.0;
    (-hours / 69.3).exp() // ln(2)/48h ≈ 1/69.3 → half-life 48h
}

/// Semantic domain similarity: compares the functional identifiers of two files.
pub fn domain_similarity(target: &Path, candidate: &Path) -> f64 {
    let t_domain = domain_tokens(target);
    let c_domain = domain_tokens(candidate);
    if t_domain.is_empty() && c_domain.is_empty() {
        return 1.0;
    }
    if t_domain.is_empty() || c_domain.is_empty() {
        return 0.0;
    }
    jaccard_weighted(&t_domain, &c_domain)
}

/// Overall similarity score combining multiple signals.
///
/// Arguments:
/// - `target` / `candidate`: project-relative paths (used for path-based signals)
/// - `target_full` / `candidate_full`: absolute paths (used for file I/O signals)
pub fn similarity_score(
    target: &Path,
    candidate: &Path,
    target_full: &Path,
    candidate_full: &Path,
) -> f64 {
    // 1. Semantic domain similarity (index/index.phtml → domain "index"; IndexController → domain "index")
    let domain_sim = domain_similarity(target, candidate);

    // 2. Primary stem similarity (handles compound extensions like .spec.ts)
    let stem_sim = stem_similarity(target, candidate);

    // 3. Jaccard on full path token sets (cross-directory same-domain detection)
    let t_tokens = normalize(target);
    let c_tokens = normalize(candidate);
    let jaccard = jaccard_weighted(&t_tokens, &c_tokens);

    // 4. Directory proximity
    let dir_prox = dir_proximity(target, candidate);

    // 5. Jaro-Winkler on full normalized path string
    let t_str = t_tokens.join(" ");
    let c_str = c_tokens.join(" ");
    let jw = jaro_winkler(&t_str, &c_str);

    // 6. Levenshtein-based similarity on full path string
    let max_len = t_str.len().max(c_str.len());
    let lev_sim = if max_len == 0 {
        1.0
    } else {
        1.0 - levenshtein(&t_str, &c_str) as f64 / max_len as f64
    };

    // Weighted base score
    // domain_sim leads: distinguishes "index action" from "search action" even across directory trees
    let base = domain_sim * 0.25
        + stem_sim  * 0.25
        + jaccard   * 0.20
        + dir_prox  * 0.15
        + jw        * 0.10
        + lev_sim   * 0.05;

    // 7. Recency bonus (additive, max +0.1)
    let recency = recency_score(candidate_full) * 0.1;

    // 8. Content similarity bonus for high-scoring candidates (base >= 0.9)
    let content_bonus = if base >= 0.9 {
        content_similarity(target_full, candidate_full) * 0.1
    } else {
        0.0
    };

    // 9. Link bonus: explicit internal URI references in target file (max +0.2)
    let link = link_bonus(target_full, candidate) * 0.2;

    base + recency + content_bonus + link
}
