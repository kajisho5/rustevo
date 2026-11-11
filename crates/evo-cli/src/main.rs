//! `evo`: the compiler, the driver and the ledger commands.
//!
//! The driver is the only way into a running app, so every input ends up in
//! the journal. A session is its journal file: each command replays it to get
//! back to where the app was, applies one event and appends one line.
use evo_core::gen;
use evo_core::plan::{canon, plan_diff, Plan};
use evo_core::runtime::{Logic, R};
use evo_core::session::{parse, read_journal, write_journal, Session};
use evo_core::wasm::WasmLogic;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const HELP: &str = "evo — rustevo の道具

  ビルド
    evo build <app>                  .evo を plan に、ロジックを WebAssembly にする
  ドライバ（見る）
    evo tree <app> <session>         画面の木
    evo state <app> <session>        状態と派生値
    evo journal <app> <session>      記録
  ドライバ（入れる。すべて記録に残る）
    evo run <app> <session> [seed]   新しい記録を始める
    evo tap <app> <session> <id>
    evo type <app> <session> <id> <text>
    evo clock <app> <session> +<ms>
    evo respond <app> <session> <依頼id> <json>
      --hold をつけると、外への依頼を自動で実行せず待たせる
  世代
    evo diff <app>                   今のビルドと親の世代の plan の差
    evo carry <app>                  判定の下見（何も書かない）
    evo seal <app> [--by 名前]       判定して世代を書く（合格でも不合格でも台帳に残る）
    evo verify <app> [--rebuild]     台帳を最初から検証する
      --rebuild をつけると、各世代の保存されたソースからロジックを作り直し、
      保存された logic.wasm と 1 バイトも違わないかも確かめる（時間がかかる）
    evo ledger <app>                 台帳を表示
    evo replay <app> <file.jsonl>    今のビルドで記録を厳密に再演する
";

fn session_path(app: &Path, name: &str) -> PathBuf {
    app.join("pending").join(format!("{name}.jsonl"))
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).expect("json")
}

/// Do the requests the app sent out, and feed each answer back as an event.
fn fulfil(s: &mut Session, logic: &mut dyn Logic, store_path: &Path) -> R<()> {
    let mut store: serde_json::Map<String, Value> = std::fs::read_to_string(store_path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    for _ in 0..1000 {
        let Some(req) = s.rt.pending.iter().find(|p| matches!(p["kind"].as_str(), Some("store.put" | "store.get" | "random"))).cloned() else {
            break;
        };
        let key = req["key"].as_str().unwrap_or("").to_string();
        let value = match req["kind"].as_str() {
            Some("store.put") => {
                store.insert(key, req["value"].clone());
                Value::Null
            }
            Some("store.get") => store.get(&key).cloned().unwrap_or(Value::Null),
            _ => {
                let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0) as i64;
                json!(nanos % req["max"].as_i64().unwrap_or(1).max(1))
            }
        };
        s.step(logic, json!({"result": {"id": req["id"], "value": value}}))?;
    }
    std::fs::write(store_path, canon(&Value::Object(store))).map_err(|e| e.to_string())
}

fn run(args: &[String]) -> R<()> {
    let hold = args.iter().any(|a| a == "--hold");
    let by = args.iter().position(|a| a == "--by").and_then(|i| args.get(i + 1)).cloned().or_else(|| std::env::var("EVO_BY").ok()).unwrap_or_else(|| "unknown".into());
    let pos: Vec<&String> = {
        let mut skip = false;
        args.iter()
            .filter(|a| {
                if skip {
                    skip = false;
                    return false;
                }
                if *a == "--by" {
                    skip = true;
                }
                !a.starts_with("--")
            })
            .collect()
    };
    let cmd = pos.first().map(|s| s.as_str()).unwrap_or("help");
    if matches!(cmd, "help" | "-h") {
        print!("{HELP}");
        return Ok(());
    }
    let app = PathBuf::from(pos.get(1).ok_or("アプリのフォルダを指定する")?);
    let arg = |i: usize, what: &str| pos.get(i).map(|s| s.to_string()).ok_or(format!("{what} を指定する"));
    match cmd {
        "build" => {
            let (plan, wasm) = gen::build(&app)?;
            println!("plan  {}\nlogic {} ({} bytes)", plan.hash(), evo_core::plan::sha(&wasm), wasm.len());
        }
        "diff" | "carry" => {
            let (plan, wasm) = gen::load_build(&app)?;
            let parent = gen::parent_of(&gen::ledger(&app)?)?;
            if cmd == "diff" {
                let Some((pn, _)) = parent else { return Err("親の世代がまだない".into()) };
                let old = Plan::from_text(&std::fs::read_to_string(app.join(format!("generations/{pn:04}/plan.json"))).map_err(|e| e.to_string())?)?;
                println!("{}", pretty(&plan_diff(&old, &plan)));
            } else {
                let change: Value = std::fs::read_to_string(app.join("change.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
                let mut fresh = vec![];
                if let Ok(rd) = std::fs::read_dir(app.join("pending")) {
                    let mut ps: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "jsonl")).collect();
                    ps.sort();
                    for p in ps {
                        fresh.push((p.file_stem().unwrap().to_string_lossy().to_string(), read_journal(&p)?));
                    }
                }
                let j = gen::judge(&app, parent.map(|p| p.0), &plan, &wasm, &change, &fresh)?;
                println!("{}", pretty(&json!({"carry": j.carries, "verdict": j.verdict})));
            }
        }
        "seal" => {
            let r = gen::seal(&app, &by)?;
            println!("{}", pretty(&r));
            if r["status"] != "sealed" {
                return Err(format!("世代 {} は不合格（台帳には残した）", r["n"]));
            }
        }
        "verify" => {
            let (report, bad) = gen::verify(&app)?;
            report.iter().for_each(|l| println!("{l}"));
            if !bad.is_empty() {
                bad.iter().for_each(|l| eprintln!("NG {l}"));
                return Err(format!("検証に失敗: {} 件", bad.len()));
            }
            println!("OK {} 世代すべて検証できた", report.len());
            if args.iter().any(|a| a == "--rebuild") {
                println!("\nソースからロジックを作り直して比べる");
                let (report, bad) = gen::rebuild(&app)?;
                report.iter().for_each(|l| println!("{l}"));
                if !bad.is_empty() {
                    bad.iter().for_each(|l| eprintln!("NG {l}"));
                    return Err(format!("作り直しの確認に失敗: {} 件", bad.len()));
                }
                println!("OK {} 世代すべて、ソースから作り直したロジックが保存分と一致した", report.len());
            }
        }
        "ledger" => {
            for l in gen::ledger(&app)? {
                let v = parse(&l)?;
                println!("{:04} {:8} {}  {}", v["n"].as_u64().unwrap_or(0), v["status"].as_str().unwrap_or(""), &v["hash"].as_str().unwrap_or("")[..12.min(v["hash"].as_str().unwrap_or("").len())], v["intent"].as_str().unwrap_or(""));
            }
        }
        "replay" => {
            let (plan, wasm) = gen::load_build(&app)?;
            let lines = read_journal(Path::new(&arg(2, "記録のファイル")?))?;
            Session::replay(&plan, &mut WasmLogic::new(&wasm)?, &lines)?;
            println!("OK {} 行、すべて一致", lines.len());
        }
        "run" | "tap" | "type" | "clock" | "respond" | "tree" | "state" | "journal" => {
            let (plan, wasm) = gen::load_build(&app)?;
            let mut logic = WasmLogic::new(&wasm)?;
            let name = arg(2, "セッション名")?;
            let path = session_path(&app, &name);
            let store = app.join("pending").join(format!("{name}.store.json"));
            if cmd == "run" {
                if path.exists() {
                    return Err(format!("{} は既にある", path.display()));
                }
                let seed = pos.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);
                let s = Session::start(&plan, &logic, seed)?;
                write_journal(&path, &s.lines)?;
                println!("{}", s.lines[0]);
                return Ok(());
            }
            let lines = read_journal(&path)?;
            let mut s = Session::replay(&plan, &mut logic, &lines)?;
            let ev = match cmd {
                "tap" => Some(json!({"tap": arg(3, "id")?})),
                "type" => Some(json!({"type": {"id": arg(3, "id")?, "text": arg(4, "文字")?}})),
                "clock" => {
                    let a = arg(3, "+ミリ秒")?;
                    let ms: i64 = a.trim_start_matches('+').parse().map_err(|_| "clock +60000 のように書く")?;
                    Some(json!({"tick": if a.starts_with('+') { s.rt.t + ms } else { ms }}))
                }
                "respond" => {
                    let v: Value = serde_json::from_str(&arg(4, "json")?).map_err(|e| format!("json を読めない: {e}"))?;
                    Some(json!({"result": {"id": arg(3, "依頼id")?, "value": v}}))
                }
                _ => None,
            };
            match ev {
                Some(ev) => {
                    let from = s.lines.len();
                    s.step(&mut logic, ev)?;
                    if !hold {
                        fulfil(&mut s, &mut logic, &store)?;
                    }
                    write_journal(&path, &s.lines)?;
                    s.lines[from..].iter().for_each(|l| println!("{l}"));
                }
                None => match cmd {
                    "tree" => println!("{}", pretty(&s.rt.build(&plan)?.tree)),
                    "state" => println!("{}", pretty(&json!({"t": s.rt.t, "state": s.rt.state, "derives": s.rt.derives(&plan)?, "pending": s.rt.pending, "broken": s.rt.broken(&plan)}))),
                    _ => s.lines.iter().for_each(|l| println!("{l}")),
                },
            }
        }
        other => return Err(format!("コマンド {other} はない。evo help を見る")),
    }
    Ok(())
}

fn main() {
    if let Some(code) = gen::rustc_shim() {
        std::process::exit(code);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(&args) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
