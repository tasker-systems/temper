//! `--with` / `--without` → [`SectionSet`] — the vocabulary that replaced `--meta-only`.
//!
//! `--meta-only` named a *response shape* and so meant two different things depending on the
//! command it was attached to: on `show` it *removed* the body, on `list` it *added* both meta
//! tiers. Once both commands answer in one [`ResourceView`], a flag with two meanings has no
//! coherent one. Sections name the *parts* instead, so each is added or removed independently —
//! including `show --with edges --without body`, which the old design could not express at all.
//!
//! [`ResourceView`]: temper_core::types::resource_view::ResourceView

use temper_core::error::{Result, TemperError};
use temper_core::types::resource_view::{ResourceSection, SectionSet};

/// `show`'s defaults: the reconstructed body and the open tier.
///
/// The managed tier is deliberately not here — it is not a section at all, because
/// `ResourceView::managed_meta` is always present. `--without body` is what `--meta-only`
/// used to mean on this command.
#[must_use]
pub fn show_defaults() -> SectionSet {
    [ResourceSection::Body, ResourceSection::OpenMeta]
        .into_iter()
        .collect()
}

/// `list`'s defaults: neither optional tier, matching the default page a `list` has always
/// returned. `--with open-meta` is what `--meta-only` used to mean on this command.
#[must_use]
pub fn list_defaults() -> SectionSet {
    SectionSet::default()
}

/// The section names `show --with`/`--without` accept — the whole vocabulary.
///
/// Derived from [`ResourceSection::ALL`] rather than typed out, so a section cannot be added
/// to the enum and left unreachable from the surface that exists to reach it.
#[must_use]
pub fn show_section_parser() -> clap::builder::PossibleValuesParser {
    clap::builder::PossibleValuesParser::new(ResourceSection::ALL.map(ResourceSection::as_str))
}

/// The section names `list --with`/`--without` accept — **`body` is not among them**.
///
/// Derived from [`ResourceSection::LIST`] rather than typed out, for the same reason
/// [`show_section_parser`] derives from `ALL`: this list used to be a local
/// `[ResourceSection::OpenMeta]` while the *server* parsed against `ALL` and filled `body`
/// for anyone who asked. That made the CLI the only door enforcing the rule — MCP, raw HTTP
/// and temper-rb walked straight past it. The refusal now has one definition and both doors
/// read it, so the surface cannot be stricter than the thing it fronts.
///
/// Why the set is what it is — the unbounded body payload, and `edges` having nothing to fill
/// it on this door — is on [`ResourceSection::LIST`].
#[must_use]
pub fn list_section_parser() -> clap::builder::PossibleValuesParser {
    clap::builder::PossibleValuesParser::new(ResourceSection::LIST.map(ResourceSection::as_str))
}

/// Resolve `--with` / `--without` against a command's defaults.
///
/// `defaults ∪ with ∖ without`. Unknown names are refused by `ResourceSection`'s
/// [`FromStr`](std::str::FromStr), which names the whole valid set in its message.
///
/// **A section named on both sides is a contradiction, and the resolution is a refusal
/// rather than a winner.** Set arithmetic has an obvious answer here — `∖` applied after
/// `∪` makes `--without` win — and taking it would be the bug: the caller has told us two
/// incompatible things about one section, so *both* possible outcomes are half wrong, and
/// picking one silently means the caller learns nothing. Precedence is only defensible
/// between a *general* setting and a *specific* override; these are two equally specific
/// statements about the same section.
pub fn resolve_sections(
    with: &[String],
    without: &[String],
    defaults: SectionSet,
) -> Result<SectionSet> {
    let added = parse_all(with)?;
    let removed = parse_all(without)?;

    if let Some(section) = ResourceSection::ALL
        .into_iter()
        .find(|s| added.contains(s) && removed.contains(s))
    {
        // `as_str`, not `{section:?}`: the refusal must repeat the word the caller TYPED
        // (`open-meta`), not the Rust variant they have never seen (`OpenMeta`). Matches
        // `ResourceSection`'s `FromStr`, which quotes the rejected input back the same way.
        let name = section.as_str();
        return Err(TemperError::BadRequest(format!(
            "--with and --without both name section {name:?}; \
             a section is either asked for or not — drop one of the two"
        )));
    }

    Ok(ResourceSection::ALL
        .into_iter()
        .filter(|s| (defaults.contains(*s) || added.contains(s)) && !removed.contains(s))
        .collect())
}

/// Parse a repeated/comma-delimited flag's values into sections, preserving the refusal
/// `ResourceSection`'s [`FromStr`](std::str::FromStr) renders.
fn parse_all(names: &[String]) -> Result<Vec<ResourceSection>> {
    names.iter().map(|n| n.parse::<ResourceSection>()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(sections: &[&str]) -> Vec<String> {
        sections.iter().map(|s| (*s).to_string()).collect()
    }

    /// A bare `show` still reconstructs the body — the expensive part is the default,
    /// because the command's whole job is to hand you the document.
    #[test]
    fn show_defaults_include_body() {
        let resolved = resolve_sections(&[], &[], show_defaults()).expect("no flags resolves");

        assert!(
            resolved.contains(ResourceSection::Body),
            "a bare `show` must still carry the body"
        );
        assert!(
            resolved.contains(ResourceSection::OpenMeta),
            "and both meta tiers, which is what made `--meta-only` a strict subset"
        );
    }

    /// `--without body` is `--meta-only`'s replacement on `show`, and it removes exactly the
    /// one section.
    #[test]
    fn show_without_body_drops_only_the_body() {
        let resolved = resolve_sections(&[], &names(&["body"]), show_defaults()).expect("resolves");

        assert!(!resolved.contains(ResourceSection::Body));
        assert!(
            resolved.contains(ResourceSection::OpenMeta),
            "removing the body must not also remove the open tier"
        );
    }

    /// **The combination the old design forbade, and the reason the vocabulary changed.**
    ///
    /// `--meta-only` carried `conflicts_with = "edges"`, so "everything except the body,
    /// plus this resource's edges" — a perfectly ordinary orientation read — was
    /// *inexpressible*. That conflict edge was not protecting an invariant; it existed
    /// because `--meta-only` named a response type, and a type cannot be half-swapped. With
    /// the parts named individually there is nothing left to conflict: adding one section
    /// and removing another are independent operations, and this test is what holds that
    /// independence in place.
    #[test]
    fn show_with_edges_without_body_is_accepted() {
        let resolved = resolve_sections(&names(&["edges"]), &names(&["body"]), show_defaults())
            .expect("--with edges --without body must be accepted, not refused");

        assert!(
            resolved.contains(ResourceSection::Edges),
            "the added section is present"
        );
        assert!(
            !resolved.contains(ResourceSection::Body),
            "the removed section is absent"
        );
        assert!(
            resolved.contains(ResourceSection::OpenMeta),
            "and the untouched default is unaffected"
        );
    }

    /// Naming one section on both sides is refused outright — `resolve_sections` does not
    /// quietly let `--without` (or `--with`) win.
    ///
    /// The bite: an implementation that applied `∖` after `∪` and returned `Ok` would pass
    /// every other test in this module, because on every other input the two agree.
    #[test]
    fn with_and_without_the_same_section_is_a_contradiction() {
        let err = resolve_sections(&names(&["body"]), &names(&["body"]), show_defaults())
            .expect_err("a section asked for and refused at once must not resolve");

        let msg = err.to_string();
        assert!(
            msg.contains("body"),
            "the refusal names the contradicted section so the caller can fix it: {msg}"
        );
    }

    /// The contradiction check is over the *pair*, not over one section — a set naming two
    /// sections with only one of them contradicted is still refused.
    #[test]
    fn a_contradiction_is_found_among_uncontradicted_siblings() {
        let err = resolve_sections(
            &names(&["edges", "open-meta"]),
            &names(&["open-meta"]),
            show_defaults(),
        )
        .expect_err("one contradicted section poisons the whole resolution");

        assert!(err.to_string().contains("open-meta"), "{err}");
    }

    /// An unknown name is refused with the whole valid set named — the caller is usually an
    /// agent that guessed and has nothing else to consult mid-call.
    #[test]
    fn an_unknown_section_name_is_refused_naming_the_valid_set() {
        let err = resolve_sections(&names(&["managed-meta"]), &[], show_defaults())
            .expect_err("the managed tier is not a section");

        let msg = err.to_string();
        for valid in ResourceSection::ALL.map(ResourceSection::as_str) {
            assert!(
                msg.contains(valid),
                "the refusal must name `{valid}`: {msg}"
            );
        }
    }

    /// A bare `list` carries neither optional tier — the default page is unchanged by the
    /// vocabulary swap.
    #[test]
    fn list_defaults_carry_no_optional_section() {
        let resolved = resolve_sections(&[], &[], list_defaults()).expect("resolves");

        for section in ResourceSection::ALL {
            assert!(
                !resolved.contains(section),
                "a bare `list` must not ask for {section}"
            );
        }
    }

    /// **`list --with body` errors.** The refusal is clap's, from the restricted value set on
    /// `list`'s `--with`, so it names the values that *are* accepted rather than the ones the
    /// enum happens to have — which is what a caller needs. `resolve_sections` itself is
    /// section-agnostic and would happily resolve it, so this asserts against the parser that
    /// actually guards the surface.
    #[test]
    fn list_never_offers_body() {
        use clap::builder::TypedValueParser;

        let cmd = clap::Command::new("list");
        let arg = clap::Arg::new("with");

        let refused = list_section_parser()
            .parse_ref(&cmd, Some(&arg), std::ffi::OsStr::new("body"))
            .expect_err("`list --with body` must not parse");
        assert!(
            refused.to_string().contains("open-meta"),
            "the refusal names what list DOES offer: {refused}"
        );

        list_section_parser()
            .parse_ref(&cmd, Some(&arg), std::ffi::OsStr::new("open-meta"))
            .expect("the tier `--meta-only` used to add is still reachable on list");

        show_section_parser()
            .parse_ref(&cmd, Some(&arg), std::ffi::OsStr::new("body"))
            .expect("and `body` stays a section on `show`, where it is the default");
    }
}
