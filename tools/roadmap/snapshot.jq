# The snapshot's body, from the records snapshot.sh assembled (or fixture/records.json).
# Run: jq -j -L tools/roadmap --arg started <t> --arg finished <t> --rawfile recipe tools/roadmap/recipe.md \
#        -f tools/roadmap/snapshot.jq <records.json>
# A partial or out-of-shape read is an error: jq exits non-zero and prints nothing on stdout.
include "roadmap";
snapshot($started; $finished; $recipe)
