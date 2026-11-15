# wire-shape-release.jq — the release-relative verdict (conventions COMPAT-1, the
# per-entry-point fallback).
#
#   jq -n -c -f wire-shape-release.jq -L <this dir> \
#      --slurpfile release r.json --slurpfile base b.json --slurpfile head h.json
#
# Inputs: three entry-point maps from wire-resolve.py — the last release's contract, the PR's
# base and its head — each {"<key>": <resolved form>}. The comparator is broke_node from
# wire-shape-lib.jq, UNCHANGED: only what it is pointed at changes. Callers MUST pass -L.
#
# broke_node returns a verdict per node and never the breaks themselves, so the doctrine's
# per-entry-point fallback is the rule here: an entry point is CHARGED when
#   release → head breaks it   AND   base ≠ head (this PR changes its resolved form).
# Base → head breaking it is NOT the test: a re-birth (shape the release had, the window
# removed, this PR restores differently) reads there as growth. Base → head is computed only
# to name what the release-relative reading reclassified.
#
# Per entry point:
#   absent at the release            → growth against the release, whatever it does
#                                      (window-born shape moves freely);
#   present at the release, absent at head → removed: a break (charged only if base had it —
#                                      a removal an earlier PR made is not this PR's);
#   present at both                  → broke_node(release; head; "mixed"): parameters and
#                                      request bodies descend as input, responses as output.
# A `component <kind>/<name>` entry is judged only where no entry point reaches it at some
# revision that holds it; its form is compared wherever it exists, so becoming reachable is
# not its removal.
#
# Output: one JSON object
#   {verdict: "moved"|"grew", charged: [key], reclassified_growth: [key], reclassified_break: [key]}
# reclassified_growth: base → head reads a break that the release does not (window-born shape);
# reclassified_break:  charged, while base → head reads growth (a re-birth, or shape that only
#                      the release still shows).
include "wire-shape-lib";

def judged_component($k):
  ($k | startswith("component "))
  and ([ $release[0][$k], $base[0][$k], $head[0][$k] ]
       | any(. != null and (.reached | not)));

def form($m; $k):
  if ($k | startswith("component ")) then ($m[$k] // null | if . == null then null else .form end)
  else $m[$k] end;

def broke($b; $h):
  if $b == null then false
  elif $h == null then true
  else broke_node($b; $h; "mixed") end;

[ ([ $release[0], $base[0], $head[0] ] | map(keys) | add | unique)[] as $k
  | select((($k | startswith("component ")) | not) or judged_component($k))
  | form($release[0]; $k) as $r
  | form($base[0]; $k) as $b
  | form($head[0]; $k) as $h
  | select($b != $h)
  | broke($r; $h) as $rh
  | broke($b; $h) as $bh
  | {key: $k, charged: $rh, base_broke: $bh} ]
| { verdict: (if any(.[]; .charged) then "moved" else "grew" end),
    charged: [ .[] | select(.charged) | .key ],
    reclassified_growth: [ .[] | select(.base_broke and (.charged | not)) | .key ],
    reclassified_break: [ .[] | select(.charged and (.base_broke | not)) | .key ] }
