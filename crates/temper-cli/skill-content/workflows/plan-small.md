# Plan/Small Workflow

## When This Applies

Quick research task. Single-session discovery to answer a specific question, investigate
an approach, or gather information. The output is a written summary, not code. Examples:
evaluating a library, researching an API, reviewing how a subsystem works, checking
feasibility of an approach.

## Steps

1. **Read the task** — find it via `temper resource list --type task` (copy its `ref`), then run `temper resource show <ref>` to load the full task content.
2. **Quick research** — gather information from multiple sources:
   - `temper search "<relevant terms>"` to find related documents in the knowledge base
   - Targeted file reads in the codebase
   - Check recent sessions for prior work: `temper resource list --type session --context @me/<ctx>`
3. **Write up findings** — produce a clear, concise summary that answers the task's
   question. Include:
   - What was investigated
   - Key findings and evidence
   - Recommendations or conclusions
4. **Save findings** — persist the research through temper:
   ```bash
   cat <<'EOF' | temper resource create --type research --title "<title>" --context @me/<ctx>
   <findings content>
   EOF
   ```

## Completion

Pipe the session summary via stdin to save it, then mark the task done:

```bash
cat <<'EOF' | temper resource create --type session --title "<title>" --context @me/<ctx>
## Goal
What we set out to do

## What Happened
Key actions, decisions, and outcomes

## Decisions
Choices made and why

## Connections
Related tasks, concepts, or contexts touched

## Next Steps
What to pick up next session
EOF

temper resource update <ref> --stage done
```
