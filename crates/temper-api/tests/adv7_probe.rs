#![cfg(feature = "test-db")]
mod common;

use sqlx::PgPool;

fn slack_cfg(config: &mut temper_services::config::ApiConfig) {
    config.slack_link = Some(temper_services::config::SlackLinkConfig {
        client_id: "slack-link-client".to_string(),
        hmac_secret: "link-secret".to_string(),
        public_base_url: "https://temper.example".to_string(),
        vault_key: temper_services::services::grant_crypto::VaultKey::from_base64(
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        )
        .expect("a 32-byte key"),
        retired_vault_keys: Default::default(),
    });
}

async fn get(app: &common::TestApp, path: &str) -> (u16, String) {
    let r = app.client.get(app.url(path)).send().await.unwrap();
    let s = r.status().as_u16();
    (s, r.text().await.unwrap())
}

fn sentence(body: &str) -> String {
    for needle in [
        "That link has expired",
        "Something went wrong",
        "incomplete",
        "cancelled",
        "Failed to deserialize",
    ] {
        if body.contains(needle) {
            return needle.to_string();
        }
    }
    body.chars().take(200).collect()
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn probe_callback_config_oracle(pool: PgPool) {
    let unconfigured = common::setup_test_app_with_config(pool.clone(), |_| {}).await;
    let configured = common::setup_test_app_with_config(pool.clone(), slack_cfg).await;
    for q in [
        "code=abc&state=0123456789abcdef",
        "code=abc&state=%00",
        "code=abc&state=a%00b",
        "code=abc&code=def&state=x",
        "code=abc&state=%FF%FE",
    ] {
        let path = format!("/api/auth/slack/callback?{q}");
        let u = get(&unconfigured, &path).await;
        let c = get(&configured, &path).await;
        println!(
            "PROBE {q}: unconfigured={} {:?} | configured={} {:?} | same_body={}",
            u.0,
            sentence(&u.1),
            c.0,
            sentence(&c.1),
            u.1 == c.1
        );
    }
    // timing
    for (name, app) in [("unconfigured", &unconfigured), ("configured", &configured)] {
        let mut ts = Vec::new();
        for i in 0..200 {
            let path = format!("/api/auth/slack/callback?code=abc&state=s{i}");
            let t = std::time::Instant::now();
            let _ = get(app, &path).await;
            ts.push(t.elapsed().as_micros());
        }
        ts.sort();
        println!(
            "TIMING {name}: median={}us p10={}us p90={}us",
            ts[100], ts[20], ts[180]
        );
    }
}
