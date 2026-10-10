//! `Caller`'s arms are public so a surface can match on them, but each wraps a sealed proof — so a
//! `Caller` cannot be built from an unclassified profile either. The forgery fails at the proof.
use temper_services::auth::{AuthenticatedProfile, Caller, HumanPrincipal};

fn forge(authed: AuthenticatedProfile) -> Caller {
    // E0423: cannot initialize a tuple struct which contains private fields.
    Caller::Human(HumanPrincipal(authed))
}

fn main() {}
