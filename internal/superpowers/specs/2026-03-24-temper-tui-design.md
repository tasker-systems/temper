# Temper TUI Design Specification

## Overview

Replace the static `temper ticket board` command with an interactive terminal UI (`temper tui`) built on ratatui and crossterm. The TUI provides navigable swimlanes for tickets by project and milestone, semantic search and context graph exploration, a unified document viewer for all vault content, and inline mutation of ticket/milestone metadata. Text editing defers to `$EDITOR`.

## Architecture

### Three-Layer Split

The implementation restructures the codebase into three layers:

```
src/
  actions/       # Pure data logic — fetch, mutate, return typed results
  commands/      # CLI wrappers: parse args → call action → format output
  tui/           # Ratatui app: render state → call action → update state
```

**`src/actions/`** — One module per domain: `ticket.rs`, `search.rs`, `context.rs`, `milestone.rs`, `normalize.rs`, `index.rs`, `session.rs`, etc. Each function takes `&Config` (plus domain-specific arguments) and returns typed results (`Vec<TicketInfo>`, `SearchResults`, etc.). No printing, no formatting. Types mostly exist already in `vault.rs` and command modules — they need to be lifted out.

**`src/commands/`** — Thin CLI wrappers. Each command parses clap args, calls the corresponding action, formats the result as text or JSON using the existing `output::*` helpers. The `ticket board` subcommand is removed.

**`src/tui/`** — Ratatui application. Owns the terminal event loop, screen state, and rendering. Calls actions to load and mutate data, holds results in app state, renders via ratatui widgets.

Both `commands/` and `tui/` are consumers of `actions/`. Neither contains business logic.

### Execution Strategy

Start with the target architecture (actions extraction) for the commands the TUI needs — ticket, milestone, search, context, index, normalize. Extract each action as the TUI consumes it. Commands the TUI doesn't touch (init, check, status, events, session, research, skill, note, project, warmup) can be migrated later or left in place — they still work.

### Async Model

The TUI uses a channel-based actor pattern for non-blocking queries:

```
TUI thread  ──QueryRequest──→  Query Actor  ──QueryResult──→  TUI thread
 (render +                      (tokio task,                   (update
  input)                         owns embedder                  state)
                                 + HNSW index)
```

**TUI thread** owns the terminal via crossterm, renders frames, handles keyboard input. On search/context keystrokes, sends a `QueryRequest` down a tokio mpsc channel.

**Query actor** runs on a dedicated `std::thread` (not a tokio task) because the embedder runs synchronous Candle inference on CPU and the HNSW index is a blocking data structure. The actor owns a `tokio::sync::mpsc::Receiver<QueryRequest>`, receives requests, debounces (drops stale requests if a new one arrives), runs the query, and sends results back via a `tokio::sync::mpsc::Sender<QueryResult>`. Using a dedicated thread rather than `spawn_blocking` ensures the embedder and index stay on a single thread without repeated task spawning.

**Main event loop** uses `tokio::select!` to poll both crossterm events and the results channel. The UI never blocks on a query.

The `tui` subcommand path initializes a tokio runtime (`#[tokio::main]` or `Runtime::new()`). Other CLI subcommands remain synchronous — no global async migration required.

```rust
enum QueryRequest {
    Search { query: String },
    Context { topic: String, depth: usize, limit: usize },
    Index { force: bool },
    Normalize { project: Option<String>, dry_run: bool, fix_slugs: bool },
}

enum QueryResult {
    SearchResults(Vec<SearchHit>),
    ContextResults { center: String, neighbors: Vec<Neighbor> },
    IndexComplete { stats: IndexStats },
    NormalizeComplete { summary: NormalizeSummary },
    Progress { message: String, pct: Option<f32> },
    Error(String),
}
```

Search debounce: ~150ms. If a new `Search` request arrives while the previous is in-flight, the actor drops the stale result.

Index and normalize are longer-running operations that send `Progress` messages back to the TUI for inline status display.

## UX Design

### Core Philosophy

**Single-pane focus.** One thing owns the screen at a time. No persistent sidebars. Tabs switch modes, Enter drills in, Esc pops back. This mirrors the CLI mental model — one command, one output — but with navigation between them.

### Navigation: Tabs + Breadcrumb Drill-down

Four tabs across the top: **Board** · **Search** · **Context** · **Maintain**

Each tab owns the full terminal width. Breadcrumb navigation within the Board tab drills from project → milestone → swimlanes. Tab switches reset the navigation stack to that tab's root.

The TUI infers the current project from CWD (matching existing `temper` CLI behavior) and lands on that project's milestone list. Users can navigate up to the all-projects view or sideways to other projects.

### Screen Stack

The app maintains a `Vec<Screen>` navigation stack:

```rust
enum Screen {
    Board(BoardState),    // project list → milestones → swimlanes
    Search(SearchState),  // query input + ranked results
    Context(ContextState),// centered topic + neighbor list with center stack
    Maintain(MaintainState), // index stats, normalize, action triggers
    Viewer(ViewerState),  // full-screen document (any vault file)
}
```

- `Enter` pushes a new screen (drill into milestone, view ticket, view search result)
- `Esc` pops back to the previous screen
- Tab switches (`1`-`4` or `:board`/`:search`/`:context`/`:maintain`) replace the entire stack with a fresh root for that tab. Previous tab state is discarded — this keeps the mental model simple and avoids stale views

### Board Tab

**Level 0: Project list** — shown when navigating up past the inferred project. Simple list of configured projects with ticket counts.

**Level 1: Milestones** — the default landing view. Lists milestones for the selected project with summary counts (N backlog, N in-progress, N done). Includes an "(unassigned)" group for tickets without a milestone. `h`/`l` cycles between projects at this level.

**Level 2: Swimlanes** — three-column kanban view (backlog, in-progress, done) for the selected milestone. `h`/`l` moves between columns, `j`/`k` moves within a column. Tickets show scope label and truncated title. Cancelled tickets are excluded from swimlanes — they are a terminal state with low browse value. They remain accessible via search.

### Search Tab

Input field at top with live search. Typing sends queries to the actor with 150ms debounce. Results appear as a ranked list showing: similarity score, file path, type badge (ticket/session/note/milestone), and a snippet.

`Tab` or `↓` moves focus from input to results list. `/` from the results list refocuses the input. `Enter` on a result opens the document in the Viewer. `c` on a result pivots to the Context tab centered on that item.

All vault content is searchable — tickets, sessions, research notes, concepts, milestones.

### Context Tab

Centered on a topic or entity, showing HNSW neighbors grouped by traversal depth. `/` to enter a topic, or arrive here via `c` from another screen.

The context tab maintains its own "center stack" — each re-center (`c` on a neighbor) pushes the previous center. `Esc` pops back through the exploration path. `+`/`-` adjusts traversal depth (1–3).

`Enter` on a neighbor opens it in the Viewer. `c` re-centers on it. The whole knowledge graph becomes walkable.

### Maintain Tab

Displays index stats (last indexed timestamp, document count, chunk count) and normalize status. Keybindings trigger actions:

- `i` — rebuild index (sends `QueryRequest::Index`, shows progress)
- `n` — run normalize (sends `QueryRequest::Normalize`, shows summary on completion)

Results and progress display inline. No navigation beyond this screen.

### Document Viewer

Unified full-screen view for any vault file. Renders:

- **Breadcrumb**: shows where you came from (e.g., "← Search results", "← Board > temper > visualization-qol")
- **Frontmatter header**: structured display of metadata fields (type, title, project, stage, scope, milestone, dates)
- **Body**: scrollable rendered markdown content

Context-sensitive keybindings in the footer:
- `e` — suspend TUI, open file in `$EDITOR`, resume and reload on exit
- `c` — pivot to Context tab centered on this document
- `s` — stage picker popup (tickets only)
- `S` — scope picker popup (tickets only)
- `Esc` — back to previous screen

### Mutation Flows

Only two mutation operations, both via inline popups:

**Stage picker** (`s` on a ticket): Small popup listing stages (backlog, in-progress, done, cancelled). `j`/`k` to select, `Enter` to confirm, `Esc` to cancel. Calls `actions::ticket::move_ticket()` and refreshes the board.

**Scope picker** (`S` on a ticket): Small popup listing scopes (patch, feature, epic) with one-line descriptions. Same interaction pattern. Calls `actions::ticket::move_ticket()` with only the scope parameter set (stage and milestone as `None`).

Both pickers work from the swimlane view (on the selected ticket) and from the Viewer (on the current document).

**$EDITOR integration** (`e` on any document): TUI calls `crossterm::terminal::disable_raw_mode()`, spawns `$EDITOR <path>` as a child process, waits for exit, re-enables raw mode, and reloads the file content. Standard ratatui pattern for editor handoff.

## Keybindings

### Direct Keys (outside command mode)

| Key | Action | Context |
|-----|--------|---------|
| `1`-`4` | Switch tab | Global |
| `j`/`k` or `↑`/`↓` | Move selection | Lists, swimlanes |
| `h`/`l` or `←`/`→` | Move between columns / cycle projects | Board |
| `Enter` | Open / drill into selected | Global |
| `Esc` | Pop back / cancel | Global |
| `e` | Open in $EDITOR | Viewer, or selected item in lists |
| `s` | Stage picker | Tickets |
| `S` | Scope picker | Tickets |
| `c` | Pivot to Context centered on item | Search, Context, Viewer |
| `+`/`-` | Adjust context depth | Context |
| `/` | Focus search input / enter topic | Search, Context |
| `Tab` | Input → results focus | Search |
| `:` | Enter command mode | Global |

### Command Mode

`:` opens a command line at the bottom of the screen. Supports vim-style abbreviation (unambiguous prefixes).

| Command | Abbreviation | Action |
|---------|-------------|--------|
| `:quit` | `:q` | Quit TUI |
| `:board` | `:b` | Switch to Board tab |
| `:search` | `:s` | Switch to Search tab |
| `:context` | `:c` | Switch to Context tab |
| `:maintain` | `:m` | Switch to Maintain tab |
| `:help` | `:?` or `:h` | Toggle help overlay |

`Esc` or `Enter` on empty input cancels command mode.

## Dependencies

New dependencies to add to `Cargo.toml`:

- `ratatui` — terminal UI framework
- `crossterm` — terminal backend (ratatui backend)
- `tokio` (rt, macros, sync) — async runtime for actor channels and select loop

Existing dependencies that remain relevant: `clap` (CLI parsing, unchanged), `anstyle`/`anstream` (CLI output, unchanged), `serde`/`serde_json` (data types), `candle-*`/`hf-hub`/`tokenizers`/`instant-distance` (search pipeline, used by query actor).

## CLI Changes

- **Add**: `temper tui` subcommand — launches the interactive TUI
- **Remove**: `temper ticket board` subcommand — replaced by the Board tab in the TUI
- All other subcommands remain unchanged

## Testing Strategy

**Actions layer**: Unit tests for each extracted action function. These replace or extend existing integration tests that currently test command functions. Use `tempfile::TempDir` fixtures as today.

**TUI layer**: The TUI is inherently interactive and difficult to unit test at the rendering level. Testing strategy:

- Test app state transitions: given a `Screen` and an `AppAction`, assert the resulting state. This covers navigation logic, stack push/pop, selection movement, and command parsing without rendering.
- Test query actor: send `QueryRequest`, assert `QueryResult`. Covers debounce behavior and search/context correctness.
- Rendering is validated manually and through the existing ratatui snapshot testing patterns if complexity warrants it.

**Integration**: Existing command tests continue to work — they test through `commands/` which calls `actions/`. No test breakage from the extraction.

## Out of Scope

- Text editing within the TUI (defers to `$EDITOR`)
- Creating new tickets or milestones from the TUI (planned follow-up, use CLI for now)
- Session management from the TUI
- Mouse support (keyboard-only)
- Configuration or theming (uses terminal colors)
- Syntax highlighting for markdown body content
