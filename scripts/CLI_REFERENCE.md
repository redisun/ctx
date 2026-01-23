# CTX CLI Quick Reference

This is a quick reference for the actual CTX CLI commands (as discovered from the implementation).

## Repository Management

### Initialize
```bash
ctx init
```
Creates a new CTX repository in `.ctx/`.

## Narrative

### Add Note
```bash
ctx add note "Your note text here"
```
Adds a timestamped note to today's log file (`log/YYYY-MM-DD.md`).

### Create Task
```bash
ctx add task "Task title" --body "Optional description"
```
Creates a new task file in `tasks/task_NNNN.md`.

### Update Task
```bash
ctx add task-update 42 --status done --note "Completed successfully"
```
Updates task #42's status and adds a note.

## Commits

### Simple Commit
```bash
ctx commit -m "Your commit message"
```
Creates a commit with current narrative files. This snapshots all narrative (log/tasks) into the commit.

Options:
- `--no-narrative` - Don't snapshot narrative files

## Session Management (Advanced)

These are for agent simulation and complex workflows:

```bash
# Start a session
ctx stage start "Task description"

# Check session status
ctx stage status

# Flush observations to staging
ctx stage flush

# Compact session into commit
ctx stage compact -m "Commit message"

# Abort session
ctx stage abort --reason "Why"

# Recover crashed session
ctx stage recover
```

## Analysis

### Analyze Rust Code
```bash
# Analyze all Rust files
ctx analyze rust

# Analyze specific file
ctx analyze rust src/lib.rs
```
Requires `rust-analyzer` to be installed. Creates semantic edges.

### Analyze Cargo Metadata
```bash
ctx analyze cargo
```
Extracts dependency graph from `Cargo.toml` and creates package/dependency edges.

### Check Status
```bash
ctx analyze status
```
Shows if analysis tools are available.

## Query

### Build Prompt Pack
```bash
ctx query "How does the Config struct work?"
```

Options:
- `--budget 16000` - Token budget (default: 16000)
- `--depth 2` - Graph expansion depth (default: 2)
- `--format json` - Output format: `json` or `text` (default: json)
- `--no-narrative` - Exclude narrative content

Output is JSON containing:
- `task` - Your query
- `retrieved` - Array of code chunks
- `graph_context` - Graph expansion metadata
- `recent_narrative` - Recent log entries
- `token_budget` - Token accounting

## Debug Commands

### Show Object
```bash
ctx debug cat <object_id>
```
Displays raw object contents (64-character hex ID).

### Show References
```bash
ctx debug refs
```
Lists HEAD, STAGE, and all refs.

### Show History
```bash
ctx debug history
ctx debug history --limit 10
```
Shows commit history from HEAD.

### Index Queries

```bash
# Show index statistics
ctx debug index stats

# Look up file path
ctx debug index path src/lib.rs

# Look up name
ctx debug index name item Config
ctx debug index name package my-crate

# Show edges for node
ctx debug index edges item Config
ctx debug index edges item Config --label calls
```

Namespaces for names:
- `package` - Cargo packages
- `module` - Rust modules
- `item` - Rust items (functions, structs, etc.)
- `task` - Tasks
- `note` - Narrative notes

### Graph Visualization

```bash
# Export as DOT (GraphViz)
ctx debug graph --format dot > graph.dot

# Filter by edge labels
ctx debug graph --labels calls,defines --max-nodes 50

# Export as JSON
ctx debug graph --format json
```

### SCC Analysis
```bash
# Show strongly connected components
ctx debug scc

# Show members of each SCC
ctx debug scc --show-members
```

### Cargo Debug

```bash
# Show workspace structure
ctx debug cargo show

# Show dependencies for package
ctx debug cargo deps my-package
```

## Maintenance

### Rebuild Index
```bash
ctx rebuild
```
Rebuilds the index from scratch. Use after manual object manipulation.

### Garbage Collection
```bash
# Dry run (show what would be deleted)
ctx gc --dry-run

# Actually delete unreferenced objects
ctx gc

# Aggressive (skip grace period)
ctx gc --aggressive
```

### Verify Integrity
```bash
# Quick verify (refs and commits)
ctx verify

# Verify objects (slower)
ctx verify --objects

# Full verify
ctx verify --full
```

## Workflows

### Basic Workflow (Simple)

```bash
# 1. Initialize
ctx init

# 2. Write code
# ... edit files ...

# 3. Add narrative
ctx add note "Implemented user authentication"

# 4. Commit
ctx commit -m "Add user auth"

# 5. Analyze
ctx analyze rust
ctx analyze cargo

# 6. Query
ctx query "How does authentication work?"
```

### Agent Workflow (Session-based)

```bash
# 1. Start session
ctx stage start "Implement search feature"

# 2. Agent does work (programmatically via library)
# ... observe_file_read(), observe_file_write(), etc ...

# 3. Flush observations
ctx stage flush

# 4. More work
# ...

# 5. Compact to commit
ctx stage compact -m "Add search feature"
```

## Environment Variables

```bash
# Enable debug logging
RUST_LOG=debug ctx <command>

# Trace-level logging
RUST_LOG=trace ctx <command>

# Log only specific modules
RUST_LOG=ctx_core::pack=debug ctx query "test"
```

## Tips

1. **Narrative is automatic**: Just use `ctx add note` - no need to manually create files
2. **Files tracked via analysis**: Don't need to explicitly add files like Git
3. **Commits snapshot narrative**: `ctx commit` automatically includes log/task files
4. **Query uses graph**: Better results after running `ctx analyze rust`
5. **JSON output**: Most commands output JSON for scripting
6. **Index is gitignored**: Safe to rebuild anytime with `ctx rebuild`

## Common Patterns

### Track Work Session
```bash
ctx add note "Starting work on feature X"
# ... work ...
ctx add note "Completed initial implementation"
ctx commit -m "Feature X initial"
```

### Deep Analysis
```bash
ctx analyze rust        # Extract symbols and calls
ctx analyze cargo       # Extract dependencies
ctx commit -m "Analysis snapshot"
ctx query "What calls function X?"
```

### Task Management
```bash
ctx add task "Fix auth bug" --body "Users can't log in"
# ... fix bug ...
ctx add task-update 1 --status done --note "Fixed by adding null check"
```

## Troubleshooting

### "rust-analyzer not found"
```bash
rustup component add rust-analyzer
```

### "content modified" warnings
Rust-analyzer detected changes while analyzing. Usually safe to ignore.

### Corrupt objects
```bash
ctx verify --full
ctx rebuild  # Rebuild index
```

### Stale session
```bash
ctx stage abort --reason "Cleanup"
# or
ctx stage recover
```

## See Also

- `scripts/test_*.sh` - Example usage
- `scripts/demo_interactive.sh` - Interactive walkthrough
- Main README.md - Project overview
