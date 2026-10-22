//! The reference evaluator — an ask's predicates over one artifact's content, with **jsonb**
//! semantics rather than `serde_json`'s (spec §3.6, leg 3).
//!
//! It is the oracle the SQL's differential runs against, so each rule here is the database's:
//!
//! - **Paths step object keys only** (§3.1). A segment meeting anything but an object is absent,
//!   so `["rows", "0", "x"]` over a stored `rows` *array* is absent, never `rows[0].x`.
//! - **Presence is conjoined with every operator** (§3.2). An absent path matches nothing; a JSON
//!   `null` at the path is present.
//! - **`contains` is jsonb's `@>`**, including its one top-level exception: an array contains a
//!   primitive without wrapping (`'["x"]' @> '"x"'` is true, the reverse false — the measured note
//!   on `PropertyOp::Contains`). Nested, an array never contains a primitive. The values it reads
//!   are the ones `validate_ask` bound, so an array path's elements arrive already wrapped (§3.4).
//! - **`compare` is type-guarded** (`jsonb_typeof(value) = jsonb_typeof(bound)`, as
//!   `_temper_property_op_match` in `20260816000010`) and then jsonb's ordering.
//!
//! # Numbers compare as exact decimals, not as `f64` (decision P9)
//!
//! jsonb numbers are `numeric`: `5` and `5.0` are equal, and ordering is exact. This crate's
//! `serde_json` is built **without** `arbitrary_precision`, so a `Number` holds a `u64`, an `i64` or
//! an `f64`, and an `f64` compared as an `f64` is not exact. So a number is compared through its
//! decimal text: `Number`'s `Display` writes the shortest text that reads back as the same value,
//! and that text is compared digit by digit, never through a float. For every number the
//! differential's generator emits — integers within ±2^53 and finite decimals with at most six
//! fractional digits — that text is exactly the decimal the generator wrote, so the comparison is
//! the database's. A number past `f64`'s precision was already rounded when it was parsed; no
//! comparison can recover it, and none of the generator's numbers are such. Under
//! `arbitrary_precision`, `Display` writes the number's original text, and the same comparison is
//! exact for every number.
//!
//! # Strings compare through the caller's comparator
//!
//! jsonb orders strings by the database collation, and a byte comparison disagrees with it on
//! non-ASCII text. So `compare` over strings — and over object keys inside a compared container —
//! asks `cmp`, and the differential supplies one that asks Postgres (decision P9). String
//! *equality* is byte equality in jsonb, as here.

use std::cmp::Ordering;

use serde_json::{Map, Number, Value};

use super::ArtifactPredicate;
use crate::types::query::filter::{OrdOp, PropertyOp};

/// Whether `content` satisfies every predicate. An ask with no predicates is the whole governed
/// population (spec §2.1), so it matches everything.
///
/// **A test oracle, never a request path.** It runs unbounded — its containment is O(n·m) over an
/// array against an array, with no step budget — because it is the reference the SQL is
/// differential-tested against (§3.6). Code that serves a request uses the counted variants,
/// `jsonb_contains_within` and `nested_contains_within`, under a step budget.
pub fn matches(
    ask_predicates: &[ArtifactPredicate],
    content: &Value,
    cmp: &dyn Fn(&str, &str) -> Ordering,
) -> bool {
    ask_predicates
        .iter()
        .all(|p| predicate_matches(p, content, cmp))
}

fn predicate_matches(
    predicate: &ArtifactPredicate,
    content: &Value,
    cmp: &dyn Fn(&str, &str) -> Ordering,
) -> bool {
    let Some(value) = get(content, &predicate.path) else {
        return false;
    };
    match &predicate.op {
        PropertyOp::HasKey => true,
        PropertyOp::Contains { values } => values.iter().any(|probe| jsonb_contains(value, probe)),
        PropertyOp::Compare {
            direction,
            value: bound,
        } => {
            JsonKind::of(value) == JsonKind::of(bound) && {
                let order = jsonb_cmp(value, bound, cmp);
                match direction {
                    OrdOp::Gt => order == Ordering::Greater,
                    OrdOp::Gte => order != Ordering::Less,
                    OrdOp::Lt => order == Ordering::Less,
                    OrdOp::Lte => order != Ordering::Greater,
                }
            }
        }
    }
}

/// The value at a path, stepping object keys only. Anything but an object on the way is absent.
fn get<'v>(content: &'v Value, path: &[String]) -> Option<&'v Value> {
    path.iter()
        .try_fold(content, |at, segment| at.as_object()?.get(segment))
}

/// `stored @> probe`, as `matches` evaluates a `contains` predicate over content.
fn jsonb_contains(stored: &Value, probe: &Value) -> bool {
    // `usize::MAX` steps cannot run out: each step is one node of a value already held in memory.
    let mut unbounded = usize::MAX;
    jsonb_contains_within(stored, probe, &mut unbounded).unwrap_or(false)
}

/// `stored @> probe`, with the top-level exception — an array contains a primitive — spending one
/// step per node compared, and the bytes of each key looked up and each string compared at
/// [`BYTES_PER_STEP`]. `None` when the steps run out before it can say.
///
/// One definition for both callers: the evaluator above, unbounded, and the validator's enum
/// membership, which holds a caller's probes to a budget.
pub(super) fn jsonb_contains_within(
    stored: &Value,
    probe: &Value,
    steps: &mut usize,
) -> Option<bool> {
    match (stored, probe) {
        (Value::Array(elements), p) if !is_container(p) => {
            for element in elements {
                take(steps)?;
                if scalar_eq_within(element, p, steps)? {
                    return Some(true);
                }
            }
            Some(false)
        }
        _ => nested_contains_within(stored, probe, steps),
    }
}

/// jsonb's containment below the top level, where the primitive exception does not apply: an
/// object contains an object whose every key it has with a contained value; an array contains an
/// array each of whose elements some element of it contains; a scalar contains only an equal
/// scalar; and nothing else contains anything. One step per node compared, and the bytes of each
/// key looked up and each string compared at [`BYTES_PER_STEP`] — a lookup hashes or compares the
/// probe's key whatever the map, so its length is charged before it is read.
pub(super) fn nested_contains_within(
    stored: &Value,
    probe: &Value,
    steps: &mut usize,
) -> Option<bool> {
    take(steps)?;
    Some(match (stored, probe) {
        (Value::Object(s), Value::Object(p)) => {
            for (key, pv) in p {
                spend(steps, 1 + key.len() / BYTES_PER_STEP)?;
                let Some(sv) = s.get(key) else {
                    return Some(false);
                };
                if !nested_contains_within(sv, pv, steps)? {
                    return Some(false);
                }
            }
            true
        }
        (Value::Array(s), Value::Array(p)) => {
            for pe in p {
                let mut held = false;
                for se in s {
                    if nested_contains_within(se, pe, steps)? {
                        held = true;
                        break;
                    }
                }
                if !held {
                    return Some(false);
                }
            }
            true
        }
        (s, p) if is_container(s) || is_container(p) => false,
        (s, p) => scalar_eq_within(s, p, steps)?,
    })
}

fn take(steps: &mut usize) -> Option<()> {
    spend(steps, 1)
}

fn spend(steps: &mut usize, n: usize) -> Option<()> {
    *steps = steps.checked_sub(n)?;
    Some(())
}

/// How many bytes one containment step pays for, beyond the step a node already costs: an object
/// key looked up costs one step plus one per this many of its bytes, and two strings compared
/// byte for byte cost one per this many bytes of them.
///
/// # 64
///
/// The walk charges author text at the same rate (`walk::BYTES_PER_VISIT`). A key or a string of
/// tens of bytes costs one step, so a real ask is unchanged; a 16 KiB probe key costs 257, so the
/// step budget bounds the bytes hashed and compared — `MAX_MEMBERSHIP_STEPS` × 64, 64 MiB — and not
/// only the nodes visited.
const BYTES_PER_STEP: usize = 64;

/// [`scalar_eq`], charged for the bytes it compares. Two strings are compared byte for byte only
/// when their lengths are equal, so only then is their length charged.
fn scalar_eq_within(a: &Value, b: &Value, steps: &mut usize) -> Option<bool> {
    let compared = match (a, b) {
        (Value::String(x), Value::String(y)) if x.len() == y.len() => x.len(),
        _ => 0,
    };
    spend(steps, compared / BYTES_PER_STEP)?;
    Some(scalar_eq(a, b))
}

fn is_container(value: &Value) -> bool {
    value.is_array() || value.is_object()
}

/// jsonb scalar equality: same type, numbers equal as decimals, strings byte-equal.
fn scalar_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => number_cmp(x, y) == Ordering::Equal,
        (Value::String(x), Value::String(y)) => x == y,
        _ => false,
    }
}

/// A text equal for two values exactly when they are jsonb-equal: `5` and `5.0` alike, object keys
/// in byte order, array order kept. The walk keys `enum` members by it, so value sets intersect and
/// union in linear time.
pub(super) fn canonical(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => Decimal::parse(&n.to_string()).write(out),
        Value::String(s) => write_string(s, out),
        Value::Array(elements) => {
            out.push('[');
            for (i, element) in elements.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(element, out);
            }
            out.push(']');
        }
        Value::Object(members) => {
            let mut keys: Vec<&String> = members.keys().collect();
            keys.sort();
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(key, out);
                out.push(':');
                write_canonical(&members[key], out);
            }
            out.push('}');
        }
    }
}

/// A string quoted, with the quote and the escape character escaped — enough that the text ends
/// where the string does, so distinct strings stay distinct.
fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        if matches!(c, '"' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
}

/// `jsonb_typeof`'s classes, in jsonb's btree order:
/// `null < string < number < boolean < array < object`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum JsonKind {
    Null,
    String,
    Number,
    Boolean,
    Array,
    Object,
}

impl JsonKind {
    fn of(value: &Value) -> JsonKind {
        match value {
            Value::Null => JsonKind::Null,
            Value::String(_) => JsonKind::String,
            Value::Number(_) => JsonKind::Number,
            Value::Bool(_) => JsonKind::Boolean,
            Value::Array(_) => JsonKind::Array,
            Value::Object(_) => JsonKind::Object,
        }
    }
}

/// jsonb's btree ordering (`compareJsonbContainers`): by type class; then a scalar by its value; an
/// array by element count and then element by element; an object by pair count and then pair by
/// pair in jsonb's storage order of keys (shorter first, then bytewise), each key compared as a
/// string. Strings go through `cmp`.
fn jsonb_cmp(a: &Value, b: &Value, cmp: &dyn Fn(&str, &str) -> Ordering) -> Ordering {
    let by_kind = JsonKind::of(a).cmp(&JsonKind::of(b));
    if by_kind != Ordering::Equal {
        return by_kind;
    }
    match (a, b) {
        (Value::String(x), Value::String(y)) => cmp(x, y),
        (Value::Number(x), Value::Number(y)) => number_cmp(x, y),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Array(x), Value::Array(y)) => x.len().cmp(&y.len()).then_with(|| {
            x.iter()
                .zip(y)
                .map(|(p, q)| jsonb_cmp(p, q, cmp))
                .find(|o| *o != Ordering::Equal)
                .unwrap_or(Ordering::Equal)
        }),
        (Value::Object(x), Value::Object(y)) => x.len().cmp(&y.len()).then_with(|| {
            storage_order(x)
                .zip(storage_order(y))
                .map(|((ka, va), (kb, vb))| cmp(ka, kb).then_with(|| jsonb_cmp(va, vb, cmp)))
                .find(|o| *o != Ordering::Equal)
                .unwrap_or(Ordering::Equal)
        }),
        // Same kind, so only `null` against `null` is left.
        _ => Ordering::Equal,
    }
}

/// An object's pairs in jsonb's storage order: keys by length, then bytewise.
fn storage_order(object: &Map<String, Value>) -> impl Iterator<Item = (&str, &Value)> {
    let mut pairs: Vec<(&str, &Value)> = object.iter().map(|(k, v)| (k.as_str(), v)).collect();
    pairs.sort_by(|(a, _), (b, _)| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
    pairs.into_iter()
}

/// Two numbers compared exactly, through their decimal text (see the module header).
fn number_cmp(a: &Number, b: &Number) -> Ordering {
    Decimal::parse(&a.to_string()).cmp(&Decimal::parse(&b.to_string()))
}

/// Whether a number is integral — what JSON Schema's `integer` admits (`5.0` is an integer).
pub(super) fn is_integral(n: &Number) -> bool {
    let d = Decimal::parse(&n.to_string());
    d.digits.is_empty() || d.exponent >= 0
}

/// A decimal as `±digits × 10^exponent`, normalised: no leading or trailing zero digit, and no
/// digits at all for zero (whose sign is ignored, as `numeric`'s is).
#[derive(Debug, PartialEq, Eq)]
struct Decimal {
    negative: bool,
    digits: Vec<u8>,
    exponent: i64,
}

impl Decimal {
    /// Read JSON number text: `-?digits(.digits)?([eE][+-]?digits)?`. `Number`'s `Display` writes
    /// nothing else; an exponent too large for `i64` saturates, which keeps its order.
    fn parse(text: &str) -> Decimal {
        let (negative, rest) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        let (mantissa, exp_text) = match rest.find(['e', 'E']) {
            Some(i) => (&rest[..i], &rest[i + 1..]),
            None => (rest, ""),
        };
        let (int_part, frac_part) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        let mut digits: Vec<u8> = int_part
            .bytes()
            .chain(frac_part.bytes())
            .filter(u8::is_ascii_digit)
            .map(|b| b - b'0')
            .collect();
        let frac_len = i64::try_from(frac_part.len()).unwrap_or(i64::MAX);
        let mut exponent = parse_exponent(exp_text).saturating_sub(frac_len);
        let leading = digits.iter().take_while(|d| **d == 0).count();
        digits.drain(..leading);
        while digits.last() == Some(&0) {
            digits.pop();
            exponent = exponent.saturating_add(1);
        }
        if digits.is_empty() {
            exponent = 0;
        }
        Decimal {
            negative: negative && !digits.is_empty(),
            digits,
            exponent,
        }
    }

    /// The decimal's one spelling: `0`, or `-?digits e exponent`.
    fn write(&self, out: &mut String) {
        if self.digits.is_empty() {
            out.push('0');
            return;
        }
        if self.negative {
            out.push('-');
        }
        out.extend(self.digits.iter().map(|d| char::from(b'0' + d)));
        out.push('e');
        out.push_str(&self.exponent.to_string());
    }

    /// The power of ten of the most significant digit, plus one — the magnitude's order.
    fn magnitude(&self) -> i64 {
        let len = i64::try_from(self.digits.len()).unwrap_or(i64::MAX);
        self.exponent.saturating_add(len)
    }

    fn cmp_magnitude(&self, other: &Decimal) -> Ordering {
        match (self.digits.is_empty(), other.digits.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            // Same leading power of ten: the digit strings, read left to right, decide — a missing
            // trailing digit is a zero.
            (false, false) => self.magnitude().cmp(&other.magnitude()).then_with(|| {
                let width = self.digits.len().max(other.digits.len());
                let at = |d: &Decimal, i: usize| d.digits.get(i).copied().unwrap_or(0);
                (0..width)
                    .map(|i| at(self, i).cmp(&at(other, i)))
                    .find(|o| *o != Ordering::Equal)
                    .unwrap_or(Ordering::Equal)
            }),
        }
    }
}

impl Ord for Decimal {
    fn cmp(&self, other: &Decimal) -> Ordering {
        match (self.negative, other.negative) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (false, false) => self.cmp_magnitude(other),
            (true, true) => other.cmp_magnitude(self),
        }
    }
}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Decimal) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn parse_exponent(text: &str) -> i64 {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let magnitude = digits
        .bytes()
        .filter(u8::is_ascii_digit)
        .fold(0i64, |acc, d| {
            acc.saturating_mul(10).saturating_add(i64::from(d - b'0'))
        });
    if negative {
        -magnitude
    } else {
        magnitude
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn bytes(a: &str, b: &str) -> Ordering {
        a.cmp(b)
    }

    fn pred(path: &[&str], op: PropertyOp) -> ArtifactPredicate {
        ArtifactPredicate {
            path: path.iter().map(|s| s.to_string()).collect(),
            op,
        }
    }

    fn contains_at(path: &[&str], values: Vec<Value>) -> ArtifactPredicate {
        pred(path, PropertyOp::Contains { values })
    }

    fn compare_at(path: &[&str], direction: OrdOp, value: Value) -> ArtifactPredicate {
        pred(path, PropertyOp::Compare { direction, value })
    }

    fn one(p: ArtifactPredicate, content: Value) -> bool {
        matches(&[p], &content, &bytes)
    }

    // ── Containment, as `PropertyOp::Contains` documents it and §3.4 relies on ──────────────

    #[test]
    fn a_top_level_array_contains_a_primitive_and_not_the_reverse() {
        // '["x"]'::jsonb @> '"x"'::jsonb -> t ; '"x"'::jsonb @> '["x"]'::jsonb -> f
        assert!(one(
            contains_at(&["a"], vec![json!("x")]),
            json!({ "a": ["x"] })
        ));
        assert!(!one(
            contains_at(&["a"], vec![json!(["x"])]),
            json!({ "a": "x" })
        ));
    }

    #[test]
    fn an_object_probe_is_not_an_element_of_an_array_until_it_is_wrapped() {
        // '[{"a":1,"b":2}]' @> '{"a":1}' -> f ; the validator's wrap makes it '[{"a":1}]' -> t.
        let content = json!({ "rows": [{ "a": 1, "b": 2 }] });
        assert!(!one(
            contains_at(&["rows"], vec![json!({ "a": 1 })]),
            content.clone()
        ));
        assert!(one(
            contains_at(&["rows"], vec![json!([{ "a": 1 }])]),
            content
        ));
    }

    #[test]
    fn an_array_probe_reads_as_a_sub_array_until_it_is_wrapped() {
        // '[[1,3],2]' @> '[1,3]' -> f ; wrapped, '[[1,3]]' -> t.
        let content = json!({ "xs": [[1, 3], 2] });
        assert!(!one(
            contains_at(&["xs"], vec![json!([1, 3])]),
            content.clone()
        ));
        assert!(one(contains_at(&["xs"], vec![json!([[1, 3]])]), content));
    }

    #[test]
    fn nested_an_array_does_not_contain_a_primitive() {
        // '{"a":[1,2]}' @> '{"a":1}' -> f : the exception is top-level only.
        assert!(!one(
            contains_at(&[], vec![json!({ "a": 1 })]),
            json!({ "a": [1, 2] })
        ));
    }

    #[test]
    fn object_containment_is_partial_and_recursive() {
        let content = json!({ "m": { "unit": "ms", "n": { "x": 1, "y": 2 } } });
        assert!(one(
            contains_at(&["m"], vec![json!({ "n": { "x": 1 } })]),
            content.clone()
        ));
        assert!(!one(
            contains_at(&["m"], vec![json!({ "n": { "z": 1 } })]),
            content
        ));
    }

    #[test]
    fn contains_is_an_or_within_its_values() {
        let content = json!({ "unit": "ms" });
        assert!(one(
            contains_at(&["unit"], vec![json!("s"), json!("ms")]),
            content.clone()
        ));
        assert!(!one(contains_at(&["unit"], vec![]), content));
    }

    // ── Paths step object keys only ───────────────────────────────────────────────────────

    #[test]
    fn a_numeric_segment_over_an_array_is_absent_not_an_index() {
        // Review Focus 2: `["rows","0","x"]` over a stored `rows` ARRAY is absent, not rows[0].x.
        let content = json!({ "rows": [{ "x": 1 }] });
        assert!(!one(pred(&["rows", "0", "x"], PropertyOp::HasKey), content));
        // Over an object whose key is "0", it is present.
        assert!(one(
            pred(&["rows", "0", "x"], PropertyOp::HasKey),
            json!({ "rows": { "0": { "x": 1 } } })
        ));
    }

    #[test]
    fn a_json_null_at_the_path_is_present() {
        assert!(one(pred(&["a"], PropertyOp::HasKey), json!({ "a": null })));
    }

    #[test]
    fn an_absent_path_matches_no_operator() {
        let content = json!({ "b": 1 });
        assert!(!one(pred(&["a"], PropertyOp::HasKey), content.clone()));
        assert!(!one(contains_at(&["a"], vec![json!(1)]), content.clone()));
        assert!(!one(compare_at(&["a"], OrdOp::Gte, json!(0)), content));
    }

    #[test]
    fn no_predicates_is_the_whole_population() {
        assert!(matches(&[], &json!({ "anything": 1 }), &bytes));
    }

    #[test]
    fn predicates_are_and_composed() {
        let content = json!({ "unit": "ms", "count": 12 });
        let both = [
            contains_at(&["unit"], vec![json!("ms")]),
            compare_at(&["count"], OrdOp::Gt, json!(10)),
        ];
        assert!(matches(&both, &content, &bytes));
        let one_fails = [
            contains_at(&["unit"], vec![json!("ms")]),
            compare_at(&["count"], OrdOp::Gt, json!(20)),
        ];
        assert!(!matches(&one_fails, &content, &bytes));
    }

    // ── Numbers, exactly (P9) ─────────────────────────────────────────────────────────────

    #[test]
    fn five_equals_five_point_zero_as_in_jsonb() {
        assert!(one(
            contains_at(&["n"], vec![json!(5.0)]),
            json!({ "n": 5 })
        ));
        assert!(one(
            contains_at(&["n"], vec![json!(5)]),
            json!({ "n": 5.0 })
        ));
        assert!(one(
            compare_at(&["n"], OrdOp::Gte, json!(5.0)),
            json!({ "n": 5 })
        ));
        assert!(!one(
            compare_at(&["n"], OrdOp::Gt, json!(5.0)),
            json!({ "n": 5 })
        ));
    }

    #[test]
    fn decimals_compare_by_their_written_value_not_their_float() {
        assert!(!one(
            compare_at(&["n"], OrdOp::Gt, json!(0.1)),
            json!({ "n": 0.1 })
        ));
        assert!(one(
            compare_at(&["n"], OrdOp::Lt, json!(0.100001)),
            json!({ "n": 0.1 })
        ));
        assert!(one(
            compare_at(&["n"], OrdOp::Lt, json!(-0.5)),
            json!({ "n": -0.75 })
        ));
        assert!(one(
            compare_at(&["n"], OrdOp::Gt, json!(-1)),
            json!({ "n": 0 })
        ));
    }

    #[test]
    fn integers_at_the_edge_of_double_precision_stay_distinct() {
        // 2^53 + 1 has no f64; as an i64 it is exact, and so is the comparison.
        assert!(one(
            compare_at(&["n"], OrdOp::Gt, json!(9_007_199_254_740_992_i64)),
            json!({ "n": 9_007_199_254_740_993_i64 })
        ));
    }

    #[test]
    fn decimal_text_normalises_zeros_signs_and_exponents() {
        let d = |s: &str| Decimal::parse(s);
        assert_eq!(d("5"), d("5.0"));
        assert_eq!(d("50"), d("5e1"));
        assert_eq!(d("0.05"), d("5E-2"));
        assert_eq!(d("-0"), d("0.0"));
        assert!(d("1e21") > d("999999999999999999999"));
        assert!(d("-2") < d("-1.5"));
        assert!(d("0.000001") > d("0"));
    }

    #[test]
    fn integral_is_decided_on_the_decimal() {
        let n = |v: Value| v.as_number().cloned().unwrap_or_else(|| Number::from(0));
        assert!(is_integral(&n(json!(5))));
        assert!(is_integral(&n(json!(5.0))));
        assert!(is_integral(&n(json!(0))));
        assert!(!is_integral(&n(json!(5.5))));
    }

    // ── Compare is type-guarded, and strings go through the comparator ────────────────────

    #[test]
    fn a_bound_of_another_json_type_is_an_honest_empty() {
        // Without the guard a numeric bound would match every string (string > number in jsonb).
        assert!(!one(
            compare_at(&["n"], OrdOp::Gt, json!(1)),
            json!({ "n": "5" })
        ));
        assert!(!one(
            compare_at(&["n"], OrdOp::Lt, json!("z")),
            json!({ "n": 5 })
        ));
    }

    #[test]
    fn strings_are_ordered_by_the_supplied_comparator() {
        let content = json!({ "s": "b" });
        let gt_a = compare_at(&["s"], OrdOp::Gt, json!("a"));
        assert!(matches(std::slice::from_ref(&gt_a), &content, &bytes));
        let reversed = |a: &str, b: &str| b.cmp(a);
        assert!(!matches(&[gt_a], &content, &reversed));
    }

    #[test]
    fn booleans_and_nulls_order_as_jsonb_orders_them() {
        assert!(one(
            compare_at(&["b"], OrdOp::Gt, json!(false)),
            json!({ "b": true })
        ));
        assert!(one(
            compare_at(&["z"], OrdOp::Gte, json!(null)),
            json!({ "z": null })
        ));
        assert!(!one(
            compare_at(&["z"], OrdOp::Gt, json!(null)),
            json!({ "z": null })
        ));
    }

    #[test]
    fn containers_order_by_size_then_element_by_element() {
        assert!(one(
            compare_at(&["a"], OrdOp::Gt, json!([9, 9])),
            json!({ "a": [1, 2, 3] })
        ));
        assert!(one(
            compare_at(&["a"], OrdOp::Lt, json!([1, 3])),
            json!({ "a": [1, 2] })
        ));
        // Same pair count: keys in storage order (shorter first), then values.
        assert!(one(
            compare_at(&["o"], OrdOp::Gt, json!({ "k": 1, "kk": 1 })),
            json!({ "o": { "k": 2, "kk": 0 } })
        ));
    }

    #[test]
    fn canonical_text_is_equal_exactly_for_jsonb_equal_values() {
        let same = |a: Value, b: Value| canonical(&a) == canonical(&b);
        assert!(same(
            json!({ "a": 1, "b": [2.0] }),
            json!({ "b": [2], "a": 1.0 })
        ));
        assert!(same(json!(-0.0), json!(0)));
        assert!(same(json!(50), json!(5e1)));
        assert!(!same(json!([1, 2]), json!([2, 1])));
        assert!(!same(json!("1"), json!(1)));
        assert!(!same(json!(null), json!("null")));
        // The quote and the escape are escaped, so a string cannot pose as two.
        assert!(!same(json!(["a\",\"b"]), json!(["a", "b"])));
        assert!(!same(json!({ "a\":1,\"b": 2 }), json!({ "a": 1, "b": 2 })));
    }

    #[test]
    fn counted_containment_stops_when_its_steps_run_out() {
        let stored = json!([1, 2, 3, 4]);
        let mut plenty = 100;
        assert_eq!(
            jsonb_contains_within(&stored, &json!(4), &mut plenty),
            Some(true)
        );
        let mut few = 2;
        assert_eq!(jsonb_contains_within(&stored, &json!(4), &mut few), None);
        // Below the top level an array never contains a primitive.
        let mut steps = 100;
        assert_eq!(
            nested_contains_within(&stored, &json!(4), &mut steps),
            Some(false)
        );
    }

    /// The steps one counted containment spends, with plenty to spend.
    fn spent(stored: &Value, probe: &Value, top_level: bool) -> usize {
        let plenty = 1_000;
        let mut steps = plenty;
        let held = if top_level {
            jsonb_contains_within(stored, probe, &mut steps)
        } else {
            nested_contains_within(stored, probe, &mut steps)
        };
        assert_eq!(held, Some(true), "{stored} @> {probe}");
        plenty - steps
    }

    /// An object key the probe supplies is charged by its length before it is looked
    /// up, since the lookup hashes (or compares) all of it. `{K: 1}` with a 6,400-byte `K` costs
    /// the object's step, 1 + 6,400 / 64 = 101 for the key, and the value's step: 103. A one-byte
    /// key costs 3. Bites the per-key charge in `nested_contains_within`: without it both cost 2.
    #[test]
    fn a_probe_key_is_charged_steps_by_its_length() {
        let key = "k".repeat(6_400);
        let long = json!({ key: 1 });
        assert_eq!(spent(&long, &long, false), 103);
        let short = json!({ "k": 1 });
        assert_eq!(spent(&short, &short, false), 3);
    }

    /// Two strings of one length are compared byte for byte, so the comparison is
    /// charged 6,400 / 64 = 100 steps beyond the node's one, nested or as an element at the top
    /// level. Bites the charge in `scalar_eq_within`: without it each costs one step.
    #[test]
    fn strings_compared_byte_for_byte_are_charged_steps_by_their_length() {
        let long = json!("x".repeat(6_400));
        assert_eq!(spent(&long, &long, false), 101);
        assert_eq!(spent(&json!([long.clone()]), &long, true), 101);
        // Strings of different lengths are unequal without reading them, and cost one step.
        let mut steps = 1_000;
        let longer = json!("x".repeat(6_401));
        assert_eq!(
            nested_contains_within(&long, &longer, &mut steps),
            Some(false)
        );
        assert_eq!(steps, 999);
    }
}
