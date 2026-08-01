#!/bin/zsh
# Caller-side probe family: real query text through the SHIPPED CLI against prod.
# Usage: ./run_queries.sh <regions_n> <limit> <outfile.jsonl>
set -e
N="$1"; LIM="$2"; OUT="$3"
: > "$OUT"
i=0
while IFS= read -r q; do
  [ -z "$q" ] && continue
  i=$((i+1))
  res=$(TEMPER_FORMAT=json temper search "$q" --wayfind --regions "$N" --limit "$LIM" 2>/dev/null)
  print -r -- "{\"qi\":$i,\"regions\":$N,\"query\":$(print -r -- "$q" | python3 -c 'import json,sys;print(json.dumps(sys.stdin.read().rstrip("\n")))'),\"payload\":$res}" >> "$OUT"
  echo "  [$i] $q" >&2
done < queries.txt
echo "wrote $OUT" >&2
