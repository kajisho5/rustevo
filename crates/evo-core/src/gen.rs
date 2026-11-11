//! Generations and the ledger. A generation is source + plan + logic +
//! evidence, linked to its parent by hash. `seal` judges a change and writes
//! the generation; `verify` re-derives everything from the stored files.
use crate::carry::*;
use crate::lang::compile;
use crate::plan::*;
use crate::runtime::*;
use crate::session::*;
use crate::wasm::WasmLogic;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

fn io<T>(r: std::io::Result<T>, p: &Path) -> R<T> {
    r.map_err(|e| format!("{}: {e}", p.display()))
}

fn read(p: &Path) -> R<String> {
    io(std::fs::read_to_string(p), p)
}

fn write(p: &Path, s: &str) -> R<()> {
    if let Some(d) = p.parent() {
        io(std::fs::create_dir_all(d), d)?;
    }
    io(std::fs::write(p, s), p)
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).expect("json") + "\n"
}

fn json_file(p: &Path) -> R<Value> {
    serde_json::from_str(&read(p)?).map_err(|e| format!("{}: {e}", p.display()))
}

/// Compile the .evo, generate the Rust types, build the logic to WebAssembly.
/// Leaves `build/plan.json` and `build/logic.wasm`.
pub fn build(app: &Path) -> R<(Plan, Vec<u8>)> {
    let plan = compile(&app.join("app.evo"))?;
    let types = rust_types(&plan);
    let st = app.join("logic/src/state.rs");
    if std::fs::read_to_string(&st).ok().as_deref() != Some(types.as_str()) {
        write(&st, &types)?;
    }
    let manifest = app.join("logic/Cargo.toml");
    let name = read(&manifest)?
        .lines()
        .find_map(|l| l.trim().strip_prefix("name").map(|r| r.trim_start_matches([' ', '=']).trim_matches('"').to_string()))
        .ok_or("logic/Cargo.toml に name がない")?;
    let target = app.join("build/target");
    let out = Command::new("cargo")
        .args(["build", "--release", "--quiet", "--target", "wasm32-unknown-unknown", "--manifest-path"])
        .arg(&manifest)
        .arg("--target-dir")
        .arg(&target)
        .output()
        .map_err(|e| format!("cargo を実行できない: {e}"))?;
    if !out.status.success() {
        return Err(format!("ロジックのビルドに失敗:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    let wasm_path = target.join("wasm32-unknown-unknown/release").join(format!("{}.wasm", name.replace('-', "_")));
    let wasm = io(std::fs::read(&wasm_path), &wasm_path)?;
    let logic = WasmLogic::new(&wasm)?;
    let _ = logic;
    write(&app.join("build/plan.json"), &plan.to_text())?;
    io(std::fs::write(app.join("build/logic.wasm"), &wasm), &app.join("build/logic.wasm"))?;
    Ok((plan, wasm))
}

pub fn load_build(app: &Path) -> R<(Plan, Vec<u8>)> {
    let p = app.join("build/plan.json");
    let w = app.join("build/logic.wasm");
    Ok((Plan::from_text(&read(&p).map_err(|_| "ビルドがない。先に evo build を実行する".to_string())?)?, io(std::fs::read(&w), &w)?))
}

pub fn ledger(app: &Path) -> R<Vec<String>> {
    let p = app.join("ledger.jsonl");
    if !p.exists() {
        return Ok(vec![]);
    }
    read_journal(&p)
}

fn gen_dir(app: &Path, n: u64) -> PathBuf {
    app.join("generations").join(format!("{n:04}"))
}

fn journals(dir: &Path) -> R<Vec<(String, Vec<String>)>> {
    let mut v = vec![];
    if dir.exists() {
        let mut names: Vec<PathBuf> = io(std::fs::read_dir(dir), dir)?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "jsonl")).collect();
        names.sort();
        for p in names {
            v.push((p.file_stem().unwrap().to_string_lossy().to_string(), read_journal(&p)?));
        }
    }
    Ok(v)
}

fn files(dir: &Path, base: &Path, out: &mut BTreeMap<String, String>) -> R<()> {
    let mut es: Vec<PathBuf> = io(std::fs::read_dir(dir), dir)?.filter_map(|e| e.ok().map(|e| e.path())).collect();
    es.sort();
    for p in es {
        if p.is_dir() {
            files(&p, base, out)?;
        } else if p.file_name().is_some_and(|n| n != "gen.json") {
            out.insert(p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/"), sha(&io(std::fs::read(&p), &p)?));
        }
    }
    Ok(())
}

fn gen_hash(dir: &Path, gen: &Value) -> R<String> {
    let mut f = BTreeMap::new();
    files(dir, dir, &mut f)?;
    let mut g = gen.clone();
    g.as_object_mut().unwrap().remove("hash");
    Ok(hash(&json!({"gen": g, "files": f})))
}

/// The last sealed generation: (number, hash).
pub fn parent_of(lines: &[String]) -> R<Option<(u64, String)>> {
    for l in lines.iter().rev() {
        let v = parse(l)?;
        if v["status"] == "sealed" {
            return Ok(Some((v["n"].as_u64().unwrap_or(0), v["hash"].as_str().unwrap_or("").into())));
        }
    }
    Ok(None)
}

pub struct Judged {
    pub carries: Vec<Value>,
    pub verdict: Value,
    pub diff: Value,
    /// journals this generation keeps: carried ones as rewritten, then the fresh ones
    pub keep: Vec<(String, Vec<String>)>,
}

/// Judge a build against its parent generation. Used by `seal`, by `carry`
/// (a dry run) and again by `verify`.
pub fn judge(app: &Path, parent: Option<u64>, plan: &Plan, wasm: &[u8], change: &Value, fresh: &[(String, Vec<String>)]) -> R<Judged> {
    let mut logic = WasmLogic::new(wasm)?;
    let (mut carries, mut keep, mut diff) = (vec![], vec![], Value::Null);
    if let Some(pn) = parent {
        let pd = gen_dir(app, pn);
        let pplan = Plan::from_text(&read(&pd.join("plan.json"))?)?;
        let mut plogic = WasmLogic::new(&io(std::fs::read(pd.join("logic.wasm")), &pd)?)?;
        diff = plan_diff(&pplan, plan);
        for (name, lines) in journals(&pd.join("journals"))? {
            let c = carry(&name, &pplan, &mut plogic, &lines, plan, &mut logic)?;
            if let Some(l) = c.lines {
                keep.push((name, l));
            }
            carries.push(c.report);
        }
    }
    let mut fresh_inv = vec![];
    for (name, lines) in fresh {
        if keep.iter().any(|(n, _)| n == name) {
            return Err(format!("記録 {name} は親から持ち越した記録と名前が同じ。別の名前にする"));
        }
        Session::replay(plan, &mut logic, lines).map_err(|e| format!("記録 {name} は今のビルドで再演できない: {e}"))?;
        let broken: Vec<Value> = lines.iter().enumerate().filter_map(|(i, l)| parse(l).ok().filter(|v| v["inv"].as_array().is_some_and(|a| !a.is_empty())).map(|v| json!({"step": i, "broken": v["inv"]}))).collect();
        fresh_inv.push((name.clone(), broken));
        keep.push((name.clone(), lines.clone()));
    }
    let verdict = verdict(change, &carries, &fresh_inv, keep.len());
    Ok(Judged { carries, verdict, diff, keep })
}

fn snapshot_src(app: &Path, dst: &Path) -> R<()> {
    for e in io(std::fs::read_dir(app), app)?.filter_map(|e| e.ok()) {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "evo") {
            io(std::fs::copy(&p, dst.join(p.file_name().unwrap())), &p)?;
        }
    }
    let ls = app.join("logic/src");
    io(std::fs::create_dir_all(dst.join("logic")), dst)?;
    for e in io(std::fs::read_dir(&ls), &ls)?.filter_map(|e| e.ok()) {
        io(std::fs::copy(e.path(), dst.join("logic").join(e.file_name())), &e.path())?;
    }
    Ok(())
}

/// Build, judge against the parent, and write the generation, sealed or
/// rejected. A rejected attempt is kept in the ledger too.
pub fn seal(app: &Path, by: &str) -> R<Value> {
    let (plan, wasm) = build(app)?;
    let lg = ledger(app)?;
    let n = lg.len() as u64 + 1;
    let parent = parent_of(&lg)?;
    let change_path = app.join("change.json");
    let change = if change_path.exists() { json_file(&change_path)? } else { Value::Null };
    if change["intent"].as_str().is_none_or(|s| s.trim().is_empty()) {
        return Err("change.json に intent（何をなぜ変えるか）を書いてから seal する".into());
    }
    let fresh = journals(&app.join("pending"))?;
    let j = judge(app, parent.as_ref().map(|p| p.0), &plan, &wasm, &change, &fresh)?;
    let status = j.verdict["status"].as_str().unwrap_or("rejected").to_string();

    let dir = gen_dir(app, n);
    if dir.exists() {
        return Err(format!("{} が既にある", dir.display()));
    }
    io(std::fs::create_dir_all(dir.join("src")), &dir)?;
    snapshot_src(app, &dir.join("src"))?;
    write(&dir.join("plan.json"), &plan.to_text())?;
    io(std::fs::write(dir.join("logic.wasm"), &wasm), &dir)?;
    write(&dir.join("change.json"), &pretty(&change))?;
    write(&dir.join("plan.diff.json"), &pretty(&j.diff))?;
    write(&dir.join("carry.json"), &pretty(&json!(j.carries)))?;
    write(&dir.join("verdict.json"), &pretty(&j.verdict))?;
    let stored: Vec<(String, Vec<String>)> = if status == "sealed" { j.keep.clone() } else { fresh.clone() };
    for (name, lines) in &stored {
        write_journal(&dir.join("journals").join(format!("{name}.jsonl")), lines)?;
    }
    let at = std::env::var("EVO_AT").ok().and_then(|s| s.parse::<u64>().ok()).unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0));
    let mut gen = json!({
        "n": n, "status": status, "app": plan.app,
        "parent": parent.as_ref().map(|p| p.1.clone()), "parent_n": parent.as_ref().map(|p| p.0),
        "plan": plan.hash(), "logic": sha(&wasm),
        "intent": change["intent"], "by": by, "runtime": VERSION, "at": at,
        "fresh": fresh.iter().map(|f| f.0.clone()).collect::<Vec<_>>(),
    });
    let h = gen_hash(&dir, &gen)?;
    gen["hash"] = json!(h);
    write(&dir.join("gen.json"), &pretty(&gen))?;
    let line = json!({"n": n, "status": status, "hash": h, "parent": gen["parent"], "intent": change["intent"], "prev": lg.last().map(|l| sha(l.as_bytes())).unwrap_or_default()});
    let mut all = lg;
    all.push(canon(&line));
    write_journal(&app.join("ledger.jsonl"), &all)?;
    if status == "sealed" {
        let _ = std::fs::remove_dir_all(app.join("pending"));
        let _ = std::fs::remove_file(&change_path);
    }
    Ok(json!({"n": n, "status": status, "hash": h, "verdict": j.verdict}))
}

/// Check the whole ledger from the stored files alone. Returns one line per
/// generation and the list of problems (empty when everything holds).
pub fn verify(app: &Path) -> R<(Vec<String>, Vec<String>)> {
    let lg = ledger(app)?;
    let (mut report, mut bad) = (vec![], vec![]);
    let mut prev = String::new();
    let mut sealed: Option<(u64, String)> = None;
    for (i, l) in lg.iter().enumerate() {
        let n = i as u64 + 1;
        let mut errs: Vec<String> = vec![];
        let line = parse(l)?;
        if line["prev"] != prev {
            errs.push("台帳の連鎖が切れている".into());
        }
        prev = sha(l.as_bytes());
        let dir = gen_dir(app, n);
        let r = (|| -> R<()> {
            let gen = json_file(&dir.join("gen.json"))?;
            let h = gen_hash(&dir, &gen)?;
            if gen["hash"] != h || line["hash"] != h {
                errs.push("ファイルの中身がハッシュと合わない（書き換えられている）".into());
            }
            if gen["parent"] != json!(sealed.as_ref().map(|s| s.1.clone())) {
                errs.push("親の世代が合わない".into());
            }
            let plan_text = read(&dir.join("plan.json"))?;
            let plan = Plan::from_text(&plan_text)?;
            let wasm = io(std::fs::read(dir.join("logic.wasm")), &dir)?;
            if gen["plan"] != sha(plan_text.as_bytes()) || gen["logic"] != sha(&wasm) {
                errs.push("plan かロジックが記載のハッシュと合わない".into());
            }
            if gen["runtime"] == VERSION {
                match compile(&dir.join("src/app.evo")) {
                    Ok(p) if p.hash() == plan.hash() => {}
                    Ok(_) => errs.push("保存されたソースをコンパイルしても同じ plan にならない".into()),
                    Err(e) => errs.push(format!("保存されたソースがコンパイルできない: {e}")),
                }
            }
            let stored = journals(&dir.join("journals"))?;
            let fresh_names: Vec<String> = gen["fresh"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
            let fresh: Vec<(String, Vec<String>)> = stored.iter().filter(|(n, _)| fresh_names.contains(n)).cloned().collect();
            let j = judge(app, sealed.as_ref().map(|s| s.0), &plan, &wasm, &json_file(&dir.join("change.json"))?, &fresh)?;
            if json!(j.carries) != json_file(&dir.join("carry.json"))? {
                errs.push("持ち越し再演をやり直すと、保存された結果と違う".into());
            }
            if j.verdict != json_file(&dir.join("verdict.json"))? || j.verdict["status"] != gen["status"] || line["status"] != gen["status"] {
                errs.push("判定をやり直すと、保存された判定と違う".into());
            }
            if gen["status"] == "sealed" {
                let mut keep = j.keep.clone();
                keep.sort();
                if keep != stored {
                    errs.push("保存された記録が、再演し直した記録と違う".into());
                }
                let mut logic = WasmLogic::new(&wasm)?;
                for (name, lines) in &stored {
                    if let Err(e) = Session::replay(&plan, &mut logic, lines) {
                        errs.push(format!("記録 {name}: {e}"));
                    }
                }
            }
            report.push(format!(
                "{n:04} {:8} 記録 {} 本  {}",
                gen["status"].as_str().unwrap_or("?"),
                stored.len(),
                gen["intent"].as_str().unwrap_or("")
            ));
            if gen["status"] == "sealed" {
                sealed = Some((n, h));
            }
            Ok(())
        })();
        if let Err(e) = r {
            errs.push(e);
        }
        bad.extend(errs.into_iter().map(|e| format!("世代 {n:04}: {e}")));
    }
    Ok((report, bad))
}
