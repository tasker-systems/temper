//! `temper health` — ask the service whether it is up. The door needs no credential, and neither
//! does this command: a logged-out caller can still tell a down service from a refused login.

use crate::error::Result;
use crate::format::OutputFormat;

pub fn run(fmt: OutputFormat) -> Result<()> {
    crate::actions::runtime::render_read(fmt, move |client| {
        Box::pin(async move { client.health().get_health().await })
    })
}
