# legacy-guidance/

Byte-exact fixtures of the files the **pre-root-shipping installer** wrote into the installed
skill's `guidance/` directory. The grounding pair now ships at the skill root and install writes
nothing into `guidance/` (the user's namespace), so those copies survive as stale revisions that
the router's "read every file in `guidance/`" step feeds to agents as current.

`install` removes a legacy duplicate only when its bytes hash to the values embedded in
`LEGACY_GUIDANCE_DUPLICATES` (`crates/temper-cli/src/commands/skill.rs`); these fixtures are the
same bytes, so the test witnesses the removal against what the cleanup actually matches. A
`guidance/` file the user has edited no longer matches and is never touched.
