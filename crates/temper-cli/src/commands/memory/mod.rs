//! `temper memory` — Claude Code's `MEMORY.md` as a rendered projection of
//! `memory`-typed Temper resources.

pub mod check;
pub mod emit;
mod fetch;
pub mod render;
pub mod status;

pub use check::check;
pub use emit::emit;
pub use status::status;
