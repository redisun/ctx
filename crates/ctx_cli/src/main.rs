//! CTX CLI - Command-line interface for CTX context management.

use anyhow::Result;
use clap::{Parser, Subcommand};

mod commands;

#[derive(Parser)]
#[command(name = "ctx")]
#[command(about = "Context management for coding agents", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new CTX repository
    Init,
    /// Add content to the repository
    Add {
        #[command(subcommand)]
        command: AddCommands,
    },
    /// Create a commit
    Commit {
        /// Commit message
        #[arg(short, long)]
        message: String,
        /// Don't snapshot narrative files
        #[arg(long)]
        no_narrative: bool,
    },
    /// Rebuild indexes from objects
    Rebuild,
    /// Session management (staging area)
    Stage {
        #[command(subcommand)]
        command: StageCommands,
    },
    /// Build a prompt pack from a query
    Query {
        /// The query or question
        query: String,
        /// Token budget
        #[arg(long, default_value = "16000")]
        budget: u32,
        /// Graph expansion depth
        #[arg(long, default_value = "2")]
        depth: u32,
        /// Output format (json, text)
        #[arg(long, default_value = "json")]
        format: String,
        /// Exclude narrative content
        #[arg(long)]
        no_narrative: bool,
    },
    /// Debug and inspection commands
    Debug {
        #[command(subcommand)]
        command: DebugCommands,
    },
    /// Analyze code semantics
    Analyze {
        #[command(subcommand)]
        command: AnalyzeCommands,
    },
    /// Garbage collect unreferenced objects
    Gc {
        /// Show what would be deleted without deleting
        #[arg(long)]
        dry_run: bool,
        /// Skip grace period, delete immediately
        #[arg(long)]
        aggressive: bool,
    },
    /// Verify repository integrity
    Verify {
        /// Check object integrity (slow)
        #[arg(long)]
        objects: bool,
        /// Check all (objects + refs + commits)
        #[arg(long)]
        full: bool,
    },
}

#[derive(Subcommand)]
enum StageCommands {
    /// Start a new session
    Start {
        /// Task description
        task: String,
    },
    /// Show current session status
    Status,
    /// Flush pending observations to staging
    Flush,
    /// Compact session into canonical commit
    Compact {
        /// Commit message
        #[arg(short, long)]
        message: String,
    },
    /// Abort current session
    Abort {
        /// Reason for aborting
        #[arg(short, long)]
        reason: Option<String>,
    },
    /// Recover session from staging (after crash)
    Recover,
}

#[derive(Subcommand)]
enum AddCommands {
    /// Add a note to today's log
    Note {
        /// The note text
        text: String,
    },
    /// Create a new task
    Task {
        /// Task title
        title: String,
        /// Task description (optional)
        #[arg(short, long)]
        body: Option<String>,
    },
    /// Update an existing task
    TaskUpdate {
        /// Task ID (numeric part, e.g., 42 for task_0042.md)
        id: u32,
        /// New status (open, in_progress, done, etc.)
        #[arg(short, long)]
        status: String,
        /// Note to append (optional)
        #[arg(short, long)]
        note: Option<String>,
    },
}

#[derive(Subcommand)]
enum DebugCommands {
    /// Print raw object contents
    Cat {
        /// Object ID (64 hex characters)
        object_id: String,
    },
    /// List all references (HEAD, STAGE, refs/*)
    Refs,
    /// Show commit history from HEAD
    History {
        /// Maximum number of commits to show
        #[arg(short, long)]
        limit: Option<usize>,
    },
    /// Query the index
    Index {
        #[command(subcommand)]
        command: IndexDebugCommands,
    },
    /// Export graph for visualization
    Graph {
        /// Output format (dot, json)
        #[arg(long, default_value = "dot")]
        format: String,
        /// Only show edges of these labels (comma-separated)
        #[arg(long)]
        labels: Option<String>,
        /// Maximum nodes to include
        #[arg(long, default_value = "100")]
        max_nodes: usize,
    },
    /// Show SCC analysis
    Scc {
        /// Show nodes in each SCC
        #[arg(long)]
        show_members: bool,
    },
    /// Show Cargo workspace info
    Cargo {
        #[command(subcommand)]
        command: CargoDebugCommands,
    },
}

#[derive(Subcommand)]
enum CargoDebugCommands {
    /// Show parsed workspace structure
    Show,
    /// Show dependencies for a package
    Deps {
        /// Package name
        package: String,
    },
}

#[derive(Subcommand)]
enum IndexDebugCommands {
    /// Look up a file path
    Path {
        /// The path to look up
        path: String,
    },
    /// Look up entities by name
    Name {
        /// Namespace (package, module, item, task, note)
        namespace: String,
        /// Name to look up
        name: String,
    },
    /// Show edges for a node
    Edges {
        /// Node kind (file, module, item, package, etc.)
        kind: String,
        /// Node ID
        id: String,
        /// Edge label (optional, shows all if omitted)
        #[arg(short, long)]
        label: Option<String>,
    },
    /// Show index statistics
    Stats,
}

#[derive(Subcommand)]
enum AnalyzeCommands {
    /// Analyze Rust code using rust-analyzer
    Rust {
        /// Specific file to analyze (or all if omitted)
        file: Option<std::path::PathBuf>,
    },
    /// Analyze Cargo workspace metadata
    Cargo,
    /// Check analysis tool availability
    Status,
}

fn main() -> Result<()> {
    // Initialize tracing subscriber
    // Respects RUST_LOG environment variable (e.g., RUST_LOG=debug)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Init => commands::init::run(),
        Commands::Add { command } => match command {
            AddCommands::Note { text } => commands::add::note(&text),
            AddCommands::Task { title, body } => commands::add::task(&title, body.as_deref()),
            AddCommands::TaskUpdate { id, status, note } => {
                commands::add::task_update(id, &status, note.as_deref())
            }
        },
        Commands::Commit {
            message,
            no_narrative,
        } => commands::commit::run(&message, no_narrative),
        Commands::Rebuild => commands::rebuild::run(),
        Commands::Query {
            query,
            budget,
            depth,
            format,
            no_narrative,
        } => commands::query::run(&query, budget, depth, &format, no_narrative),
        Commands::Stage { command } => match command {
            StageCommands::Start { task } => commands::stage::start(&task),
            StageCommands::Status => commands::stage::status(),
            StageCommands::Flush => commands::stage::flush(),
            StageCommands::Compact { message } => commands::stage::compact(&message),
            StageCommands::Abort { reason } => commands::stage::abort(reason),
            StageCommands::Recover => commands::stage::recover(),
        },
        Commands::Debug { command } => match command {
            DebugCommands::Cat { object_id } => commands::debug::cat(&object_id),
            DebugCommands::Refs => commands::debug::refs(),
            DebugCommands::History { limit } => commands::debug::history(limit),
            DebugCommands::Index { command } => match command {
                IndexDebugCommands::Path { path } => commands::debug::index_path(&path),
                IndexDebugCommands::Name { namespace, name } => {
                    commands::debug::index_name(&namespace, &name)
                }
                IndexDebugCommands::Edges { kind, id, label } => {
                    commands::debug::index_edges(&kind, &id, label.as_deref())
                }
                IndexDebugCommands::Stats => commands::debug::index_stats(),
            },
            DebugCommands::Graph {
                format,
                labels,
                max_nodes,
            } => commands::debug::graph(&format, labels.as_deref(), max_nodes),
            DebugCommands::Scc { show_members } => commands::debug::scc(show_members),
            DebugCommands::Cargo { command } => match command {
                CargoDebugCommands::Show => commands::debug::cargo_show(),
                CargoDebugCommands::Deps { package } => commands::debug::cargo_deps(&package),
            },
        },
        Commands::Analyze { command } => match command {
            AnalyzeCommands::Rust { file } => commands::analyze::analyze_rust(file.as_deref()),
            AnalyzeCommands::Cargo => commands::analyze::analyze_cargo(),
            AnalyzeCommands::Status => commands::analyze::status(),
        },
        Commands::Gc { dry_run, aggressive } => commands::gc::run(dry_run, aggressive),
        Commands::Verify { objects, full } => commands::verify::run(objects, full),
    }
}
