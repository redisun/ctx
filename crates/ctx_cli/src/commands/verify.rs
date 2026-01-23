//! Repository verification command.

use anyhow::Result;
use console::style;
use ctx_core::{CtxRepo, VerifyConfig};
use indicatif::{ProgressBar, ProgressStyle};

/// Verify repository integrity.
pub fn run(objects: bool, full: bool) -> Result<()> {
    let repo = CtxRepo::open(".")?;

    let config = if full {
        VerifyConfig {
            check_objects: true,
            check_refs: true,
            check_commits: true,
            verbose: false,
        }
    } else if objects {
        VerifyConfig {
            check_objects: true,
            check_refs: false,
            check_commits: false,
            verbose: false,
        }
    } else {
        VerifyConfig::default()
    };

    let check_objects = config.check_objects;

    // Show a spinner for verification
    let spinner = if check_objects {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")
                .unwrap(),
        );
        pb.set_message("Verifying object integrity...");
        pb.enable_steady_tick(std::time::Duration::from_millis(100));
        Some(pb)
    } else {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")
                .unwrap(),
        );
        pb.set_message("Verifying repository...");
        pb.enable_steady_tick(std::time::Duration::from_millis(100));
        Some(pb)
    };

    let report = repo.verify(config)?;

    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }

    println!();
    println!("{}", style("Verification Report:").bold());

    if check_objects {
        println!(
            "  Objects checked:    {}",
            style(report.objects_checked).cyan()
        );
        if !report.objects_corrupted.is_empty() {
            println!(
                "  Corrupted objects:  {}",
                style(report.objects_corrupted.len()).red()
            );
            for id in &report.objects_corrupted {
                println!("    {} {}", style("×").red(), id.as_hex());
            }
        }
    }

    println!(
        "  Refs checked:       {}",
        style(report.refs_checked).cyan()
    );
    if !report.refs_dangling.is_empty() {
        println!(
            "  Dangling refs:      {}",
            style(report.refs_dangling.len()).yellow()
        );
        for ref_name in &report.refs_dangling {
            println!("    {} {}", style("⚠").yellow(), ref_name);
        }
    }

    println!(
        "  Commits checked:    {}",
        style(report.commits_checked).cyan()
    );
    if !report.commits_invalid.is_empty() {
        println!(
            "  Invalid commits:    {}",
            style(report.commits_invalid.len()).red()
        );
        for id in &report.commits_invalid {
            println!("    {} {}", style("×").red(), id.as_hex());
        }
    }

    println!();
    if report.has_issues() {
        println!("{}", style(&report.summary()).yellow().bold());
    } else {
        println!(
            "{} {}",
            style("✓").green(),
            style(&report.summary()).green()
        );
    }

    if report.has_issues() {
        println!();
        println!("{}", style("Recommendations:").bold());
        if !report.objects_corrupted.is_empty() {
            println!(
                "  {} Run {} to remove corrupted objects",
                style("→").cyan(),
                style("ctx gc").cyan()
            );
        }
        if !report.refs_dangling.is_empty() {
            println!(
                "  {} Dangling refs may indicate incomplete operations",
                style("→").cyan()
            );
        }
        if !report.commits_invalid.is_empty() {
            println!(
                "  {} Invalid commits may require manual recovery",
                style("→").cyan()
            );
            println!(
                "  {} Try {} to regenerate the index",
                style("→").cyan(),
                style("ctx rebuild").cyan()
            );
        }
    }

    Ok(())
}
