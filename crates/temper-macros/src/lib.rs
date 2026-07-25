//! Attribute macros for temper's span conventions.
//!
//! One macro so far, and it exists for a reason worth stating rather than assuming: **a constant
//! that exists "so things cannot drift" prevents nothing unless something asserts its consumers
//! against it.** `temper_services::backend::ACT_SPAN_FIELDS` named the act-grain field set, and ten
//! `#[tracing::instrument]` attributes then spelled that set out by hand — so the constant could
//! claim a field no span carried, and every gate would still pass. The eleventh write command
//! (`record_citation_audit`) simply forgot the attribute altogether and nothing noticed.
//!
//! `tracing`'s span *name* is static metadata per macro invocation, so the same constraint that made
//! `root_span!` a macro applies here — and attribute arguments cannot be produced by a
//! `macro_rules!` at all (`#[tracing::instrument(fields(act_fields!()))]` does not parse). A
//! proc-macro attribute is the mechanism that leaves exactly one declaration of the field list.

use proc_macro::TokenStream;

/// The act span: the parent for one authored write's events.
///
/// Expands to
/// `#[tracing::instrument(skip_all, fields(correlation_id = Empty, invocation_id = Empty))]`,
/// which is what every write command in `temper_services::backend::db_backend` used to spell for
/// itself. `temper_services`'s `act_span_declares_every_act_field` asserts this expansion against
/// `ACT_SPAN_FIELDS`, so the constant and the spans cannot disagree.
///
/// ## `skip_all` is part of the macro, not a per-site decision
///
/// Commands carry request bodies, tokens and secrets. `#[tracing::instrument]` records **every**
/// argument by default, so the correct attribute here has always needed `skip_all` — and a new
/// command's author had to know that. Folding it in makes the leak unrepresentable rather than
/// remembered: the ids that *should* be on the span are recorded explicitly by `act_context`, which
/// is the one place every authored write already passes through.
///
/// ## It does not parse the item
///
/// The annotated function is passed through verbatim. That is what lets this compose with
/// `#[async_trait]` — whichever expands first, the other sees an item shaped the way it expects,
/// including after async_trait has rewritten `async fn` into a boxed-future `fn`.
#[proc_macro_attribute]
pub fn act_span(args: TokenStream, item: TokenStream) -> TokenStream {
    if !args.is_empty() {
        // A field set that admits per-site additions is a field set with no single definition,
        // which is the defect this macro exists to close. Reach for `#[tracing::instrument]`
        // directly if a span genuinely needs different fields — then it is not an act span.
        return r#"compile_error!("`act_span` takes no arguments: the act field set has one definition, in temper-macros, asserted against temper_services::backend::ACT_SPAN_FIELDS");"#
            .parse()
            .expect("compile_error! parses");
    }

    let mut expanded: TokenStream = ACT_SPAN_ATTRIBUTE.parse().expect("the attribute parses");
    expanded.extend(item);
    expanded
}

/// The single declaration of the act-grain field set.
///
/// Fully-qualified (`::tracing`) so it resolves in any crate that applies the attribute, rather than
/// depending on what that crate happens to have in scope.
const ACT_SPAN_ATTRIBUTE: &str = "#[::tracing::instrument(skip_all, fields(\
     correlation_id = ::tracing::field::Empty, \
     invocation_id = ::tracing::field::Empty))]";
