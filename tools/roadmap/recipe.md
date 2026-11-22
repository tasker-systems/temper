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
