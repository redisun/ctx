#!/bin/bash
# Interactive demo of CTX features

set -e

echo "╔════════════════════════════════════════════════════════╗"
echo "║         CTX Interactive Demo                          ║"
echo "║  Git-like Context Management for Coding Agents        ║"
echo "╚════════════════════════════════════════════════════════╝"
echo

# Function to wait for user
wait_for_user() {
    echo
    echo -n "Press Enter to continue..."
    read
    echo
}

# Setup test directory
TEST_DIR=$(mktemp -d)
echo "Setting up demo in: $TEST_DIR"
cd "$TEST_DIR"
wait_for_user

# Demo 1: Initialization
echo "════════════════════════════════════════════════════════"
echo "DEMO 1: Repository Initialization"
echo "════════════════════════════════════════════════════════"
echo
echo "Let's initialize a CTX repository:"
echo "$ ctx init"
echo
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- init
echo
echo "✓ Repository initialized!"
echo "  Created .ctx/ directory with:"
echo "  - Object store (content-addressed)"
echo "  - Index (for fast queries)"
echo "  - Narrative space (human-readable logs)"
ls -la .ctx/
wait_for_user

# Demo 2: Creating Files
echo "════════════════════════════════════════════════════════"
echo "DEMO 2: Creating Source Files"
echo "════════════════════════════════════════════════════════"
echo
echo "Creating a simple Rust file:"
cat > example.rs << 'EOF'
pub struct User {
    pub id: u64,
    pub name: String,
}

impl User {
    pub fn new(id: u64, name: String) -> Self {
        Self { id, name }
    }
}
EOF
echo
cat example.rs
echo
echo "✓ File created!"
echo
echo "Note: CTX doesn't explicitly track files like Git."
echo "Instead, it discovers them through analysis."
wait_for_user

# Demo 3: Narrative
echo "════════════════════════════════════════════════════════"
echo "DEMO 3: Adding Narrative Context"
echo "════════════════════════════════════════════════════════"
echo
echo "CTX lets you add human-readable context alongside code:"
echo "$ ctx add note 'Created a basic User struct...'"
echo
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- add note "Created a basic User struct with id and name fields. Has a new() constructor. Next steps: Add authentication fields and implement validation logic."
echo
echo "✓ Narrative added!"
echo
echo "This creates a timestamped entry in today's log file"
echo "that future agents can read to understand context."
wait_for_user

# Demo 4: Commit
echo "════════════════════════════════════════════════════════"
echo "DEMO 4: Creating a Commit"
echo "════════════════════════════════════════════════════════"
echo
echo "Now let's commit our changes:"
echo "$ ctx commit --message 'Add User model'"
echo
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- commit --message "Add User model"
echo
echo "✓ Commit created!"
echo
echo "The commit is stored as a content-addressed object."
echo "It contains:"
echo "  - File snapshots"
echo "  - Narrative references"
echo "  - Semantic relationships (edges)"
echo "  - Parent commit(s)"
wait_for_user

# Demo 5: Semantic Analysis
echo "════════════════════════════════════════════════════════"
echo "DEMO 5: Semantic Analysis"
echo "════════════════════════════════════════════════════════"
echo
echo "CTX can analyze Rust code to extract semantic relationships:"
echo "$ ctx analyze rust example.rs"
echo
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- analyze rust example.rs || echo "  (rust-analyzer not available, but would extract function calls, type definitions, etc.)"
echo
echo "This creates edges in the graph like:"
echo "  - User --defines--> new()"
echo "  - User --has-field--> id"
echo "  - User --has-field--> name"
wait_for_user

# Demo 6: Query
echo "════════════════════════════════════════════════════════"
echo "DEMO 6: Querying Context"
echo "════════════════════════════════════════════════════════"
echo
echo "Let's create more code to query:"
cat > auth.rs << 'EOF'
use example::User;

pub fn authenticate(user: &User, password: &str) -> bool {
    // TODO: implement real authentication
    !password.is_empty()
}

pub fn create_session(user: User) -> String {
    format!("session_{}", user.id)
}
EOF

cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- add note "Added basic authentication functions: authenticate() validates user credentials, create_session() creates session token. Currently using placeholder logic."
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- commit -m "Add auth module"

echo
echo "Now let's query the context:"
echo "$ ctx query 'How is User used?'"
echo
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- query "How is User used?" | head -50 || echo "  (Would show graph expansion and retrieved code snippets)"
echo
echo "The query:"
echo "  1. Parses the question to find seed nodes"
echo "  2. Expands the graph to find related code"
echo "  3. Retrieves relevant file contents"
echo "  4. Compiles a prompt pack for an LLM"
wait_for_user

# Demo 7: Object Store
echo "════════════════════════════════════════════════════════"
echo "DEMO 7: Content-Addressed Storage"
echo "════════════════════════════════════════════════════════"
echo
echo "All data is stored as content-addressed objects:"
echo
find .ctx/objects -type f | head -5 | while read obj; do
    shard=$(basename $(dirname "$obj"))
    id=$(basename "$obj")
    size=$(stat -f%z "$obj" 2>/dev/null || stat -c%s "$obj" 2>/dev/null)
    echo "  $shard/$id ($size bytes)"
done
echo
echo "Objects are:"
echo "  - Immutable (never modified)"
echo "  - Deduplicated (same content = same hash)"
echo "  - Compressed (using zstd)"
echo "  - Sharded (first byte for distribution)"
wait_for_user

# Demo 8: Graph Structure
echo "════════════════════════════════════════════════════════"
echo "DEMO 8: Relationship Graph"
echo "════════════════════════════════════════════════════════"
echo
echo "CTX builds a graph of semantic relationships:"
echo
echo "Nodes represent:"
echo "  - Files (file://src/lib.rs)"
echo "  - Functions (fn://module::function)"
echo "  - Types (type://MyStruct)"
echo "  - Modules (mod://crate::module)"
echo
echo "Edges represent:"
echo "  - calls: function A calls function B"
echo "  - defines: module defines function"
echo "  - imports: file imports module"
echo "  - uses: function uses type"
echo
echo "This enables intelligent context retrieval!"
wait_for_user

# Summary
echo "════════════════════════════════════════════════════════"
echo "DEMO COMPLETE"
echo "════════════════════════════════════════════════════════"
echo
echo "You've seen:"
echo "  ✓ Repository initialization"
echo "  ✓ File tracking"
echo "  ✓ Narrative context"
echo "  ✓ Commits"
echo "  ✓ Semantic analysis"
echo "  ✓ Querying"
echo "  ✓ Content-addressed storage"
echo "  ✓ Relationship graphs"
echo
echo "Demo directory: $TEST_DIR"
echo
echo "Try exploring:"
echo "  cd $TEST_DIR"
echo "  cargo run --manifest-path=\"$OLDPWD/Cargo.toml\" --bin ctx -- debug index stats"
echo "  cargo run --manifest-path=\"$OLDPWD/Cargo.toml\" --bin ctx -- debug history"
echo "  cargo run --manifest-path=\"$OLDPWD/Cargo.toml\" --bin ctx -- query 'your question'"
echo
echo "Thank you for trying CTX!"
