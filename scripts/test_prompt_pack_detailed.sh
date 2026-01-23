#!/bin/bash
# Detailed prompt pack testing with rich example

set -e

echo "=== Detailed Prompt Pack Testing ==="
echo

# Setup test directory
TEST_DIR=$(mktemp -d)
echo "Test directory: $TEST_DIR"
cd "$TEST_DIR"

# Initialize CTX repo
echo
echo "1. Initializing CTX repository..."
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- init

# Create a more complex project with multiple interconnected modules
echo
echo "2. Creating complex Rust project with multiple modules..."

mkdir -p src

# Module 1: Data models
cat > src/models.rs << 'EOF'
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: u64,
    pub username: String,
    pub email: String,
    pub is_active: bool,
}

impl User {
    pub fn new(id: u64, username: String, email: String) -> Self {
        Self {
            id,
            username,
            email,
            is_active: true,
        }
    }

    pub fn deactivate(&mut self) {
        self.is_active = false;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Post {
    pub id: u64,
    pub author_id: u64,
    pub title: String,
    pub content: String,
    pub published: bool,
}

impl Post {
    pub fn new(id: u64, author_id: u64, title: String, content: String) -> Self {
        Self {
            id,
            author_id,
            title,
            content,
            published: false,
        }
    }

    pub fn publish(&mut self) {
        self.published = true;
    }
}
EOF

# Module 2: Business logic
cat > src/service.rs << 'EOF'
use crate::models::{User, Post};
use crate::storage::Storage;

pub struct UserService {
    storage: Storage,
}

impl UserService {
    pub fn new(storage: Storage) -> Self {
        Self { storage }
    }

    pub fn create_user(&mut self, username: String, email: String) -> Result<User, String> {
        let id = self.storage.next_user_id();
        let user = User::new(id, username, email);
        self.storage.save_user(user.clone());
        Ok(user)
    }

    pub fn get_user(&self, id: u64) -> Option<User> {
        self.storage.get_user(id)
    }

    pub fn deactivate_user(&mut self, id: u64) -> Result<(), String> {
        match self.storage.get_user(id) {
            Some(mut user) => {
                user.deactivate();
                self.storage.save_user(user);
                Ok(())
            }
            None => Err(format!("User {} not found", id)),
        }
    }
}

pub struct PostService {
    storage: Storage,
}

impl PostService {
    pub fn new(storage: Storage) -> Self {
        Self { storage }
    }

    pub fn create_post(&mut self, author_id: u64, title: String, content: String) -> Result<Post, String> {
        // Verify author exists
        if self.storage.get_user(author_id).is_none() {
            return Err(format!("Author {} not found", author_id));
        }

        let id = self.storage.next_post_id();
        let post = Post::new(id, author_id, title, content);
        self.storage.save_post(post.clone());
        Ok(post)
    }

    pub fn publish_post(&mut self, id: u64) -> Result<(), String> {
        match self.storage.get_post(id) {
            Some(mut post) => {
                post.publish();
                self.storage.save_post(post);
                Ok(())
            }
            None => Err(format!("Post {} not found", id)),
        }
    }

    pub fn get_posts_by_author(&self, author_id: u64) -> Vec<Post> {
        self.storage.get_posts_by_author(author_id)
    }
}
EOF

# Module 3: Storage layer
cat > src/storage.rs << 'EOF'
use crate::models::{User, Post};
use std::collections::HashMap;

#[derive(Clone)]
pub struct Storage {
    users: HashMap<u64, User>,
    posts: HashMap<u64, Post>,
    next_user_id: u64,
    next_post_id: u64,
}

impl Storage {
    pub fn new() -> Self {
        Self {
            users: HashMap::new(),
            posts: HashMap::new(),
            next_user_id: 1,
            next_post_id: 1,
        }
    }

    pub fn next_user_id(&mut self) -> u64 {
        let id = self.next_user_id;
        self.next_user_id += 1;
        id
    }

    pub fn next_post_id(&mut self) -> u64 {
        let id = self.next_post_id;
        self.next_post_id += 1;
        id
    }

    pub fn save_user(&mut self, user: User) {
        self.users.insert(user.id, user);
    }

    pub fn get_user(&self, id: u64) -> Option<User> {
        self.users.get(&id).cloned()
    }

    pub fn save_post(&mut self, post: Post) {
        self.posts.insert(post.id, post);
    }

    pub fn get_post(&self, id: u64) -> Option<Post> {
        self.posts.get(&id).cloned()
    }

    pub fn get_posts_by_author(&self, author_id: u64) -> Vec<Post> {
        self.posts
            .values()
            .filter(|p| p.author_id == author_id)
            .cloned()
            .collect()
    }
}
EOF

# Main library file
cat > src/lib.rs << 'EOF'
pub mod models;
pub mod service;
pub mod storage;

pub use models::{User, Post};
pub use service::{UserService, PostService};
pub use storage::Storage;
EOF

cat > Cargo.toml << 'EOF'
[package]
name = "blog-system"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
EOF

# Add narrative
echo
echo "3. Adding narrative context..."
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- add note "Created a blog system with users and posts. The system has three layers: models (User, Post), services (UserService, PostService for business logic), and storage (in-memory HashMap storage). Services verify data integrity (e.g., author must exist before creating post)."

# Commit
echo
echo "4. Creating initial commit..."
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- commit -m "Initial blog system"

# Run analysis
echo
echo "5. Running semantic analysis..."
echo "   This will extract:"
echo "   - Structs: User, Post, UserService, PostService, Storage"
echo "   - Methods and their call relationships"
echo "   - Module imports and dependencies"
echo
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- analyze rust
echo
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- analyze cargo

# Show what edges were created
echo
echo "6. Inspecting created edges..."
echo
echo "Checking edges for UserService:"
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- debug index edges item UserService 2>/dev/null || echo "  (No edges found for UserService)"
echo
echo "Checking edges for User model:"
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- debug index edges item User 2>/dev/null || echo "  (No edges found for User)"

# Check what's in the index
echo
echo "7. Index statistics:"
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- debug index stats

# Now try different query strategies
echo
echo "8. Testing different query strategies..."
echo
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Query 1: By struct name (Item node)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- query "User" --format text | head -80

echo
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Query 2: By file path (File node)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- query "src/models.rs" --format text | head -80

echo
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Query 3: Natural language question"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- query "How does user creation work?" --format text | head -80

echo
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Query 4: JSON output for full details"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- query "PostService" --format json > query_output.json 2>&1 || true

if [ -f query_output.json ]; then
    echo "Retrieved chunks count:"
    grep -o '"retrieved":\s*\[' query_output.json | wc -l
    echo
    echo "Expanded nodes:"
    grep '"expanded_nodes"' query_output.json -A 10 | head -15
    echo
    echo "First retrieved chunk (if any):"
    grep '"title"' query_output.json | head -3
fi

echo
echo "9. Let's check what file paths are in the index:"
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- debug index path src/lib.rs 2>/dev/null || echo "src/lib.rs not found"
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- debug index path src/models.rs 2>/dev/null || echo "src/models.rs not found"
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- debug index path src/service.rs 2>/dev/null || echo "src/service.rs not found"

echo
echo "=== Test Complete ==="
echo "Test directory: $TEST_DIR"
echo
echo "Full JSON output saved to: $TEST_DIR/query_output.json"
echo
echo "Key observations:"
echo "  - Item queries (like 'User') may not retrieve file content if no File edges exist"
echo "  - File path queries (like 'src/models.rs') should retrieve the file content"
echo "  - The graph expansion depends on edges created by analysis"
echo
echo "To investigate further:"
echo "  cd $TEST_DIR"
echo "  cargo run --manifest-path=\"$OLDPWD/Cargo.toml\" --bin ctx -- debug index stats"
echo "  cargo run --manifest-path=\"$OLDPWD/Cargo.toml\" --bin ctx -- debug graph --format dot > graph.dot"
