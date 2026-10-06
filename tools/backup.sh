#!/usr/bin/env bash
# Backup of Piuma's essential files for recompilation / project restore.
#
# Creates a timestamped tar.gz that preserves the full folder structure
# (Cargo.toml, Cargo.lock, src/, docs, .git with the local commit history),
# excluding only target/ (12 GB, rebuilt with `cargo build --release`).
# The archive goes to the sibling folder ../piuma-backups, i.e. outside
# the project, so it survives deleting or corrupting the project folder.
#
# Usage:  tools/backup.sh
# Restore: mkdir -p <project> && tar -xzf <archive> -C <project>
set -euo pipefail

# Project root = the folder containing tools/ (runnable from anywhere).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
BACKUP_DIR="$(dirname "$PROJECT_DIR")/piuma-backups"

mkdir -p "$BACKUP_DIR"
ARCHIVE="$BACKUP_DIR/piuma-$(date +%Y%m%d-%H%M%S).tar.gz"

# tar runs from PROJECT_DIR so the member paths are project-relative:
# the extracted tree is a byte-identical copy of the folder structure.
# --exclude matches the whole ./target subtree; .git is kept on purpose
# (there is no remote, the archive is the only off-folder copy).
tar -czf "$ARCHIVE" \
    --exclude='./target' \
    -C "$PROJECT_DIR" \
    .

# Verify the archive reads back and report what went in.
FILES="$(tar -tzf "$ARCHIVE" | wc -l)"
echo "Backup created : $ARCHIVE"
echo "Size           : $(du -h "$ARCHIVE" | cut -f1)"
echo "Entries        : $FILES"
echo "Restore with   : mkdir -p <project> && tar -xzf \"$ARCHIVE\" -C <project>"
