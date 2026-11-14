//! Conditional meta writes: the validation an `expect` gets before the transaction, and the one
//! statement that judges it under the resource's writer lock.
//!
//! The sequence that uses these lives in `DbBackend::update_resource`. Only the pieces that read no
//! state, and the judgment itself, are here, so that the order of refusals stays visible in one
//! function.

use serde_json::Value;
use sqlx::PgConnection;

use temper_core::error::TemperError;
use temper_core::types::data_artifact::holds_nul;
use temper_core::types::ids::ResourceId;
use temper_core::types::precondition::{
    FailedExpectation, MetaExpect, PreconditionFailure, PreconditionTier,
};
use temper_substrate::keys::{is_managed_property_key, key_fate, KeyFate};

/// The `400`s an `expect` earns from its own shape, before anything is read: every rule here is a
/// function of the request alone.
///
/// - an expectation that names no key;
/// - U+0000 anywhere in it, which `jsonb` cannot hold and would otherwise fail under the lock;
/// - an `open` key that is not open-tier state: `facet`, `doc_type`, a managed key, or an identity
///   key the meta path never stores as a property;
/// - a `managed` key outside the managed workflow and provenance vocabulary;
/// - a key also named in `open_meta_add`, whose union is computed from a read taken before the
///   guard is judged, and whose list union drops a stored non-array value outright.
pub(crate) fn refuse_malformed(
    expect: &MetaExpect,
    open_meta_add: Option<&Value>,
) -> Result<(), TemperError> {
    if expect.is_empty() {
        return Err(TemperError::BadRequest(
            "expect names no key; send at least one expectation under `open` or `managed`, or \
             omit `expect` for an unconditional write"
                .into(),
        ));
    }
    // First, so no later message can quote a key that holds the character.
    let nul = expect
        .open
        .iter()
        .chain(&expect.managed)
        .any(|(key, value)| key.contains('\0') || holds_nul(value));
    if nul {
        return Err(TemperError::BadRequest(
            "expect holds the character U+0000 (NUL), which no stored value can hold; remove it \
             from every key and string in `expect`"
                .into(),
        ));
    }
    for key in expect.open.keys() {
        if key == "facet" || key == "doc_type" || is_managed_property_key(key) {
            return Err(TemperError::BadRequest(format!(
                "expect.open.{key} is not open-tier state and cannot be guarded as one; a managed \
                 key goes under expect.managed, and `facet` and `doc_type` cannot be guarded"
            )));
        }
        if key_fate(key) != KeyFate::Property {
            return Err(TemperError::BadRequest(format!(
                "expect.open.{key} names an identity key, which is never stored as metadata and \
                 cannot be guarded"
            )));
        }
    }
    for key in expect.managed.keys() {
        if !is_managed_property_key(key) {
            return Err(TemperError::BadRequest(format!(
                "expect.managed.{key} is not a managed metadata key; guard open-tier keys under \
                 expect.open"
            )));
        }
    }
    if let Some(added) = open_meta_add.and_then(Value::as_object) {
        if let Some(key) = added
            .keys()
            .find(|k| expect.open.contains_key(*k) || expect.managed.contains_key(*k))
        {
            return Err(TemperError::BadRequest(format!(
                "{key} is named in both expect and open_meta_add; an additive write cannot be \
                 guarded on the key it adds to. Read the key, then write the whole list through \
                 open_meta with the guard"
            )));
        }
    }
    Ok(())
}

/// The `400` for an `expect` on a request that writes nothing, judged on the assembled write: no
/// property set (a same-value set counts, because it fires events), no open key unset, and no
/// body, title, move or goal change. A managed key the meta path drops writes nothing. A check
/// without a write is a read, and is refused as one.
pub(crate) fn refuse_without_a_write(writes_something: bool) -> Result<(), TemperError> {
    if writes_something {
        Ok(())
    } else {
        Err(TemperError::BadRequest(
            "expect was sent on a request that writes nothing; a precondition guards a write. \
             Read the resource to check its metadata"
                .into(),
        ))
    }
}

/// Judge every expectation against the resource's newest live property rows, in one statement on
/// `conn`. The caller holds the resource's writer lock on `conn`'s transaction and has already
/// established that the caller can read the resource, so `current` discloses only what a read
/// would.
///
/// `Ok(Err(failure))` when any expectation fails; the failure lists exactly those, `open` before
/// `managed` and by key within a tier.
pub(crate) async fn judge(
    conn: &mut PgConnection,
    resource: ResourceId,
    expect: &MetaExpect,
) -> anyhow::Result<Result<(), PreconditionFailure>> {
    let open = serde_json::to_value(&expect.open)?;
    let managed = serde_json::to_value(&expect.managed)?;
    // `live` is the rows `readback::meta` classifies, newest first per key, so its newest-wins is
    // that read's last-write-wins over `ORDER BY created, id`. `NULLIF` applies AFTER `DISTINCT ON`
    // and never filters inside `live`: a newest row holding a top-level `null` reads as absent, as
    // the read shows it, and an older value beneath it must never surface. A missing row is the
    // `LEFT JOIN`'s SQL NULL. The comparison is jsonb `=` after the write-time normalization, so a
    // bare-string `tags` matches its stored array and `1` matches `1.0`; never compare in Rust.
    let rows = sqlx::query!(
        r#"WITH exp AS (
             SELECT 'open'::text AS tier, key, value FROM jsonb_each($2::jsonb)
             UNION ALL
             SELECT 'managed'::text, key, value FROM jsonb_each($3::jsonb)
           ), live AS (
             SELECT DISTINCT ON (property_key) property_key, property_value
               FROM kb_properties
              WHERE owner_table = 'kb_resources' AND owner_id = $1 AND NOT is_folded
                AND property_key IN (SELECT key FROM exp)
              ORDER BY property_key, created DESC, id DESC
           )
           SELECT e.tier AS "tier!",
                  e.key AS "key!",
                  e.value AS "expected!",
                  NULLIF(l.property_value, 'null'::jsonb) AS current,
                  CASE WHEN e.value = 'null'::jsonb
                       THEN NULLIF(l.property_value, 'null'::jsonb) IS NULL
                       ELSE NULLIF(l.property_value, 'null'::jsonb) IS NOT NULL
                            AND l.property_value = _property_value_normalized(e.key, e.value)
                  END AS "holds!"
             FROM exp e
             LEFT JOIN live l ON l.property_key = e.key
            ORDER BY e.tier = 'managed', e.key"#,
        resource.uuid(),
        open,
        managed,
    )
    .fetch_all(&mut *conn)
    .await?;

    let failed: Vec<FailedExpectation> = rows
        .into_iter()
        .filter(|row| !row.holds)
        .map(|row| FailedExpectation {
            tier: if row.tier == "managed" {
                PreconditionTier::Managed
            } else {
                PreconditionTier::Open
            },
            key: Some(row.key),
            expected: row.expected,
            current: row.current.unwrap_or(Value::Null),
        })
        .collect();
    if failed.is_empty() {
        Ok(Ok(()))
    } else {
        Ok(Err(PreconditionFailure { failed }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn expect(v: Value) -> MetaExpect {
        serde_json::from_value(v).expect("a well-formed expectation")
    }

    fn refused(e: &MetaExpect, add: Option<Value>) -> String {
        match refuse_malformed(e, add.as_ref()) {
            Err(TemperError::BadRequest(m)) => m,
            other => panic!("expected a 400, got {other:?}"),
        }
    }

    #[test]
    fn an_open_guard_on_an_unlisted_temper_key_is_allowed() {
        refuse_malformed(&expect(json!({"open": {"temper-foo": null}})), None)
            .expect("an unlisted temper- key is open-tier state");
        refuse_malformed(
            &expect(json!({"open": {"claim": "x"}, "managed": {"temper-stage": "doing"}})),
            Some(&json!({"tags": ["y"]})),
        )
        .expect("a guard and an add on different keys");
    }

    #[test]
    fn each_tier_rule_refuses() {
        for key in ["facet", "doc_type", "temper-stage", "temper-pr"] {
            let m = refused(&expect(json!({"open": {key: null}})), None);
            assert!(m.contains(&format!("expect.open.{key}")), "{m}");
        }
        for key in [
            "temper-title",
            "temper-slug",
            "temper-id",
            "temper-context",
            "temper-goal",
            "temper-type",
        ] {
            let m = refused(&expect(json!({"open": {key: null}})), None);
            assert!(m.contains("identity key"), "{key}: {m}");
        }
        for key in ["claim", "temper-title", "facet"] {
            let m = refused(&expect(json!({"managed": {key: null}})), None);
            assert!(m.contains(&format!("expect.managed.{key}")), "{m}");
        }
    }

    #[test]
    fn nul_anywhere_refuses_without_quoting_it() {
        for e in [
            json!({"open": {"a\u{0}b": 1}}),
            json!({"open": {"a": "x\u{0}"}}),
            json!({"open": {"a": {"deep": ["\u{0}"]}}}),
            json!({"managed": {"temper-stage": "\u{0}"}}),
        ] {
            let m = refused(&expect(e), None);
            assert!(m.contains("U+0000") && !m.contains('\0'), "{m}");
        }
    }

    #[test]
    fn the_same_key_in_expect_and_open_meta_add_refuses() {
        let m = refused(
            &expect(json!({"open": {"derived_from": "X"}})),
            Some(json!({"derived_from": ["Y"]})),
        );
        assert!(
            m.contains("derived_from") && m.contains("open_meta_add"),
            "{m}"
        );
    }

    #[test]
    fn an_empty_expectation_refuses() {
        refused(&expect(json!({})), None);
        refused(&expect(json!({"open": {}, "managed": {}})), None);
    }
}
