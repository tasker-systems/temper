#!/usr/bin/env python3
"""wire-resolve.py — an OpenAPI document's entry points, each in its resolved form (COMPAT-1).

The release-relative half of the declared-class gate (wire-class-crosscheck.sh) judges entry
points, not named components: a component is judged where a client meets it. This program turns
one contract into the map the comparator is pointed at. It is not a comparator: what counts as a
break is broke_node's, in wire-shape-lib.jq, unchanged.

    python3 -I wire-resolve.py openapi.json > entry-points.json

Output: one JSON object, {"<entry point key>": <resolved form>, ...}.

Entry point keys (conventions doctrine compat.md, "Terms"):
  "op <METHOD> <template>"   an operation; the template's parameter names are erased, so
                             /items/{id} and /items/{item_id} are one operation.
  "webhook <name> <METHOD>"  a webhook, resolved as an operation is.
  "component <kind>/<name>"  a component no entry point reaches in this document. It is judged
                             at every revision where it exists, reachable there or not, so
                             becoming reachable is never read as its removal.

The resolved form of an operation is the operation object with:
  * every `$ref` reached from it inlined in place, through components of every kind. A `$ref`'s
    prose siblings (description, summary, title) overlay the target; any other sibling is kept
    beside it as `allOf: [target, siblings]`, never silently merged;
  * a reference back into a node already being resolved replaced by {"$cycle": <depth>}, the
    distance up the resolution stack to the enclosing node, so a cycle ends and a rename alone
    changes nothing;
  * its parameters merged with the path item's (the operation's win on the same in+name), and
    each path parameter's name replaced by its position in the template ("{0}", "{1}"): the
    name is not on the wire, the position is;
  * `servers` replaced by the effective servers (operation, else path item, else document,
    else the default "/") and `security` by the effective requirements with each scheme
    resolved — both under the same keys, so broke_node reads them as it reads any other key;
  * each discriminator's effective mapping (explicit entries, plus the implicit entry for every
    `$ref` arm of the sibling oneOf/anyOf), each value resolved in place;
  * the dialect its schemas are read in: {"openapi", "jsonSchemaDialect"} under `x-dialect`.

Fails closed: an unresolvable local `$ref`, a key collision after erasure, or an unreadable
document exits non-zero, and the gate reads that as "could not look", never as "nothing moved".
A non-local `$ref` (another document) is left as written; the contract has none, and comparing
the string is the strictest reading available.
"""

from __future__ import annotations

import json
import re
import sys
from typing import Any

METHODS = ("get", "put", "post", "delete", "options", "head", "patch", "trace")
PROSE = ("description", "summary", "title")
TEMPLATE_PARAM = re.compile(r"\{([^}]*)\}")


class Unresolvable(Exception):
    pass


def pointer_get(doc: Any, pointer: str) -> Any:
    node = doc
    for raw in pointer[2:].split("/") if pointer != "#" else []:
        part = raw.replace("~1", "/").replace("~0", "~")
        if isinstance(node, dict) and part in node:
            node = node[part]
        elif isinstance(node, list) and part.isdigit() and int(part) < len(node):
            node = node[int(part)]
        else:
            raise Unresolvable(f"$ref {pointer} does not resolve")
    return node


def component_of(pointer: str) -> str | None:
    """'#/components/schemas/Foo/properties/x' -> 'schemas/Foo'."""
    parts = pointer[2:].split("/")
    if len(parts) >= 3 and parts[0] == "components":
        return f"{parts[1]}/{parts[2]}"
    return None


class Resolver:
    def __init__(self, doc: dict) -> None:
        self.doc = doc
        self.touched: set[str] = set()

    def resolve(self, node: Any, stack: tuple[str, ...]) -> Any:
        if isinstance(node, list):
            return [self.resolve(x, stack) for x in node]
        if not isinstance(node, dict):
            return node
        ref = node.get("$ref")
        if isinstance(ref, str) and ref.startswith("#"):
            siblings = {k: v for k, v in node.items() if k != "$ref"}
            if ref in stack:
                target: Any = {"$cycle": len(stack) - stack.index(ref)}
            else:
                comp = component_of(ref)
                if comp:
                    self.touched.add(comp)
                target = self.resolve(pointer_get(self.doc, ref), (*stack, ref))
            return self.overlay(target, self.resolve(siblings, stack))
        out = {k: self.resolve(v, stack) for k, v in node.items()}
        if isinstance(out.get("discriminator"), dict):
            out["discriminator"] = self.discriminator(node, out["discriminator"], stack)
        return out

    @staticmethod
    def overlay(target: Any, siblings: dict) -> Any:
        if not siblings:
            return target
        if isinstance(target, dict) and all(k in PROSE for k in siblings):
            return {**target, **siblings}
        return {"allOf": [target, siblings]}

    def discriminator(self, raw: dict, resolved: dict, stack: tuple[str, ...]) -> dict:
        mapping: dict[str, Any] = {}
        explicit = (raw.get("discriminator") or {}).get("mapping") or {}
        named = set()
        for value, target in explicit.items():
            pointer = target if target.startswith("#") else f"#/components/schemas/{target}"
            named.add(pointer)
            mapping[value] = self.resolve({"$ref": pointer}, stack)
        for arm in (raw.get("oneOf") or []) + (raw.get("anyOf") or []):
            pointer = arm.get("$ref") if isinstance(arm, dict) else None
            if (
                isinstance(pointer, str)
                and pointer.startswith("#/components/schemas/")
                and pointer not in named
            ):
                mapping[pointer.rsplit("/", 1)[1]] = self.resolve({"$ref": pointer}, stack)
        return {**resolved, "mapping": mapping}


def erase(template: str) -> str:
    return TEMPLATE_PARAM.sub("{}", template)


def operation_form(r: Resolver, doc: dict, template: str | None, item: dict, op: dict) -> dict:
    form = r.resolve(op, ())
    item_params = r.resolve(item.get("parameters") or [], ())
    own = form.get("parameters") or []
    merged: dict[tuple, Any] = {}
    for p in list(item_params) + list(own):
        key = (p.get("in"), p.get("name")) if isinstance(p, dict) else ("?", json.dumps(p))
        merged[key] = p
    positions = {}
    if template is not None:
        positions = {name: i for i, name in enumerate(TEMPLATE_PARAM.findall(template))}
    params = []
    for p in merged.values():
        if isinstance(p, dict) and p.get("in") == "path" and p.get("name") in positions:
            p = {**p, "name": f"{{{positions[p['name']]}}}"}
        params.append(p)
    if params or "parameters" in op or "parameters" in item:
        form["parameters"] = params
    form["servers"] = r.resolve(
        op.get("servers") or item.get("servers") or doc.get("servers") or [{"url": "/"}], ()
    )
    requirements = op.get("security", doc.get("security", []))
    schemes = (doc.get("components") or {}).get("securitySchemes") or {}
    security = []
    for req in requirements or []:
        entry = {}
        for name, scopes in req.items():
            if name not in schemes:
                raise Unresolvable(f"security requirement names an undefined scheme {name}")
            r.touched.add(f"securitySchemes/{name}")
            entry[name] = {"scopes": scopes, "scheme": r.resolve(schemes[name], ())}
        security.append(entry)
    form["security"] = security
    form["x-dialect"] = {
        "openapi": doc.get("openapi"),
        "jsonSchemaDialect": doc.get("jsonSchemaDialect"),
    }
    return form


def entry_points(doc: dict) -> dict[str, Any]:
    r = Resolver(doc)
    out: dict[str, Any] = {}

    def put(key: str, form: Any) -> None:
        if key in out:
            raise Unresolvable(f"two entry points share the key {key!r}")
        out[key] = form

    for template, raw_item in sorted((doc.get("paths") or {}).items()):
        item = r.resolve(raw_item, ()) if "$ref" in raw_item else raw_item
        for method in METHODS:
            if isinstance(item.get(method), dict):
                put(
                    f"op {method.upper()} {erase(template)}",
                    operation_form(r, doc, template, item, item[method]),
                )
    for name, raw_item in sorted((doc.get("webhooks") or {}).items()):
        item = r.resolve(raw_item, ()) if "$ref" in raw_item else raw_item
        for method in METHODS:
            if isinstance(item.get(method), dict):
                put(
                    f"webhook {name} {method.upper()}",
                    operation_form(r, doc, None, item, item[method]),
                )

    reached = set(r.touched)
    for kind, members in sorted((doc.get("components") or {}).items()):
        if not isinstance(members, dict):
            continue
        for name in sorted(members):
            comp = f"{kind}/{name}"
            pointer = f"#/components/{kind}/{name.replace('~', '~0').replace('/', '~1')}"
            form = r.resolve({"$ref": pointer}, ())
            out[f"component {comp}"] = {"reached": comp in reached, "form": form}
    return out


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: wire-resolve.py OPENAPI.json", file=sys.stderr)
        return 2
    try:
        with open(argv[1], encoding="utf-8") as handle:
            doc = json.load(handle)
        if not isinstance(doc, dict):
            raise Unresolvable("the document is not a JSON object")
        result = entry_points(doc)
    except (OSError, ValueError, Unresolvable, RecursionError) as error:
        print(f"wire-resolve: {error}", file=sys.stderr)
        return 1
    json.dump(result, sys.stdout, sort_keys=True, separators=(",", ":"))
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    sys.setrecursionlimit(20000)
    sys.exit(main(sys.argv))
