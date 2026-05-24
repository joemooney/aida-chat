// trace:STORY-15 | ai:claude
//
// Concrete agent backends. Both expose a `run_turn(...)` function with
// the same signature; `agent::run_turn` picks one at dispatch time.

pub mod anthropic;
pub mod claude_cli;
