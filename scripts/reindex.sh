#!/usr/bin/env bash
# Rebuild ~/.config/seal/index.json from the macOS login keychain.
#
# `seal list` reads a local index because keychain APIs cannot enumerate items.
# If that index is missing or stale (fresh machine, restored keychain, entries
# saved by another build), rebuild it from keychain *metadata*: this script only
# reads service/account names, never secret values.
#
#   scripts/reindex.sh          # rebuild and print the keys
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "reindex.sh only supports macOS (uses the security CLI)" >&2
  exit 1
fi

INDEX_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/seal"
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
