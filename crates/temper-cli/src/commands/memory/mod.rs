//! `temper memory` — Claude Code's `MEMORY.md` as a rendered projection of
//! `memory`-typed Temper resources.

pub mod emit;
pub mod render;
pub mod status;

pub use emit::emit;
pub use status::status;
