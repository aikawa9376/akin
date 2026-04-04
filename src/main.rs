use clap::Parser;
use ignore::WalkBuilder;
use std::path::{Path, PathBuf};
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

/// Split a path into tokens by separators and word boundaries (camelCase, snake_case)
fn tokenize(path: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for segment in path.split(['/', '\\', '.', '-']) {
        if segment.is_empty() {
            continue;
        }
        // Split camelCase and snake_case
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

/// Normalize a path: remove extension, lowercase, tokenize
fn normalize(path: &Path) -> Vec<String> {
    let without_ext = path.with_extension("");
    let s = without_ext.to_string_lossy().to_string();
    tokenize(&s)
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

/// Compute filename-focused similarity (high weight on the file stem)
fn filename_bonus(target_tokens: &[String], candidate_tokens: &[String]) -> f64 {
    let t_last = target_tokens.last().map(|s| s.as_str()).unwrap_or("");
    let c_last = candidate_tokens.last().map(|s| s.as_str()).unwrap_or("");
    if t_last.is_empty() || c_last.is_empty() {
        return 0.0;
    }
    // Use Jaro-Winkler on the stem tokens
    jaro_winkler(t_last, c_last)
}

/// Overall similarity score combining multiple algorithms
fn similarity_score(target: &Path, candidate: &Path) -> f64 {
    let t_tokens = normalize(target);
    let c_tokens = normalize(candidate);

    // 1. Jaccard on token sets (catches cross-directory same-domain files)
    let jaccard = jaccard_weighted(&t_tokens, &c_tokens);

    // 2. Jaro-Winkler on full normalized path string (prefix similarity)
    let t_str = t_tokens.join(" ");
    let c_str = c_tokens.join(" ");
    let jw = jaro_winkler(&t_str, &c_str);

    // 3. Levenshtein-based similarity on full path string
    let max_len = t_str.len().max(c_str.len());
    let lev_sim = if max_len == 0 {
        1.0
    } else {
        1.0 - levenshtein(&t_str, &c_str) as f64 / max_len as f64
    };

    // 4. Filename bonus (strong signal)
    let fname_bonus = filename_bonus(&t_tokens, &c_tokens);

    // Weighted combination
    let score = jaccard * 0.35 + jw * 0.25 + lev_sim * 0.15 + fname_bonus * 0.25;

    // Extra boost if the last token (file stem) is a substring of the other
    let t_last = t_tokens.last().map(|s| s.as_str()).unwrap_or("");
    let c_last = c_tokens.last().map(|s| s.as_str()).unwrap_or("");
    if !t_last.is_empty() && !c_last.is_empty()
        && (c_last.contains(t_last) || t_last.contains(c_last))
    {
        score + 0.15
    } else {
        score
    }
}

fn main() {
    let cli = Cli::parse();

    let target = &cli.target;
    if !target.exists() {
        eprintln!("Error: target file '{}' does not exist", target.display());
        std::process::exit(1);
    }

    // Resolve the target to a canonical path
    let target_canonical = target.canonicalize().unwrap_or_else(|_| target.clone());

    // Determine project root (walk from the current directory)
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

            // Skip the target file itself
            if canonical == target_canonical {
                return None;
            }

            // Compute relative paths for comparison
            let rel_target = target.strip_prefix(&project_root).unwrap_or(target);
            let rel_candidate = path.strip_prefix(&project_root).unwrap_or(&path);

            let score = similarity_score(rel_target, rel_candidate);
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

