use evo_logic::{Outcome, Value};

mod state;
use state::*;

const FOCUS: i64 = 1500;
const REST: i64 = 300;

/// Put the timer into `phase` with a full focus length on the clock.
fn reset_to(phase: Phase) -> Outcome {
    Outcome::new().set("phase", phase).set("remaining", FOCUS)
}

fn start(_s: &State, _: &[Value]) -> Outcome {
    reset_to(Phase::Focus)
}

fn stop(_s: &State, _: &[Value]) -> Outcome {
    reset_to(Phase::Idle)
}

fn tick(s: &State, _: &[Value]) -> Outcome {
    if s.remaining > 1 {
        Outcome::new().set("remaining", s.remaining - 1)
    } else if s.phase == Phase::Focus {
        let mut log = s.log.clone();
        log.push(Log { n: s.done + 1, task: s.task.clone() });
        Outcome::new()
            .set("log", log)
            .set("phase", Phase::Rest)
            .set("remaining", REST)
            .set("done", s.done + 1)
            .effect(evo_logic::store_put("done", s.done + 1))
    } else {
        reset_to(Phase::Idle)
    }
}

fn skip(s: &State, _: &[Value]) -> Outcome {
    if s.phase == Phase::Rest {
        reset_to(Phase::Idle)
    } else {
        Outcome::new()
    }
}

evo_logic::actions!(State; start, stop, tick, skip);
