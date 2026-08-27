"""Shared response fixtures.

The generated models VALIDATE on deserialize: pydantic raises for a required field
that arrives missing. `ResourceView` — the one shape every resource read and write
answers in — declares eleven required attributes, so a stub returning `{}` fails
inside the client, not in the assertion. Stub with a real view.

This is the same lesson `clients/temper-rb/spec/spec_helper.rb` carries, and the same
trap: no local `cargo make check` tier runs these tests, so a DTO field added without
updating a fixture here stays green locally and fails only in CI's test-python job.
"""

from __future__ import annotations

from typing import Any

RESOURCE_ID = "019f4912-3f20-7fd3-814f-13a5ddbe3cd7"
PROFILE_ID = "019d4add-f49d-7c43-a87d-dda470e5dd9c"


def resource_row(resource_id: str = RESOURCE_ID, **overrides: Any) -> dict[str, Any]:
    row = {
        "id": resource_id,
        "origin_uri": "",
        "title": "A Resource",
        "originator_profile_id": PROFILE_ID,
        "owner_profile_id": PROFILE_ID,
        "is_active": True,
        "created": "2026-07-10T12:00:00Z",
        "updated": "2026-07-10T12:00:00Z",
        "doc_type_name": "note",
        "owner_handle": "j-cole-taylor",
        # Not a column: the decorated, self-resolving address the view derives from
        # title + id. Required on the wire, so a fixture without it fails validation.
        "ref": f"a-resource-{resource_id}",
    }
    row.update(overrides)
    return row


def profile_with_entitlements(**overrides: Any) -> dict[str, Any]:
    """`GET /api/profile` — the shape `Client.whoami()` deserializes into.

    `ProfileWithEntitlements` is `allOf: [Profile, {entitlements}]`, so a fixture must
    satisfy Profile's seven required fields AND carry entitlements; a stub returning
    `{}` fails inside pydantic, not in the assertion.
    """
    row = {
        "id": PROFILE_ID,
        "display_name": "J. Cole Taylor",
        "slug": "j-cole-taylor",
        "preferences": {},
        "vault_config": {},
        "created": "2026-07-10T12:00:00Z",
        "updated": "2026-07-10T12:00:00Z",
        "entitlements": {"system_access": True, "is_admin": False},
    }
    row.update(overrides)
    return row


def search_response(**overrides: Any) -> dict[str, Any]:
    """`POST /api/search` — two arms that are never combined, plus the shared scope.

    Every one of `exact`, `wide`, `scope` is required, and each has required fields of
    its own, so the empty-result case is still eight keys deep.
    """
    row = {
        "exact": {"hits": [], "reason": "no_match"},
        "wide": {"hits": [], "reason": "no_match", "degraded": False},
        "scope": {"kind": "global"},
    }
    row.update(overrides)
    return row
