//! Garbage collection command.

use anyhow::Result;
use console::style;
use ctx_core::{CtxRepo, GcConfig};
use indicatif::{ProgressBar, ProgressStyle};

/// Run garbage collection.
pub fn run(dry_run: bool, aggressive: bool) -> Result<()> {
    let mut repo = CtxRepo::open(".")?;

    let config = GcConfig {
        dry_run,
        aggressive,
        grace_period_days: 7,
    };

    if dry_run {
        println!(
            "{} Running GC in dry-run mode (no objects will be deleted)...",
            style("→").cyan()
        );
    } else {
        // Safety: warn user and require confirmation for real deletion
        println!();
        println!(
            "{} {}",
            style("⚠").yellow().bold(),
            style("WARNING:").yellow().bold()
        );
        println!("  Garbage collection will permanently delete unreferenced objects.");
        if aggressive {
            println!(
                "  {} mode: No grace period - deletes all unreachable objects immediately.",
                style("Aggressive").red()
            );
        } else {
            println!("  Grace period: {} days", config.grace_period_days);
            println!("  Objects newer than this will be kept even if unreachable.");
        }
        println!();
        println!(
            "  {} Run with {} first to see what would be deleted.",
            style("Tip:").cyan(),
            style("--dry-run").cyan()
        );
        println!();

        // Require confirmation
        print!("Continue with garbage collection? [y/N]: ");
        use std::io::{self, Write};
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            println!("{} Garbage collection cancelled.", style("✓").green());
            return Ok(());
        }

        if aggressive {
            println!(
                "{} Running aggressive GC (no grace period)...",
                style("→").yellow()
            );
        } else {
            println!(
                "{} Running GC with {}-day grace period...",
                style("→").cyan(),
                config.grace_period_days
            );
        }
    }

    // Create progress bar
    let pb = ProgressBar::new(100);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} {msg:20} [{bar:40.cyan/blue}] {pos}/{len}")
            .unwrap()
            .progress_chars("█▓▒░  "),
    );

    // Clone pb for the closure
    let pb_clone = pb.clone();
    let report = repo.gc_with_progress(config, &move |current, total, phase| {
        pb_clone.set_length(total as u64);
        pb_clone.set_position(current as u64);
        pb_clone.set_message(format!("Phase: {}", phase));
    })?;

    pb.finish_and_clear();

    println!();
    println!("{}", style("Garbage Collection Report:").bold());
    println!(
        "  Objects scanned:   {}",
        style(report.objects_scanned).cyan()
    );
    println!(
        "  Objects reachable: {}",
        style(report.objects_reachable).green()
    );
    println!(
        "  Objects deleted:   {}",
        if report.objects_deleted > 0 {
            style(report.objects_deleted).yellow()
        } else {
            style(report.objects_deleted).green()
        }
    );
    println!(
        "  Bytes freed:       {} ({:.2} MB)",
        style(format!("{}", report.bytes_freed)).cyan(),
        report.bytes_freed as f64 / 1_048_576.0
    );

    if !report.errors.is_empty() {
        println!();
        println!("{}", style("Errors encountered:").red().bold());
        for error in &report.errors {
            println!("  {} {}", style("×").red(), error);
        }
    }

    if dry_run && report.objects_deleted > 0 {
        println!();
        println!("{}", style("ℹ").blue());
        println!("This was a dry run. To actually delete objects, run:");
        println!("  {}", style("ctx gc").cyan());
    } else if !dry_run && report.objects_deleted > 0 {
        println!();
        println!(
            "{} Successfully freed {:.2} MB",
            style("✓").green(),
            report.bytes_freed as f64 / 1_048_576.0
        );
    }

    Ok(())
}
