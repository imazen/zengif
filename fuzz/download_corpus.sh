#!/bin/bash
# Download additional GIF fuzzing corpora
#
# Sources:
# - dvyukov/go-fuzz-corpus: Go fuzzing corpus with GIF samples
# - peterdn/gif-test-suite: Systematic GIF decoder test suite

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CORPUS_DIR="$SCRIPT_DIR/corpus/external"

mkdir -p "$CORPUS_DIR"

echo "Downloading go-fuzz-corpus GIF samples..."
if [ ! -d "$CORPUS_DIR/go-fuzz-corpus" ]; then
    git clone --depth 1 --filter=blob:none --sparse \
        https://github.com/dvyukov/go-fuzz-corpus.git \
        "$CORPUS_DIR/go-fuzz-corpus"
    cd "$CORPUS_DIR/go-fuzz-corpus"
    git sparse-checkout set gif
    cd "$SCRIPT_DIR"
fi

echo "Downloading gif-test-suite..."
if [ ! -d "$CORPUS_DIR/gif-test-suite" ]; then
    git clone --depth 1 \
        https://github.com/peterdn/gif-test-suite.git \
        "$CORPUS_DIR/gif-test-suite"
fi

# Merge all GIF files into a flat corpus directory
MERGED_DIR="$SCRIPT_DIR/corpus/merged"
mkdir -p "$MERGED_DIR"

echo "Merging corpus files..."
find "$SCRIPT_DIR/corpus/seed" -name "*.gif" -exec cp {} "$MERGED_DIR/" \;
find "$CORPUS_DIR/go-fuzz-corpus" -name "*.gif" -exec cp {} "$MERGED_DIR/" \; 2>/dev/null || true
find "$CORPUS_DIR/gif-test-suite/gifs" -name "*.gif" -exec cp {} "$MERGED_DIR/" \; 2>/dev/null || true

# Also copy any raw corpus files (no extension) from go-fuzz
if [ -d "$CORPUS_DIR/go-fuzz-corpus/gif/corpus" ]; then
    for f in "$CORPUS_DIR/go-fuzz-corpus/gif/corpus"/*; do
        if [ -f "$f" ]; then
            # Hash the filename to avoid conflicts
            hash=$(sha256sum "$f" | cut -c1-12)
            cp "$f" "$MERGED_DIR/gofuzz_$hash"
        fi
    done
fi

COUNT=$(find "$MERGED_DIR" -type f | wc -l)
echo "Corpus ready: $COUNT files in $MERGED_DIR"
echo ""
echo "To run fuzzing with this corpus:"
echo "  cargo fuzz run fuzz_decode corpus/merged/"
echo "  cargo fuzz run fuzz_decode corpus/merged/ -- -dict=gif.dict"
