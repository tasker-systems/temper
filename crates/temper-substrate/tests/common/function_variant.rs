//! Test-only variants of deployed SQL functions — how a bite cuts one conjunct out of the body that
//! is actually deployed, so the bite cannot drift from the code it bites.

use sqlx::PgConnection;

/// A `pg_temp` copy of `function`'s deployed body (the overload with `nargs` arguments, in its own
/// language) with the one occurrence of `cut` replaced by `with`, created on `conn` as `name`. The
/// cut must occur exactly once, so a body that moved under a test fails here, by name, rather than
/// biting nothing.
///
/// `name` and `signature` must be the calling test's own literals: they are formatted into the
/// `CREATE FUNCTION` text. The language and the body are the deployed function's, the body edited
/// here.
pub async fn variant(
    conn: &mut PgConnection,
    function: &str,
    nargs: i16,
    signature: &str,
    name: &str,
    cut: &str,
    with: &str,
) {
    let (body, language): (String, String) = sqlx::query_as(
        "SELECT p.prosrc, l.lanname::text FROM pg_proc p JOIN pg_language l ON l.oid = p.prolang \
          WHERE p.proname = $1 AND p.pronargs = $2",
    )
    .bind(function)
    .bind(nargs)
    .fetch_one(&mut *conn)
    .await
    .unwrap();
    assert_eq!(
        body.matches(cut).count(),
        1,
        "`{cut}` must appear exactly once in {function}'s deployed body:\n{body}"
    );
    let edited = body.replacen(cut, with, 1);
    sqlx::query(&format!(
        "CREATE FUNCTION pg_temp.{name}{signature} LANGUAGE {language} STABLE \
         AS $variant${edited}$variant$"
    ))
    .execute(&mut *conn)
    .await
    .unwrap();
}
