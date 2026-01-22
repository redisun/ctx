# Contributing to CTX

Thank you for your interest in contributing to CTX! This document provides guidelines and instructions for contributing.

## Getting Started

### Prerequisites

- Rust 1.75 or later (check with `rustc --version`)
- Git
- A code editor (VS Code with rust-analyzer recommended)

### Setting Up the Development Environment

1. **Clone the repository:**
   ```bash
   git clone https://github.com/yourorg/ctx.git
   cd ctx
   ```

2. **Build the project:**
   ```bash
   cargo build
   ```

3. **Run tests:**
   ```bash
   cargo test
   ```

4. **Run the CLI (for testing):**
   ```bash
   cargo run -p ctx_cli -- --help
   ```

## Project Structure

```
ctx/
├── crates/
│   ├── ctx_core/      # Core library (main crate)
│   └── ctx_cli/        # CLI for debugging/manual ops
├── tests/
│   └── e2e/            # End-to-end integration tests
├── docs/               # Documentation
├── tests/fixtures/     # Test fixtures
└── Cargo.toml          # Workspace manifest
```

### Key Modules

- `ctx_core/src/object_store.rs` - Content-addressed storage
- `ctx_core/src/session.rs` - Session lifecycle management
- `ctx_core/src/repo.rs` - Main repository API
- `ctx_core/src/pack.rs` - Prompt pack compilation
- `ctx_core/src/graph.rs` - Relationship graph traversal
- `ctx_core/src/index.rs` - Fast lookup indexes

## Development Workflow

### Making Changes

1. **Create a branch:**
   ```bash
   git checkout -b feature/your-feature-name
   # or
   git checkout -b fix/your-bug-fix
   ```

2. **Make your changes** following the coding standards below

3. **Run tests:**
   ```bash
   # Run all tests
   cargo test
   
   # Run specific test suite
   cargo test --test e2e
   
   # Run with output
   cargo test -- --nocapture
   ```

4. **Check formatting:**
   ```bash
   cargo fmt --check
   ```

5. **Check lints:**
   ```bash
   cargo clippy --all-targets -- -D warnings
   ```

6. **Commit your changes** (see commit message guidelines below)

7. **Push and create a pull request**

## Code Style

### Rust Style

- Follow standard Rust formatting (enforced by `cargo fmt`)
- Use `cargo clippy` to catch common issues
- Prefer explicit error handling over panics in library code
- Use `thiserror` for library errors, `anyhow` for application errors

### Naming Conventions

- Types: `PascalCase` (e.g., `CtxRepo`, `SessionState`)
- Functions: `snake_case` (e.g., `start_session`, `observe_file_read`)
- Constants: `SCREAMING_SNAKE_CASE` (e.g., `MAX_RETRIES`)
- Modules: `snake_case` (e.g., `object_store`, `rust_parse`)

### Documentation

- All public APIs must have doc comments
- Use `///` for public items, `//!` for module-level docs
- Include examples in doc comments where helpful:
  ```rust
  /// Creates a new session for tracking work.
  ///
  /// # Examples
  ///
  /// ```
  /// let mut repo = CtxRepo::open(".")?;
  /// let session = repo.start_session("Add feature")?;
  /// ```
  pub fn start_session(&mut self, task: &str) -> Result<&mut Session> {
      // ...
  }
  ```

### Error Handling

- Library code (`ctx_core`): Return `Result<T, CtxError>` using `thiserror`
- Application code (`ctx_cli`): Use `anyhow::Result<T>`
- Always provide context with errors:
  ```rust
  fs::read(&path)
      .with_context(|| format!("Failed to read file: {}", path.display()))?;
  ```

### Deterministic Serialization

**Critical:** All serialized data must be deterministic. Same input = same bytes.

- Use `BTreeMap` instead of `HashMap`
- Use `BTreeSet` instead of `HashSet`
- Sort collections before serializing if needed
- Never use floating point with NaN (breaks determinism)

```rust
// ✅ CORRECT
#[derive(Serialize, Deserialize)]
struct Config {
    settings: BTreeMap<String, String>,
}

// ❌ WRONG
#[derive(Serialize, Deserialize)]
struct Config {
    settings: HashMap<String, String>, // Non-deterministic iteration
}
```

## Testing

### Test Structure

- **Unit tests:** In each module with `#[cfg(test)] mod tests`
- **Integration tests:** In `tests/e2e/` directory
- **Fixtures:** In `tests/fixtures/` directory

### Writing Tests

1. **Unit tests** should test individual functions:
   ```rust
   #[cfg(test)]
   mod tests {
       use super::*;
       
       #[test]
       fn test_basic_functionality() {
           // Test code
       }
   }
   ```

2. **Integration tests** use the scenario DSL:
   ```rust
   #[test]
   fn test_feature() {
       Scenario::new("test_name")
           .from_fixture("default")
           .user_starts_task("Task description")
           .agent_writes("file.rs", b"content")
           .agent_flushes()
           .agent_completes("Done")
           .user_confirms()
           .assert_commit_count(2)
           .run()
           .unwrap();
   }
   ```

### Running Tests

```bash
# All tests
cargo test

# Specific crate
cargo test -p ctx_core

# Specific test
cargo test test_name

# E2E tests
cargo test --test e2e

# With output
cargo test -- --nocapture
```

### Test Requirements

- All tests must pass before submitting PR
- New features must include tests
- Bug fixes must include regression tests
- Aim for high code coverage

## Commit Messages

Follow conventional commit format:

```
<type>: <short description>

<longer explanation if needed>

<type> is one of:
- feat: New feature
- fix: Bug fix
- refactor: Code restructuring
- test: Adding tests
- docs: Documentation
- chore: Maintenance
- perf: Performance improvement
```

### Examples

```
feat: Add support for stale session auto-compaction

Implements automatic compaction of sessions that have been idle
for more than 7 days. Sessions are compacted with a special
commit type indicating they were auto-saved.

fix: Correct object store path resolution on Windows

The shard directory calculation was using forward slashes on
Windows, causing object lookups to fail. Now uses PathBuf
for proper cross-platform path handling.
```

## Pull Request Process

1. **Update documentation** if you've changed APIs or behavior
2. **Add tests** for new features or bug fixes
3. **Ensure all tests pass** (`cargo test`)
4. **Check formatting** (`cargo fmt --check`)
5. **Check lints** (`cargo clippy`)
6. **Update CHANGELOG.md** if applicable (if it exists)
7. **Create PR** with a clear description

### PR Description Template

```markdown
## Description
Brief description of changes

## Type of Change
- [ ] Bug fix
- [ ] New feature
- [ ] Breaking change
- [ ] Documentation update

## Testing
How was this tested?

## Checklist
- [ ] Tests pass
- [ ] Code formatted
- [ ] Documentation updated
- [ ] No breaking changes (or documented)
```

## Critical Invariants

When contributing, be aware of these critical invariants:

### Object Store

- **Immutability:** Objects are never modified after creation
- **Content addressing:** Object ID = BLAKE3(canonical_bytes)
- **Atomic writes:** Always use temp file + rename pattern
- **Deduplication:** Same content = same ID

### Serialization

- **Deterministic:** Same value = same bytes every time
- **No HashMap/HashSet:** Use BTreeMap/BTreeSet or sorted Vec
- **No NaN floats:** Breaks determinism

### Session Management

- Only one active session per repository
- Session always has valid `base` commit
- Staging chain always walkable back to base
- `flush_step()` must be called before state queries

## Getting Help

- Check existing documentation in `docs/`
- Review `AGENTS.md` for agent-specific guidance
- Review `CLAUDE.md` for Claude-specific instructions
- Open an issue for questions or discussions

## License

By contributing, you agree that your contributions will be licensed under the same dual license (MIT OR Apache-2.0) as the project.

## Code of Conduct

- Be respectful and inclusive
- Welcome newcomers and help them learn
- Focus on constructive feedback
- Respect different viewpoints and experiences

Thank you for contributing to CTX! 🎉
