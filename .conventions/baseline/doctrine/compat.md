# Compatibility gates

How a gate that judges compatibility decides what counts as a break. Such a gate might be a
declared-class gate over an OpenAPI spec (temper's `wire-class-crosscheck.sh`), a semver check over
a published crate or package, or any other check that compares one revision's shape with another's.
This file does not say which surfaces need such a gate, or how to compare two shapes: that is the
consumer's own comparator. It says **what the comparator is pointed at**. Enforcement status per
repo is in [`rules.md`](rules.md).

Terms:

- **The last release** is, for each stream that ships the surface, its newest release that
  qualifies under COMPAT-2.
- **The window**, for each stream, is every change merged since that stream's last release.
- **An entry point** is what a client calls or names.
  - For an HTTP API it is an operation, identified by its method and path template with parameter
    names erased, so `/items/{id}` and `/items/{item_id}` are one operation.
  - A webhook is an entry point per method, identified by its name and method, and resolved as an
    operation is.
  - For a library, an entry point is a public item, identified by its path.
- **The resolved form** of an entry point is its effective shape, with every reference resolved in
  place. For an operation, that includes:
  - its parameters, including the path item's;
  - its request body, responses, headers and callbacks;
  - its effective servers;
  - its effective security requirements, with the schemes they name;
  - every `$ref` reached from these, through components of every kind;
  - each discriminator's effective mapping, implicit entries included, because the discriminator
    value is on the wire;
  - the dialect its schemas are read in: the document's `openapi` version, `jsonSchemaDialect`
    and any `$schema`.

  A reference back into a node already being resolved is compared as a reference to that enclosing
  node, not by name, so a cycle ends and a rename alone changes nothing.
- **A break** is what the comparator reports when it judges one form against another. Two breaks
  are the same only when the comparator reports them at the same location, of the same kind, with
  the same values before and after. A report that differs in any of these is a different break, so
  a break this PR deepens is charged. A report at the same location and of the same kind, with
  different values, is not charged when base against head reports no break at that location:
  everything that fails at head already failed at base. A comparator that merges the breaks at one
  location (all the removed enum values as one report, for example), or does not report their
  values, is used under the per-entry-point fallback below.
- **A declared break** is the PR's own statement that a break is intended: a version class, a
  register entry, a label.

**COMPAT-1 — a gate charges a PR only with breaks a client of the last release can observe, and
only with the ones the PR made.** The gate judges every entry point present at the release, the
base or the head, each in its resolved form. It judges release against head, and release against
base. The PR is charged with the breaks that release against head reports and release against base
does not. A part of the contract that no entry point reaches is judged as an entry point of its
own, identified by its component name and kind, at every revision where it is present; becoming
reachable is not its removal. Base against head is used only for the value rule in the terms
above, and by the fallback below.

The consequences:

- **Shape born in the window** (an entry point absent at the release, or a schema reached only
  through a property, parameter or response the release did not have) may move, be renamed or be
  removed freely. Against the release, it is growth.
- **Released shape** is judged against its released form, wherever the change is made: in the entry
  point itself, in a component it reaches, or in a window-born component now held at a released
  location. A window-born parameter that becomes required on a released operation is charged. A
  released entry point that this PR removes is charged.
- **A re-birth** is shape the release had, which the window removed and the PR restores. It is
  judged against its released form at every depth: operation, schema or single property. It is
  charged when it differs as a break would, and not charged when it is restored unchanged.
- **An earlier PR's declared break** is not charged again, because release against base already
  reports it. A further or deeper break this PR makes at the same place is a different break, and
  is charged.

Subtracting release against base assumes that every break it reports was charged when it landed. A
break that landed while the gate gave the base-only verdict (COMPAT-2) was not. So before the
release tooling publishes, it judges the candidate it is about to publish against the last release
of each stream that ships the surface, and refuses any break no declaration in the window names. A
break at the same location and of the same kind as a named one, whose values it relaxes, counts as
named. Where a consumer declares only a version class, it refuses a break the window's class does
not admit, and records that it cannot tell breaks apart.

**The per-entry-point fallback.** Where the comparator returns only a verdict per entry point, and
not the breaks themselves, the gate charges an entry point when release against head breaks it
*and* the PR changes its resolved form (base and head differ). Base against head breaking it is not
the test, because a re-birth reads there as growth. This fallback can charge a PR with an earlier
declared break on an entry point it touches. It never waves through a break the PR made. The
consumer records that residue in its status.

**The comparator is unchanged; only what it is pointed at changes.** Every tolerance and every
residue it has carries over. One residue matters more here, because judging against the release
can expose it where judging against the base would hide it.

- **What exposes it:** a property that is new by name, on a schema whose released form declared
  `additionalProperties` or `unevaluatedProperties` as `true` or as a schema, or a
  `patternProperties` pattern the new name matches.
- **Why it counts as released shape:** on the input side, a client of the release could already
  send that property. On the output side, the release told clients what such a property holds.
- **The omitted keyword:** a schema that omits the keyword is read as not admitting the property.
  Generated schemas omit it by default, and the server did not promise to accept unknown
  properties. This departs from JSON Schema's own reading. A consumer whose server accepted them
  records the gap as residue.
- **The consumer's part:** where the comparator reads such a property as growth, the consumer
  records the gap as residue in its status.

Every context the comparator derives from a pair of contracts (temper's input-only side, for
example) is derived from the pair being judged.

*Why:* a break is something a client built against a release can observe. For a surface that
clients obtain only through a release, no client has seen shape born since that release. A gate
that measures only against the PR's base reports breaks no such client can see. That pushes its
author to declare a false break or to bend the design around the gate. A gate that measures only
against the release charges a PR with every earlier PR's declared break. Taking the difference of
the two release-relative judgements charges exactly the observable breaks this PR introduced.
Judging resolved entry points, rather than named components, follows what a client actually sends
and receives. A component is judged where it is used, so a window-born component held at a
released location cannot carry a break past the gate.

*Residue (provisional, pending an owner's ruling):* a surface deployed continuously from the
protected branch (a hosted API that auto-deploys `main`) reaches clients before it is released. A
client that integrated against the deployment can observe window-born shape, and this rule does
not protect that client. A consumer whose deployed surface needs that protection measures that
surface against the PR's base alone, and says so in its status.

**COMPAT-2 — the last release comes from the release tooling's own derivation, and a ref that is
not provably the newest release fails closed.** The gate calls the derivation its release tooling
uses to open the window. It does not keep a second copy. A ref is the last release of a stream only
when all of these hold:

- it is a tag in the release tooling's own tag form for that stream;
- it is an ancestor of the PR's base or, for a stream released from a declared maintenance line,
  of that line's head as the remote holds it when the gate runs;
- it was found in a history that holds it: the clone is not cut short above it, and tags were
  fetched;
- no tag of the same stream on the remote (`git ls-remote --tags`, for example) is both an ancestor
  of that same commit and a descendant of it.

A gate that cannot list the remote's tags, or cannot place a listed tag's commit in its history, or
cannot read a declared maintenance line's head from the remote, cannot show the last condition, and
gives the base-only verdict. A derivation's fallback is never a release: a root commit, a shallow
boundary, a hand-given override. The derivation reports a fallback in its output, not only in a log
line, and a gate that cannot tell whether it got one treats it as one.

What was present at the release is read from what the release published. Where that differs from
the tree at the tag (a crate on its registry), the published copy is read. Presence is never
inferred from history. "The latest commit that added a node", for example, falls in the window for
a re-birth.

The gate measures against the last release as of merge, so a release cut while the PR is open is
the one it measures against. Either a merge queue runs the gate at merge, or pushing a release tag
re-runs it on open PRs. Requiring an up-to-date base is not enough on its own, because a tag cut on
the base adds no commit. A consumer with neither records release staleness as residue.

**Streams.** A repository may have several release streams: per-crate or per-package tags, or
releases cut on a maintenance line the base does not contain, such as a hotfix. Each stream's
clients hold their own release. So a surface is judged against the last release of every stream
that ships it, and it breaks if it breaks against any of them. A release on a declared maintenance
line qualifies for that line's stream without being an ancestor of the base. Where the gate does
not judge a stream that ships the surface, the consumer records that as residue.

When no ref qualifies, or the artifact cannot be read at it, the gate measures against the PR's
base alone and says so.

*Why:* two derivations of "the last release" drift, and then the gate and the version calculator
disagree about which window a change belongs to. A fallback ref is worse than none: a shallow CI
clone can make the PR's own head the "root", and every entry point then equals its "released form".
An older genuine tag is as bad, because shape shipped since it reads as window-born. A gate that
cannot find the release and treats everything as new waves every break through. "I could not look"
is not "nothing shipped". The base-only verdict is stricter than the release-relative one everywhere
except two places, which only the release can reveal: a re-birth, and a property the release
admitted through `additionalProperties`, `unevaluatedProperties` or a name pattern. So falling back
can redden a PR, and passes nothing COMPAT-1 refuses except those two. The pre-publish check in
COMPAT-1 catches a re-birth, and catches the second only where the comparator judges it; otherwise
it remains the residue COMPAT-1 records.

## Bite cases

A gate that claims COMPAT-1 and COMPAT-2 carries these in its bite test ([TW-5](tripwires.md)).

COMPAT-1:

1. A schema born in the window, reached only through a new optional property of a released
   operation, changes in a way that base against head reports as a break (a property of it becomes
   required). It passes as growth. This is temper#1388's shape.
2. A released operation whose resolved form breaks against the release fails without a declared
   break.
3. With the release reference removed from the gate, case 1 fails.
4. A re-birth that differs from its released form fails, once as an operation and once as a single
   property. Under the per-entry-point fallback, it fails too.
5. A window-born parameter component on a released operation that becomes required fails.
6. An operation that an earlier PR broke, with a declared break, passes when this PR only grows it
   or relaxes the break (a request property's `maxLength` 10→5 declared, then 5→8). It fails when
   this PR breaks it further, including deeper at the same place (a second enum value removed).
7. An operation whose path template parameter was renamed is judged against its released form.
8. A property the release admitted through `additionalProperties`, then narrowed by name, fails.
9. A changed security scheme, webhook payload or discriminator mapping value on a released entry
   point fails.
10. A released entry point that this PR removes fails. One that an earlier declared PR removed does
    not.
11. A component no entry point reaches, broken by this PR, fails; one this PR makes reachable is
    not read as removed.
12. A rename inside a reference cycle passes. A change to servers or to a path-item parameter of a
    released operation fails. A deepened break fails under the fallback with a merging comparator.
13. A break that landed undeclared under the base-only verdict is refused before publish.

COMPAT-2:

14. With no release ref resolvable, the gate gives the base-only verdict.
15. A shallow clone without tags gives the base-only verdict, never a verdict against the fallback
    commit.
16. A tag in the release form, on a commit that is neither an ancestor of the base nor on a
    declared maintenance line, is not taken as the release.
17. A clone holding an older release tag, but not the newest, gives the base-only verdict, and so
    does a remote whose tags cannot be listed. On a declared maintenance line, its newest tag is
    measured against, not an older one.
18. A release tag cut on the base while the PR is open is the one the gate measures against at
    merge.

## Not specified

- **Which surfaces need a compatibility gate.** That is a separate ruling per surface. A consumer
  without one records these rules as `n/a`, with that reason.
- **A shared checker.** None ships from this repository. Only the consumer's own comparator can
  enforce the rule, and a copy here would be either a trivial presence check or a second
  comparator. A shared "last release" helper would be the second derivation COMPAT-2 forbids.
- **How a comparator resolves references** (inlining, or walking with a visited set). Any method
  that judges the resolved form, and ends on a cycle as the terms above say, will do.
