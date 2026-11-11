//! The state machine. The whole app is one function:
//!   step(state, event) -> (state, requests to the outside)
//! Everything from outside (input, time, results of requests) arrives as an
//! event. Nothing else can change the state.
use crate::plan::*;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub type R<T> = Result<T, String>;
pub type Vals = BTreeMap<String, Value>;

/// What an action returns: changes to the state and requests to the outside.
/// It returns them as data; it does no I/O itself.
#[derive(Default, Debug, Clone)]
pub struct Outcome {
    pub set: Vals,
    pub effects: Vec<Value>,
}

pub trait Logic {
    fn call(&mut self, action: &str, state: &Value, args: &[Value]) -> R<Outcome>;
    fn hash(&self) -> String;
}

pub struct Env<'a> {
    pub state: &'a Vals,
    pub derives: &'a Vals,
    pub scope: Vec<(String, Value)>,
}

fn int(v: &Value) -> R<i64> {
    v.as_i64().ok_or(format!("整数が必要: {v}"))
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        x => x.to_string(),
    }
}

pub fn eval(e: &Expr, env: &Env) -> R<Value> {
    Ok(match e {
        Expr::Lit(v) => v.clone(),
        Expr::Ref(n) => {
            if let Some((_, v)) = env.scope.iter().rev().find(|(k, _)| k == n) {
                v.clone()
            } else if let Some(v) = env.derives.get(n).or_else(|| env.state.get(n)) {
                v.clone()
            } else {
                return Err(format!("'{n}' の値がない"));
            }
        }
        Expr::Field(a, f) => eval(a, env)?.get(f).cloned().ok_or(format!("項目 {f} がない"))?,
        Expr::Un(op, a) => {
            let v = eval(a, env)?;
            match op.as_str() {
                "!" => json!(!v.as_bool().ok_or("真偽が必要")?),
                _ => json!(int(&v)?.checked_neg().ok_or("桁あふれ")?),
            }
        }
        Expr::Bin(op, a, b) => {
            let l = eval(a, env)?;
            match op.as_str() {
                "&&" => return Ok(json!(l.as_bool().ok_or("真偽が必要")? && eval(b, env)?.as_bool().ok_or("真偽が必要")?)),
                "||" => return Ok(json!(l.as_bool().ok_or("真偽が必要")? || eval(b, env)?.as_bool().ok_or("真偽が必要")?)),
                _ => {}
            }
            let r = eval(b, env)?;
            match op.as_str() {
                "==" => json!(l == r),
                "!=" => json!(l != r),
                "+" if l.is_string() || r.is_string() => json!(text(&l) + &text(&r)),
                _ => {
                    let (x, y) = (int(&l)?, int(&r)?);
                    let of = "桁あふれ";
                    match op.as_str() {
                        "+" => json!(x.checked_add(y).ok_or(of)?),
                        "-" => json!(x.checked_sub(y).ok_or(of)?),
                        "*" => json!(x.checked_mul(y).ok_or(of)?),
                        "/" => json!(x.checked_div(y).ok_or("0 で割った")?),
                        "%" => json!(x.checked_rem(y).ok_or("0 で割った")?),
                        "<" => json!(x < y),
                        "<=" => json!(x <= y),
                        ">" => json!(x > y),
                        ">=" => json!(x >= y),
                        o => return Err(format!("演算子 {o} はない")),
                    }
                }
            }
        }
        Expr::If(c, a, b) => {
            if eval(c, env)?.as_bool().ok_or("真偽が必要")? {
                eval(a, env)?
            } else {
                eval(b, env)?
            }
        }
        Expr::Call(f, args) => {
            let v: Vec<Value> = args.iter().map(|a| eval(a, env)).collect::<R<_>>()?;
            match f.as_str() {
                "len" => match &v[0] {
                    Value::Array(a) => json!(a.len() as i64),
                    Value::String(s) => json!(s.chars().count() as i64),
                    x => return Err(format!("len はリストか文字列に使う: {x}")),
                },
                "contains" => json!(v[0].as_array().ok_or("contains はリストに使う")?.contains(&v[1])),
                "mmss" => {
                    let s = int(&v[0])?;
                    json!(format!("{}{:02}:{:02}", if s < 0 { "-" } else { "" }, s.abs() / 60, s.abs() % 60))
                }
                "str" => json!(text(&v[0])),
                "min" => json!(int(&v[0])?.min(int(&v[1])?)),
                "max" => json!(int(&v[0])?.max(int(&v[1])?)),
                "abs" => json!(int(&v[0])?.checked_abs().ok_or("桁あふれ")?),
                o => return Err(format!("関数 {o} はない")),
            }
        }
        Expr::Cat(parts) => json!(parts.iter().map(|p| eval(p, env).map(|v| text(&v))).collect::<R<Vec<_>>>()?.concat()),
        Expr::List(items) => Value::Array(items.iter().map(|p| eval(p, env)).collect::<R<_>>()?),
    })
}

/// One event handler found in the built tree, with the loop variables it sees.
#[derive(Clone)]
pub struct Handler {
    pub act: Act,
    pub scope: Vec<(String, Value)>,
}

pub struct Built {
    pub tree: Value,
    pub handlers: BTreeMap<String, BTreeMap<String, Handler>>,
    /// id -> the node without its children, plus its parent id
    pub flat: Vals,
}

struct Builder<'a> {
    plan: &'a Plan,
    handlers: BTreeMap<String, BTreeMap<String, Handler>>,
    flat: Vals,
    autos: BTreeMap<String, usize>,
}

impl Builder<'_> {
    fn items(&mut self, items: &[Item], env: &mut Env, parent: &str, suffix: &str, out: &mut Vec<Value>) -> R<()> {
        for it in items {
            match it {
                Item::If { cond, then, els } => {
                    let c = eval(cond, env)?.as_bool().ok_or("if の条件は真偽")?;
                    self.items(if c { then } else { els }, env, parent, suffix, out)?;
                }
                Item::For { var, list, key, body } => {
                    let l = eval(list, env)?;
                    for x in l.as_array().ok_or("for はリストに使う")? {
                        env.scope.push((var.clone(), x.clone()));
                        let k = text(&eval(key, env)?);
                        let r = self.items(body, env, parent, &format!("{suffix}[{k}]"), out);
                        env.scope.pop();
                        r?;
                    }
                }
                Item::Node(n) => {
                    let id = match &n.id {
                        Some(id) => format!("{id}{suffix}"),
                        None => {
                            let c = self.autos.entry(format!("{parent}/{}", n.tag)).or_insert(0);
                            *c += 1;
                            format!("{parent}/{}{}", n.tag, *c - 1)
                        }
                    };
                    let mut props = Map::new();
                    if let Some(s) = &n.style {
                        for (k, v) in &self.plan.styles[s] {
                            props.insert(k.clone(), v.clone());
                        }
                    }
                    for (k, e) in &n.props {
                        props.insert(k.clone(), eval(e, env).map_err(|e| format!("#{id} {k}: {e}"))?);
                    }
                    let mut node = Map::new();
                    node.insert("id".into(), json!(id));
                    node.insert("tag".into(), json!(n.tag));
                    node.insert("props".into(), Value::Object(props));
                    if let Some(t) = &n.text {
                        node.insert("text".into(), json!(text(&eval(t, env).map_err(|e| format!("#{id}: {e}"))?)));
                    }
                    if !n.on.is_empty() {
                        node.insert("on".into(), json!(n.on.keys().collect::<Vec<_>>()));
                        let hs = n.on.iter().map(|(ev, a)| (ev.clone(), Handler { act: a.clone(), scope: env.scope.clone() })).collect();
                        self.handlers.insert(id.clone(), hs);
                    }
                    let mut flat = node.clone();
                    flat.insert("parent".into(), json!(parent));
                    if self.flat.insert(id.clone(), Value::Object(flat)).is_some() {
                        return Err(format!("id #{id} が画面に2つある（for の key が重なっていないか）"));
                    }
                    let mut kids = vec![];
                    self.items(&n.children, env, &id, suffix, &mut kids)?;
                    node.insert("children".into(), Value::Array(kids));
                    out.push(Value::Object(node));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Runtime {
    pub state: Vals,
    pub t: i64,
    /// next firing time of each `every`, or none while its condition is false
    pub timers: Vec<Option<i64>>,
    /// requests sent out and not answered yet
    pub pending: Vec<Value>,
    pub eseq: u64,
    pub seed: u64,
}

#[derive(Default, Debug)]
pub struct StepOut {
    pub changed: Vals,
    pub requests: Vec<Value>,
    pub fires: u64,
}

const MAX_FIRES: u64 = 5_000_000;

impl Runtime {
    pub fn new(plan: &Plan, seed: u64) -> R<Runtime> {
        let mut rt = Runtime {
            state: plan.state.iter().map(|f| (f.name.clone(), f.init.clone())).collect(),
            t: 0,
            timers: vec![None; plan.timers.len()],
            pending: vec![],
            eseq: 0,
            seed,
        };
        rt.arm(plan)?;
        Ok(rt)
    }

    pub fn derives(&self, plan: &Plan) -> R<Vals> {
        let mut d = Vals::new();
        for x in &plan.derives {
            let v = eval(&x.expr, &Env { state: &self.state, derives: &d, scope: vec![] }).map_err(|e| format!("derive {}: {e}", x.name))?;
            d.insert(x.name.clone(), v);
        }
        Ok(d)
    }

    pub fn build(&self, plan: &Plan) -> R<Built> {
        let d = self.derives(plan)?;
        let mut b = Builder { plan, handlers: BTreeMap::new(), flat: Vals::new(), autos: BTreeMap::new() };
        let mut out = vec![];
        b.items(&plan.root, &mut Env { state: &self.state, derives: &d, scope: vec![] }, "", "", &mut out)?;
        Ok(Built { tree: Value::Array(out), handlers: b.handlers, flat: b.flat })
    }

    /// Sources of the invariants that do not hold now.
    pub fn broken(&self, plan: &Plan) -> Vec<String> {
        let d = match self.derives(plan) {
            Ok(d) => d,
            Err(e) => return vec![format!("derive を計算できない: {e}")],
        };
        let env = Env { state: &self.state, derives: &d, scope: vec![] };
        plan.invariants.iter().filter(|i| eval(&i.expr, &env).ok().and_then(|v| v.as_bool()) != Some(true)).map(|i| i.src.clone()).collect()
    }

    pub fn sys(&self) -> Value {
        json!({"t": self.t, "timers": self.timers, "pending": self.pending, "eseq": self.eseq, "seed": self.seed})
    }

    fn arm(&mut self, plan: &Plan) -> R<()> {
        let d = self.derives(plan)?;
        let env = Env { state: &self.state, derives: &d, scope: vec![] };
        for (i, tm) in plan.timers.iter().enumerate() {
            let on = eval(&tm.when, &env)?.as_bool().ok_or("every の when は真偽")?;
            if !on {
                self.timers[i] = None;
            } else if self.timers[i].is_none() {
                self.timers[i] = Some(self.t + tm.ms);
            }
        }
        Ok(())
    }

    fn set(&mut self, plan: &Plan, name: &str, v: Value) -> R<()> {
        let f = plan.field(name).ok_or(format!("state {name} はない"))?;
        if !f.ty.check(&v) {
            return Err(format!("state {name} に型の合わない値: {v}"));
        }
        self.state.insert(name.into(), v);
        Ok(())
    }

    fn call(&mut self, plan: &Plan, logic: &mut dyn Logic, action: &str, args: &[Value], out: &mut StepOut) -> R<()> {
        let st = Value::Object(self.state.clone().into_iter().collect());
        let o = logic.call(action, &st, args).map_err(|e| format!("action {action}: {e}"))?;
        for (k, v) in o.set {
            self.set(plan, &k, v).map_err(|e| format!("action {action}: {e}"))?;
        }
        for mut e in o.effects {
            let m = e.as_object_mut().ok_or(format!("action {action}: 依頼は kind つきのオブジェクトで返す"))?;
            if !m.get("kind").is_some_and(|k| k.is_string()) {
                return Err(format!("action {action}: 依頼に kind がない"));
            }
            m.insert("id".into(), json!(format!("e{}", self.eseq)));
            self.eseq += 1;
            out.requests.push(e.clone());
            self.pending.push(e);
        }
        self.arm(plan)
    }

    fn act(&mut self, plan: &Plan, logic: &mut dyn Logic, h: &Handler, extra: Option<(&str, Value)>, out: &mut StepOut) -> R<()> {
        let d = self.derives(plan)?;
        let mut scope = h.scope.clone();
        if let Some((k, v)) = extra {
            scope.push((k.into(), v));
        }
        let env = Env { state: &self.state, derives: &d, scope };
        match &h.act {
            Act::Set { field, expr } => {
                let v = eval(expr, &env)?;
                self.set(plan, field, v)?;
                self.arm(plan)
            }
            Act::Do { action, args } => {
                let a: Vec<Value> = args.iter().map(|x| eval(x, &env)).collect::<R<_>>()?;
                self.call(plan, logic, action, &a, out)
            }
        }
    }

    fn input(&mut self, plan: &Plan, logic: &mut dyn Logic, id: &str, ev: &str, extra: Option<(&str, Value)>, out: &mut StepOut) -> R<()> {
        let b = self.build(plan)?;
        let hs = b.handlers.get(id);
        if !b.flat.contains_key(id) {
            return Err(format!("node:{id}: この id は今の画面にない"));
        }
        let h = hs.and_then(|m| m.get(ev)).ok_or(format!("node:{id}: {ev} を受けない"))?;
        self.act(plan, logic, h, extra, out)
    }

    /// Apply one event. On error the runtime may be half-changed: callers keep
    /// a copy and put it back.
    pub fn apply(&mut self, plan: &Plan, logic: &mut dyn Logic, ev: &Value) -> R<StepOut> {
        let before = self.state.clone();
        let mut out = StepOut::default();
        let (kind, arg) = ev.as_object().and_then(|m| if m.len() == 1 { m.iter().next() } else { None }).ok_or("できごとの形が違う")?;
        match kind.as_str() {
            "tap" => self.input(plan, logic, arg.as_str().ok_or("tap は id")?, "tap", None, &mut out)?,
            "type" => {
                let id = arg["id"].as_str().ok_or("type は id と text")?;
                let v = arg["text"].as_str().ok_or("type は id と text")?;
                self.input(plan, logic, id, "input", Some(("value", json!(v))), &mut out)?
            }
            "tick" => {
                let to = arg.as_i64().ok_or("tick は時刻")?;
                if to < self.t {
                    return Err("時計は戻せない".into());
                }
                loop {
                    let next = self.timers.iter().enumerate().filter_map(|(i, t)| t.map(|t| (t, i))).filter(|(t, _)| *t <= to).min();
                    let Some((at, i)) = next else { break };
                    self.t = at;
                    self.timers[i] = Some(at + plan.timers[i].ms);
                    self.call(plan, logic, &plan.timers[i].action.clone(), &[], &mut out)?;
                    out.fires += 1;
                    if out.fires > MAX_FIRES {
                        return Err("1回の tick で動いた回数が多すぎる".into());
                    }
                }
                self.t = to;
            }
            "result" => {
                let id = arg["id"].as_str().ok_or("result は id と value")?;
                let pos = self.pending.iter().position(|p| p["id"] == id).ok_or(format!("依頼 {id} は待っていない"))?;
                let req = self.pending.remove(pos);
                if let Some(then) = req["then"].as_str() {
                    self.call(plan, logic, then, &[arg["value"].clone()], &mut out)?;
                }
            }
            k => return Err(format!("できごと {k} はない")),
        }
        let keys: BTreeSet<&String> = before.keys().chain(self.state.keys()).collect();
        for k in keys {
            if before.get(k) != self.state.get(k) {
                out.changed.insert(k.clone(), self.state[k].clone());
            }
        }
        Ok(out)
    }
}
