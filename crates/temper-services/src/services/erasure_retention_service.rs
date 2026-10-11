//! The deployment's retention policy for person erasure (R4 of the 2026-10-10 erasure inventory).
//!
//! A person erasure removes every identifier of the subject and every free-text field about them.
//! An organisation may keep a category of free text or replay evidence by naming it here, with a
//! basis and a period; the act then keeps it and its record names it `retained under policy
//! <name>`. Identifiers and credentials are not retainable: [`RetentionCategory`] does not name
//! one, and the table's CHECK is the same closed set.
//!
//! System admin only: every function takes the sealed [`SystemAdmin`] proof. A change is not a
//! ledger event, like the instance settings it sits beside; the act records the policy it applied,
//! as it stood, in its own `principal_erased` payload, and that is the record an audit reads.
//!
//! **Nothing deletes kept rows when their period ends.** The period is stated, not enforced; that
//! sweep is its own task.

use sqlx::PgPool;

use temper_core::types::erasure::{
    RetentionCategory, RetentionPolicies, RetentionPolicy, RetentionPolicySet,
};
use temper_core::types::ids::ProfileId;

use crate::auth::SystemAdmin;
use crate::error::{ApiError, ApiResult};

fn category_from(s: &str) -> ApiResult<RetentionCategory> {
    serde_json::from_value(serde_json::Value::String(s.to_string()))
        .map_err(|_| ApiError::Internal(format!("unknown retention category in the table: {s}")))
}

/// Every policy, in category order.
pub async fn list(pool: &PgPool, _admin: &SystemAdmin) -> ApiResult<RetentionPolicies> {
    let rows = sqlx::query!(
        r#"SELECT name, category, basis, period::text AS "period!", set_by_profile_id, updated
             FROM kb_erasure_retention_policies
            ORDER BY category"#
    )
    .fetch_all(pool)
    .await?;
    let policies = rows
        .into_iter()
        .map(|r| {
            Ok(RetentionPolicy {
                name: r.name,
                category: category_from(&r.category)?,
                basis: r.basis,
                period: r.period,
                set_by: ProfileId::from(r.set_by_profile_id),
                updated: r.updated,
            })
        })
        .collect::<ApiResult<Vec<_>>>()?;
    Ok(RetentionPolicies { policies })
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 63
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Set the policy for one category, replacing any the category had. Answers the list.
pub async fn set(
    pool: &PgPool,
    admin: &SystemAdmin,
    policy: &RetentionPolicySet,
) -> ApiResult<RetentionPolicies> {
    if !valid_name(&policy.name) {
        return Err(ApiError::BadRequest(
            "a policy name is a slug: lower-case letters, digits and hyphens, at most 63".into(),
        ));
    }
    if policy.basis.trim().is_empty() {
        return Err(ApiError::BadRequest("a policy states its basis".into()));
    }
    let result = sqlx::query!(
        r#"INSERT INTO kb_erasure_retention_policies (name, category, basis, period, set_by_profile_id)
           VALUES ($1, $2, $3, $4::text::interval, $5)
           ON CONFLICT (category) DO UPDATE
              SET name = EXCLUDED.name, basis = EXCLUDED.basis, period = EXCLUDED.period,
                  set_by_profile_id = EXCLUDED.set_by_profile_id, updated = now()"#,
        policy.name,
        policy.category.as_str(),
        policy.basis,
        policy.period,
        admin.actor().uuid(),
    )
    .execute(pool)
    .await;
    match result {
        Ok(_) => {}
        Err(sqlx::Error::Database(e)) => {
            return Err(match e.code().as_deref() {
                // Not an interval; or a period that is not positive.
                Some("22007") | Some("22P02") | Some("22015") | Some("23514") => {
                    ApiError::BadRequest(format!(
                        "the period {:?} is not a positive interval (e.g. `1 year`)",
                        policy.period
                    ))
                }
                // The name already names another category's policy.
                Some("23505") => ApiError::Conflict(format!(
                    "the policy name {:?} is already used by another category",
                    policy.name
                )),
                _ => ApiError::from(sqlx::Error::Database(e)),
            });
        }
        Err(e) => return Err(e.into()),
    }
    tracing::info!(
        actor = %admin.actor(),
        category = policy.category.as_str(),
        policy = %policy.name,
        "erasure retention policy set"
    );
    list(pool, admin).await
}

/// Remove the category's policy, if it has one. Answers the list.
pub async fn remove(
    pool: &PgPool,
    admin: &SystemAdmin,
    category: RetentionCategory,
) -> ApiResult<RetentionPolicies> {
    sqlx::query!(
        "DELETE FROM kb_erasure_retention_policies WHERE category = $1",
        category.as_str()
    )
    .execute(pool)
    .await?;
    tracing::info!(
        actor = %admin.actor(),
        category = category.as_str(),
        "erasure retention policy removed"
    );
    list(pool, admin).await
}
