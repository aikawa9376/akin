use std::collections::HashSet;
use std::path::Path;
use std::time::SystemTime;

use strsim::{jaro_winkler, levenshtein};

use crate::content::{content_similarity, link_bonus};
use crate::tokenizer::{
    NOISE_TOKENS, basename_without_extensions, domain_tokens, extension_tokens, normalize,
    primary_stem,
};

/// Compute token weight (lower for noise tokens)
pub fn token_weight(token: &str) -> f64 {
    if NOISE_TOKENS.contains(&token) {
        0.2
    } else {
        1.0
    }
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
    if union_weight == 0.0 {
        0.0
    } else {
        intersection_weight / union_weight
    }
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
    let boost = if c.contains(t.as_str()) || t.contains(c.as_str()) {
        0.15
    } else {
        0.0
    };
    (jw + boost).min(1.0)
}

/// Score based on the full filename after stripping all extensions.
/// This favors files whose real names match even when the suffixes differ.
pub fn filename_similarity(target: &Path, candidate: &Path) -> f64 {
    let t = basename_without_extensions(target);
    let c = basename_without_extensions(candidate);
    if t.is_empty() || c.is_empty() {
        return 0.0;
    }
    let jw = jaro_winkler(&t, &c);
    let boost = if c.contains(t.as_str()) || t.contains(c.as_str()) {
        0.10
    } else {
        0.0
    };
    (jw + boost).min(1.0)
}

/// Score based on shared extension-like suffixes such as ".spec.ts" or ".html".
/// Kept intentionally low so extension matches only act as a weak tie-breaker.
pub fn extension_similarity(target: &Path, candidate: &Path) -> f64 {
    let t_ext = extension_tokens(target);
    let c_ext = extension_tokens(candidate);
    if t_ext.is_empty() && c_ext.is_empty() {
        return 1.0;
    }
    if t_ext.is_empty() || c_ext.is_empty() {
        return 0.0;
    }
    jaccard_weighted(&t_ext, &c_ext)
}

/// Fraction of shared directory components (proximity in tree).
/// e.g. target="src/controllers/user.ts", candidate="src/controllers/user.spec.ts" → 1.0
///      target="src/controllers/user.ts", candidate="src/views/user.html"           → 0.5
pub fn dir_proximity(target: &Path, candidate: &Path) -> f64 {
    let t_dirs: Vec<_> = target
        .parent()
        .map(|p| p.components().map(|c| c.as_os_str().to_owned()).collect())
        .unwrap_or_default();
    let c_dirs: Vec<_> = candidate
        .parent()
        .map(|p| p.components().map(|c| c.as_os_str().to_owned()).collect())
        .unwrap_or_default();

    let common = t_dirs
        .iter()
        .zip(c_dirs.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let max_depth = t_dirs.len().max(c_dirs.len());
    if max_depth == 0 {
        1.0
    } else {
        common as f64 / max_depth as f64
    }
}

/// Recency score: 1.0 for just-modified, decaying exponentially.
/// Half-life ≈ 48 hours (score ~0.5 at 2 days, ~0.1 at ~5 days).
pub fn recency_score(path: &Path) -> f64 {
    let Ok(meta) = std::fs::metadata(path) else {
        return 0.0;
    };
    let Ok(modified) = meta.modified() else {
        return 0.0;
    };
    let Ok(elapsed) = SystemTime::now().duration_since(modified) else {
        return 0.0;
    };
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

    // 3. Full filename similarity after stripping all extensions
    let filename_sim = filename_similarity(target, candidate);

    // 4. Extension similarity (.spec.ts, .html, etc.) as a weak tie-breaker
    let extension_sim = extension_similarity(target, candidate);

    // 5. Jaccard on full path token sets (cross-directory same-domain detection)
    let t_tokens = normalize(target);
    let c_tokens = normalize(candidate);
    let jaccard = jaccard_weighted(&t_tokens, &c_tokens);

    // 6. Directory proximity
    let dir_prox = dir_proximity(target, candidate);

    // 7. Jaro-Winkler on full normalized path string
    let t_str = t_tokens.join(" ");
    let c_str = c_tokens.join(" ");
    let jw = jaro_winkler(&t_str, &c_str);

    // 8. Levenshtein-based similarity on full path string
    let max_len = t_str.len().max(c_str.len());
    let lev_sim = if max_len == 0 {
        1.0
    } else {
        1.0 - levenshtein(&t_str, &c_str) as f64 / max_len as f64
    };

    // Weighted base score
    // domain_sim leads: distinguishes "index action" from "search action" even across directory trees
    let base = domain_sim * 0.22
        + stem_sim * 0.26
        + filename_sim * 0.20
        + extension_sim * 0.02
        + jaccard * 0.15
        + dir_prox * 0.10
        + jw * 0.03
        + lev_sim * 0.02;

    // 9. Recency bonus (additive, max +0.1)
    let recency = recency_score(candidate_full) * 0.1;

    // 10. Content similarity bonus for high-scoring candidates (base >= 0.9)
    let content_bonus = if base >= 0.9 {
        content_similarity(target_full, candidate_full) * 0.1
    } else {
        0.0
    };

    // 11. Link bonus: explicit internal URI references in target file (max +0.2)
    let link = link_bonus(target_full, candidate) * 0.2;

    base + recency + content_bonus + link
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{extension_similarity, filename_similarity, similarity_score};

    #[test]
    fn full_filename_match_ignores_extension_suffixes() {
        let target = Path::new("src/controllers/UserController.ts");
        let candidate = Path::new("tests/UserController.spec.ts");

        assert_eq!(filename_similarity(target, candidate), 1.0);
    }

    #[test]
    fn extension_similarity_is_only_partial_for_compound_suffixes() {
        let target = Path::new("src/controllers/UserController.ts");
        let candidate = Path::new("tests/UserController.spec.ts");

        assert!(extension_similarity(target, candidate) < 1.0);
    }

    #[test]
    fn same_filename_beats_same_extension() {
        let target = Path::new("src/controllers/UserController.ts");
        let same_name_diff_ext = Path::new("tests/UserController.rs");
        let different_name_same_ext = Path::new("src/controllers/PostController.ts");

        let same_name_score =
            similarity_score(target, same_name_diff_ext, target, same_name_diff_ext);
        let different_name_score = similarity_score(
            target,
            different_name_same_ext,
            target,
            different_name_same_ext,
        );

        assert!(same_name_score > different_name_score);
    }
}
