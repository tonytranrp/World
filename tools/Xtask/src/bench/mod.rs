//! `xtask bench`: run Criterion benchmarks and gate on regressions.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::process::{self, cargo};

/// The crates that carry benchmarks.
const BENCH_PACKAGES: &[&str] = &["world_testkit"];

pub fn run(
    save: Option<String>,
    check: Option<String>,
    threshold_percent: f64,
) -> Result<(), String> {
    match (save, check) {
        (Some(name), _) => bench(&["--save-baseline", &name]),
        (None, Some(name)) => compare(&name, threshold_percent),
        (None, None) => bench(&[]),
    }
}

fn bench(criterion_args: &[&str]) -> Result<(), String> {
    for package in BENCH_PACKAGES {
        let mut command = cargo();
        // `--benches` keeps libtest targets out: they reject Criterion's flags.
        command
            .args(["bench", "-p", package, "--benches", "--"])
            .args(criterion_args);
        process::run(&mut command)?;
    }
    Ok(())
}

fn compare(baseline: &str, threshold_percent: f64) -> Result<(), String> {
    let started = SystemTime::now();
    bench(&["--baseline", baseline]).map_err(|e| {
        format!("{e}\nDoes baseline {baseline:?} exist? Save one with `cargo xtask bench --save {baseline}`.")
    })?;

    let criterion_dir = PathBuf::from("target").join("criterion");
    let mut regressions = Vec::new();
    let mut compared = 0;
    for file in find_change_files(&criterion_dir) {
        // Ignore comparisons left over from earlier runs.
        if fs::metadata(&file)
            .and_then(|m| m.modified())
            .is_ok_and(|t| t < started)
        {
            continue;
        }
        let text = fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let json: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
        let Some(change) = json["mean"]["point_estimate"].as_f64() else {
            return Err(format!("{} has no mean change estimate", file.display()));
        };
        compared += 1;
        let percent = change * 100.0;
        let name = benchmark_name(&file, &criterion_dir);
        println!("{name}: {percent:+.1}% versus baseline {baseline:?}");
        if percent > threshold_percent {
            regressions.push(format!(
                "{name} changed by {percent:+.1}% (allowed {threshold_percent}%)"
            ));
        }
    }

    if compared == 0 {
        return Err(format!(
            "no comparison against baseline {baseline:?} was produced. Save one first with `cargo xtask bench --save {baseline}`."
        ));
    }
    if regressions.is_empty() {
        println!("\nNo regression beyond {threshold_percent}% across {compared} benchmarks.");
        Ok(())
    } else {
        Err(format!(
            "performance regression:\n  {}",
            regressions.join("\n  ")
        ))
    }
}

/// Every `.../change/estimates.json` below `dir`.
fn find_change_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(find_change_files(&path));
        } else if path.file_name().is_some_and(|n| n == "estimates.json")
            && path
                .parent()
                .and_then(|p| p.file_name())
                .is_some_and(|n| n == "change")
        {
            found.push(path);
        }
    }
    found
}

fn benchmark_name(change_file: &Path, root: &Path) -> String {
    let relative = change_file.strip_prefix(root).unwrap_or(change_file);
    let mut parts: Vec<_> = relative
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    parts.truncate(parts.len().saturating_sub(2)); // drop "change/estimates.json"
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benchmark_name_drops_the_change_suffix() {
        let root = Path::new("target/criterion");
        let file = root
            .join("dense_apply_10k_edits")
            .join("change")
            .join("estimates.json");
        assert_eq!(benchmark_name(&file, root), "dense_apply_10k_edits");
    }

    #[test]
    fn missing_directory_finds_nothing() {
        assert!(find_change_files(Path::new("does/not/exist")).is_empty());
    }
}
