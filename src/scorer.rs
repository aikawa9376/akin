use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::SystemTime;

use strsim::{jaro_winkler, levenshtein};

use crate::content::{content_similarity, link_bonus};
use crate::tokenizer::{
    NOISE_TOKENS, basename_without_extensions, domain_tokens, extension_tokens, normalize,
    primary_stem,
};

/// Project-wide filename frequency statistics used to downweight generic names.
pub struct ProjectStats {
    total_files: usize,
    basename_freq: HashMap<String, usize>,
}

impl ProjectStats {
    pub fn from_paths<'a>(paths: impl IntoIterator<Item = &'a Path>) -> Self {
        let mut total_files = 0usize;
        let mut basename_freq = HashMap::new();

        for path in paths {
            total_files += 1;
            let basename = basename_without_extensions(path);
            if !basename.is_empty() {
                *basename_freq.entry(basename).or_insert(0) += 1;
            }
        }

        Self {
            total_files,
            basename_freq,
        }
    }

    /// IDF-like specificity score in [0, 1].
    /// Frequently repeated basenames such as "index" or "style" are pushed down.
    pub fn filename_specificity(&self, path: &Path) -> f64 {
        let basename = basename_without_extensions(path);
        if basename.is_empty() || self.total_files <= 1 {
            return 1.0;
        }

        let freq = self.basename_freq.get(&basename).copied().unwrap_or(1) as f64;
        if freq <= 2.0 {
            return 1.0;
        }

        let total = self.total_files as f64;
        let adjusted_freq = freq - 1.0;
        let idf = (1.0 + total / adjusted_freq).ln() / (1.0 + total).ln();
        idf.powf(2.0).clamp(0.0, 1.0)
    }
}

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

pub fn exact_basename_match(target: &Path, candidate: &Path) -> f64 {
    if basename_without_extensions(target) == basename_without_extensions(candidate) {
        1.0
    } else {
        0.0
    }
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

/// Exact sibling-directory match.
pub fn same_directory(target: &Path, candidate: &Path) -> f64 {
    if target.parent() == candidate.parent() {
        1.0
    } else {
        0.0
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
    stats: &ProjectStats,
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

    let target_specificity = stats.filename_specificity(target);
    let candidate_specificity = stats.filename_specificity(candidate);
    let domain_signal_scale = 0.25 + target_specificity * 0.75;
    let name_signal_scale = 0.15 + target_specificity * 0.85;
    let candidate_name_scale = 0.35 + candidate_specificity * 0.65;

    // When the target filename is generic (style/index/etc. in this project),
    // path context should dominate over raw filename equality.
    let context_score = domain_sim * 0.26 * domain_signal_scale
        + jaccard * 0.18
        + dir_prox * 0.15
        + jw * 0.03
        + lev_sim * 0.02;
    let filename_score = (stem_sim * 0.22 + filename_sim * 0.12 + extension_sim * 0.02)
        * name_signal_scale
        * candidate_name_scale;
    let exact_name_bonus =
        exact_basename_match(target, candidate) * target_specificity * candidate_specificity * 0.60;
    let sibling_bonus = same_directory(target, candidate) * (1.0 - target_specificity) * 0.30;
    let base = context_score + filename_score + exact_name_bonus + sibling_bonus;

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

    use super::{
        ProjectStats, extension_similarity, filename_similarity, same_directory, similarity_score,
    };

    fn stats(paths: &[&Path]) -> ProjectStats {
        ProjectStats::from_paths(paths.iter().copied())
    }

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
        let stats = stats(&[target, same_name_diff_ext, different_name_same_ext]);

        let same_name_score = similarity_score(
            target,
            same_name_diff_ext,
            target,
            same_name_diff_ext,
            &stats,
        );
        let different_name_score = similarity_score(
            target,
            different_name_same_ext,
            target,
            different_name_same_ext,
            &stats,
        );

        assert!(same_name_score > different_name_score);
    }

    #[test]
    fn repeated_basenames_get_lower_specificity() {
        let style_a = Path::new("src/styles/style.css");
        let style_b = Path::new("src/components/style.css");
        let style_c = Path::new("src/pages/style.css");
        let index_a = Path::new("src/pages/home/index.tsx");
        let index_b = Path::new("src/pages/search/index.tsx");
        let index_c = Path::new("src/pages/admin/index.tsx");
        let unique = Path::new("src/controllers/UserController.ts");
        let stats = stats(&[style_a, style_b, style_c, index_a, index_b, index_c, unique]);

        assert!(stats.filename_specificity(style_a) < stats.filename_specificity(unique));
        assert!(stats.filename_specificity(index_a) < stats.filename_specificity(unique));
    }

    #[test]
    fn common_filenames_are_penalized_more_than_rare_ones() {
        let target = Path::new("src/pages/home/index.tsx");
        let candidate = Path::new("src/pages/search/index.tsx");

        let common_stats = stats(&[
            target,
            candidate,
            Path::new("src/pages/admin/index.tsx"),
            Path::new("src/pages/help/index.tsx"),
            Path::new("src/styles/style.css"),
        ]);
        let rare_stats = stats(&[
            target,
            candidate,
            Path::new("src/controllers/UserController.ts"),
            Path::new("src/models/User.rs"),
            Path::new("src/views/dashboard.html"),
        ]);

        let common_score = similarity_score(target, candidate, target, candidate, &common_stats);
        let rare_score = similarity_score(target, candidate, target, candidate, &rare_stats);

        assert!(common_score < rare_score);
    }

    #[test]
    fn generic_target_prefers_siblings_over_same_named_files_elsewhere() {
        let target = Path::new("src/components/button/style.css");
        let sibling = Path::new("src/components/button/Button.tsx");
        let other_style = Path::new("src/components/card/style.css");
        let stats = stats(&[
            target,
            sibling,
            other_style,
            Path::new("src/layouts/main/style.css"),
            Path::new("src/pages/home/style.css"),
            Path::new("src/components/card/Card.tsx"),
        ]);

        let sibling_score = similarity_score(target, sibling, target, sibling, &stats);
        let other_style_score = similarity_score(target, other_style, target, other_style, &stats);

        assert_eq!(same_directory(target, sibling), 1.0);
        assert!(sibling_score > other_style_score);
    }
}
