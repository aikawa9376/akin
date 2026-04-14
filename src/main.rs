mod content;
mod scorer;
mod tokenizer;

use clap::Parser;
use ignore::WalkBuilder;
use std::path::PathBuf;

use scorer::similarity_score;

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
