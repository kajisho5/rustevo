use evo_logic::{Outcome, Value};

mod state;
use state::*;

/// Reads the wall clock, which logic must not do. The test expects this to stop.
fn read_clock(_s: &State, _: &[Value]) -> Outcome {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    Outcome::new().set("stamp", now as i64)
}

evo_logic::actions!(State; read_clock);
