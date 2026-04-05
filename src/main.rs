use clap::Parser;
use ignore::WalkBuilder;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use strsim::{jaro_winkler, levenshtein};

#[derive(Parser)]
#[command(name = "akin", about = "Find files related to a target file by path similarity")]
struct Cli {
    /// Target file path to find related files for
    target: PathBuf,

    /// Number of results to show
    #[arg(short = 'n', long, default_value = "10")]
    top: usize,

    /// Minimum similarity score threshold (0.0 - 1.0)
    #[arg(short, long, default_value = "0.3")]
    threshold: f64,
}

/// Noise tokens that should be weighted down (common directory names)
const NOISE_TOKENS: &[&str] = &[
    "app", "src", "lib", "resources", "assets", "main", "index",
    "mod", "bin", "pkg", "internal", "common", "shared",
];

/// Architectural role words — indicate *type* of file, not its domain
const TYPE_WORDS: &[&str] = &[
    "controller", "model", "view", "service", "repository", "factory",
    "helper", "manager", "handler", "test", "spec", "dto", "dao",
    "middleware", "component", "provider", "resolver", "presenter",
    "interactor", "usecase", "query", "command", "listener", "observer",
];

/// Generic filename stems that carry no domain information on their own.
/// When a file has one of these names, the parent directory is the real domain.
const GENERIC_STEMS: &[&str] = &[
    "index", "show", "create", "edit", "delete", "update", "store",
    "main", "home", "base", "default", "init", "bootstrap",
];

/// Split a path string into tokens by separators and word boundaries (camelCase, snake_case)
fn tokenize(path: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for segment in path.split(['/', '\\', '.', '-']) {
        if segment.is_empty() {
            continue;
        }
        let mut current = String::new();
        let chars: Vec<char> = segment.chars().collect();
        for (i, &ch) in chars.iter().enumerate() {
            if ch == '_' {
                if !current.is_empty() {
                    tokens.push(current.to_lowercase());
                    current = String::new();
                }
            } else if ch.is_uppercase() && i > 0 && chars[i - 1].is_lowercase() {
                if !current.is_empty() {
                    tokens.push(current.to_lowercase());
                    current = String::new();
                }
                current.push(ch);
            } else {
                current.push(ch);
            }
        }
        if !current.is_empty() {
            tokens.push(current.to_lowercase());
        }
    }
    tokens
}

/// Normalize a path: strip all extensions, lowercase, tokenize.
/// e.g. "src/UserController.spec.ts" → ["src", "user", "controller"]
fn normalize(path: &Path) -> Vec<String> {
    // Build path string with all extensions stripped
    let mut p = path.to_path_buf();
    // Strip extensions until none remain (handles .spec.ts, .test.js, etc.)
    while p.extension().is_some() {
        p = p.with_extension("");
    }
    let s = p.to_string_lossy().to_string();
    tokenize(&s)
}

/// Extract the primary stem: the first dot-delimited segment of the filename.
/// e.g. "UserController.spec.ts" → "usercontroller"
///      "user_service.test.js"   → "user_service"
fn primary_stem(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .split('.')
        .next()
        .unwrap_or("")
        .to_lowercase()
}

/// Extract the original-case stem (before lowercasing) for proper camelCase splitting.
/// e.g. "IndexController.php" → "IndexController"
fn primary_stem_original(path: &Path) -> &str {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .split('.')
        .next()
        .unwrap_or("")
}

/// Extract the domain tokens — the functional/business identifier of a file.
///
/// Two strategies:
/// - Generic filename (index, show, create…): the parent directory name is the real domain.
///   e.g. `view/search/index.phtml` → domain = ["search"]
///        `view/index/index.phtml`  → domain = ["index"]
/// - Specific filename (IndexController, UserService…): strip TYPE_WORDS from tokens.
///   e.g. `IndexController.php` → tokens ["index","controller"] → domain = ["index"]
///        `UserService.ts`      → tokens ["user","service"]     → domain = ["user"]
///
/// CamelCase splitting uses the *original-case* stem to correctly split "IndexController".
fn domain_tokens(path: &Path) -> Vec<String> {
    let stem_original = primary_stem_original(path); // "IndexController" (case preserved)
    let stem_lower = stem_original.to_lowercase();

    if GENERIC_STEMS.contains(&stem_lower.as_str()) {
        // Generic filename: domain lives in the parent directory name
        let parent = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("");
        let parent_tokens: Vec<String> = tokenize(parent)
            .into_iter()
            .filter(|t| !TYPE_WORDS.contains(&t.as_str()))
            .collect();
        if !parent_tokens.is_empty() {
            return parent_tokens;
        }
    }

    // Specific filename: tokenize original-case stem (for camelCase), strip type words
    tokenize(stem_original)
        .into_iter()
        .filter(|t| !TYPE_WORDS.contains(&t.as_str()))
        .collect()
}

/// Semantic domain similarity: compares the functional identifiers of two files.
fn domain_similarity(target: &Path, candidate: &Path) -> f64 {
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

/// Compute token weight (lower for noise tokens)
fn token_weight(token: &str) -> f64 {
    if NOISE_TOKENS.contains(&token) { 0.2 } else { 1.0 }
}

/// Jaccard similarity on token sets, with noise weighting
fn jaccard_weighted(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let mut intersection_weight = 0.0;
    let mut union_weight = 0.0;

    let all_tokens: std::collections::HashSet<&String> =
        a.iter().chain(b.iter()).collect();

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
fn stem_similarity(target: &Path, candidate: &Path) -> f64 {
    let t = primary_stem(target);
    let c = primary_stem(candidate);
    if t.is_empty() || c.is_empty() {
        return 0.0;
    }
    let jw = jaro_winkler(&t, &c);
    // Substring containment boost (e.g. "user" ⊂ "userservice")
    let boost = if c.contains(t.as_str()) || t.contains(c.as_str()) { 0.15 } else { 0.0 };
    (jw + boost).min(1.0)
}

/// Fraction of shared directory components (proximity in tree).
/// e.g. target="src/controllers/user.ts", candidate="src/controllers/user.spec.ts" → 1.0
///      target="src/controllers/user.ts", candidate="src/views/user.html"           → 0.5
fn dir_proximity(target: &Path, candidate: &Path) -> f64 {
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
fn recency_score(path: &Path) -> f64 {
    let Ok(meta) = std::fs::metadata(path) else { return 0.0 };
    let Ok(modified) = meta.modified() else { return 0.0 };
    let Ok(elapsed) = SystemTime::now().duration_since(modified) else { return 0.0 };
    let hours = elapsed.as_secs_f64() / 3600.0;
    (-hours / 69.3).exp() // ln(2)/48h ≈ 1/69.3 → half-life 48h
}

/// Read file content and return a set of word tokens (length >= 2)
fn content_tokens(path: &Path) -> std::collections::HashSet<String> {
    let Ok(bytes) = std::fs::read(path) else { return std::collections::HashSet::new() };
    let text = String::from_utf8_lossy(&bytes);
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() >= 2)
        .map(|s| s.to_lowercase())
        .collect()
}

/// Jaccard similarity between file contents using word tokens
fn content_similarity(a: &Path, b: &Path) -> f64 {
    let a_tokens = content_tokens(a);
    let b_tokens = content_tokens(b);
    if a_tokens.is_empty() && b_tokens.is_empty() {
        return 0.0;
    }
    let intersection = a_tokens.intersection(&b_tokens).count();
    let union = a_tokens.union(&b_tokens).count();
    if union == 0 { 0.0 } else { intersection as f64 / union as f64 }
}

/// Overall similarity score combining multiple signals
fn similarity_score(target: &Path, candidate: &Path, target_full: &Path, candidate_full: &Path) -> f64 {
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

    // 7. Recency bonus (additive, max +0.1)
    let recency = recency_score(candidate_full) * 0.1;

    // Weighted base score
    // domain_sim leads: distinguishes "index action" from "search action" even across directory trees
    let base = domain_sim * 0.25
        + stem_sim  * 0.25
        + jaccard   * 0.20
        + dir_prox  * 0.15
        + jw        * 0.10
        + lev_sim   * 0.05;

    // 8. Content similarity bonus for high-scoring candidates (path score >= 0.9)
    let content_bonus = if base >= 0.9 {
        content_similarity(target_full, candidate_full) * 0.1
    } else {
        0.0
    };

    base + recency + content_bonus
}

fn main() {
    let cli = Cli::parse();

    let target = &cli.target;
    if !target.exists() {
        eprintln!("Error: target file '{}' does not exist", target.display());
        std::process::exit(1);
    }

    let target_canonical = target.canonicalize().unwrap_or_else(|_| target.clone());
    let project_root = std::env::current_dir().expect("cannot get current directory");

    let mut results: Vec<(f64, PathBuf)> = WalkBuilder::new(&project_root)
        .hidden(false)
        .git_ignore(true)
        .build()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|ft| ft.is_file()).unwrap_or(false))
        .filter_map(|entry| {
            let path = entry.into_path();
            let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());

            if canonical == target_canonical {
                return None;
            }

            let rel_target = target.strip_prefix(&project_root).unwrap_or(target);
            let rel_candidate = path.strip_prefix(&project_root).unwrap_or(&path);

            let score = similarity_score(rel_target, rel_candidate, target, &path);
            if score >= cli.threshold {
                Some((score, path))
            } else {
                None
            }
        })
        .collect();

    results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(cli.top);

    if results.is_empty() {
        eprintln!("No related files found (threshold: {:.2})", cli.threshold);
    } else {
        for (score, path) in &results {
            let rel = path.strip_prefix(&project_root).unwrap_or(path);
            println!("{:.4}  {}", score, rel.display());
        }
    }
}

