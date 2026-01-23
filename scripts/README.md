# CTX Manual Testing Scripts

This directory contains bash scripts for manually testing and demonstrating CTX functionality.

**Quick Links:**
- [CLI Reference](CLI_REFERENCE.md) - Complete CLI command reference
- [Understanding Pack Output](UNDERSTANDING_PACK_OUTPUT.md) - **Why is `retrieved` empty?**
- Scripts below - Automated testing and demos

## Available Scripts

### 🎯 `test_prompt_pack.sh`
Tests prompt pack generation and shows what it outputs.

**What it does:**
- Creates a test Rust project
- Adds files and narrative to CTX
- Runs semantic analysis
- Builds a prompt pack
- Shows the compiled context

**Use when:** You want to see how CTX compiles context for LLM consumption.

```bash
./scripts/test_prompt_pack.sh
```

### 🔍 `test_analyzer.sh`
Tests the semantic analyzer on various Rust code patterns.

**What it does:**
- Creates test files with different Rust patterns (functions, structs, traits, generics)
- Runs rust-analyzer on each file
- Shows extracted symbols, calls, and edges
- Demonstrates cross-file analysis

**Use when:** You want to verify that semantic analysis is working correctly.

```bash
./scripts/test_analyzer.sh
```

### 🔄 `test_full_workflow.sh`
Simulates a complete realistic coding session.

**What it does:**
- Initializes a CTX repo
- Creates a todo app across multiple commits
- Adds narrative context
- Runs semantic analysis
- Queries the context
- Shows commit history

**Use when:** You want to see the complete CTX workflow from start to finish.

```bash
./scripts/test_full_workflow.sh
```

### 🔎 `inspect_objects.sh`
Inspects the object store contents of the current repository.

**What it does:**
- Shows object count and distribution
- Displays HEAD and refs
- Shows staging status
- Samples objects and their headers
- Reports storage statistics

**Use when:** You want to debug or understand what's in the object store.

```bash
cd /path/to/ctx/repo
./scripts/inspect_objects.sh
```

### 🎬 `demo_interactive.sh`
Interactive walkthrough of CTX features.

**What it does:**
- Step-by-step demo with pauses
- Shows initialization, tracking, commits, queries
- Explains concepts as it goes
- Great for learning or showing others

**Use when:** You want to learn CTX or demonstrate it to someone.

```bash
./scripts/demo_interactive.sh
```

## Running the Scripts

All scripts should be run from the CTX project root:

```bash
# Make them executable (first time only)
chmod +x scripts/*.sh

# Run a script
./scripts/test_prompt_pack.sh
```

## What Each Script Tests

| Script | Tests | Output |
|--------|-------|--------|
| `test_prompt_pack.sh` | Pack building, graph expansion, token budget | JSON pack structure, retrieved chunks |
| `test_analyzer.sh` | Rust semantic analysis, edge extraction | Symbol counts, call graphs, edges |
| `test_full_workflow.sh` | End-to-end workflow, multi-commit | Query results, history |
| `inspect_objects.sh` | Object store integrity, compression | Storage stats, object headers |
| `demo_interactive.sh` | User experience, core concepts | Interactive walkthrough |

## Interpreting Output

### Prompt Pack Output
Look for:
- `retrieved`: Array of chunks with code snippets
- `graph_context`: Shows which nodes were expanded
- `token_budget`: Token usage accounting
- `recent_narrative`: Markdown context

### Analyzer Output
Look for:
- `Symbols found`: Number of functions/structs/traits extracted
- `Calls resolved`: Number of function call edges created
- `Edges generated`: Total relationship count
- `Edge batch ID`: Object ID of the edge batch

### Object Store Output
Look for:
- Even distribution across shards (first byte of hash)
- `CTXO1` magic in object headers
- Compression ratios (should be 2:1 or better for text)
- Object integrity (no corruption)

## Troubleshooting

### "rust-analyzer not found"
The analyzer tests require rust-analyzer to be installed:
```bash
rustup component add rust-analyzer
# or
brew install rust-analyzer  # macOS
apt install rust-analyzer   # Debian/Ubuntu
```

### "No .ctx directory found"
The `inspect_objects.sh` script must be run from within a CTX repository:
```bash
cd /path/to/your/ctx/repo
/path/to/ctx/scripts/inspect_objects.sh
```

### Scripts fail to build
Make sure you've built CTX first:
```bash
cargo build --release
```

Or the scripts will try to build on each run (slower).

## Adding Your Own Tests

To add a new test script:

1. Create `scripts/test_yourfeature.sh`
2. Start with the template:
```bash
#!/bin/bash
set -e

echo "=== Your Feature Test ==="
TEST_DIR=$(mktemp -d)
cd "$TEST_DIR"

# Your test logic here
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- init
# ... more commands ...

echo "Test directory: $TEST_DIR"
```

3. Make it executable: `chmod +x scripts/test_yourfeature.sh`
4. Document it in this README

## Tips

- All test scripts create temporary directories - check the output for the path
- Test directories are preserved so you can explore after the script finishes
- Use `head -N` to limit output when showing large results
- Set `CTX_LOG=debug` environment variable for more verbose output
- Scripts use `cargo run` so they work even without installing CTX

## Cleanup

Test scripts create temporary directories but don't delete them (so you can explore). To clean up:

```bash
# macOS/Linux
find /tmp -name "tmp.*" -type d -mtime +1 -exec rm -rf {} +

# Or manually
rm -rf /tmp/tmp.XXXXXXXX
```
