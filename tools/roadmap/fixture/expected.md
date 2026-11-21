**As of 2026-11-21. Every lane is read from the records.** This is a snapshot of direction, not a promise. A newer snapshot supersedes it, and this one stays as it was. It is read from the goal records in `+temper-dev/core` and the work items in `+temper-dev/core-workflows`, from 2026-11-21T03:00:00Z to 2026-11-21T03:00:04Z, by `tools/roadmap/snapshot.sh` in the temper repository; the newest record it read last changed at 2026-11-20T10:00:00Z.

## How to read it

- **Grouped by product area.** Each goal is grouped under its one area, from a closed vocabulary of 12. Under each area are its goals. The line under each goal says what it brings or closes, in words its owner wrote for this page.
- **Each goal's counts** are its work items: how many are **claimed**, how many are **ready** and unclaimed, and how many are **filed** in all. A work item is filed while it is open (in backlog or in progress), and it counts toward the one goal it advances. A small claimed item and a large body of filed work therefore read differently.
- **The lanes, all read from the records:**
  - **Now:** some of the goal's work is claimed, or ready and unclaimed.
  - **Next:** the goal has filed work, none of it claimed or ready.
  - **Later:** the goal has no filed work.
- **Paused goals follow the same rule.** A paused goal is still open, so it sits in the lane its work puts it in, and its line is marked *paused*.
- **A work item's stage does not set its lane.** "Ready" and "claimed" are marks on the work item itself, and "in progress" is neither. 2 work items are in progress without a claim; each counts as filed, and as ready where it is marked ready.
- **No scores.** Value, risk and size are not scored per item, and goals are ordered by area and lane, not by any score. Each goal states what it protects; read that instead.

## At a glance

| Area | Now | Next | Later |
|---|---|---|---|
| Identity and access | 1 |  |  |
| Surfaces | 1 |  | 2 |
| Reliability |  | 1 |  |
| Release and deploy |  |  |  |
| Reads and search |  |  |  |
| Interop |  |  |  |
| Knowledge model |  |  |  |
| Records and ledger |  |  |  |
| Privacy |  |  | 1 |
| Ingest and writes |  |  |  |
| Process |  |  |  |
| UI | 1 |  |  |
| No area | 1 | 1 |  |
| **All areas** | **4** | **2** | **3** |

11 open goals: 9 active, 2 paused; 9 are shown and 2 withheld (see the end of this page). Of the 17 filed work items, 9 advance an open goal shown here: 2 claimed, 2 ready and unclaimed, and 5 filed and waiting.

## Identity and access

- **Now** · [Every session ends on time](./10000000-0000-7000-8000-000000000001) · 1 claimed, 0 ready, 2 filed  
  Sessions end when their window closes.

## Surfaces

- **Now** · [Tool descriptions say what the code does](./10000000-0000-7000-8000-000000000002) · 0 claimed, 1 ready, 3 filed  
  The tool descriptions are checked against the source.
- **Later** · [An active goal waits](./10000000-0000-7000-8000-000000000013) · 0 claimed, 0 ready, 0 filed  
  Nothing is filed for it yet.
- **Later · paused** · [A paused goal waits](./10000000-0000-7000-8000-000000000012) · 0 claimed, 0 ready, 0 filed  
  Nothing is filed for it.

## Reliability

- **Next · paused** · [Every lock is \[registered\] | ranked](./10000000-0000-7000-8000-000000000003) · 0 claimed, 0 ready, 1 filed  
  Every lock is registered with its reason.

## Release and deploy

No open goal is shown here.

## Reads and search

No open goal is shown here.

## Interop

No open goal is shown here.

## Knowledge model

No open goal is shown here.

## Records and ledger

No open goal is shown here.

## Privacy

- **Later** · [Erasure can be proven](./10000000-0000-7000-8000-000000000004) · 0 claimed, 0 ready, 0 filed  
  Erasure completes, and its completion can be proven.

## Ingest and writes

No open goal is shown here.

## Process

No open goal is shown here.

## UI

- **Now** · [The UI acknowledges every act](./10000000-0000-7000-8000-000000000011) · 1 claimed, 0 ready, 1 filed  
  Content still arriving is never shown as missing.

## No area

Each of these carries no `open_meta.area`, or one outside the vocabulary, so it sits in no area above.

- **Now** · [An outside goal](./10000000-0000-7000-8000-000000000008) · 0 claimed, 1 ready, 1 filed  
  Its area is not in the vocabulary.
- **Next** · [An ungrouped goal](./10000000-0000-7000-8000-000000000007) · 0 claimed, 0 ready, 1 filed  
  It has no area.

## Not on this map

- **6 of the 17 filed work items advance no open goal.** Of them, 2 advance a goal that is completed or cancelled, 3 advance no goal, and 1 advances more than one goal and counts for none. They are not counted above. 2 of them are claimed or in progress.
- **Completed and cancelled goals are not listed** (1 completed, 1 cancelled). A snapshot shows what is outstanding.
- **2 goals shown carry no area from the vocabulary,** and are listed by name under *No area*.
- **2 open goals are withheld.** A goal is shown only under a public title and outcome line its owner wrote for this page, so that no line describes a weakness that is not yet fixed. These carry none yet, so they are counted here and not named, and the 2 filed work items that advance them are not counted above.

## Recipe

Every line above was drawn by this recipe, so a reader with the same records draws the same lanes and counts.

1. **Read.** Every goal in `+temper-dev/core`, of every status, and every task in `+temper-dev/core-workflows`, each listed whole. A listing that says it is truncated, or holds fewer rows than its total, stops the draw, as does any read that fails: nothing is written.
2. **Link.** A task's goal is the target of an `advances` edge whose source is that task. Every task's edges are read the same way, by a one-hop graph walk seeded with every task listed, in batches of at most 250 seeds, each task in exactly one batch. An `advances` edge into a task counts for nothing. A walk that fails, has no node for a task it was seeded with, or holds more or fewer edges at that task than its node's degree stops the draw: nothing is written.
3. **Open goals.** A goal is open when its status is `active` or `paused`. Completed and cancelled goals are left out, and counted.
4. **Count.** For each open goal, over the tasks that advance it and no other goal:
   - *filed*: its stage is `backlog` or `in-progress`;
   - *claimed*: filed, and carrying a claim;
   - *ready*: filed, marked ready, and not claimed. A ready item that is claimed counts as claimed.

   A done or cancelled task counts nowhere.
5. **Place.** *Now* when claimed plus ready is above zero; otherwise *Next* when filed is above zero; otherwise *Later*.
6. **Show, or withhold.** A goal is shown only under the public title and outcome line its owner wrote on it for this page. An open goal without both is withheld: it is counted, never named, and its work is not counted toward any lane. No work item is named.
7. **Group and order.** By product area, in the order of its written vocabulary. Within an area, goals go by lane (Now, Next, Later), active before paused, then by public title. A shown goal with no area, or one outside the vocabulary, is listed under *No area*. No goal is ordered by a score.
8. **Leave out, and say so.** The filed work that advances no open goal is counted, split by why. The goal statuses omitted are named with their counts, and so is the number of goals withheld.

Two draws over the same records differ only in the stated date and the times the records were read.
