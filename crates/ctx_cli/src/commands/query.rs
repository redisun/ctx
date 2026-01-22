//! Query command - build prompt packs.

use anyhow::{Context, Result};
use ctx_core::{CtxRepo, RetrievalConfig};

/// Run the query command to build a prompt pack.
pub fn run(query: &str, budget: u32, depth: u32, format: &str, no_narrative: bool) -> Result<()> {
    let mut repo = CtxRepo::open(".")?;

    // Configure retrieval
    let config = RetrievalConfig {
        token_budget: budget,
        expansion_depth: depth,
        include_active_task: !no_narrative,
        include_log: !no_narrative,
        ..Default::default()
    };

    // Build prompt pack
    let pack = repo
        .build_pack(query, &config)
        .context("Failed to build prompt pack")?;

    // Output in requested format
    match format {
        "json" => {
            let json = pack.to_json().context("Failed to serialize to JSON")?;
            println!("{}", json);
        }
        "text" => {
            let text = pack.to_text();
            println!("{}", text);
        }
        _ => {
            anyhow::bail!("Unsupported format: {}. Use 'json' or 'text'.", format);
        }
    }

    Ok(())
}
