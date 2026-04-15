mod content;
mod feature;
mod history;
mod scorer;
mod tokenizer;

use clap::Parser;
use ignore::WalkBuilder;
use std::path::PathBuf;

use history::CochangeIndex;
use scorer::{ProjectStats, ScoreBreakdown, similarity_breakdown};

#[derive(Parser)]
#[command(
    name = "akin",
    about = "Find files related to a target file by path similarity"
)]
struct Cli {
    /// Target file path to find related files for
    target: PathBuf,

    /// Number of results to show
    #[arg(short = 'n', long, default_value = "10")]
    top: usize,

    /// Minimum similarity score threshold (0.0 - 1.0)
    #[arg(short, long, default_value = "0.3")]
    threshold: f64,

    /// Show why each candidate was ranked highly
    #[arg(long, default_value_t = false)]
    explain: bool,

    /// Number of git commits to scan for co-change ranking
    #[arg(long, default_value_t = 400)]
    history_limit: usize,
}

fn explain_labels(breakdown: &ScoreBreakdown) -> Vec<String> {
    let mut labels = Vec::new();

    if breakdown.feature_bonus >= 0.20 {
        labels.push(format!("direct-ref +{:.2}", breakdown.feature_bonus));
    }
    if breakdown.cochange_bonus >= 0.08 {
        labels.push(format!(
            "co-change +{:.2} ({} commits)",
            breakdown.cochange_bonus, breakdown.cochange_commits
        ));
    }
    if breakdown.same_directory && breakdown.target_specificity < 0.8 {
        labels.push("same-dir".to_string());
    }
    if breakdown.exact_name_match && breakdown.target_specificity >= 0.8 {
        labels.push("exact-name".to_string());
    }
    if breakdown.stem_similarity >= 0.95 && !breakdown.exact_name_match {
        labels.push(format!("stem {:.2}", breakdown.stem_similarity));
    }
    if breakdown.domain_similarity >= 0.85 {
        labels.push(format!("domain {:.2}", breakdown.domain_similarity));
    }
    if breakdown.filename_similarity >= 0.9 {
        labels.push(format!("filename {:.2}", breakdown.filename_similarity));
    }
    if breakdown.content_bonus >= 0.05 {
        labels.push(format!("content +{:.2}", breakdown.content_bonus));
    }
    if breakdown.recency_bonus >= 0.05 {
        labels.push(format!("recent +{:.2}", breakdown.recency_bonus));
    }

    if labels.is_empty() {
        labels.push(format!(
            "path {:.2}, dir {:.2}",
            breakdown.jaccard_similarity, breakdown.dir_proximity
        ));
    }

    labels
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

    let all_files: Vec<PathBuf> = WalkBuilder::new(&project_root)
        .hidden(false)
        .git_ignore(true)
        .build()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|ft| ft.is_file()).unwrap_or(false))
        .map(|entry| entry.into_path())
        .collect();

    let stats = ProjectStats::from_paths(all_files.iter().map(|path| path.as_path()));
    let cochange = CochangeIndex::from_git(&project_root, cli.history_limit);

    let mut results: Vec<(f64, PathBuf, ScoreBreakdown)> = all_files
        .into_iter()
        .filter_map(|path| {
            let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());

            if canonical == target_canonical {
                return None;
            }

            let rel_target = target.strip_prefix(&project_root).unwrap_or(target);
            let rel_candidate = path.strip_prefix(&project_root).unwrap_or(&path);

            let breakdown =
                similarity_breakdown(rel_target, rel_candidate, target, &path, &stats, &cochange);
            if breakdown.total >= cli.threshold {
                Some((breakdown.total, path, breakdown))
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
        for (score, path, breakdown) in &results {
            let rel = path.strip_prefix(&project_root).unwrap_or(path);
            println!("{:.4}  {}", score, rel.display());
            if cli.explain {
                println!("        {}", explain_labels(breakdown).join(" | "));
            }
        }
    }
}
