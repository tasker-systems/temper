# Memories

{% if surface == "cli" -%}
> **Memories.** This machine's `MEMORY.md` is generated from Temper by `temper memory emit`; do not
> hand-edit it. Run `temper memory status` to see what this machine carries and whether it has
> adopted the convention. `temper memory check` fails if the index has drifted.
{%- else -%}
> **Memories are resources.** Durable working knowledge is stored as resources of type `memory`,
> carrying `open_meta.status` (`active` / `superseded`) and `open_meta.verified` (the date the claim
> was last checked). Read the active ones and honour them. A `verified` date far in the past means
> **nobody has re-checked the claim** — not that it is false.
{%- endif %}
