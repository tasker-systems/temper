# Analysis render harness (`/dev/analysis`)

The receiver half of the graph harness. Everything — why it exists, how to run it, how the fixtures
are captured and sanitized, and what it does not cover — is documented once, next door:

**[`../graph/README.md`](../graph/README.md)**

This route runs off `src/test/fixtures/graph-analysis-anchors.json`, which needed no new capture: it
was already an untrimmed capture of all three shapes the door receives, including a cogmap that has
never materialized a region.
