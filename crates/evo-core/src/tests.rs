use crate::carry::{carry, verdict};
use crate::lang::compile;
use crate::plan::*;
use crate::runtime::*;
use crate::session::*;
use serde_json::{json, Value};

/// Logic for the tests, written natively: a countdown.
struct Count {
    step: i64,
}

impl Logic for Count {
    fn call(&mut self, action: &str, state: &Value, _args: &[Value]) -> R<Outcome> {
        let left = state["left"].as_i64().unwrap();
        let mut o = Outcome::default();
        match action {
            "start" => {
                o.set.insert("on".into(), json!(true));
            }
            "tick" => {
                o.set.insert("left".into(), json!(left - self.step));
                if left - self.step <= 0 {
                    o.set.insert("on".into(), json!(false));
                    o.effects.push(json!({"kind": "store.put", "key": "last", "value": 0}));
                }
            }
            other => return Err(format!("no action {other}")),
        }
        Ok(o)
    }
    fn hash(&self) -> String {
        format!("count{}", self.step)
    }
}

const SRC: &str = r#"
app T: Main
state on: bool = false
state left: int = 5
derive label = "left {left}"
invariant left >= 0
every 1s when on do tick
view Main {
  column #root {
    text #label { label }
    button #go { "go"; on tap do start }
  }
}
"#;

fn plan_of(name: &str, src: &str) -> R<Plan> {
    let p = std::env::temp_dir().join(format!("evo-test-{}-{name}.evo", std::process::id()));
    std::fs::write(&p, src).unwrap();
    let r = compile(&p);
    let _ = std::fs::remove_file(&p);
    r
}

fn record(plan: &Plan, logic: &mut dyn Logic) -> Vec<String> {
    let mut s = Session::start(plan, logic, 7).unwrap();
    s.step(logic, json!({"tap": "go"})).unwrap();
    s.step(logic, json!({"tick": 2500})).unwrap();
    s.step(logic, json!({"tick": 9000})).unwrap();
    s.lines
}

#[test]
fn the_same_source_gives_the_same_plan() {
    assert_eq!(plan_of("a", SRC).unwrap().hash(), plan_of("b", SRC).unwrap().hash());
}

#[test]
fn two_runs_of_the_same_events_write_the_same_journal() {
    let plan = plan_of("det", SRC).unwrap();
    let a = record(&plan, &mut Count { step: 1 });
    let b = record(&plan, &mut Count { step: 1 });
    assert_eq!(a, b);
    assert_eq!(a.len(), 4);
    // the countdown reached zero, stopped its own timer and sent one request
    let last = parse(&a[3]).unwrap();
    assert_eq!(last["changed"], json!({"left": 0, "on": false}));
    assert_eq!(last["fires"], 3);
    assert_eq!(last["req"][0]["kind"], "store.put");
}

#[test]
fn a_journal_replays_exactly_and_one_changed_byte_is_caught() {
    let plan = plan_of("replay", SRC).unwrap();
    let lines = record(&plan, &mut Count { step: 1 });
    assert!(Session::replay(&plan, &mut Count { step: 1 }, &lines).is_ok());
    let mut bad = lines.clone();
    bad[2] = bad[2].replace("\"left\":3", "\"left\":4");
    assert!(Session::replay(&plan, &mut Count { step: 1 }, &bad).is_err());
    // a different logic is refused before anything runs
    assert!(Session::replay(&plan, &mut Count { step: 2 }, &lines).err().unwrap().contains("別のロジック"));
}

#[test]
fn an_event_that_fails_leaves_nothing_behind() {
    let plan = plan_of("atomic", SRC).unwrap();
    let mut logic = Count { step: 1 };
    let mut s = Session::start(&plan, &logic, 1).unwrap();
    let before = s.rt.clone();
    assert!(s.step(&mut logic, json!({"tap": "nowhere"})).unwrap_err().starts_with("node:nowhere"));
    assert_eq!(s.rt, before);
    assert_eq!(s.lines.len(), 1);
}

#[test]
fn carry_finds_what_changed_and_the_verdict_checks_the_declaration() {
    let plan = plan_of("carry", SRC).unwrap();
    let lines = record(&plan, &mut Count { step: 1 });

    // nothing changed: every step is the same
    let same = carry("j", &plan, &mut Count { step: 1 }, &lines, &plan, &mut Count { step: 1 }).unwrap();
    assert_eq!(same.report["result"], "same");

    // only the wording of the label changed
    let plan2 = plan_of("carry2", &SRC.replace("\"left {left}\"", "\"{left} to go\"")).unwrap();
    let c = carry("j", &plan, &mut Count { step: 1 }, &lines, &plan2, &mut Count { step: 1 }).unwrap();
    assert_eq!(c.report["result"], "diff");
    assert_eq!(c.report["fields"], json!({}));
    assert_eq!(c.report["derives"], json!({"label": 0}));
    assert_eq!(c.report["nodes"], json!({"label": 0}));

    let ok = verdict(&json!({"derives": ["label"], "nodes": ["label"]}), &[c.report.clone()], &[], 1);
    assert_eq!(ok["status"], "sealed");
    let no = verdict(&json!({"nodes": ["label"]}), &[c.report.clone()], &[], 1);
    assert_eq!(no["status"], "rejected");
    assert_eq!(no["undeclared"], json!({"derives": ["label"]}));
    // declared, but it did not change: sealed, with the unused declaration named
    let extra = verdict(&json!({"derives": ["label"], "nodes": ["label", "go"]}), &[c.report], &[], 1);
    assert_eq!(extra["status"], "sealed");
    assert_eq!(extra["declared_unchanged"], json!({"nodes": ["go"]}));
}

#[test]
fn carry_reports_a_broken_invariant_and_a_removed_button() {
    let plan = plan_of("inv", SRC).unwrap();
    let lines = record(&plan, &mut Count { step: 1 });
    // the new logic counts down by 2 and goes below zero
    let c = carry("j", &plan, &mut Count { step: 1 }, &lines, &plan, &mut Count { step: 2 }).unwrap();
    assert!(!c.report["inv"].as_array().unwrap().is_empty());
    assert_eq!(verdict(&json!({"fields": ["left", "on"], "derives": ["label"], "nodes": ["label"]}), &[c.report], &[], 1)["status"], "rejected");

    // the button the journal taps is gone
    let plan3 = plan_of("gone", &SRC.replace("button #go { \"go\"; on tap do start }", "")).unwrap();
    let c = carry("j", &plan, &mut Count { step: 1 }, &lines, &plan3, &mut Count { step: 1 }).unwrap();
    assert_eq!(c.report["result"], "inapplicable");
    assert_eq!(c.report["subject"], "node:go");
    assert!(c.lines.is_none());
}

#[test]
fn the_language_refuses_what_is_outside_it() {
    let e = |src: &str| plan_of("bad", src).unwrap_err();
    assert!(e("app T: Main\nwhile x {}\n").contains("という宣言はない"));
    assert!(e("app T: Main\nderive a = b\nderive b = a\nview Main { box #r {} }").contains("輪"));
    assert!(e("app T: Main\nview Main { text { \"hi\" } }").contains("#id が要る"));
    assert!(e("app T: Main\nstate n: int = 0\nview Main { text #a { nope } }").contains("という名前はない"));
    assert!(e("app T: Main\nstate n: int = 0\nview Main { text #a { sqrt(n) } }").contains("関数"));
    assert!(e("app T: Main\nstate n: int = \"x\"\nview Main { box #r {} }").contains("初期値"));
    assert!(e("app T: Main\nview Main { box #a {}\n box #a {} }").contains("2回"));
}

#[test]
fn a_keyed_list_gives_each_row_its_own_id() {
    let src = r#"
app T: Main
state items: list<{ id: int, name: text }> = [{ id: 1, name: "a" }, { id: 2, name: "b" }]
state picked: int = 0
view Row(item) {
  button #row { item.name; on tap set picked = item.id }
}
view Main {
  column #root {
    for it in items key it.id { Row { item: it } }
  }
}
"#;
    let plan = plan_of("list", src).unwrap();
    let mut logic = Count { step: 1 };
    let mut s = Session::start(&plan, &logic, 1).unwrap();
    assert!(s.snaps[0].nodes.contains_key("row[1]") && s.snaps[0].nodes.contains_key("row[2]"));
    s.step(&mut logic, json!({"tap": "row[2]"})).unwrap();
    assert_eq!(s.rt.state["picked"], 2);
}
