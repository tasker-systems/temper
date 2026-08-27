"""Act context for a write. Optional on every call.

The constructor invariant mirrors `ActInput::into_act_context`: Rust's
`AgentAuthorship.confidence` is non-Option, so authorship without confidence is a 400.
Rejecting it here is the parse-don't-validate answer — an invalid `Act` cannot be
constructed, so no call site can send one.

`correlation` and `invocation` are exempt: correlation is provenance, never
authorship, and an act with no supplied correlation self-roots to its own event id.
Nothing gates on it, so a client may always omit it.

Ported from `clients/temper-rb/lib/temper/act.rb`.
"""

from __future__ import annotations

from typing import Any

_AUTHORSHIP_FIELDS = ("reasoning", "rationale", "persona", "model")


class Act:
    def __init__(
        self,
        *,
        confidence: Any | None = None,
        reasoning: str | None = None,
        rationale: str | None = None,
        persona: str | None = None,
        model: str | None = None,
        correlation: str | None = None,
        invocation: str | None = None,
    ) -> None:
        authorship = {
            "reasoning": reasoning,
            "rationale": rationale,
            "persona": persona,
            "model": model,
        }
        if confidence is None:
            supplied = [k for k in _AUTHORSHIP_FIELDS if authorship[k] is not None]
            if supplied:
                raise ValueError("Act requires `confidence` when supplying " + ", ".join(supplied))

        fields: dict[str, Any] = dict(authorship)
        fields["confidence"] = None if confidence is None else str(confidence)
        fields["correlation_id"] = correlation
        fields["invocation_id"] = invocation
        # Nils omitted: the server distinguishes an absent key from null.
        self._fields = {k: v for k, v in fields.items() if v is not None}

    def to_dict(self) -> dict[str, Any]:
        """The seven `ActInput` wire keys.

        They flatten into ~30 write bodies and onto the query string of
        `DELETE /api/resources/{id}`, so this is a plain dict rather than a model:
        both destinations take it as keyword arguments.
        """
        return dict(self._fields)
