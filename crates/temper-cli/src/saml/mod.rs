//! SAML provisioning emitter — pure core (no I/O). One `SamlProvisionConfig` renders the
//! consistent env bundle + SQL; keygen produces a PKCS#8 Ed25519 PEM the TypeScript AS
//! (`packages/temper-cloud/src/oauth/keys.ts` → jose `importPKCS8`) accepts verbatim.

use crate::error::{Result, TemperError};
use base64::Engine as _;
use ed25519_dalek::SigningKey;
use pkcs8::EncodePrivateKey;
use rand::RngCore as _;
use std::collections::BTreeMap;

/// A generated AS signing key: the PKCS#8 PEM plus its published key id.
#[derive(Debug, Clone)]
pub struct GeneratedKey {
    pub pem: String,
    pub kid: String,
}

/// Generate an Ed25519 signing key as a PKCS#8 PEM (`-----BEGIN PRIVATE KEY-----`), compatible
/// with the TypeScript AS's `importPKCS8(pem, "EdDSA")`. `kid` defaults to `as-<YYYY-MM>`.
pub fn generate_signing_key(
    kid_override: Option<String>,
    now_yyyymm: &str,
) -> Result<GeneratedKey> {
    let mut secret = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut secret);
    let signing = SigningKey::from_bytes(&secret);
    let pem = signing
        .to_pkcs8_pem(pkcs8::LineEnding::LF)
        .map_err(|e| TemperError::Config(format!("PKCS#8 encode: {e}")))?
        .to_string();
    let kid = kid_override.unwrap_or_else(|| format!("as-{now_yyyymm}"));
    Ok(GeneratedKey { pem, kid })
}

/// Generate a strong shared reconcile secret: 32 random bytes, base64 (standard, padded).
pub fn generate_reconcile_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The single source of truth for a SAML provisioning run. Every shared value across the two
/// Vercel functions is DERIVED from these fields, so `AS_AUDIENCE == AUTH_AUDIENCE`,
/// `AUTH_ISSUER == AS_ISSUER`, `AUTH_PROVIDER_NAME == saml:<idp_key>`, and the one
/// `INTERNAL_RECONCILE_SECRET` cannot drift.
#[derive(Debug, Clone)]
pub struct SamlProvisionConfig {
    pub instance_url: String,
    pub api_origin: String,
    pub idp_key: String,
    pub signing_key_pem: String,
    pub signing_kid: String,
    pub reconcile_secret: String,
    pub clients: Vec<(String, Vec<String>)>,
    pub access_ttl_secs: u32,
    pub refresh_ttl_secs: u32,
    pub idp_cert: String,
    pub idp_sso_url: String,
    pub idp_entity_id: String,
    pub nameid_format: String,
    pub email_attr: String,
    pub stable_id_attr: String,
    pub groups_attr: Option<String>,
}

impl SamlProvisionConfig {
    fn issuer(&self) -> &str {
        self.instance_url.trim_end_matches('/')
    }
    fn audience(&self) -> String {
        format!("{}/api", self.issuer())
    }
    fn sp_entity_id(&self) -> String {
        format!("{}/saml/metadata", self.issuer())
    }
    fn acs_url(&self) -> String {
        format!("{}/oauth/saml/acs", self.issuer())
    }
    fn provider_name(&self) -> String {
        format!("saml:{}", self.idp_key)
    }

    fn clients_json(&self) -> String {
        let map: BTreeMap<&str, &Vec<String>> =
            self.clients.iter().map(|(c, r)| (c.as_str(), r)).collect();
        serde_json::to_string(&map).expect("client map serializes")
    }

    /// Render the full env bundle (AS-side + api-side + shared). Emit-only — the operator pastes
    /// these into both Vercel functions (or a .env). Shared values are equal by construction.
    pub fn render_env(&self) -> String {
        let issuer = self.issuer();
        let audience = self.audience();
        format!(
            "# ── Authorization Server (temper-cloud) ──────────────────────────\n\
             AS_ISSUER={issuer}\n\
             AS_AUDIENCE={audience}\n\
             AS_SIGNING_KEY_PKCS8={key}\n\
             AS_SIGNING_KID={kid}\n\
             AS_CLIENTS={clients}\n\
             AS_ACCESS_TTL_SECONDS={access}\n\
             AS_REFRESH_TTL_SECONDS={refresh}\n\
             # ── temper-api ───────────────────────────────────────────────────\n\
             JWKS_URL={issuer}/oauth/jwks\n\
             AUTH_ISSUER={issuer}\n\
             AUTH_AUDIENCE={audience}\n\
             AUTH_PROVIDER_NAME={provider}\n\
             # ── shared (BOTH functions, identical value) ─────────────────────\n\
             INTERNAL_RECONCILE_SECRET={secret}\n\
             INTERNAL_RECONCILE_URL={api}/internal/saml/reconcile\n",
            issuer = issuer,
            audience = audience,
            key = self.signing_key_pem.replace('\n', "\\n"),
            kid = self.signing_kid,
            clients = self.clients_json(),
            access = self.access_ttl_secs,
            refresh = self.refresh_ttl_secs,
            provider = self.provider_name(),
            secret = self.reconcile_secret,
            api = self.api_origin.trim_end_matches('/'),
        )
    }
}

/// Single-quote a string literal for emitted SQL, doubling embedded quotes.
fn sql_str(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

impl SamlProvisionConfig {
    /// Render the `kb_saml_idp` INSERT for this IdP (active row). Emit-only unless `--apply`.
    pub fn render_idp_sql(&self) -> String {
        let groups = match &self.groups_attr {
            Some(g) => sql_str(g),
            None => "NULL".to_owned(),
        };
        format!(
            "INSERT INTO kb_saml_idp (\n  \
             idp_key, is_active, idp_cert, idp_sso_url, idp_entity_id,\n  \
             sp_entity_id, acs_url, nameid_format, email_attr, stable_id_attr, groups_attr\n\
             ) VALUES (\n  \
             {idp_key}, true, {cert}, {sso}, {entity},\n  \
             {sp}, {acs}, {nameid}, {email}, {stable}, {groups}\n);\n",
            idp_key = sql_str(&self.idp_key),
            cert = sql_str(&self.idp_cert),
            sso = sql_str(&self.idp_sso_url),
            entity = sql_str(&self.idp_entity_id),
            sp = sql_str(&self.sp_entity_id()),
            acs = sql_str(&self.acs_url()),
            nameid = sql_str(&self.nameid_format),
            email = sql_str(&self.email_attr),
            stable = sql_str(&self.stable_id_attr),
            groups = groups,
        )
    }
}

/// Render one `kb_saml_group_mappings` INSERT (`group → (team, role)`).
pub fn render_group_mapping_sql(
    idp_key: &str,
    group_value: &str,
    team_id: uuid::Uuid,
    role: &str,
) -> String {
    format!(
        "INSERT INTO kb_saml_group_mappings (idp_key, group_value, team_id, role)\n\
         VALUES ({idp}, {group}, '{team}', {role})\nON CONFLICT DO NOTHING;\n",
        idp = sql_str(idp_key),
        group = sql_str(group_value),
        team = team_id,
        role = sql_str(role),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keygen_emits_pkcs8_pem_and_kid() {
        let k = generate_signing_key(None, "2026-07").unwrap();
        assert!(k.pem.starts_with("-----BEGIN PRIVATE KEY-----"));
        assert!(k.pem.trim_end().ends_with("-----END PRIVATE KEY-----"));
        assert_eq!(k.kid, "as-2026-07");
        // Re-parse proves it's a valid PKCS#8 Ed25519 key (what jose will do).
        use pkcs8::DecodePrivateKey;
        ed25519_dalek::SigningKey::from_pkcs8_pem(&k.pem).unwrap();

        let overridden = generate_signing_key(Some("custom-kid".into()), "2026-07").unwrap();
        assert_eq!(overridden.kid, "custom-kid");
    }

    #[test]
    fn reconcile_secret_is_strong_and_unique() {
        let a = generate_reconcile_secret();
        let b = generate_reconcile_secret();
        assert_ne!(a, b);
        // ≥32 raw bytes → ≥43 base64 chars (unpadded) / ≥44 (padded).
        assert!(a.len() >= 43);
    }

    fn sample_config() -> SamlProvisionConfig {
        SamlProvisionConfig {
            instance_url: "https://temper.acme.com".into(),
            api_origin: "https://temper.acme.com".into(),
            idp_key: "acme-okta".into(),
            signing_key_pem: "-----BEGIN PRIVATE KEY-----\nAAA\n-----END PRIVATE KEY-----\n".into(),
            signing_kid: "as-2026-07".into(),
            reconcile_secret: "c2VjcmV0c2VjcmV0c2VjcmV0c2VjcmV0c2VjcmV0MDE=".into(),
            clients: vec![
                (
                    "temper-cli".into(),
                    vec!["https://temper.acme.com/api/auth/cli-callback".into()],
                ),
                (
                    "temper-ui".into(),
                    vec!["https://app.acme.com/auth/callback".into()],
                ),
            ],
            access_ttl_secs: 900,
            refresh_ttl_secs: 2_592_000,
            idp_cert: "-----BEGIN CERTIFICATE-----\nX\n-----END CERTIFICATE-----".into(),
            idp_sso_url: "https://idp.acme.com/sso".into(),
            idp_entity_id: "http://www.okta.com/x".into(),
            nameid_format: "urn:oasis:names:tc:SAML:2.0:nameid-format:persistent".into(),
            email_attr: "email".into(),
            stable_id_attr: "uid".into(),
            groups_attr: Some("groups".into()),
        }
    }

    #[test]
    fn env_shared_values_are_consistent_by_construction() {
        let env = sample_config().render_env();
        let get = |k: &str| {
            env.lines()
                .find_map(|l| l.strip_prefix(&format!("{k}=")))
                .unwrap()
        };
        // The whole point: shared values are equal because they are derived from one source.
        assert_eq!(get("AS_ISSUER"), get("AUTH_ISSUER"));
        assert_eq!(get("AS_AUDIENCE"), get("AUTH_AUDIENCE"));
        assert_eq!(get("AS_AUDIENCE"), "https://temper.acme.com/api");
        assert_eq!(get("AUTH_PROVIDER_NAME"), "saml:acme-okta");
        assert_eq!(get("JWKS_URL"), "https://temper.acme.com/oauth/jwks");
        assert_eq!(
            get("INTERNAL_RECONCILE_URL"),
            "https://temper.acme.com/internal/saml/reconcile"
        );
        // AS_CLIENTS is valid JSON of the client→redirects map.
        let clients: serde_json::Value = serde_json::from_str(get("AS_CLIENTS")).unwrap();
        assert_eq!(
            clients["temper-cli"][0],
            "https://temper.acme.com/api/auth/cli-callback"
        );
    }

    #[test]
    fn idp_sql_has_all_columns_and_escapes_quotes() {
        let mut cfg = sample_config();
        cfg.idp_key = "a'quote".into(); // SQL-escaping must double it.
        let sql = cfg.render_idp_sql();
        assert!(sql.contains("INSERT INTO kb_saml_idp"));
        assert!(sql.contains("is_active"));
        assert!(sql.contains("groups_attr"));
        assert!(sql.contains("'a''quote'"));
        assert!(sql.contains("saml/metadata")); // derived sp_entity_id
    }

    #[test]
    fn group_mapping_sql_renders() {
        let team = uuid::Uuid::nil();
        let sql = render_group_mapping_sql("acme-okta", "engineering", team, "member");
        assert!(sql.contains("INSERT INTO kb_saml_group_mappings"));
        assert!(sql.contains("'engineering'"));
        assert!(sql.contains(&team.to_string()));
        assert!(sql.contains("'member'"));
    }
}
