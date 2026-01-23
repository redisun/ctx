#!/bin/bash
# Inspect object store contents

set -e

echo "=== Object Store Inspector ==="
echo

if [ ! -d ".ctx" ]; then
    echo "Error: Not in a CTX repository (no .ctx directory found)"
    exit 1
fi

CTX_ROOT=".ctx"
OBJECTS_DIR="$CTX_ROOT/objects"

if [ ! -d "$OBJECTS_DIR" ]; then
    echo "Error: No objects directory found"
    exit 1
fi

echo "Repository: $(pwd)"
echo "CTX root: $CTX_ROOT"
echo

# Count objects
TOTAL_OBJECTS=$(find "$OBJECTS_DIR" -type f | wc -l)
echo "Total objects: $TOTAL_OBJECTS"
echo

# Show shard distribution
echo "=== Shard Distribution ==="
for shard in "$OBJECTS_DIR"/*; do
    if [ -d "$shard" ]; then
        shard_name=$(basename "$shard")
        count=$(find "$shard" -type f | wc -l)
        printf "  %s: %3d objects\n" "$shard_name" "$count"
    fi
done
echo

# Show HEAD
echo "=== HEAD ==="
if [ -f "$CTX_ROOT/HEAD" ]; then
    head_commit=$(cat "$CTX_ROOT/HEAD")
    echo "Current HEAD: $head_commit"
else
    echo "No HEAD found"
fi
echo

# Show refs
echo "=== References ==="
if [ -d "$CTX_ROOT/refs" ]; then
    find "$CTX_ROOT/refs" -type f | while read ref; do
        ref_name=$(echo "$ref" | sed "s|$CTX_ROOT/refs/||")
        ref_value=$(cat "$ref")
        echo "  $ref_name: $ref_value"
    done
else
    echo "No refs directory"
fi
echo

# Show staging status
echo "=== Staging Status ==="
if [ -f "$CTX_ROOT/STAGE" ]; then
    echo "Active staging session:"
    cat "$CTX_ROOT/STAGE"
else
    echo "No active staging session"
fi
echo

# Sample a few objects
echo "=== Sample Objects ==="
find "$OBJECTS_DIR" -type f | head -5 | while read obj; do
    obj_id=$(basename "$obj")
    shard=$(basename $(dirname "$obj"))
    size=$(stat -f%z "$obj" 2>/dev/null || stat -c%s "$obj" 2>/dev/null || echo "?")

    echo "Object: $shard/$obj_id"
    echo "  Size (compressed): $size bytes"

    # Try to decompress and show header
    if command -v zstd >/dev/null 2>&1; then
        header=$(zstd -d < "$obj" 2>/dev/null | head -c 20 | xxd -p | tr -d '\n')
        magic=$(zstd -d < "$obj" 2>/dev/null | head -c 5)
        echo "  Header (hex): $header"
        if [ "$magic" = "CTXO1" ]; then
            echo "  Magic: CTXO1 ✓"
            kind_byte=$(zstd -d < "$obj" 2>/dev/null | head -c 6 | tail -c 1 | xxd -p)
            echo "  Kind byte: 0x$kind_byte"
        else
            echo "  Magic: Unknown (expected CTXO1)"
        fi
    fi
    echo
done

# Show storage size
echo "=== Storage Statistics ==="
total_size=$(du -sh "$OBJECTS_DIR" 2>/dev/null | cut -f1)
echo "Total storage used: $total_size"

# Check compression ratio if possible
if command -v zstd >/dev/null 2>&1; then
    echo
    echo "Sample compression ratios (first 5 objects):"
    find "$OBJECTS_DIR" -type f | head -5 | while read obj; do
        compressed=$(stat -f%z "$obj" 2>/dev/null || stat -c%s "$obj" 2>/dev/null || echo "0")
        uncompressed=$(zstd -d < "$obj" 2>/dev/null | wc -c | tr -d ' ')
        if [ "$compressed" != "0" ] && [ "$uncompressed" != "0" ]; then
            ratio=$(echo "scale=2; $uncompressed / $compressed" | bc)
            echo "  $ratio:1"
        fi
    done
fi

echo
echo "=== Inspection Complete ==="
