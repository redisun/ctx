//! Analyze commands for semantic code analysis.

use anyhow::Result;
use ctx_core::{CtxRepo, RustAnalyzer};
use std::path::Path;

/// Analyze Rust code using rust-analyzer.
pub fn analyze_rust(file: Option<&Path>) -> Result<()> {
    let mut repo = CtxRepo::open(".")?;

    match file {
        Some(path) => {
            // Analyze single file
            println!("Analyzing {}...", path.display());

            let report = repo.analyze_rust_file(path)?;

            println!("Analysis complete:");
            println!("  Symbols found: {}", report.symbols);
            println!("  Calls resolved: {}", report.calls);
            println!("  Edges generated: {}", report.edges);
            println!("  Edge batch ID: {}", report.edge_batch_id.as_hex());
        }
        None => {
            // Analyze all Rust files
            println!("Analyzing all Rust files in project...");

            let report = repo.analyze_rust()?;

            println!("Analysis complete:");
            println!("  Files analyzed: {}", report.files_analyzed);
            println!("  Symbols found: {}", report.symbols_found);
            println!("  Calls resolved: {}", report.calls_resolved);
            println!("  Edges generated: {}", report.edges_generated);
            println!("  Edge batch ID: {}", report.edge_batch_id.as_hex());
        }
    }

    Ok(())
}

/// Analyze Cargo workspace metadata.
pub fn analyze_cargo() -> Result<()> {
    let mut repo = CtxRepo::open(".")?;

    println!("Analyzing Cargo workspace...");

    let report = repo.analyze_cargo()?;

    println!("Cargo analysis complete:");
    println!("  Packages found: {}", report.packages_found);
    println!("  Targets found: {}", report.targets_found);
    println!("  Dependencies found: {}", report.dependencies_found);
    println!("  Edges generated: {}", report.edges_generated);
    println!("  Snapshot ID: {}", report.snapshot_id.as_hex());
    println!("  Edge batch ID: {}", report.edge_batch_id.as_hex());
    println!("  Commit ID: {}", report.commit_id.as_hex());

    Ok(())
}

/// Show analysis tool availability status.
pub fn status() -> Result<()> {
    println!("Analysis Tool Status:");
    println!();

    // Check rust-analyzer
    if RustAnalyzer::is_available() {
        println!("  rust-analyzer: installed ✓");

        // Try to get version
        if let Ok(output) = std::process::Command::new("rust-analyzer")
            .arg("--version")
            .output()
        {
            if let Ok(version) = String::from_utf8(output.stdout) {
                println!("    Version: {}", version.trim());
            }
        }
    } else {
        println!("  rust-analyzer: not found ✗");
        println!();
        println!("To install rust-analyzer:");
        println!("  rustup component add rust-analyzer");
        println!();
        println!("Or via package manager:");
        println!("  brew install rust-analyzer  # macOS");
        println!("  apt install rust-analyzer   # Debian/Ubuntu");
    }

    Ok(())
}
