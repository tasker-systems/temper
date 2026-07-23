//! `VerifiedSlackPrincipal` has a private field — constructing one by struct literal outside
//! `slack_mint_service` must not compile. This is the seal: the only way to hold one is to pass
//! `verify_mint_request`, which does the HMAC verify, so a proof cannot exist without a
//! signature-verified request. The mint gate mints one and inserts it into request extensions; the
//! handler extracts it. A forger — any other temper-api code path — cannot mint one, which is what
//! makes "naming a principal must not be sufficient to mint its token" a compile-time guarantee.
use temper_services::services::slack_mint_service::VerifiedSlackPrincipal;

fn main() {
    // E0451: field `id` of `VerifiedSlackPrincipal` is private.
    let _forged = VerifiedSlackPrincipal {
        id: "slack:T1:U1".to_string(),
    };
}
