#!/bin/bash
# Test the semantic analyzer on various Rust code patterns

set -e

echo "=== Analyzer Testing Script ==="
echo

# Setup test directory
TEST_DIR=$(mktemp -d)
echo "Test directory: $TEST_DIR"
cd "$TEST_DIR"

# Initialize CTX repo
echo
echo "1. Initializing CTX repository..."
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- init

# Check analyzer availability
echo
echo "2. Checking analyzer availability..."
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- analyze status

# Create test files with various patterns
echo
echo "3. Creating test files with various patterns..."

mkdir -p src

# File 1: Basic module with functions
cat > src/basic.rs << 'EOF'
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub fn multiply(x: i32, y: i32) -> i32 {
    x * y
}

pub fn complex_calc(n: i32) -> i32 {
    let sum = add(n, 10);
    multiply(sum, 2)
}
EOF

# File 2: Structs and impls
cat > src/structs.rs << 'EOF'
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }

    pub fn distance(&self, other: &Point) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }
}

pub struct Circle {
    center: Point,
    radius: f64,
}

impl Circle {
    pub fn new(center: Point, radius: f64) -> Self {
        Circle { center, radius }
    }

    pub fn area(&self) -> f64 {
        std::f64::consts::PI * self.radius * self.radius
    }
}
EOF

# File 3: Traits and generics
cat > src/traits.rs << 'EOF'
pub trait Drawable {
    fn draw(&self);
}

pub trait Movable {
    fn move_to(&mut self, x: f64, y: f64);
}

pub struct Rectangle {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl Drawable for Rectangle {
    fn draw(&self) {
        println!("Drawing rectangle at ({}, {})", self.x, self.y);
    }
}

impl Movable for Rectangle {
    fn move_to(&mut self, x: f64, y: f64) {
        self.x = x;
        self.y = y;
    }
}

pub fn draw_all<T: Drawable>(items: &[T]) {
    for item in items {
        item.draw();
    }
}
EOF

# File 4: Complex cross-references
cat > src/lib.rs << 'EOF'
mod basic;
mod structs;
mod traits;

pub use basic::{add, multiply, complex_calc};
pub use structs::{Point, Circle};
pub use traits::{Drawable, Movable, Rectangle};

pub fn demo() {
    // Using basic functions
    let result = add(5, 3);
    let doubled = multiply(result, 2);

    // Using structs
    let p1 = Point::new(0.0, 0.0);
    let p2 = Point::new(3.0, 4.0);
    let dist = p1.distance(&p2);

    // Using traits
    let mut rect = Rectangle {
        x: 0.0,
        y: 0.0,
        width: 10.0,
        height: 20.0,
    };
    rect.draw();
    rect.move_to(5.0, 5.0);
}
EOF

cat > Cargo.toml << 'EOF'
[package]
name = "analyzer-test"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/lib.rs"
EOF

echo
echo "4. Analyzing individual files..."
echo

for file in src/*.rs; do
    echo "--- Analyzing $file ---"
    cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- analyze rust "$file" || echo "Analysis failed for $file"
    echo
done

echo
echo "5. Analyzing entire project..."
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- analyze rust

echo
echo "6. Analyzing Cargo metadata..."
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- analyze cargo

# Commit to make edges queryable
echo
echo "7. Committing analysis results..."
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- commit "Analysis test"

# Show what edges were created
echo
echo "=== Generated Edges ==="
echo "Querying for call relationships..."
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- query "show me all function calls" | head -50

echo
echo "Querying for struct definitions..."
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- query "what structs are defined" | head -50

echo
echo "=== Debugging Index ==="
cargo run --manifest-path="$OLDPWD/Cargo.toml" --bin ctx -- debug index stats

echo
echo "=== Test Complete ==="
echo "Test directory preserved at: $TEST_DIR"
echo
echo "To explore further:"
echo "  cd $TEST_DIR"
echo "  cargo run --manifest-path=\"$OLDPWD/Cargo.toml\" --bin ctx -- query 'your question'"
echo "  cargo run --manifest-path=\"$OLDPWD/Cargo.toml\" --bin ctx -- debug index stats"
