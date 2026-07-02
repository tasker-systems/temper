//! `temper admin saml` command shell — I/O around the pure `crate::saml` core. Emit-by-default.
//!
//! `provision` generates the AS signing key + reconcile secret, then renders the consistent env
//! bundle and `kb_saml_idp` SQL from one `SamlProvisionConfig` (shared values equal by
//! construction). It emits to stdout by default, or to `--env-out`/`--sql-out`; `--apply` runs the
//! SQL against `$DATABASE_URL` via `psql`.

use std::io::Write as _;
use std::process::{Command, Stdio};

use dialoguer::{theme::ColorfulTheme, Input};

use crate::error::{Result, TemperError};
use crate::output;
use crate::saml::{self, SamlProvisionConfig};

/// AS access-token TTL baked into the emitted env bundle (15 minutes).
const ACCESS_TTL_SECS: u32 = 900;
/// AS refresh-token TTL baked into the emitted env bundle (30 days).
const REFRESH_TTL_SECS: u32 = 2_592_000;

/// The clap `Provision` fields, by value. A params struct (not a long argument list) per the
/// repo's >5-param convention — the interactive-vs-switched surface is wide.
#[derive(Debug)]
pub struct ProvisionArgs {
    pub no_interactive: bool,
    pub instance_url: Option<String>,
    pub api_origin: Option<String>,
    pub idp_key: Option<String>,
    pub idp_cert_file: Option<String>,
    pub idp_sso_url: Option<String>,
    pub idp_entity_id: Option<String>,
    pub nameid_format: String,
    pub email_attr: String,
    pub stable_id_attr: String,
    pub groups_attr: Option<String>,
    pub kid: Option<String>,
    pub clients: Vec<String>,
    pub env_out: Option<String>,
    pub sql_out: Option<String>,
    pub apply: bool,
}

/// Convert a dialoguer prompt failure into a `TemperError` (a setup problem, not a vault-state
/// problem). Mirrors `commands::init::prompt_err`.
fn prompt_err(e: dialoguer::Error) -> TemperError {
    TemperError::Config(format!("prompt error: {e}"))
}

/// Current month as `YYYY-MM` — the default signing-key-id suffix (`as-<YYYY-MM>`).
fn current_yyyymm() -> String {
    chrono::Local::now().format("%Y-%m").to_string()
}

/// Resolve a required field: use the provided flag when non-empty; else prompt (interactive) or
/// error requiring the flag (`--no-interactive`). Mirrors `init::self_host_from_flags`' error style.
fn resolve_field(
    theme: &ColorfulTheme,
    no_interactive: bool,
    provided: Option<String>,
    flag: &str,
    prompt: &str,
) -> Result<String> {
    if let Some(v) = provided.filter(|s| !s.trim().is_empty()) {
        return Ok(v.trim().to_string());
    }
    if no_interactive {
        return Err(TemperError::Config(format!(
            "--no-interactive requires --{flag}"
        )));
    }
    let v: String = Input::with_theme(theme)
        .with_prompt(prompt)
        .interact_text()
        .map_err(prompt_err)?;
    Ok(v.trim().to_string())
}

/// Parse repeated `client_id=redirect_uri` entries into the grouped shape
/// `Vec<(client_id, Vec<redirect_uri>)>`. Multiple entries with the same id accumulate their URIs.
/// Errors on any entry missing `=`.
fn parse_clients(raw: &[String]) -> Result<Vec<(String, Vec<String>)>> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for entry in raw {
        let (id, uri) = entry.split_once('=').ok_or_else(|| {
            TemperError::Config(format!(
                "--client '{entry}' must be client_id=redirect_uri (missing '=')"
            ))
        })?;
        let id = id.trim().to_string();
        let uri = uri.trim().to_string();
        if id.is_empty() || uri.is_empty() {
            return Err(TemperError::Config(format!(
                "--client '{entry}' must be client_id=redirect_uri (empty side)"
            )));
        }
        match out.iter_mut().find(|(c, _)| *c == id) {
            Some(existing) => existing.1.push(uri),
            None => out.push((id, vec![uri])),
        }
    }
    Ok(out)
}

/// `temper admin saml provision` — generate keys, render the env bundle + kb_saml_idp SQL, and
/// emit (stdout by default, or files); `--apply` runs the SQL via psql. No authenticated client is
/// needed — emit is inert.
pub fn provision(args: ProvisionArgs) -> Result<()> {
    let theme = ColorfulTheme::default();

    // 1. Resolve every field: prompt when interactive, else require the flag.
    let instance_url = resolve_field(
        &theme,
        args.no_interactive,
        args.instance_url,
        "instance-url",
        "Instance base URL (e.g. https://temper.acme.com)",
    )?;
    let api_origin = match args.api_origin.filter(|s| !s.trim().is_empty()) {
        Some(v) => v.trim().to_string(),
        None => instance_url.clone(),
    };
    let idp_key = resolve_field(
        &theme,
        args.no_interactive,
        args.idp_key,
        "idp-key",
        "IdP key (short identifier, e.g. acme-okta)",
    )?;
    let idp_cert_file = resolve_field(
        &theme,
        args.no_interactive,
        args.idp_cert_file,
        "idp-cert-file",
        "Path to the IdP signing certificate (PEM file)",
    )?;
    let idp_sso_url = resolve_field(
        &theme,
        args.no_interactive,
        args.idp_sso_url,
        "idp-sso-url",
        "IdP SSO URL (SAML SingleSignOnService)",
    )?;
    let idp_entity_id = resolve_field(
        &theme,
        args.no_interactive,
        args.idp_entity_id,
        "idp-entity-id",
        "IdP entity id (issuer)",
    )?;

    // groups_attr is optional (omit for authn-only). Interactive prompt allows empty → None.
    let groups_attr = match args.groups_attr.filter(|s| !s.trim().is_empty()) {
        Some(v) => Some(v.trim().to_string()),
        None if args.no_interactive => None,
        None => {
            let v: String = Input::with_theme(&theme)
                .with_prompt("Group-list assertion attribute (Enter to skip — authn-only)")
                .default(String::new())
                .allow_empty(true)
                .interact_text()
                .map_err(prompt_err)?;
            let v = v.trim().to_string();
            if v.is_empty() {
                None
            } else {
                Some(v)
            }
        }
    };

    // Clients: use the repeated --client flags; if interactive and none given, prompt one line.
    let raw_clients = if args.clients.is_empty() && !args.no_interactive {
        let line: String = Input::with_theme(&theme)
            .with_prompt("AS clients (comma-separated client_id=redirect_uri)")
            .default(String::new())
            .allow_empty(true)
            .interact_text()
            .map_err(prompt_err)?;
        line.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
    } else {
        args.clients
    };
    let clients = parse_clients(&raw_clients)?;

    // 2. Read --idp-cert-file into a String.
    let idp_cert = std::fs::read_to_string(&idp_cert_file).map_err(|e| {
        TemperError::Config(format!("reading IdP cert file '{idp_cert_file}': {e}"))
    })?;

    // 3. Generate the AS signing key (PKCS#8 Ed25519 PEM + key id).
    let key = saml::generate_signing_key(args.kid, &current_yyyymm())?;
    // 4. Generate the shared reconcile secret.
    let secret = saml::generate_reconcile_secret();

    // 5. Build the single source of truth for this provisioning run.
    let cfg = SamlProvisionConfig {
        instance_url,
        api_origin,
        idp_key,
        signing_key_pem: key.pem,
        signing_kid: key.kid,
        reconcile_secret: secret,
        clients,
        access_ttl_secs: ACCESS_TTL_SECS,
        refresh_ttl_secs: REFRESH_TTL_SECS,
        idp_cert: idp_cert.trim_end().to_string(),
        idp_sso_url,
        idp_entity_id,
        nameid_format: args.nameid_format,
        email_attr: args.email_attr,
        stable_id_attr: args.stable_id_attr,
        groups_attr,
    };

    // 6. Render both artifacts from the one config.
    let env = cfg.render_env();
    let sql = cfg.render_idp_sql();

    // 7. Emit env → stdout or --env-out (chmod 0600 — contains the private key).
    match &args.env_out {
        Some(path) => {
            write_env_file(path, &env)?;
            output::dim(format!("Wrote env bundle to {path} (mode 0600)"));
        }
        None => println!("{env}"),
    }
    // Emit SQL → stdout or --sql-out.
    match &args.sql_out {
        Some(path) => {
            std::fs::write(path, &sql)
                .map_err(|e| TemperError::Config(format!("writing SQL to '{path}': {e}")))?;
            output::dim(format!("Wrote kb_saml_idp SQL to {path}"));
        }
        None => println!("{sql}"),
    }

    // 8. Apply the SQL, or guide the operator.
    if args.apply {
        apply_sql_via_psql(&sql)?;
        output::success("Applied kb_saml_idp SQL via psql.");
    } else {
        output::hint(
            "Paste the env bundle into BOTH Vercel functions (temper-api and the Authorization \
             Server), then apply the kb_saml_idp SQL (re-run with --apply, or pipe it to psql).",
        );
    }
    Ok(())
}

/// Write the env bundle to `path` and, on unix, restrict it to owner read/write (0600) since it
/// carries the AS private key.
fn write_env_file(path: &str, env: &str) -> Result<()> {
    std::fs::write(path, env)
        .map_err(|e| TemperError::Config(format!("writing env bundle to '{path}': {e}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, perms)
            .map_err(|e| TemperError::Config(format!("setting 0600 on env file '{path}': {e}")))?;
    }
    Ok(())
}

/// Run emitted SQL against `$DATABASE_URL` via `psql` (fail-fast on errors). Requires `psql` on
/// PATH and `DATABASE_URL` set — the same operator-with-DB-credentials contract as
/// `scripts/bootstrap/system-bootstrap.sh --run-root`.
pub fn apply_sql_via_psql(sql: &str) -> Result<()> {
    let db = std::env::var("DATABASE_URL").map_err(|_| {
        TemperError::Config("--apply needs DATABASE_URL (the direct/unpooled Neon URL)".into())
    })?;
    let mut child = Command::new("psql")
        .arg(&db)
        .arg("--set=ON_ERROR_STOP=1")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| {
            TemperError::Config(format!("failed to launch psql (is it installed?): {e}"))
        })?;
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(sql.as_bytes())
        .map_err(|e| TemperError::Config(format!("writing SQL to psql: {e}")))?;
    let status = child
        .wait()
        .map_err(|e| TemperError::Config(format!("waiting on psql: {e}")))?;
    if !status.success() {
        return Err(TemperError::Config(format!("psql exited with {status}")));
    }
    Ok(())
}

/// `temper admin saml map-group` — resolve `team` via the authenticated client, render the
/// `kb_saml_group_mappings` INSERT via `saml::render_group_mapping_sql`, then emit or `--apply`.
pub async fn map_group(
    client: &temper_client::TemperClient,
    idp_key: &str,
    group: &str,
    team: &str,
    role: &str,
    apply: bool,
) -> Result<()> {
    let team_id = crate::actions::cogmap::resolve_team_id(client, team).await?;
    let sql = saml::render_group_mapping_sql(idp_key, group, team_id, role);
    if apply {
        apply_sql_via_psql(&sql)?;
        output::success(format!("mapped '{group}' → {team} ({role})"));
    } else {
        println!("{sql}");
    }
    Ok(())
}

/// List groups the IdP has actually asserted (kb_saml_seen_groups), most-recent first.
pub fn from_seen(idp_key: &str) -> Result<()> {
    let db = std::env::var("DATABASE_URL")
        .map_err(|_| TemperError::Config("--from-seen needs DATABASE_URL".into()))?;
    let out = std::process::Command::new("psql")
        .arg(&db)
        .arg("-tA")
        .arg("-c")
        .arg(format!(
            "SELECT group_value, last_seen FROM kb_saml_seen_groups \
             WHERE idp_key = '{}' ORDER BY last_seen DESC",
            idp_key.replace('\'', "''")
        ))
        .output()
        .map_err(|e| TemperError::Config(format!("failed to launch psql: {e}")))?;
    if !out.status.success() {
        return Err(TemperError::Config(format!(
            "psql failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    print!("{}", String::from_utf8_lossy(&out.stdout));
    Ok(())
}

/// `temper admin saml verify` — probe a provisioned instance: AS metadata reachable, caller is a
/// system admin (proves the `gating_team_slug` gate is correctly set up), and — with `--db` —
/// exactly one active `kb_saml_idp` row.
pub async fn verify(
    client: &temper_client::TemperClient,
    instance_url: &str,
    db_check: bool,
) -> Result<()> {
    let base = instance_url.trim_end_matches('/');
    let http = reqwest::Client::new();
    let mut ok = true;

    // 1. AS metadata + JWKS reachable ⇒ AS mode on.
    for path in ["/.well-known/oauth-authorization-server", "/oauth/jwks"] {
        let url = format!("{base}{path}");
        match http.get(&url).send().await {
            Ok(r) if r.status().is_success() => {
                output::success(format!("AS reachable: {path}"));
            }
            Ok(r) => {
                ok = false;
                output::error(format!("{path} → HTTP {}", r.status()));
            }
            Err(e) => {
                ok = false;
                output::error(format!("{path} unreachable: {e}"));
            }
        }
    }

    // 2. Caller is a system admin (the gating_team_slug silent-403 check).
    match client.admin().get_settings().await {
        Ok(_) => output::success("caller is a system admin (is_system_admin = true)"),
        Err(e) => {
            ok = false;
            output::error(format!(
                "admin check failed ({e}) — verify gating_team_slug is set AND you own that team"
            ));
        }
    }

    // 3. Optional: exactly one active kb_saml_idp row.
    if db_check {
        let db = std::env::var("DATABASE_URL")
            .map_err(|_| TemperError::Config("--db needs DATABASE_URL".into()))?;
        let out = std::process::Command::new("psql")
            .arg(&db)
            .arg("-tA")
            .arg("-c")
            .arg("SELECT count(*) FROM kb_saml_idp WHERE is_active")
            .output()
            .map_err(|e| TemperError::Config(format!("failed to launch psql: {e}")))?;
        if !out.status.success() {
            ok = false;
            output::error(format!(
                "psql failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        } else {
            let count = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            if count == "1" {
                output::success("exactly one active kb_saml_idp row");
            } else {
                ok = false;
                output::error(format!("expected 1 active kb_saml_idp row, found {count}"));
            }
        }
    }

    if ok {
        Ok(())
    } else {
        Err(TemperError::Api("one or more SAML checks failed".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_requires_database_url() {
        // SAFETY: single-threaded test; no other test reads DATABASE_URL concurrently here.
        let saved = std::env::var("DATABASE_URL").ok();
        std::env::remove_var("DATABASE_URL");
        let err = apply_sql_via_psql("SELECT 1;").unwrap_err();
        assert!(format!("{err}").contains("DATABASE_URL"));
        if let Some(v) = saved {
            std::env::set_var("DATABASE_URL", v);
        }
    }

    #[test]
    fn parse_clients_groups_by_id_and_requires_equals() {
        let parsed = parse_clients(&[
            "temper-cli=https://x/a".to_string(),
            "temper-cli=https://x/b".to_string(),
            "temper-ui=https://x/c".to_string(),
        ])
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0, "temper-cli");
        assert_eq!(parsed[0].1, vec!["https://x/a", "https://x/b"]);
        assert_eq!(parsed[1].0, "temper-ui");

        let err = parse_clients(&["no-equals".to_string()]).unwrap_err();
        assert!(format!("{err}").contains("client_id=redirect_uri"));
    }
}
