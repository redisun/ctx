#!/bin/bash
# Test prompt pack generation and show what it outputs

set -e

echo "=== Prompt Pack Testing Script ==="
echo

# Setup test directory
TEST_DIR=$(mktemp -d)
echo "Test directory: $TEST_DIR"
cd "$TEST_DIR"

# Initialize CTX repo
echo
echo "1. Initializing CTX repository..."
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- init

# Create some test files with known content
echo
echo "2. Creating test files..."
mkdir -p src
cat > src/lib.rs << 'EOF'
/// Core library module
pub mod utils {
    /// Utility function that does something
    pub fn helper() -> i32 {
        42
    }
}

pub mod data {
    use super::utils;

    /// Main data structure
    pub struct Config {
        pub name: String,
        pub value: i32,
    }

    impl Config {
        pub fn new(name: String) -> Self {
            Self {
                name,
                value: utils::helper(),
            }
        }
    }
}
EOF

cat > src/main.rs << 'EOF'
use lib::data::Config;

fn main() {
    let config = Config::new("test".to_string());
    println!("Config: {}, {}", config.name, config.value);
}
EOF

cat > Cargo.toml << 'EOF'
[package]
name = "test-project"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1.0", features = ["derive"] }
EOF

# Add some narrative context
echo
echo "3. Adding narrative context..."
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- add note "Created a simple Rust project to test the CTX system. The project has a lib.rs with utility functions and a Config struct, a main.rs that uses the Config, and a basic Cargo.toml with serde dependency. This is a test scenario to see how prompt packs compile context."

# Create initial commit
echo
echo "4. Creating commit..."
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- commit -m "Initial test setup"

# Run analysis to create edges
echo
echo "5. Running semantic analysis..."
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- analyze rust
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- analyze cargo

# Now try to build a pack
echo
echo "6. Building prompt pack..."
echo
echo "Query: 'How does the Config struct work?'"
echo

# Use the query command which internally builds a pack
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- query "How does the Config struct work?" > pack_output.json 2>&1 || true

echo
echo "=== Pack Output ==="
if [ -f pack_output.json ]; then
    cat pack_output.json | head -100
    echo
    echo "(showing first 100 lines, full output in $TEST_DIR/pack_output.json)"
else
    echo "No pack output generated"
fi

# Try to inspect what's in the object store
echo
echo "=== Object Store Contents ==="
echo "Objects in store:"
find .ctx/objects -type f | wc -l

echo
echo "Sample objects (first 5):"
find .ctx/objects -type f | head -5 | while read obj; do
    echo "  - $(basename $(dirname $obj))/$(basename $obj)"
done

# Show index contents
echo
echo "=== Index Contents ==="
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- debug index stats

echo
echo "=== Test Complete ==="
echo "Test directory preserved at: $TEST_DIR"
echo
echo "To explore further:"
echo "  cd $TEST_DIR"
echo "  cargo run --manifest-path=\"$OLDPWD/Cargo.toml\" --bin ctx -- query 'your question'"
echo "  cargo run --manifest-path=\"$OLDPWD/Cargo.toml\" --bin ctx -- debug index stats"
echo "  cargo run --manifest-path=\"$OLDPWD/Cargo.toml\" --bin ctx -- debug cat <object_id>"
