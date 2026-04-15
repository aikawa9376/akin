use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CochangeStats {
    pub weighted_count: f64,
    pub commit_count: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CochangeEvidence {
    pub score: f64,
    pub weighted_count: f64,
    pub commit_count: usize,
}

#[derive(Clone, Debug, Default)]
pub struct CochangeIndex {
    related: HashMap<String, HashMap<String, CochangeStats>>,
}

impl CochangeIndex {
    pub fn from_git(repo_root: &Path, max_commits: usize) -> Self {
        if max_commits == 0 {
            return Self::default();
        }

        let output = Command::new("git")
            .arg("--no-pager")
            .arg("log")
            .arg("--name-only")
            .arg("--format=__AKIN_COMMIT__")
            .arg(format!("-n{max_commits}"))
            .arg("--diff-filter=ACMR")
            .current_dir(repo_root)
            .output();

        let Ok(output) = output else {
            return Self::default();
        };
        if !output.status.success() {
            return Self::default();
        }

        let text = String::from_utf8_lossy(&output.stdout);
        Self::from_commits(parse_git_name_only_log(&text))
    }

    fn from_commits(commits: Vec<Vec<String>>) -> Self {
        let mut related: HashMap<String, HashMap<String, CochangeStats>> = HashMap::new();

        for mut files in commits {
            files.sort();
            files.dedup();
            if files.len() < 2 || files.len() > 64 {
                continue;
            }

            let weight = 1.0 / (files.len() as f64).sqrt();
            for (idx, left) in files.iter().enumerate() {
                for right in files.iter().skip(idx + 1) {
                    let left_related = related.entry(left.clone()).or_default();
                    let left_stats = left_related.entry(right.clone()).or_default();
                    left_stats.weighted_count += weight;
                    left_stats.commit_count += 1;

                    let right_related = related.entry(right.clone()).or_default();
                    let right_stats = right_related.entry(left.clone()).or_default();
                    right_stats.weighted_count += weight;
                    right_stats.commit_count += 1;
                }
            }
        }

        Self { related }
    }

    pub fn evidence(&self, target: &Path, candidate: &Path) -> Option<CochangeEvidence> {
        let target = target.to_string_lossy();
        let candidate = candidate.to_string_lossy();
        let related = self.related.get(target.as_ref())?;
        let stats = related.get(candidate.as_ref())?;
        let max_weight = related
            .values()
            .map(|stats| stats.weighted_count)
            .fold(0.0f64, f64::max);
        let score = if max_weight > 0.0 {
            stats.weighted_count / max_weight
        } else {
            0.0
        };

        Some(CochangeEvidence {
            score,
            weighted_count: stats.weighted_count,
            commit_count: stats.commit_count,
        })
    }
}

fn parse_git_name_only_log(output: &str) -> Vec<Vec<String>> {
    let mut commits = Vec::new();
    let mut current = Vec::new();

    for line in output.lines() {
        if line == "__AKIN_COMMIT__" {
            if !current.is_empty() {
                commits.push(std::mem::take(&mut current));
            }
            continue;
        }

        let line = line.trim();
        if !line.is_empty() {
            current.push(line.to_string());
        }
    }

    if !current.is_empty() {
        commits.push(current);
    }

    commits
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{CochangeIndex, parse_git_name_only_log};

    #[test]
    fn parses_git_name_only_log_into_commit_file_sets() {
        let parsed = parse_git_name_only_log(
            "__AKIN_COMMIT__\nsrc/a.rs\nsrc/b.rs\n\n__AKIN_COMMIT__\nsrc/c.rs\n",
        );

        assert_eq!(
            parsed,
            vec![
                vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
                vec!["src/c.rs".to_string()],
            ]
        );
    }

    #[test]
    fn normalizes_cochange_score_per_target() {
        let index = CochangeIndex::from_commits(vec![
            vec!["src/a.rs".into(), "src/b.rs".into()],
            vec!["src/a.rs".into(), "src/b.rs".into()],
            vec!["src/a.rs".into(), "src/c.rs".into()],
        ]);

        let best = index
            .evidence(Path::new("src/a.rs"), Path::new("src/b.rs"))
            .unwrap();
        let weaker = index
            .evidence(Path::new("src/a.rs"), Path::new("src/c.rs"))
            .unwrap();

        assert_eq!(best.commit_count, 2);
        assert_eq!(weaker.commit_count, 1);
        assert!(best.score > weaker.score);
        assert_eq!(best.score, 1.0);
    }
}
