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
    prose siblings (description, summary, title, example, examples) overlay the target; any
    other sibling is kept beside it as `allOf: [target, siblings]`, never silently merged
    (a parameter or a path item, which is no schema, takes the siblings as an overlay);
  * a reference back into a node already being resolved replaced by {"$cycle": <location>}, the
    enclosing node's place in this form (a path of property names from the entry point's root),
    so a cycle ends, a rename alone changes nothing, and two different enclosing nodes never
    read as one;
  * its parameters merged with the path item's (the operation's win on the same in+name), each
    with `required` written out (default false), and each path parameter's name replaced by its
    position in the template ("{0}", "{1}"): the name is not on the wire, the position is;
  * `servers` replaced by the effective servers (operation, else path item, else document,
    else the default "/") and `security` by the effective requirements with each scheme
    resolved — both under the same keys, so broke_node reads them as it reads any other key;
  * each discriminator's effective mapping (explicit entries, plus the implicit entry for every
    `$ref` arm of the sibling oneOf/anyOf), each value resolved in place;
  * the dialect its schemas are read in: {"openapi", "jsonSchemaDialect"} under `x-dialect`;
  * each callback operation built as a webhook's is (callback_forms), and in a webhook and in
    every callback the request is SERVER-SENT: its `requestBody` and
    `parameters` are renamed (SERVER_SENT below) so broke_node judges them on the strict side,
    never with the input-side tolerance a client's request gets.

A `$ref` into the middle of a component reaches only that part, so the component is still judged
as one no entry point reaches.

Fails closed: an unresolvable local `$ref`, a key collision after erasure, a parameter with no
in+name or listed twice, or an unreadable document exits non-zero, and the gate reads that as "could not look", never as "nothing moved".
A non-local `$ref` (another document) is left as written; the contract has none, and comparing
the string is the strictest reading available.
"""

from __future__ import annotations

import json
import re
import sys
from typing import Any

METHODS = ("get", "put", "post", "delete", "options", "head", "patch", "trace")
# The keys broke_node treats as prose (wire-shape-lib.jq, sans_prose): beside a `$ref` they overlay
# the target; any other sibling is kept as `allOf: [target, siblings]`.
PROSE = ("description", "summary", "title", "example", "examples")
TEMPLATE_PARAM = re.compile(r"\{([^}]*)\}")
# In a webhook, or a callback, the server SENDS the request: its body and parameters are output.
# broke_node's side follows key names (`requestBody`/`parameters` descend as input), so those keys
# are renamed in a server-sent operation and judged on the strict, mixed side instead.
SERVER_SENT = {"requestBody": "x-server-sent-requestBody", "parameters": "x-server-sent-parameters"}


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
    """'#/components/schemas/Foo' -> 'schemas/Foo'. A ref into the middle of a component
    ('#/components/schemas/Foo/properties/x') does not reach the whole component, so it is not
    counted: the rest of Foo is still judged as a component no entry point reaches."""
    parts = pointer[2:].split("/")
    if len(parts) == 3 and parts[0] == "components":
        return f"{parts[1]}/{parts[2]}"
    return None


Stack = tuple[tuple[str, str], ...]  # (ref, the resolved-tree location it was inlined at)


class Resolver:
    def __init__(self, doc: dict) -> None:
        self.doc = doc
        self.touched: set[str] = set()

    def resolve(self, node: Any, stack: Stack = (), at: str = "") -> Any:
        """Inline every local `$ref` under `node`. `at` is the node's location in the form being
        built, from the entry point's root: a reference back into a node already being resolved
        becomes {"$cycle": <that node's location>}. The location names properties, never
        components, so a rename alone changes nothing, and two different enclosing nodes never
        read as the same one."""
        if isinstance(node, list):
            return [self.resolve(x, stack, f"{at}/{i}") for i, x in enumerate(node)]
        if not isinstance(node, dict):
            return node
        ref = node.get("$ref")
        if isinstance(ref, str) and ref.startswith("#"):
            siblings = {k: v for k, v in node.items() if k != "$ref"}
            # With non-prose siblings the target sits at allOf/0 of the form built here (see
            # overlay), so it is entered there: a chain of refs that all land on one location
            # must not give two different enclosing nodes one cycle marker.
            wrapped = bool(siblings) and not all(k in PROSE for k in siblings)
            t_at, s_at = (f"{at}/allOf/0", f"{at}/allOf/1") if wrapped else (at, at)
            entered = dict(stack)
            if ref in entered:
                target: Any = {"$cycle": entered[ref] or "/"}
            else:
                comp = component_of(ref)
                if comp:
                    self.touched.add(comp)
                target = self.resolve(pointer_get(self.doc, ref), (*stack, (ref, t_at)), t_at)
            return self.overlay(target, self.resolve(siblings, stack, s_at))
        out = {k: self.resolve(v, stack, f"{at}/{k}") for k, v in node.items()}
        if isinstance(out.get("discriminator"), dict):
            out["discriminator"] = self.discriminator(node, out["discriminator"], stack, at)
        return out

    @staticmethod
    def overlay(target: Any, siblings: dict) -> Any:
        if not siblings:
            return target
        if isinstance(target, dict) and all(k in PROSE for k in siblings):
            return {**target, **siblings}
        return {"allOf": [target, siblings]}

    def discriminator(self, raw: dict, resolved: dict, stack: Stack, at: str) -> dict:
        mapping: dict[str, Any] = {}
        explicit = (raw.get("discriminator") or {}).get("mapping") or {}
        named = set()
        here = f"{at}/discriminator/mapping"
        for value, target in explicit.items():
            pointer = target if target.startswith("#") else f"#/components/schemas/{target}"
            named.add(pointer)
            mapping[value] = self.resolve({"$ref": pointer}, stack, f"{here}/{value}")
        for arm in (raw.get("oneOf") or []) + (raw.get("anyOf") or []):
            pointer = arm.get("$ref") if isinstance(arm, dict) else None
            if (
                isinstance(pointer, str)
                and pointer.startswith("#/components/schemas/")
                and pointer not in named
            ):
                value = pointer.rsplit("/", 1)[1]
                mapping[value] = self.resolve({"$ref": pointer}, stack, f"{here}/{value}")
        return {**resolved, "mapping": mapping}

    def path_item(self, raw: dict) -> dict:
        """A path item, its own `$ref` resolved. Its siblings overlay the target: a path item is a
        map of operations, never a schema, so an `allOf` wrapper would make its operations
        vanish. A non-prose field defined on both sides is refused."""
        if "$ref" not in raw:
            return raw
        target = pointer_get(self.doc, raw["$ref"])
        if not isinstance(target, dict):
            raise Unresolvable(f"path item $ref {raw['$ref']} is not an object")
        comp = component_of(raw["$ref"])
        if comp:
            self.touched.add(comp)
        siblings = {k: v for k, v in raw.items() if k != "$ref"}
        # A field in both is undefined behaviour (OAS 3.1, Path Item Object): refuse to guess.
        clash = sorted(k for k in siblings if k in target and k not in PROSE)
        if clash:
            raise Unresolvable(f"path item $ref {raw['$ref']} and its siblings both define {clash}")
        return self.path_item({**target, **siblings})


def erase(template: str) -> str:
    return TEMPLATE_PARAM.sub("{}", template)


def as_parameter(p: Any) -> dict:
    """A resolved parameter, with its identity. A `$ref`'d parameter with non-prose siblings
    resolves to `allOf: [target, siblings]`; a parameter is no schema, and OAS (3.0 and 3.1,
    Reference Object) has a Reference Object's other siblings ignored, so the target stands with
    only its prose siblings, as a generator reads it. A sibling can never re-key or re-shape the
    parameter. A parameter with no `in`+`name` cannot be told apart from another: refuse."""
    if (
        isinstance(p, dict)
        and set(p) == {"allOf"}
        and len(p["allOf"]) == 2
        and isinstance(p["allOf"][0], dict)
        and "in" in p["allOf"][0]
        and isinstance(p["allOf"][1], dict)
    ):
        prose = {k: v for k, v in p["allOf"][1].items() if k in PROSE}
        p = {**p["allOf"][0], **prose}
    if not (
        isinstance(p, dict) and isinstance(p.get("in"), str) and isinstance(p.get("name"), str)
    ):
        raise Unresolvable(f"a parameter has no in+name identity: {json.dumps(p)[:120]}")
    # `required` defaults to false (OAS 3.1, Parameter Object): written out, so a parameter that
    # gains it compares as the boolean it is. broke_node reads a `required` key gained from
    # absent as a schema's required array, and errors on a boolean.
    return {"required": False, **p}


def parameter_list(resolved: list, where: str) -> list[dict]:
    seen = set()
    out = []
    for p in resolved:
        p = as_parameter(p)
        key = (p["in"], p["name"])
        if key in seen:
            raise Unresolvable(f"{where} lists the parameter {key} twice")
        seen.add(key)
        out.append(p)
    return out


def with_required(x: Any) -> Any:
    """`required` defaults to false on a request body and a header (OAS 3.1), as on a parameter:
    written out, so one that gains it compares as a boolean instead of crashing broke_node."""
    return {"required": False, **x} if isinstance(x, dict) else x


def with_required_content(content: Any) -> Any:
    """The same default on every encoding header under a `content` map."""
    if not isinstance(content, dict):
        return content
    for media in content.values():
        if isinstance(media, dict) and isinstance(media.get("encoding"), dict):
            for enc in media["encoding"].values():
                if isinstance(enc, dict) and isinstance(enc.get("headers"), dict):
                    enc["headers"] = {k: with_required(h) for k, h in enc["headers"].items()}
    return content


def normalize_operation(form: dict, where: str) -> dict:
    """The `required` defaults inside one resolved operation: parameters, request body, and the
    headers of responses and of encodings."""
    if "parameters" in form:
        form["parameters"] = parameter_list(form["parameters"] or [], where)
    if isinstance(form.get("requestBody"), dict):
        form["requestBody"] = with_required(form["requestBody"])
        with_required_content(form["requestBody"].get("content"))
    for response in (form.get("responses") or {}).values():
        if isinstance(response, dict):
            if isinstance(response.get("headers"), dict):
                response["headers"] = {k: with_required(h) for k, h in response["headers"].items()}
            with_required_content(response.get("content"))
    return form


def callback_forms(r: Resolver, doc: dict, raw_callbacks: Any) -> Any:
    """An operation's callbacks, each callback operation built exactly as a webhook's is: its
    path item resolved through path_item (siblings overlaid, clashes refused), its path-item
    parameters merged in, and the whole judged server-sent."""
    if not isinstance(raw_callbacks, dict):
        return raw_callbacks
    out: dict[str, Any] = {}
    for name, cb in raw_callbacks.items():
        if isinstance(cb, dict) and isinstance(cb.get("$ref"), str):
            comp = component_of(cb["$ref"])
            if comp:
                r.touched.add(comp)
            cb = pointer_get(doc, cb["$ref"])
        if not isinstance(cb, dict):
            raise Unresolvable(f"callback {name} is not an object")
        resolved: dict[str, Any] = {}
        for expr, raw_item in cb.items():
            if not isinstance(raw_item, dict):
                raise Unresolvable(f"callback {name} {expr} is not a path item")
            item = r.path_item(raw_item)
            ops = {
                m.upper(): operation_form(r, doc, None, item, item[m], sent=True)
                for m in METHODS
                if isinstance(item.get(m), dict)
            }
            if not ops:
                raise Unresolvable(f"callback {name} {expr} declares no operation")
            resolved[expr] = ops
        out[name] = resolved
    return out


def server_sent(form: dict) -> dict:
    return {SERVER_SENT.get(k, k): v for k, v in form.items()}


def operation_form(
    r: Resolver, doc: dict, template: str | None, item: dict, op: dict, sent: bool
) -> dict:
    form = r.resolve(op)
    where = f"{template or 'a webhook'}"
    item_params = parameter_list(r.resolve(item.get("parameters") or [], (), "/parameters"), where)
    own = parameter_list(form.get("parameters") or [], where)
    merged: dict[tuple, Any] = {}
    for p in item_params + own:
        merged[(p["in"], p["name"])] = p
    positions = {}
    if template is not None:
        positions = {name: i for i, name in enumerate(TEMPLATE_PARAM.findall(template))}
    params = []
    for p in merged.values():
        if p["in"] == "path" and p["name"] in positions:
            p = {**p, "name": f"{{{positions[p['name']]}}}"}
        params.append(p)
    if params or "parameters" in op or "parameters" in item:
        form["parameters"] = params
    form["servers"] = r.resolve(
        op.get("servers") or item.get("servers") or doc.get("servers") or [{"url": "/"}],
        (),
        "/servers",
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
            entry[name] = {
                "scopes": scopes,
                "scheme": r.resolve(schemes[name], (), f"/security/{name}"),
            }
        security.append(entry)
    form["security"] = security
    form["x-dialect"] = {
        "openapi": doc.get("openapi"),
        "jsonSchemaDialect": doc.get("jsonSchemaDialect"),
    }
    # Request-body and header defaults; callbacks rebuilt from the raw operation, so each is
    # judged as a server-sent operation of its own (path-item parameters and $ref'd path items
    # included), never as the generic resolution of a nested map.
    form = normalize_operation(form, where)
    if "callbacks" in op:
        form["callbacks"] = callback_forms(r, doc, op["callbacks"])
    return server_sent(form) if sent else form


def entry_points(doc: dict) -> dict[str, Any]:
    r = Resolver(doc)
    out: dict[str, Any] = {}

    def put(key: str, form: Any) -> None:
        if key in out:
            raise Unresolvable(f"two entry points share the key {key!r}")
        out[key] = form

    for template, raw_item in sorted((doc.get("paths") or {}).items()):
        item = r.path_item(raw_item)
        for method in METHODS:
            if isinstance(item.get(method), dict):
                put(
                    f"op {method.upper()} {erase(template)}",
                    operation_form(r, doc, template, item, item[method], sent=False),
                )
    for name, raw_item in sorted((doc.get("webhooks") or {}).items()):
        item = r.path_item(raw_item)
        for method in METHODS:
            if isinstance(item.get(method), dict):
                put(
                    f"webhook {name} {method.upper()}",
                    operation_form(r, doc, None, item, item[method], sent=True),
                )

    reached = set(r.touched)
    for kind, members in sorted((doc.get("components") or {}).items()):
        if not isinstance(members, dict):
            continue
        for name in sorted(members):
            comp = f"{kind}/{name}"
            pointer = f"#/components/{kind}/{name.replace('~', '~0').replace('/', '~1')}"
            form = r.resolve({"$ref": pointer})
            if kind == "parameters":
                form = as_parameter(form)
            elif kind in ("headers", "requestBodies"):
                form = with_required(form)
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
