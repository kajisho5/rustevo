//! The public ledger: static pages written from the stored generations.
//!
//! `site` verifies the ledger first and writes nothing if that fails, so a
//! page that says "verified" was verified by the run that wrote it. Step by
//! step values come from replaying again (`gen::details`); no new stored
//! format. English at the root, Japanese under `ja/`. No JavaScript is needed
//! to read anything; the only script copies a command.
use crate::gen;
use crate::runtime::{Vals, R};
use crate::session::{parse, Snap};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

const CSS: &str = include_str!("site.css");

pub struct Opts {
    /// the app's path as a reader would type it, e.g. `apps/pomodoro`
    pub app_path: String,
    /// e.g. `https://github.com/owner/repo`
    pub repo: Option<String>,
    pub commit: Option<String>,
}

/// Language of a page.
#[derive(Clone, Copy, PartialEq)]
struct L {
    ja: bool,
}

impl L {
    fn t(self, en: &'static str, ja: &'static str) -> &'static str {
        if self.ja {
            ja
        } else {
            en
        }
    }
    fn code(self) -> &'static str {
        self.t("en", "ja")
    }
    /// Text the tool stored in Japanese (verdict reasons, carry reasons).
    fn stored(self, s: &str) -> String {
        if self.ja {
            return s.to_string();
        }
        match s {
            "宣言していない所が変わった" => "Something outside the declaration changed",
            "不変条件が破れた" => "An invariant was broken",
            "記録を流せなかった理由が宣言で説明できない" => "A journal could not be carried, and the declaration does not explain why",
            "この世代に記録が1つもない" => "This generation has no journal",
            "記録にある結果の受け取り手が新しいビルドにない" => "The new build has nothing waiting for a result in the journal",
            "新しいビルドが出す依頼の結果が記録にない" => "The new build sends a request the journal has no result for",
            other => other,
        }
        .to_string()
    }
    fn kind(self, k: &str) -> &'static str {
        match k {
            "fields" => self.t("state", "状態"),
            "derives" => self.t("derived", "派生値"),
            "nodes" => self.t("node", "要素"),
            "effects" => self.t("effect", "外への依頼"),
            "state" => self.t("state", "状態"),
            "actions" => self.t("action", "アクション"),
            "invariants" => self.t("invariant", "不変条件"),
            "styles" => self.t("style", "スタイル"),
            "timers" => self.t("timer", "タイマー"),
            _ => "",
        }
    }
}

fn e(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn date(secs: u64) -> String {
    // days since 1970-01-01 to a calendar date (proleptic Gregorian, UTC)
    let z = (secs / 86400) as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let (d, m) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
    format!("{:04}-{m:02}-{d:02}", yoe + era * 400 + i64::from(m <= 2))
}

const IC_OK: &str = r#"<svg class="ic ok" viewBox="0 0 20 20" aria-hidden="true"><circle cx="10" cy="10" r="10"/><path d="M5.8 10.3l3 3 5.6-6.4" fill="none" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>"#;
const IC_NG: &str = r#"<svg class="ic ng" viewBox="0 0 20 20" aria-hidden="true"><circle cx="10" cy="10" r="9" stroke-width="2"/><path d="M6.4 6.4l7.2 7.2M13.6 6.4l-7.2 7.2" stroke-width="2" stroke-linecap="round"/></svg>"#;

struct Gen {
    n: u64,
    sealed: bool,
    gen: Value,
    change: Value,
    verdict: Value,
    carry: Value,
    diff: Value,
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect()
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

fn page(l: L, title: &str, top_left: &str, other_lang: &str, body: &str) -> String {
    let lang = if l.ja {
        format!(r#"<a href="{other_lang}" lang="en">EN</a><span>日本語</span>"#)
    } else {
        format!(r#"<span>EN</span><a href="{other_lang}" lang="ja">日本語</a>"#)
    };
    format!(
        r#"<!doctype html>
<html lang="{}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light dark">
<title>{}</title>
<style>
{CSS}</style>
</head>
<body>
<header class="top"><div class="wrap">{top_left}<nav class="lang" aria-label="Language">{lang}</nav></div></header>
<main class="wrap">
{body}</main>
</body>
</html>
"#,
        l.code(),
        e(title)
    )
}

fn badge(l: L, sealed: bool) -> String {
    if sealed {
        format!(r#"<span class="badge ok">{}</span>"#, l.t("sealed", "合格"))
    } else {
        format!(r#"<span class="badge ng">{}</span>"#, l.t("rejected", "不合格"))
    }
}

fn index(l: L, gens: &[Gen], o: &Opts) -> String {
    let mut b = String::new();
    let app = gens.first().map(|g| s(&g.gen, "app")).unwrap_or("");
    let ok = gens.iter().filter(|g| g.sealed).count();
    let n = gens.len();
    let _ = writeln!(b, r#"<p class="kicker">{}</p>"#, l.t("PUBLIC LEDGER", "PUBLIC LEDGER  ·  公開台帳"));
    let _ = writeln!(b, "<h1>{}</h1>", e(app));
    let lead = if l.ja {
        format!("エージェントがこのアプリを{n}回書き換えた記録です。何が変わり、何が変わらなかったかを、すべて再演で確かめられます。")
    } else {
        format!("An agent rewrote this app {n} times. This is the record. What changed and what did not can all be checked again by replay.")
    };
    let _ = writeln!(b, r#"<p class="lead">{lead}</p>"#);
    // verified card: only reached when verify passed in this run
    let head = if l.ja { format!("{n}世代すべて検証済み") } else { format!("All {n} generations verified") };
    let mut facts = vec![format!("evo {}", gen::VERSION), std::env::consts::ARCH.to_string()];
    if let Some(c) = &o.commit {
        facts.push(c.chars().take(7).collect());
    }
    let more = match &o.repo {
        Some(r) => format!(r#"<a class="more" href="{}#readme">{}</a>"#, e(r), l.t("Check it yourself", "自分で確かめる")),
        None => String::new(),
    };
    let _ = writeln!(
        b,
        r#"<section class="card verified"><div class="head">{IC_OK}<span>{head}</span></div><p class="mono">{}</p><div class="row"><span class="chip">evo verify {}</span>{more}</div></section>"#,
        e(&facts.join("  ·  ")),
        e(&o.app_path)
    );
    let _ = writeln!(
        b,
        r#"<section class="stats"><div class="card"><b>{n}</b><span>{}</span></div><div class="card ok"><b>{ok}</b><span>{}</span></div><div class="card ng"><b>{}</b><span>{}</span></div></section>"#,
        l.t("attempts", "試み"),
        l.t("sealed", "合格"),
        n - ok,
        l.t("rejected", "不合格")
    );
    let _ = writeln!(b, r#"<div class="sec"><h2>{}</h2><small>{}</small></div>"#, l.t("Generations", "世代"), l.t("newest first", "新しい順"));
    b.push_str("<ol class=\"gens\" reversed>\n");
    for g in gens.iter().rev() {
        let v = &g.verdict;
        let meta = if !g.sealed {
            format!("{}{}", l.t("Why: ", "理由: "), strs(&v["why"]).iter().map(|w| l.stored(w)).collect::<Vec<_>>().join(" / "))
        } else if g.gen["parent"].is_null() {
            let k = strs(&g.gen["fresh"]).len();
            if l.ja {
                format!("最初の世代  ·  記録 {k} 本")
            } else {
                format!("First generation  ·  {k} journals")
            }
        } else {
            let (sm, df, rt) = (strs(&v["same"]).len(), strs(&v["diff"]).len(), v["retired"].as_array().map_or(0, |a| a.len()));
            let mut parts = vec![format!("{} {sm}", l.t("same", "一致")), format!("{} {df}", l.t("differs", "相違"))];
            if rt > 0 {
                parts.push(format!("{} {rt}", l.t("retired", "引退")));
            }
            if df == 0 && rt == 0 {
                parts.push(l.t("every journal the same", "全記録が一致").into());
            }
            parts.join("  ·  ")
        };
        let _ = writeln!(
            b,
            r#"<li class="{}">{}<a class="g" href="g/{:04}.html"><div class="l1"><span class="n">{:04}</span>{}<span class="h">{}</span></div><div class="it">{}</div><div class="m">{}</div></a></li>"#,
            if g.sealed { "ok" } else { "ng" },
            if g.sealed { IC_OK } else { IC_NG },
            g.n,
            g.n,
            badge(l, g.sealed),
            e(&s(&g.gen, "hash").chars().take(8).collect::<String>()),
            e(s(&g.gen, "intent")),
            e(&meta)
        );
    }
    b.push_str("</ol>\n");
    let _ = writeln!(
        b,
        r#"<div class="legend"><p><b class="ok">{}</b>  {}</p><p><b class="ng">{}</b>  {}</p></div>"#,
        l.t("sealed", "合格"),
        l.t("every difference is inside what the change declared", "変わった所がすべて宣言の内側"),
        l.t("rejected", "不合格"),
        l.t("something outside the declaration changed, or an invariant broke", "宣言の外が変わった、または不変条件が破れた")
    );
    if let Some(r) = &o.repo {
        let _ = writeln!(b, r#"<a class="foot" href="{}">{}</a>"#, e(r), e(r.trim_start_matches("https://")));
    }
    let title = format!("{app} — {}", l.t("public ledger", "公開台帳"));
    page(l, &title, r#"<span class="brand">rustevo</span>"#, if l.ja { "../index.html" } else { "ja/index.html" }, &b)
}

fn show(v: Option<&Value>) -> String {
    let t = match v {
        None => return "—".into(),
        Some(Value::String(x)) => format!("\"{x}\""),
        Some(x) => x.to_string(),
    };
    if t.chars().count() > 80 {
        t.chars().take(79).collect::<String>() + "…"
    } else {
        t
    }
}

/// A node for a reader: its text if it has one, otherwise what it is.
fn show_node(v: Option<&Value>) -> String {
    match v {
        None => "—".into(),
        Some(n) => match n["text"].as_str() {
            Some(t) => format!("\"{t}\""),
            None => match n["props"]["value"].as_str() {
                Some(v) => format!("{} \"{v}\"", s(n, "tag")),
                None => s(n, "tag").to_string(),
            },
        },
    }
}

fn event(l: L, line: &Value, prev_t: i64) -> String {
    let ev = &line["ev"];
    if ev.get("init").is_some() {
        l.t("start", "開始").into()
    } else if let Some(id) = ev["tap"].as_str() {
        format!("tap {id}")
    } else if ev.get("type").is_some() {
        format!("type {} \"{}\"", s(&ev["type"], "id"), s(&ev["type"], "text"))
    } else if let Some(t) = ev["tick"].as_i64() {
        format!("{} +{}ms", l.t("clock", "時計"), t - prev_t)
    } else if ev.get("result").is_some() {
        format!("{} {} = {}", l.t("result", "結果"), s(&ev["result"], "id"), show(Some(&ev["result"]["value"])))
    } else {
        ev.to_string()
    }
}

fn differing(a: &Vals, b: &Vals) -> Vec<String> {
    let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    keys.into_iter().filter(|k| a.get(*k) != b.get(*k)).cloned().collect()
}

fn screen(node: &Value, hl: &BTreeSet<String>, out: &mut String) {
    let tag = s(node, "tag");
    let mark = if hl.contains(s(node, "id")) { " hl" } else { "" };
    let text = e(s(node, "text"));
    match tag {
        "column" | "row" => {
            let _ = write!(out, r#"<div class="{}{mark}">"#, if tag == "row" { "row" } else { "col" });
            node["children"].as_array().into_iter().flatten().for_each(|c| screen(c, hl, out));
            out.push_str("</div>");
        }
        "button" => {
            let _ = write!(out, r#"<div class="bt{mark}">{text}</div>"#);
        }
        "input" => {
            let _ = write!(out, r#"<div class="in{mark}">{}</div>"#, e(s(&node["props"], "value")));
        }
        _ => {
            let big = if node["props"]["size"].as_i64().is_some_and(|x| x >= 32) { " big" } else { "" };
            let _ = write!(out, r#"<div class="tx{big}{mark}">{text}</div>"#);
        }
    }
}

fn phone(title: &str, snap: &Snap, hl: &BTreeSet<String>) -> String {
    let mut body = String::new();
    snap.tree.as_array().into_iter().flatten().for_each(|n| screen(n, hl, &mut body));
    format!(r#"<div class="card phone"><div class="ttl">{}</div><div class="scr">{body}</div></div>"#, e(title))
}

fn generation(l: L, g: &Gen, carried: &[gen::CarriedRun], fresh: &[gen::FreshRun], o: &Opts) -> String {
    let mut b = String::new();
    let v = &g.verdict;
    let nn = format!("{:04}", g.n);
    let parent = g.gen["parent_n"].as_u64().map(|p| format!("{p:04}"));
    let _ = writeln!(b, r#"<div class="ghead"><span class="n">{} {nn}</span>{}</div>"#, l.t("GENERATION", "世代"), badge(l, g.sealed));
    let _ = writeln!(b, r#"<h1 class="intent">{}</h1>"#, e(s(&g.gen, "intent")));
    let mut who = vec![s(&g.gen, "by").to_string(), date(g.gen["at"].as_u64().unwrap_or(0))];
    who.push(match &parent {
        Some(p) => format!("{} {p}", l.t("parent", "親")),
        None => l.t("no parent", "親なし").into(),
    });
    let _ = writeln!(b, r#"<p class="meta">{}</p>"#, e(&who.join("  ·  ")));

    // the verdict, in one sentence each
    let undeclared: Vec<(String, String)> = ["fields", "derives", "nodes", "effects"].iter().flat_map(|k| strs(&v["undeclared"][*k]).into_iter().map(|n| (k.to_string(), n))).collect();
    let mut why: Vec<String> = vec![];
    if !undeclared.is_empty() {
        let dec: Vec<String> = ["fields", "derives", "nodes", "effects"].iter().flat_map(|k| strs(&g.change[*k]).into_iter().map(|n| format!("{} {n}", l.kind(k)))).collect();
        let und: Vec<String> = undeclared.iter().map(|(k, n)| format!("{} {n}", l.kind(k))).collect();
        let none = l.t("nothing", "なし").to_string();
        let dec = if dec.is_empty() { none } else { dec.join(l.t(", ", "・")) };
        why.push(if l.ja {
            format!("宣言は {dec} だけ。再演すると、{} も変わっていました。", und.join("・"))
        } else {
            format!("Declared: {dec}. Replay showed these changed too: {}.", und.join(", "))
        });
    }
    for j in v["inv"].as_array().into_iter().flatten() {
        for x in j["inv"].as_array().into_iter().flatten() {
            let (name, step, what) = (s(j, "journal"), x["step"].as_u64().unwrap_or(0), strs(&x["broken"]).join(", "));
            why.push(if l.ja {
                format!("記録 {name} のステップ {step} で、不変条件 {what} が破れました。")
            } else {
                format!("In journal {name}, step {step}, the invariant {what} was broken.")
            });
        }
    }
    for x in v["other"].as_array().into_iter().flatten() {
        why.push(format!("{}: {}", s(x, "journal"), l.stored(s(x, "reason"))));
    }
    let head = if g.sealed && parent.is_none() {
        l.t("The first generation", "最初の世代").to_string()
    } else if g.sealed {
        l.t("Every difference is inside the declaration", "変わった所はすべて宣言の内側").to_string()
    } else {
        strs(&v["why"]).iter().map(|w| l.stored(w)).collect::<Vec<_>>().join(" / ")
    };
    if g.sealed {
        let (sm, df, rt) = (strs(&v["same"]).len(), strs(&v["diff"]).len(), v["retired"].as_array().map_or(0, |a| a.len()));
        why.push(if parent.is_none() {
            l.t("There is no parent to compare with. Its journals are the starting point.", "比べる親はありません。この世代の記録が出発点です。").into()
        } else if l.ja {
            format!("親の記録 {} 本を再演しました。一致 {sm}、相違 {df}、引退 {rt}。", sm + df + rt)
        } else {
            format!("Replayed the parent's {} journals: {sm} the same, {df} differ, {rt} retired.", sm + df + rt)
        });
    }
    let _ = writeln!(
        b,
        r#"<section class="verdict {}"><div class="head">{}<span>{}</span></div>{}</section>"#,
        if g.sealed { "ok" } else { "ng" },
        if g.sealed { IC_OK } else { IC_NG },
        e(&head),
        why.iter().map(|w| format!("<p>{}</p>", e(w))).collect::<String>()
    );

    // declared against measured
    let mut measured: BTreeSet<(String, String)> = BTreeSet::new();
    for c in g.carry.as_array().into_iter().flatten() {
        for k in ["fields", "derives", "nodes"] {
            c[k].as_object().into_iter().flatten().for_each(|(n, _)| {
                measured.insert((k.to_string(), n.clone()));
            });
        }
        let sub = s(c, "subject");
        if let Some(id) = sub.strip_prefix("node:") {
            measured.insert(("nodes".into(), id.into()));
        } else if let Some(k) = sub.strip_prefix("effect:") {
            measured.insert(("effects".into(), k.into()));
        }
    }
    let mut rows: Vec<(String, String, bool, bool)> = measured.iter().map(|(k, n)| (k.clone(), n.clone(), !undeclared.contains(&(k.clone(), n.clone())), true)).collect();
    for k in ["fields", "derives", "nodes", "effects"] {
        strs(&v["declared_unchanged"][k]).into_iter().for_each(|n| rows.push((k.to_string(), n, true, false)));
    }
    rows.sort_by_key(|r| (r.2, ["fields", "derives", "nodes", "effects"].iter().position(|k| *k == r.0)));
    rows.reverse();
    if parent.is_some() {
        let sub = if l.ja { format!("親の記録 {} 本を再演", carried.len()) } else { format!("replayed {} parent journals", carried.len()) };
        let _ = writeln!(b, r#"<div class="sec"><h2>{}</h2><small>{sub}</small></div>"#, l.t("Declared and measured", "宣言と実測"));
        if rows.is_empty() {
            let _ = writeln!(b, r#"<p class="note">{}</p>"#, l.t("Nothing was declared and nothing changed.", "宣言はなく、変わった所もありません。"));
        } else {
            let _ = writeln!(
                b,
                r#"<table class="dm"><thead><tr><th>{}</th><th>{}</th><th class="c">{}</th><th class="c">{}</th></tr></thead><tbody>"#,
                l.t("Kind", "種類"),
                l.t("Name", "名前"),
                l.t("Declared", "宣言"),
                l.t("Measured", "実測")
            );
            let (yes, no) = (l.t("yes", "あり"), l.t("no", "なし"));
            for (k, name, dec, obs) in &rows {
                let bad = *obs && !*dec;
                let d = if *dec { format!(r#"<span class="dot" role="img" aria-label="{yes}"></span>"#) } else { format!(r#"<span class="dash" role="img" aria-label="{no}"></span>"#) };
                let m = if *obs {
                    format!(r#"<span class="dot {}" role="img" aria-label="{yes}"></span>"#, if bad { "ng" } else { "ok" })
                } else {
                    format!(r#"<span class="dash" role="img" aria-label="{no}"></span>"#)
                };
                let out = if bad { format!(r#"<span class="out">{}</span>"#, l.t("undeclared", "宣言外")) } else { String::new() };
                let _ = writeln!(b, r#"<tr class="{}"><td class="k">{}</td><td class="nm">{}</td><td class="c">{d}</td><td class="c">{m}{out}</td></tr>"#, if bad { "bad" } else { "" }, l.kind(k), e(name));
            }
            b.push_str("</tbody></table>\n");
        }
    }

    // screens: the step that shows the most, counting undeclared nodes first;
    // the earliest such step of the earliest journal
    let mut first: Option<(&gen::CarriedRun, usize)> = None;
    let mut best = (0, 0);
    for c in carried {
        for i in 0..c.old.len().min(c.new.len()) {
            let d = differing(&c.old[i].nodes, &c.new[i].nodes);
            let score = (d.iter().filter(|n| undeclared.contains(&("nodes".to_string(), (*n).clone()))).count(), d.len());
            if score > best {
                best = score;
                first = Some((c, i));
            }
        }
    }
    let note = l.t("The screen is drawn from the logical tree, for reference. It is not used for the verdict.", "画面は論理的な木からの参考表示です。合否には使いません。");
    if let (Some((c, i)), Some(p)) = (first, &parent) {
        let hl: BTreeSet<String> = differing(&c.old[i].nodes, &c.new[i].nodes).into_iter().collect();
        let sub = if l.ja { format!("記録 {}  ·  ステップ {i}", c.name) } else { format!("journal {}  ·  step {i}", c.name) };
        let _ = writeln!(b, r#"<div class="sec"><h2>{}</h2><small>{}</small></div>"#, l.t("Screen", "画面"), e(&sub));
        let _ = writeln!(
            b,
            r#"<div class="screens">{}{}</div><p class="note">{note}</p>"#,
            phone(&format!("{} {p}", l.t("parent", "親")), &c.old[i], &hl),
            phone(&format!("{} {nn}", l.t("this", "この世代")), &c.new[i], &hl)
        );
    } else if let Some(snap) = fresh.first().and_then(|f| f.snaps.first()).or_else(|| carried.first().and_then(|c| c.new.first())) {
        let _ = writeln!(b, r#"<div class="sec"><h2>{}</h2><small>{}</small></div>"#, l.t("Screen", "画面"), l.t("at the start", "開始時"));
        let _ = writeln!(b, r#"<div class="screens one">{}</div><p class="note">{note}</p>"#, phone(&nn, snap, &BTreeSet::new()));
    }

    // journals
    let _ = writeln!(b, r#"<div class="sec"><h2>{}</h2></div>"#, l.t("Journal by journal", "記録ごとの結果"));
    let mut opened = false;
    for c in carried {
        let result = s(&c.report, "result");
        // a journal that could not be carried is a plain retirement only
        // when the declaration covers the reason
        let sub = s(&c.report, "subject");
        let unexplained = v["other"].as_array().into_iter().flatten().any(|x| s(x, "journal") == c.name)
            || sub.strip_prefix("node:").is_some_and(|n| undeclared.contains(&("nodes".to_string(), n.to_string())))
            || sub.strip_prefix("effect:").is_some_and(|n| undeclared.contains(&("effects".to_string(), n.to_string())));
        let (cls, label) = match result {
            "same" => ("ok", l.t("same", "一致")),
            "diff" => ("warn", l.t("differs", "相違")),
            _ if unexplained => ("ng", l.t("could not be carried", "流せなかった")),
            _ => ("plain", l.t("retired", "引退")),
        };
        let stop_at = c.report["at"].as_u64().map(|a| a as usize);
        let firsts: Vec<u64> = ["fields", "derives", "nodes"].iter().flat_map(|k| c.report[*k].as_object().into_iter().flatten().filter_map(|(_, v)| v.as_u64())).collect();
        let st = match (result, firsts.iter().min()) {
            ("diff", Some(m)) => {
                if l.ja {
                    format!("ステップ {m} から相違")
                } else {
                    format!("differs from step {m}")
                }
            }
            ("same", _) => {
                if l.ja {
                    format!("{} ステップ", c.lines.len())
                } else {
                    format!("{} steps", c.lines.len())
                }
            }
            _ => {
                let at = c.report["at"].as_u64().unwrap_or(0);
                if l.ja {
                    format!("ステップ {at} で流せなくなった")
                } else {
                    format!("could not be carried past step {at}")
                }
            }
        };
        let open = if result != "same" && !opened {
            opened = true;
            " open"
        } else {
            ""
        };
        let _ = writeln!(b, r#"<details class="card j"{open}><summary><span class="nm">{}</span><span class="badge {cls}">{label}</span><span class="st">{}</span></summary><ol class="steps">"#, e(&c.name), e(&st));
        let mut prev_t = 0;
        for (i, line) in c.lines.iter().enumerate() {
            let line = parse(line).unwrap_or(Value::Null);
            let ev = event(l, &line, prev_t);
            prev_t = line["t"].as_i64().unwrap_or(prev_t);
            let mut subs = String::new();
            let mut bad = false;
            match c.new.get(i) {
                Some(new) => {
                    let old = &c.old[i];
                    for (k, a, bb, node) in [("fields", &old.state, &new.state, false), ("derives", &old.derives, &new.derives, false), ("nodes", &old.nodes, &new.nodes, true)] {
                        for name in differing(a, bb) {
                            bad = true;
                            let f = if node { show_node } else { show };
                            let (mut x, mut y) = (f(a.get(&name)), f(bb.get(&name)));
                            if x == y {
                                // the short form hides what differs (a style, the parent): show it all
                                (x, y) = (show(a.get(&name)), show(bb.get(&name)));
                            }
                            let _ = write!(subs, r#"<li><span class="k">{}</span>{} <span class="old">{}</span> → <span class="new">{}</span></li>"#, l.kind(k), e(&name), e(&x), e(&y));
                        }
                    }
                    for x in &new.inv {
                        bad = true;
                        let _ = write!(subs, r#"<li class="inv">{} {}</li>"#, l.t("invariant broken:", "不変条件が破れた:"), e(x));
                    }
                }
                None => {}
            }
            if stop_at == Some(i) {
                bad = true;
                let _ = write!(subs, r#"<li class="inv">{}</li>"#, e(&l.stored(s(&c.report, "reason"))));
            }
            let subs = if subs.is_empty() { subs } else { format!("<ul>{subs}</ul>") };
            let _ = writeln!(b, r#"<li class="{}"><span class="i">{i}</span><span>{}</span>{subs}</li>"#, if bad { "bad" } else { "" }, e(&ev));
        }
        b.push_str("</ol></details>\n");
    }
    for f in fresh {
        let broke = f.snaps.iter().any(|x| !x.inv.is_empty());
        let st = if l.ja { format!("{} ステップ", f.lines.len()) } else { format!("{} steps", f.lines.len()) };
        let open = if broke && !opened {
            opened = true;
            " open"
        } else {
            ""
        };
        let _ = writeln!(
            b,
            r#"<details class="card j"{open}><summary><span class="nm">{}</span><span class="badge {}">{}</span><span class="st">{}</span></summary><ol class="steps">"#,
            e(&f.name),
            if broke { "ng" } else { "plain" },
            if broke { l.t("invariant broken", "不変条件が破れた") } else { l.t("new", "新規") },
            e(&st)
        );
        let mut prev_t = 0;
        for (i, line) in f.lines.iter().enumerate() {
            let line = parse(line).unwrap_or(Value::Null);
            let ev = event(l, &line, prev_t);
            prev_t = line["t"].as_i64().unwrap_or(prev_t);
            let mut subs = String::new();
            for (k, val) in line["changed"].as_object().into_iter().flatten() {
                let _ = write!(subs, r#"<li><span class="k">{}</span>{} = <span class="new">{}</span></li>"#, l.kind("fields"), e(k), e(&show(Some(val))));
            }
            let inv = f.snaps.get(i).map(|x| x.inv.clone()).unwrap_or_default();
            for x in &inv {
                let _ = write!(subs, r#"<li class="inv">{} {}</li>"#, l.t("invariant broken:", "不変条件が破れた:"), e(x));
            }
            let subs = if subs.is_empty() { subs } else { format!("<ul>{subs}</ul>") };
            let _ = writeln!(b, r#"<li class="{}"><span class="i">{i}</span><span>{}</span>{subs}</li>"#, if inv.is_empty() { "" } else { "bad" }, e(&ev));
        }
        b.push_str("</ol></details>\n");
    }
    if carried.is_empty() && fresh.is_empty() {
        let _ = writeln!(b, r#"<p class="note">{}</p>"#, l.t("No journals.", "記録はありません。"));
    }

    // plan diff
    if let Some(d) = g.diff.as_object() {
        let _ = writeln!(b, r#"<div class="sec"><h2>{}</h2></div><div class="card pd">"#, l.t("Plan diff", "plan の差分"));
        let mut any = false;
        for (cat, v) in d {
            for (op, cls, label) in [("added", "ok", l.t("added", "追加")), ("changed", "plain", l.t("changed", "変更")), ("removed", "warn", l.t("removed", "削除"))] {
                let names = strs(&v[op]);
                if !names.is_empty() {
                    any = true;
                    let _ = writeln!(
                        b,
                        r#"<p><span class="badge {cls}">{label}</span>{}<span class="meta">{} {}</span></p>"#,
                        names.iter().map(|n| format!("<code>{}</code>", e(n))).collect::<Vec<_>>().join(" "),
                        l.kind(cat),
                        names.len()
                    );
                }
            }
        }
        if !any {
            let _ = writeln!(b, r#"<p class="note">{}</p>"#, l.t("The plan is the same as the parent's. Any difference is in the logic.", "plan は親と同じです。違いがあるならロジックの中です。"));
        }
        b.push_str("</div>\n");
    }

    // hashes
    let _ = writeln!(b, r#"<div class="sec"><h2>{}</h2></div><dl class="card hash">"#, l.t("Hashes", "ハッシュ"));
    for (k, key) in [(l.t("generation", "世代"), "hash"), (l.t("parent", "親"), "parent"), ("plan", "plan"), (l.t("logic", "ロジック"), "logic")] {
        let _ = writeln!(b, "<div><dt>{k}</dt><dd>{}</dd></div>", e(g.gen[key].as_str().unwrap_or("—")));
    }
    b.push_str("</dl>\n");
    let cmd = format!("evo verify {}", o.app_path);
    let _ = writeln!(
        b,
        r#"<div class="cmd"><span>$ {}</span><button type="button" hidden onclick="navigator.clipboard.writeText('{}');this.textContent='{}'">{}</button></div><script>if(navigator.clipboard)document.querySelector('.cmd button').hidden=false</script>"#,
        e(&cmd),
        e(&cmd.replace('\\', "\\\\").replace('\'', "\\'")),
        l.t("Copied", "コピーした"),
        l.t("Copy", "コピー")
    );
    if let Some(r) = &o.repo {
        let at = o.commit.as_deref().unwrap_or("main");
        let _ = writeln!(b, r#"<a class="files more" href="{}/tree/{}/{}/generations/{nn}">{}</a>"#, e(r), e(at), e(&o.app_path), l.t("See the source and evidence files", "ソースと証跡のファイルを見る"));
    }
    let title = format!("{nn} — {}", s(&g.gen, "intent"));
    let back = format!(r#"<a class="back" href="../index.html">{}</a>"#, l.t("Ledger", "台帳"));
    page(l, &title, &back, &if l.ja { format!("../../g/{nn}.html") } else { format!("../ja/g/{nn}.html") }, &b)
}

/// Verify the ledger, then write the pages into `out`. Returns how many
/// files were written. Nothing is written when verification fails.
pub fn site(app: &Path, out: &Path, o: &Opts) -> R<usize> {
    let (_, bad) = gen::verify(app)?;
    if !bad.is_empty() {
        return Err(format!("検証に失敗したのでページを作らない: {}", bad.join(" / ")));
    }
    let read = |p: std::path::PathBuf| -> R<Value> { serde_json::from_str(&std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?).map_err(|e| format!("{}: {e}", p.display())) };
    let mut gens = vec![];
    for n in 1..=gen::ledger(app)?.len() as u64 {
        let d = app.join("generations").join(format!("{n:04}"));
        let gen = read(d.join("gen.json"))?;
        gens.push(Gen { n, sealed: gen["status"] == "sealed", gen, change: read(d.join("change.json"))?, verdict: read(d.join("verdict.json"))?, carry: read(d.join("carry.json"))?, diff: read(d.join("plan.diff.json"))? });
    }
    if gens.is_empty() {
        return Err("台帳が空".into());
    }
    let mut files: BTreeMap<String, String> = BTreeMap::new();
    for l in [L { ja: false }, L { ja: true }] {
        let pre = if l.ja { "ja/" } else { "" };
        files.insert(format!("{pre}index.html"), index(l, &gens, o));
    }
    for g in &gens {
        let (carried, fresh) = gen::details(app, g.n)?;
        for l in [L { ja: false }, L { ja: true }] {
            let pre = if l.ja { "ja/" } else { "" };
            files.insert(format!("{pre}g/{:04}.html", g.n), generation(l, g, &carried, &fresh, o));
        }
    }
    for (name, text) in &files {
        let p = out.join(name);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        std::fs::write(&p, text).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    Ok(files.len())
}
