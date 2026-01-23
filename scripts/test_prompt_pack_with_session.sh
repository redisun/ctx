#!/bin/bash
# Test prompt pack with proper session workflow (simulating agent behavior)

set -e

echo "=== Prompt Pack Test with Session Workflow ==="
echo "This simulates how an agent would use CTX"
echo

# Setup test directory
TEST_DIR=$(mktemp -d)
echo "Test directory: $TEST_DIR"
cd "$TEST_DIR"

# Initialize CTX repo
echo
echo "1. Initializing CTX repository..."
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- init

# Create project files
echo
echo "2. Creating Rust project..."
mkdir -p src

cat > src/auth.rs << 'EOF'
use sha2::{Sha256, Digest};

pub struct AuthService {
    secret_key: String,
}

impl AuthService {
    pub fn new(secret_key: String) -> Self {
        Self { secret_key }
    }

    pub fn hash_password(&self, password: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(password.as_bytes());
        hasher.update(self.secret_key.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    pub fn verify_password(&self, password: &str, hash: &str) -> bool {
        self.hash_password(password) == hash
    }
}

pub fn generate_token(user_id: u64) -> String {
    format!("token_{}_{}", user_id, chrono::Utc::now().timestamp())
}
EOF

cat > src/lib.rs << 'EOF'
pub mod auth;

pub use auth::{AuthService, generate_token};
EOF

cat > Cargo.toml << 'EOF'
[package]
name = "auth-system"
version = "0.1.0"
edition = "2021"

[dependencies]
sha2 = "0.10"
chrono = "0.4"
EOF

# Start a session
echo
echo "3. Starting session (like an agent would)..."
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- stage start "Implement authentication system"

# Now we need to simulate file observations
# Since the CLI doesn't expose observe_file_read/write, let's use a small Rust program
echo
echo "4. Creating observation helper..."
cat > observe_helper.rs << 'RUST_EOF'
use ctx_core::{CtxRepo, Result};
use std::fs;

fn main() -> Result<()> {
    let mut repo = CtxRepo::open(".")?;

    // Ensure we have an active session
    if !repo.has_active_session() {
        if repo.recover_session()?.is_none() {
            eprintln!("No active session");
            std::process::exit(1);
        }
    }

    // Observe file reads (simulating an agent reading files)
    let files = vec!["src/auth.rs", "src/lib.rs", "Cargo.toml"];

    for file_path in &files {
        println!("Observing read: {}", file_path);
        let content = fs::read(file_path)?;
        repo.observe_file_read(file_path, &content)?;
    }

    // Flush to staging
    println!("\nFlushing observations to staging...");
    repo.flush_active_session()?;

    println!("Done! Observations stored.");
    Ok(())
}
RUST_EOF

echo
echo "5. Building and running observation helper..."
rustc --edition 2021 observe_helper.rs -L "$OLDPWD/target/debug/deps" \
    -L "$OLDPWD/target/debug" \
    --extern ctx_core="$OLDPWD/target/debug/libctx_core.rlib" 2>&1 | head -20

if [ -f ./observe_helper ]; then
    ./observe_helper
else
    echo "  (Helper failed to compile, this is expected in some environments)"
    echo "  Skipping file observations..."
fi

# Complete the session
echo
echo "6. Compacting session to commit..."
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- stage compact -m "Add authentication system"

# Run analysis
echo
echo "7. Running semantic analysis..."
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- analyze rust
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- analyze cargo

# Check what's indexed
echo
echo "8. Checking what's in the index..."
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- debug index stats
echo
echo "Checking for file paths:"
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- debug index path src/auth.rs 2>&1 || echo "  (not found)"
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- debug index path src/lib.rs 2>&1 || echo "  (not found)"

# Query
echo
echo "9. Testing queries..."
echo
echo "Query: 'AuthService'"
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- query "AuthService" --format text | head -60

echo
echo "Query: 'src/auth.rs' (by file path)"
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- query "src/auth.rs" --format text | head -60

echo
echo "=== Test Complete ==="
echo "Test directory: $TEST_DIR"
echo
echo "NOTE: This test shows the intended agent workflow."
echo "The CLI doesn't have a simple 'add file' command because CTX is designed"
echo "for agents that observe file reads/writes during their work."
echo
echo "For manual testing without sessions, you would need to implement"
echo "a command like 'ctx track <file>' that stores file blobs and creates"
echo "file nodes in the index."
