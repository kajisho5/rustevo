//! The journal: one line per event, append only, each line carrying the hash
//! of the line before it. Replaying a journal on the build that wrote it must
//! reproduce every line exactly.
use crate::plan::*;
use crate::runtime::*;
use serde_json::{json, Value};
use std::path::Path;

/// What the app looked like after one step, kept for comparing two runs.
#[derive(Clone)]
pub struct Snap {
    pub state: Vals,
    pub derives: Vals,
    pub nodes: Vals,
    pub tree: Value,
    pub inv: Vec<String>,
}

pub struct Session<'a> {
    pub plan: &'a Plan,
    pub rt: Runtime,
    pub lines: Vec<String>,
    pub snaps: Vec<Snap>,
}

impl<'a> Session<'a> {
    pub fn start(plan: &'a Plan, logic: &dyn Logic, seed: u64) -> R<Session<'a>> {
        let mut s = Session { plan, rt: Runtime::new(plan, seed)?, lines: vec![], snaps: vec![] };
        s.record(json!({"init": {"plan": plan.hash(), "logic": logic.hash(), "seed": seed}}), StepOut::default())?;
        Ok(s)
    }

    fn record(&mut self, ev: Value, out: StepOut) -> R<()> {
        let b = self.rt.build(self.plan)?;
        let d = self.rt.derives(self.plan)?;
        let inv = self.rt.broken(self.plan);
        let line = json!({
            "seq": self.lines.len(),
            "t": self.rt.t,
            "ev": ev,
            "changed": out.changed,
            "state": hash(&json!({"s": self.rt.state, "sys": self.rt.sys()})),
            "derive": hash(&json!(d)),
            "tree": hash(&b.tree),
            "req": out.requests,
            "fires": out.fires,
            "inv": inv,
            "prev": self.lines.last().map(|l| sha(l.as_bytes())).unwrap_or_default(),
        });
        self.lines.push(canon(&line));
        self.snaps.push(Snap { state: self.rt.state.clone(), derives: d, nodes: b.flat, tree: b.tree, inv });
        Ok(())
    }

    /// Apply one outside event and write its line. On error nothing changes.
    pub fn step(&mut self, logic: &mut dyn Logic, ev: Value) -> R<()> {
        let keep = self.rt.clone();
        let r = self.rt.apply(self.plan, logic, &ev).and_then(|out| self.record(ev, out));
        if r.is_err() {
            self.rt = keep;
        }
        r
    }

    /// Strict replay: run the journal's events on this plan and logic and
    /// require every line to come out byte for byte the same.
    pub fn replay(plan: &'a Plan, logic: &mut dyn Logic, lines: &[String]) -> R<Session<'a>> {
        let first = parse(lines.first().ok_or("記録が空")?)?;
        let init = &first["ev"]["init"];
        if init["plan"] != plan.hash() {
            return Err("この記録は別の plan で取られたもの".into());
        }
        if init["logic"] != logic.hash() {
            return Err("この記録は別のロジックで取られたもの".into());
        }
        let mut s = Session::start(plan, logic, init["seed"].as_u64().ok_or("init に seed がない")?)?;
        same(&s.lines[0], &lines[0], 0)?;
        for (i, l) in lines.iter().enumerate().skip(1) {
            let ev = parse(l)?["ev"].clone();
            s.step(logic, ev).map_err(|e| format!("step {i}: {e}"))?;
            same(&s.lines[i], l, i)?;
        }
        Ok(s)
    }
}

pub fn parse(line: &str) -> R<Value> {
    serde_json::from_str(line).map_err(|e| format!("記録の行を読めない: {e}"))
}

fn same(got: &str, want: &str, i: usize) -> R<()> {
    if got == want {
        return Ok(());
    }
    let (g, w) = (parse(got)?, parse(want).unwrap_or(Value::Null));
    let keys: Vec<&str> = ["seq", "t", "ev", "changed", "state", "derive", "tree", "req", "fires", "inv", "prev"]
        .into_iter()
        .filter(|k| g[*k] != w[*k])
        .collect();
    Err(format!("step {i}: 再演の結果が記録と違う（{}）", if keys.is_empty() { "書式".into() } else { keys.join(", ") }))
}

pub fn read_journal(p: &Path) -> R<Vec<String>> {
    let s = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(s.lines().filter(|l| !l.trim().is_empty()).map(String::from).collect())
}

pub fn write_journal(p: &Path, lines: &[String]) -> R<()> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(p, lines.join("\n") + "\n").map_err(|e| format!("{}: {e}", p.display()))
}
