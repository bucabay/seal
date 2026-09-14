#!/usr/bin/env bash
# Rebuild the seal name index from the macOS login keychain.
#
# Mostly obsolete: `seal list` now enumerates the keychain itself on macOS and
# rewrites the index every time, so a missing or stale index repairs itself.
# Kept as a standalone repair tool for machines still running an older `seal`
# (check with `seal --version`), where `list` trusts the index without checking
# the keychain. Like `seal list`, it reads service/account *names*, never values.
#
#   scripts/reindex.sh          # rebuild and print the keys
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "reindex.sh only supports macOS (uses the security CLI)" >&2
  exit 1
fi

# Must match Rust's dirs::config_dir(), which on macOS is Application Support —
# *not* ~/.config. Writing to ~/.config here made this script a silent no-op.
INDEX_DIR="$HOME/Library/Application Support/seal"
INDEX="$INDEX_DIR/index.json"
mkdir -p "$INDEX_DIR"

# dump-keychain prints one attribute block per item. Items seal created have
# svce="seal" and acct="<vault>:<key>". Values are never printed without -d.
security dump-keychain 2>/dev/null \
  | awk '
      /^keychain:/            { acct=""; svce="" }
      /"acct"<blob>=/         { sub(/^.*"acct"<blob>="/, ""); sub(/"$/, ""); acct=$0 }
      /"svce"<blob>=/         { sub(/^.*"svce"<blob>="/, ""); sub(/"$/, ""); svce=$0
                                if (svce=="seal" && acct!="") print acct; acct=""; svce="" }
    ' \
  | sort -u \
  | python3 -c '
import json, sys
index = {}
for line in sys.stdin:
    line = line.rstrip("\n")
    if ":" not in line:
        continue
    vault, key = line.split(":", 1)
    index.setdefault(vault, []).append(key)
for v in index:
    index[v].sort()
sys.stdout.write(json.dumps(dict(sorted(index.items())), indent=2) + "\n")
' > "$INDEX.tmp"

mv "$INDEX.tmp" "$INDEX"
count=$(python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(sum(len(v) for v in d.values()))' "$INDEX")
echo "wrote $INDEX ($count keys)"
seal list
