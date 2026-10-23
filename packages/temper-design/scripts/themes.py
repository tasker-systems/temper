"""Theme tooling: validate a theme against the contract, check it is legible, emit its CSS.

A theme is data (theme.json). The CSS beside it is generated from that data and never
edited by hand; `check` proves the two agree. Invoked through themes.sh, which picks
the Python.
"""

import json
import math
import re
import sys
from pathlib import Path

from jsonschema import Draft202012Validator, FormatChecker

ROOT = Path(__file__).resolve().parent.parent
SCHEMA_PATH = ROOT / "contract" / "theme.schema.json"
PREFIX = "--tp-"

# ─── Legibility floors ────────────────────────────────────────────────────────
# (role, backgrounds, floor). A background named "<role>-wash" is composited over
# ground first, because that is how it paints. Floors follow WCAG 2.x: 4.5:1 for
# text, 3:1 for graphical marks and focus indicators, 7:1 for primary reading text.
TEXT_GROUNDS = ["ground", "surface", "surface-raised"]
FLOORS = [
    ("text", TEXT_GROUNDS, 7.0),
    ("text-muted", TEXT_GROUNDS, 4.5),
    ("text-subtle", TEXT_GROUNDS, 4.5),
    ("text-faint", ["ground"], 2.5),
    ("accent", ["ground", "surface"], 4.5),
    ("on-accent", ["accent"], 4.5),
    ("focus", ["ground", "surface"], 3.0),
    ("author-human", ["ground"], 3.0),
    ("author-agent", ["ground"], 3.0),
]
for _role in [
    "notice",
    "success",
    "danger",
    "pending",
    "region-arriving",
    "region-empty",
    "region-gave-up",
    "region-failed",
]:
    FLOORS.append((_role, [f"{_role}-wash"], 4.5))
# Slots carry doc-type marks, and surfaces set a doc type's word and its labels in its slot on
# every ground text sits on — a panel or rail on surface, a chip on surface-raised — so a slot is
# held to the text floor on all three, not only on ground. cat-neutral is the family's ninth
# member and clears the same floors.
for _i in range(1, 9):
    FLOORS.append((f"cat-{_i}", TEXT_GROUNDS, 4.5))
FLOORS.append(("cat-neutral", TEXT_GROUNDS, 4.5))
for _c in ["key", "string", "number", "comment"]:
    FLOORS.append((f"code-{_c}", ["surface"], 4.5))

# ─── Distinctness ─────────────────────────────────────────────────────────────
# Sets whose members must never read alike. Colour is one channel of several for
# the region states (marker, words, border style carry the rest), but a theme that
# collapses two of them onto one hue has removed a channel, and that is checkable.
# Threshold is CIE76 ΔE on the colours as painted over ground.
SLOT_FAMILY = [f"cat-{i}" for i in range(1, 9)] + ["cat-neutral"]
# The chrome and state roles a slot must never read as: a doc type bound to a slot
# that paints like the accent (or a region state, an authorship mark) would dress
# vocabulary in meaning it does not carry.
CHROME_AND_STATE = [
    "accent",
    "author-human",
    "author-agent",
    "notice",
    "region-arriving",
    "region-empty",
    "region-gave-up",
    "region-failed",
]
DISTINCT = [
    ("region states", ["region-arriving", "region-empty", "region-gave-up", "region-failed"], 15.0),
    ("categorical slots", SLOT_FAMILY, 12.0),
    ("authorship", ["author-human", "author-agent"], 15.0),
    ("conditions", ["notice", "success", "danger", "region-failed"], 12.0),
]
CROSS_DISTINCT = [("slot vs chrome/state", SLOT_FAMILY, CHROME_AND_STATE, 12.0)]


def parse_color(value):
    value = value.strip()
    if value.startswith("#"):
        h = value[1:]
        return (int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16), 1.0)
    m = re.match(r"rgba\(([^)]*)\)", value)
    parts = [p.strip() for p in m.group(1).split(",")]
    return (int(parts[0]), int(parts[1]), int(parts[2]), float(parts[3]))


def over(fg, bg):
    a = fg[3]
    return tuple(fg[i] * a + bg[i] * (1 - a) for i in range(3)) + (1.0,)


def luminance(c):
    def channel(v):
        v /= 255
        return v / 12.92 if v <= 0.03928 else ((v + 0.055) / 1.055) ** 2.4

    return 0.2126 * channel(c[0]) + 0.7152 * channel(c[1]) + 0.0722 * channel(c[2])


def contrast(fg, bg):
    a, b = luminance(fg), luminance(bg)
    hi, lo = max(a, b), min(a, b)
    return (hi + 0.05) / (lo + 0.05)


def lab(c):
    def lin(v):
        v /= 255
        return v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4

    r, g, b = (lin(c[i]) for i in range(3))
    x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047
    y = 0.2126 * r + 0.7152 * g + 0.0722 * b
    z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883

    def f(t):
        return t ** (1 / 3) if t > 0.008856 else 7.787 * t + 16 / 116

    fx, fy, fz = f(x), f(y), f(z)
    return (116 * fy - 16, 500 * (fx - fy), 200 * (fy - fz))


def delta_e(c1, c2):
    return math.dist(lab(c1), lab(c2))


def load(path):
    with open(path) as f:
        return json.load(f)


def schema_errors(theme):
    validator = Draft202012Validator(load(SCHEMA_PATH), format_checker=FormatChecker())
    return [
        f"{'/'.join(str(p) for p in e.path) or '<root>'}: {e.message}"
        for e in sorted(validator.iter_errors(theme), key=lambda e: list(e.path))
    ]


def painted(colors):
    """Every colour role as it paints: composited over ground (washes and surfaces included)."""
    ground = parse_color(colors["ground"])
    return {role: over(parse_color(v), ground) for role, v in colors.items()}


def legibility_errors(theme):
    colors = theme["tokens"]["color"]
    paint = painted(colors)
    errors = []
    for role, grounds, floor in FLOORS:
        for bg_role in grounds:
            fg = over(parse_color(colors[role]), paint[bg_role])
            ratio = contrast(fg, paint[bg_role])
            if ratio + 1e-9 < floor:
                # Two places hide a miss of under 0.005 ("4.50:1 < 4.5:1"); three show it.
                shown = f"{ratio:.2f}" if f"{ratio:.2f}" != f"{floor:.2f}" else f"{ratio:.3f}"
                errors.append(f"contrast {role} on {bg_role}: {shown}:1 < {floor}:1")
    for label, roles, threshold in DISTINCT:
        for i, a in enumerate(roles):
            for b in roles[i + 1 :]:
                d = delta_e(paint[a], paint[b])
                if d < threshold:
                    errors.append(f"distinct {label}: {a} vs {b} ΔE {d:.1f} < {threshold}")
    for label, family, others, threshold in CROSS_DISTINCT:
        for a in family:
            for b in others:
                d = delta_e(paint[a], paint[b])
                if d < threshold:
                    errors.append(f"distinct {label}: {a} vs {b} ΔE {d:.1f} < {threshold}")
    return errors


def css_for(theme):
    t = theme["tokens"]
    lines = [
        f"/* Generated from {theme['name']}/theme.json by themes/scripts/themes.sh build. Do not edit. */",
        f'[data-theme="{theme["name"]}"] {{',
        f"  color-scheme: {theme['appearance']};",
    ]
    for role, value in t["color"].items():
        lines.append(f"  {PREFIX}{role}: {value};")
    for group in ["font", "radius", "motion", "tracking"]:
        for key, value in t[group].items():
            lines.append(f"  {PREFIX}{group}-{key}: {value};")
    lines.append("}")
    return "\n".join(lines) + "\n"


def tailwind_utilities():
    """Contract roles as Tailwind v4 utilities, theme-independent: every value is a var.

    Generated from the schema, so a role added to the contract cannot lack a utility.
    Utilities carry a `tp-` prefix (`bg-tp-ground`, `text-tp-text-muted`) so they never
    collide with shadcn's names (`accent`, `muted`, `border`), which mean other things.
    """
    props = load(SCHEMA_PATH)["properties"]["tokens"]["properties"]
    out = ["@theme inline {"]
    for role in props["color"]["properties"]:
        out.append(f"  --color-tp-{role}: var({PREFIX}{role});")
    for key in props["font"]["properties"]:
        out.append(f"  --font-tp-{key}: var({PREFIX}font-{key});")
    for key in props["radius"]["properties"]:
        out.append(f"  --radius-tp-{key}: var({PREFIX}radius-{key});")
    for key in props["tracking"]["properties"]:
        out.append(f"  --tracking-tp-{key}: var({PREFIX}tracking-{key});")
    out += [
        f"  --default-transition-duration: var({PREFIX}motion-quick);",
        f"  --default-transition-timing-function: var({PREFIX}motion-easing);",
        "}",
    ]
    return "\n".join(out) + "\n"


def tailwind_roles_css():
    """The utilities alone, leaving Tailwind's default palette in place — for a consumer moving
    off that palette one area at a time. It switches to `tailwind.css` once nothing reads it."""
    return "\n".join(
        [
            "/* Generated from contract/theme.schema.json by themes/scripts/themes.sh build. Do not edit.",
            "   The roles as utilities WITHOUT the default-palette reset: for a consumer still moving",
            "   off Tailwind's palette. Import after tailwindcss and before any theme.css. */",
            "",
            "",
        ]
    ) + tailwind_utilities()


def tailwind_css():
    """The utilities with Tailwind's default palette reset: a component cannot reach for
    `zinc-800`. Both files are built from `tailwind_utilities()`, so they cannot disagree."""
    return "\n".join(
        [
            "/* Generated from contract/theme.schema.json by themes/scripts/themes.sh build. Do not edit.",
            "   Import after tailwindcss and before any theme.css. */",
            "",
            "@theme {",
            "  --color-*: initial;",
            "}",
            "",
            "",
        ]
    ) + tailwind_utilities()

# Both generated Tailwind files: the roles alone, and the roles with the default palette reset.
TAILWIND_FILES = {"tailwind.css": tailwind_css, "tailwind-roles.css": tailwind_roles_css}


def theme_paths():
    return sorted(p for p in ROOT.glob("*/theme.json"))


INDEX_FIELDS = ("name", "title", "appearance", "counterpart")


def themes_index():
    """The theme index a consumer imports to list and pair themes without reading every theme.

    It carries only what selection needs. A consumer that globbed the theme directories would
    depend on where the package happens to be installed, so the index is generated here, beside
    the themes it names, and checked like theme.css.
    """
    entries = []
    for path in theme_paths():
        theme = load(path)
        entries.append({k: theme[k] for k in INDEX_FIELDS if k in theme})
    return json.dumps(entries, indent=2, ensure_ascii=False) + "\n"


def check(path):
    """All errors for one theme file: contract, legibility, generated CSS in sync, counterpart."""
    theme = load(path)
    errors = schema_errors(theme)
    if errors:
        return errors  # the rest reads roles the schema just said may be missing
    errors += legibility_errors(theme)
    if path.parent.parent == ROOT:
        if theme["name"] != path.parent.name:
            errors.append(f"name {theme['name']!r} does not match directory {path.parent.name!r}")
        css = path.parent / "theme.css"
        if not css.exists() or css.read_text() != css_for(theme):
            errors.append("theme.css is stale or missing - run themes.sh build")
        cp = theme.get("counterpart")
        if cp:
            cp_path = ROOT / cp / "theme.json"
            if not cp_path.exists():
                errors.append(f"counterpart {cp!r} has no theme.json")
            else:
                other = load(cp_path)
                if other.get("counterpart") != theme["name"]:
                    errors.append(f"counterpart {cp!r} does not point back")
                if other.get("appearance") == theme["appearance"]:
                    errors.append(f"counterpart {cp!r} has the same appearance")
    return errors


def cmd_build():
    for name, generate in TAILWIND_FILES.items():
        (ROOT / "contract" / name).write_text(generate())
        print(f"wrote contract/{name}")
    for path in theme_paths():
        theme = load(path)
        errs = schema_errors(theme)
        if errs:
            print(f"build: {path.relative_to(ROOT)} does not satisfy the contract", file=sys.stderr)
            for e in errs:
                print(f"  {e}", file=sys.stderr)
            return 1
        (path.parent / "theme.css").write_text(css_for(theme))
        print(f"wrote {(path.parent / 'theme.css').relative_to(ROOT)}")
    (ROOT / "themes.json").write_text(themes_index())
    print("wrote themes.json")
    return 0


def cmd_check(paths):
    rc = 0
    for p in paths:
        path = Path(p).resolve()
        shown = path.relative_to(ROOT) if ROOT in path.parents else p
        errors = check(path)
        if errors:
            rc = 1
            print(f"FAIL {shown}", file=sys.stderr)
            for e in errors:
                print(f"  {e}", file=sys.stderr)
        else:
            print(f"ok   {shown}")
    return rc


def cmd_contrast(path):
    theme = load(Path(path).resolve())
    colors = theme["tokens"]["color"]
    paint = painted(colors)
    for role, grounds, floor in FLOORS:
        for bg_role in grounds:
            ratio = contrast(over(parse_color(colors[role]), paint[bg_role]), paint[bg_role])
            flag = "  " if ratio + 1e-9 >= floor else "!!"
            print(f"{flag} {role:<22} on {bg_role:<22} {ratio:6.2f}  (floor {floor})")
    return 0


def cmd_slots(path):
    """The slot canvas: within-family and slot-vs-chrome/state ΔE, flagged at their thresholds.

    This is the measurement the slot count is decided against — a canvas holding core's doc
    types and one plugin's together. Eight slots carry any number of terms (many terms may
    share a slot); they are short only if the family cannot hold nine mutually-distinct
    marks clear of the chrome and state roles. `check` enforces this; `slots` shows it.
    """
    theme = load(Path(path).resolve())
    paint = painted(theme["tokens"]["color"])
    within = next(t for label, _, t in DISTINCT if label == "categorical slots")
    threshold = CROSS_DISTINCT[0][3]
    print(f"{theme['name']} — within the slot family (floor {within}):")
    for i, a in enumerate(SLOT_FAMILY):
        for b in SLOT_FAMILY[i + 1 :]:
            d = delta_e(paint[a], paint[b])
            flag = "  " if d + 1e-9 >= within else "!!"
            print(f"{flag} {a:<12} vs {b:<12} ΔE {d:5.1f}")
    print(f"{theme['name']} — slots vs chrome/state (floor {threshold}):")
    for a in SLOT_FAMILY:
        for b in CHROME_AND_STATE:
            d = delta_e(paint[a], paint[b])
            flag = "  " if d + 1e-9 >= threshold else "!!"
            print(f"{flag} {a:<12} vs {b:<12} ΔE {d:5.1f}")
    return 0


def contract_vars():
    """Every --tp-* name a theme defines, derived from the schema."""
    props = load(SCHEMA_PATH)["properties"]["tokens"]["properties"]
    names = {f"{PREFIX}{r}" for r in props["color"]["properties"]}
    for group in ["font", "radius", "motion", "tracking"]:
        names |= {f"{PREFIX}{group}-{k}" for k in props[group]["properties"]}
    return names


def contract_css_failures():
    """Hand-written contract CSS consumes roles only: no literal colours, no unknown roles."""
    failures = 0
    known = contract_vars()
    for css in sorted((ROOT / "contract").glob("*.css")):
        text = re.sub(r"/\*.*?\*/", "", css.read_text(), flags=re.S)
        literal = re.search(r"#[0-9a-fA-F]{3,8}\b|rgba?\(|hsla?\(|oklch\(", text)
        unknown = sorted(set(re.findall(r"--tp-[a-z0-9-]+", text)) - known)
        if literal:
            print(
                f"FAIL contract/{css.name} carries a literal colour ({literal.group(0)})",
                file=sys.stderr,
            )
            failures += 1
        if unknown:
            print(
                f"FAIL contract/{css.name} references roles the contract lacks: {', '.join(unknown)}",
                file=sys.stderr,
            )
            failures += 1
        if not literal and not unknown:
            print(f"ok   contract/{css.name} consumes contract roles only")
    return failures


def cmd_self_check():
    failures = 0
    Draft202012Validator.check_schema(load(SCHEMA_PATH))
    print("ok   contract/theme.schema.json is a valid draft 2020-12 schema")
    for name, generate in TAILWIND_FILES.items():
        tw = ROOT / "contract" / name
        if not tw.exists() or tw.read_text() != generate():
            print(
                f"FAIL contract/{name} is stale or missing - run themes.sh build", file=sys.stderr
            )
            failures += 1
        else:
            print(f"ok   contract/{name} covers every role in the contract")
    failures += contract_css_failures()
    themes = theme_paths()
    if len(themes) < 2:
        print(
            "FAIL fewer than two themes - the contract is only proven by more than one",
            file=sys.stderr,
        )
        failures += 1
    failures += cmd_check([str(p) for p in themes])
    index = ROOT / "themes.json"
    if not index.exists() or index.read_text() != themes_index():
        print("FAIL themes.json is stale or missing - run themes.sh build", file=sys.stderr)
        failures += 1
    else:
        print("ok   themes.json indexes every theme")
    for fixture in sorted((ROOT / "tests" / "fixtures" / "broken").glob("*.json")):
        errors = check(fixture)
        if errors:
            print(f"ok   {fixture.relative_to(ROOT)} fails as required ({errors[0]})")
        else:
            print(
                f"FAIL {fixture.relative_to(ROOT)} was accepted but must be refused",
                file=sys.stderr,
            )
            failures += 1
    for svg in sorted((ROOT / "brand").glob("*.svg")):
        if re.search(r"#[0-9a-fA-F]{3,6}\b", svg.read_text()):
            print(
                f"FAIL {svg.relative_to(ROOT)} hard-codes a colour; brand marks paint in currentColor",
                file=sys.stderr,
            )
            failures += 1
        else:
            print(f"ok   {svg.relative_to(ROOT)} paints in currentColor")
    return 1 if failures else 0


def main(argv):
    if argv[:1] == ["build"]:
        return cmd_build()
    if argv[:1] == ["check"] and len(argv) > 1:
        return cmd_check(argv[1:])
    if argv[:1] == ["contrast"] and len(argv) == 2:
        return cmd_contrast(argv[1])
    if argv[:1] == ["slots"] and len(argv) == 2:
        return cmd_slots(argv[1])
    if argv == ["--self-check"]:
        return cmd_self_check()
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
