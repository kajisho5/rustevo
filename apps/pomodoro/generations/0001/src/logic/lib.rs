use evo_logic::{Outcome, Value};

mod state;
use state::*;

const FOCUS: i64 = 1500;

fn start(_s: &State, _: &[Value]) -> Outcome {
    Outcome::new().set("phase", Phase::Focus).set("remaining", FOCUS)
}

fn stop(_s: &State, _: &[Value]) -> Outcome {
    Outcome::new().set("phase", Phase::Idle).set("remaining", FOCUS)
}

fn tick(s: &State, _: &[Value]) -> Outcome {
    if s.remaining > 1 {
        Outcome::new().set("remaining", s.remaining - 1)
    } else {
        Outcome::new().set("remaining", FOCUS).set("phase", Phase::Idle).set("done", s.done + 1)
    }
}

evo_logic::actions!(State; start, stop, tick);
