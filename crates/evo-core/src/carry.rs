//! Carry: replay the parent generation's journals on the new build and say
//! what came out different. Then the verdict: is every difference inside what
//! the change declared?
use crate::plan::*;
use crate::runtime::*;
use crate::session::*;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub struct Carried {
    pub report: Value,
    /// The journal as the new build writes it; none when it could not be carried.
    pub lines: Option<Vec<String>>,
    /// The app after every step, on the parent build and on the new one
    /// (the new list is shorter when the journal could not be carried).
    pub old_snaps: Vec<Snap>,
    pub new_snaps: Vec<Snap>,
}

fn req_key(r: &Value) -> String {
    format!("{}:{}", r["kind"].as_str().unwrap_or(""), r["key"].as_str().unwrap_or(""))
}

fn note(m: &mut BTreeMap<String, usize>, a: &Vals, b: &Vals, step: usize) {
    let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    for k in keys {
        if a.get(k) != b.get(k) {
            m.entry(k.clone()).or_insert(step);
        }
    }
}

pub fn carry(name: &str, old_plan: &Plan, old_logic: &mut dyn Logic, old_lines: &[String], new_plan: &Plan, new_logic: &mut dyn Logic) -> R<Carried> {
    let old = Session::replay(old_plan, old_logic, old_lines).map_err(|e| format!("{name}: 親の記録が親のビルドで再演できない: {e}"))?;
    let seed = parse(&old_lines[0])?["ev"]["init"]["seed"].as_u64().unwrap_or(0);
    let mut reqs: BTreeMap<String, String> = BTreeMap::new();
    for l in old_lines {
        for r in parse(l)?["req"].as_array().into_iter().flatten() {
            reqs.insert(r["id"].as_str().unwrap_or("").into(), req_key(r));
        }
    }
    let mut new = Session::start(new_plan, new_logic, seed)?;
    let (mut fields, mut derives, mut nodes) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    let mut stop: Option<(usize, String, String)> = None;
    let mut cmp = |new: &Session, i: usize| {
        note(&mut fields, &old.snaps[i].state, &new.snaps[i].state, i);
        note(&mut derives, &old.snaps[i].derives, &new.snaps[i].derives, i);
        note(&mut nodes, &old.snaps[i].nodes, &new.snaps[i].nodes, i);
    };
    cmp(&new, 0);
    for (i, l) in old_lines.iter().enumerate().skip(1) {
        let mut ev = parse(l)?["ev"].clone();
        if let Some(id) = ev["result"]["id"].as_str() {
            let key = reqs.get(id).cloned().unwrap_or_default();
            match new.rt.pending.iter().find(|p| req_key(p) == key) {
                Some(p) => ev["result"]["id"] = p["id"].clone(),
                None => {
                    stop = Some((i, format!("effect:{key}"), "記録にある結果の受け取り手が新しいビルドにない".into()));
                    break;
                }
            }
        }
        if let Err(e) = new.step(new_logic, ev) {
            let subject = match e.strip_prefix("node:") {
                Some(rest) => format!("node:{}", rest.split(':').next().unwrap_or("")),
                None => "error".into(),
            };
            stop = Some((i, subject, e));
            break;
        }
        cmp(&new, i);
    }
    if stop.is_none() {
        // a request the new build sends that the old journal never answered
        let mut left: Vec<String> = old.rt.pending.iter().map(req_key).collect();
        for p in &new.rt.pending {
            match left.iter().position(|k| *k == req_key(p)) {
                Some(i) => {
                    left.remove(i);
                }
                None => {
                    stop = Some((old_lines.len() - 1, format!("effect:{}", req_key(p)), "新しいビルドが出す依頼の結果が記録にない".into()));
                    break;
                }
            }
        }
    }
    let inv: Vec<Value> = new.snaps.iter().enumerate().filter(|(_, s)| !s.inv.is_empty()).map(|(i, s)| json!({"step": i, "broken": s.inv})).collect();
    let differs = !(fields.is_empty() && derives.is_empty() && nodes.is_empty());
    let mut report = json!({
        "journal": name,
        "result": if stop.is_some() { "inapplicable" } else if differs { "diff" } else { "same" },
        "steps": old_lines.len(),
        "fields": fields, "derives": derives, "nodes": nodes,
        "inv": inv,
    });
    if let Some((at, subject, reason)) = &stop {
        report["at"] = json!(at);
        report["subject"] = json!(subject);
        report["reason"] = json!(reason);
    }
    Ok(Carried { report, old_snaps: old.snaps, new_snaps: new.snaps, lines: if stop.is_none() { Some(new.lines) } else { None } })
}

fn matches(pats: &[String], s: &str) -> Option<usize> {
    pats.iter().position(|p| match p.strip_suffix('*') {
        Some(pre) => s.starts_with(pre),
        None => p == s,
    })
}

fn list(v: &Value, k: &str) -> Vec<String> {
    v[k].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect()
}

/// `fresh`: for each journal recorded on the new build, the invariants it broke.
pub fn verdict(change: &Value, carries: &[Value], fresh: &[(String, Vec<Value>)], journals_after: usize) -> Value {
    let kinds = ["fields", "derives", "nodes", "effects"];
    let declared: BTreeMap<&str, Vec<String>> = kinds.iter().map(|k| (*k, list(change, k))).collect();
    let mut used: BTreeMap<&str, BTreeSet<usize>> = BTreeMap::new();
    let mut undeclared: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    let mut hit = |kind: &'static str, name: &str| match matches(&declared[kind], name) {
        Some(i) => {
            used.entry(kind).or_default().insert(i);
        }
        None => {
            undeclared.entry(kind).or_default().insert(name.to_string());
        }
    };
    let (mut same, mut diff, mut retired, mut inv, mut other) = (vec![], vec![], vec![], vec![], vec![]);
    for c in carries {
        let name = c["journal"].as_str().unwrap_or("").to_string();
        for kind in ["fields", "derives", "nodes"] {
            for k in c[kind].as_object().into_iter().flatten().map(|(k, _)| k) {
                hit(kind, k);
            }
        }
        if !c["inv"].as_array().is_none_or(|a| a.is_empty()) {
            inv.push(json!({"journal": name, "inv": c["inv"]}));
        }
        match c["result"].as_str() {
            Some("same") => same.push(name),
            Some("diff") => diff.push(name),
            _ => {
                let subject = c["subject"].as_str().unwrap_or("");
                if let Some(id) = subject.strip_prefix("node:") {
                    hit("nodes", id);
                } else if let Some(k) = subject.strip_prefix("effect:") {
                    hit("effects", k);
                } else {
                    other.push(json!({"journal": name, "reason": c["reason"]}));
                }
                retired.push(json!({"journal": name, "at": c["at"], "reason": c["reason"], "subject": subject}));
            }
        }
    }
    for (name, broken) in fresh {
        if !broken.is_empty() {
            inv.push(json!({"journal": name, "inv": broken}));
        }
    }
    let unused: BTreeMap<&str, Vec<String>> = kinds
        .iter()
        .map(|k| (*k, declared[k].iter().enumerate().filter(|(i, _)| !used.get(k).is_some_and(|u| u.contains(i))).map(|(_, p)| p.clone()).collect::<Vec<_>>()))
        .filter(|(_, v)| !v.is_empty())
        .collect();
    let mut why = vec![];
    if !undeclared.is_empty() {
        why.push("宣言していない所が変わった");
    }
    if !inv.is_empty() {
        why.push("不変条件が破れた");
    }
    if !other.is_empty() {
        why.push("記録を流せなかった理由が宣言で説明できない");
    }
    if journals_after == 0 {
        why.push("この世代に記録が1つもない");
    }
    json!({
        "status": if why.is_empty() { "sealed" } else { "rejected" },
        "why": why,
        "undeclared": undeclared,
        "declared_unchanged": unused,
        "inv": inv,
        "other": other,
        "same": same, "diff": diff, "retired": retired,
    })
}
