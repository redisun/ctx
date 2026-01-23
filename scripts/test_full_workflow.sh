#!/bin/bash
# Test complete workflow: init -> stage -> commit -> query

set -e

echo "=== Full Workflow Testing Script ==="
echo "This simulates a realistic coding session with CTX"
echo

# Setup test directory
TEST_DIR=$(mktemp -d)
echo "Test directory: $TEST_DIR"
cd "$TEST_DIR"

# Initialize
echo
echo "Step 1: Initialize repository"
echo "$ ctx init"
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- init

# Create initial project
echo
echo "Step 2: Create a new Rust project"
cat > Cargo.toml << 'EOF'
[package]
name = "todo-app"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
EOF

mkdir -p src
cat > src/lib.rs << 'EOF'
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Todo {
    pub id: u64,
    pub title: String,
    pub completed: bool,
}

impl Todo {
    pub fn new(id: u64, title: String) -> Self {
        Self {
            id,
            title,
            completed: false,
        }
    }

    pub fn complete(&mut self) {
        self.completed = true;
    }
}
EOF

# Add narrative
echo
echo "Step 3: Add narrative context"
echo "$ ctx add note 'Created basic Todo struct with id, title, completed fields. Has constructor (new) and complete() method to mark as done. Next: Add TodoList manager struct'"
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- add note "Created basic Todo struct with id, title, completed fields. Has constructor (new) and complete() method to mark as done. Next: Add TodoList manager struct"

# Commit
echo
echo "Step 4: Create first commit"
echo "$ ctx commit -m 'Initial todo structure'"
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- commit -m "Initial todo structure"

# Add more functionality
echo
echo "Step 5: Add TodoList manager"
cat > src/manager.rs << 'EOF'
use crate::Todo;

pub struct TodoList {
    todos: Vec<Todo>,
    next_id: u64,
}

impl TodoList {
    pub fn new() -> Self {
        Self {
            todos: Vec::new(),
            next_id: 1,
        }
    }

    pub fn add(&mut self, title: String) -> &Todo {
        let todo = Todo::new(self.next_id, title);
        self.next_id += 1;
        self.todos.push(todo);
        self.todos.last().unwrap()
    }

    pub fn complete(&mut self, id: u64) -> Result<(), String> {
        if let Some(todo) = self.todos.iter_mut().find(|t| t.id == id) {
            todo.complete();
            Ok(())
        } else {
            Err(format!("Todo {} not found", id))
        }
    }

    pub fn list(&self) -> &[Todo] {
        &self.todos
    }
}
EOF

# Update lib.rs to include manager
cat > src/lib.rs << 'EOF'
use serde::{Deserialize, Serialize};

mod manager;
pub use manager::TodoList;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Todo {
    pub id: u64,
    pub title: String,
    pub completed: bool,
}

impl Todo {
    pub fn new(id: u64, title: String) -> Self {
        Self {
            id,
            title,
            completed: false,
        }
    }

    pub fn complete(&mut self) {
        self.completed = true;
    }
}
EOF

# Add narrative and commit
echo
echo "Step 6: Add narrative for this iteration"
echo "$ ctx add note 'Added TodoList struct to manage multiple todos: add() creates new todo, complete() marks todo as done, list() returns all todos. The TodoList owns a Vec<Todo> and manages ID generation.'"
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- add note "Added TodoList struct to manage multiple todos: add() creates new todo, complete() marks todo as done, list() returns all todos. The TodoList owns a Vec<Todo> and manages ID generation."

echo
echo "Step 7: Commit changes"
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- commit -m "Add TodoList manager"

# Run analysis
echo
echo "Step 8: Run semantic analysis"
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- analyze rust
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- analyze cargo

# Query the context
echo
echo "Step 9: Query the context"
echo
echo "Q: How does the Todo completion work?"
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- query "How does the Todo completion work?" | head -100

echo
echo
echo "Q: What is the relationship between TodoList and Todo?"
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- query "What is the relationship between TodoList and Todo?" | head -100

# Show history
echo
echo
echo "=== Commit History ==="
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- debug history

# Show stats
echo
echo "=== Repository Statistics ==="
echo "Objects:"
find .ctx/objects -type f | wc -l
echo
echo "Commits:"
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- debug history | grep -c "Commit" || echo "0"
echo
echo "Narrative entries:"
find .ctx/narrative/log -type f 2>/dev/null | wc -l

echo
echo "=== Test Complete ==="
echo "Test directory preserved at: $TEST_DIR"
echo
echo "To explore further:"
echo "  cd $TEST_DIR"
echo "  cargo run --manifest-path=\"$OLDPWD/Cargo.toml\" --bin ctx -- query 'your question'"
echo "  cargo run --manifest-path=\"$OLDPWD/Cargo.toml\" --bin ctx -- debug index stats"
