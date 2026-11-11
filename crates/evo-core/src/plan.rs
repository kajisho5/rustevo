//! The plan: what a `.evo` app compiles to. Written out in one fixed form,
//! so the same source always gives the same bytes and the same hash.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const PLAN_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Ty {
    Bool,
    Int,
    Text,
    Enum(Vec<String>),
    List(Box<Ty>),
    Record(BTreeMap<String, Ty>),
}

impl Ty {
    pub fn check(&self, v: &Value) -> bool {
        match (self, v) {
            (Ty::Bool, Value::Bool(_)) => true,
            (Ty::Int, Value::Number(n)) => n.is_i64(),
            (Ty::Text, Value::String(_)) => true,
            (Ty::Enum(vs), Value::String(s)) => vs.contains(s),
            (Ty::List(t), Value::Array(a)) => a.iter().all(|x| t.check(x)),
            (Ty::Record(m), Value::Object(o)) => o.len() == m.len() && m.iter().all(|(k, t)| o.get(k).is_some_and(|x| t.check(x))),
            _ => false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Expr {
    Lit(Value),
    Ref(String),
    Field(Box<Expr>, String),
    Un(String, Box<Expr>),
    Bin(String, Box<Expr>, Box<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    Cat(Vec<Expr>),
    List(Vec<Expr>),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Field {
    pub name: String,
    pub ty: Ty,
    pub init: Value,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Derive {
    pub name: String,
    pub expr: Expr,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Inv {
    pub src: String,
    pub expr: Expr,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Timer {
    pub ms: i64,
    pub when: Expr,
    pub action: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Act {
    Set { field: String, expr: Expr },
    Do { action: String, args: Vec<Expr> },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Node {
    pub tag: String,
    pub id: Option<String>,
    pub style: Option<String>,
    pub props: BTreeMap<String, Expr>,
    pub text: Option<Expr>,
    pub on: BTreeMap<String, Act>,
    pub children: Vec<Item>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Item {
    Node(Node),
    If { cond: Expr, then: Vec<Item>, els: Vec<Item> },
    For { var: String, list: Expr, key: Expr, body: Vec<Item> },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Plan {
    pub v: u32,
    pub app: String,
    pub state: Vec<Field>,
    pub derives: Vec<Derive>,
    pub invariants: Vec<Inv>,
    pub timers: Vec<Timer>,
    pub styles: BTreeMap<String, BTreeMap<String, Value>>,
    pub root: Vec<Item>,
    pub actions: Vec<String>,
}

/// JSON with object keys sorted and no spaces: one value, one byte string.
pub fn canon(v: &Value) -> String {
    fn w(v: &Value, o: &mut String) {
        match v {
            Value::Array(a) => {
                o.push('[');
                for (i, x) in a.iter().enumerate() {
                    if i > 0 {
                        o.push(',');
                    }
                    w(x, o);
                }
                o.push(']');
            }
            Value::Object(m) => {
                let mut ks: Vec<&String> = m.keys().collect();
                ks.sort();
                o.push('{');
                for (i, k) in ks.iter().enumerate() {
                    if i > 0 {
                        o.push(',');
                    }
                    o.push_str(&Value::String((*k).clone()).to_string());
                    o.push(':');
                    w(&m[*k], o);
                }
                o.push('}');
            }
            x => o.push_str(&x.to_string()),
        }
    }
    let mut o = String::new();
    w(v, &mut o);
    o
}

pub fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

pub fn hash(v: &Value) -> String {
    sha(canon(v).as_bytes())
}

impl Plan {
    pub fn to_text(&self) -> String {
        canon(&serde_json::to_value(self).expect("plan"))
    }
    pub fn hash(&self) -> String {
        sha(self.to_text().as_bytes())
    }
    pub fn from_text(s: &str) -> Result<Plan, String> {
        serde_json::from_str(s).map_err(|e| format!("plan を読めない: {e}"))
    }
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.state.iter().find(|f| f.name == name)
    }
}

fn static_nodes(items: &[Item], path: &str, out: &mut BTreeMap<String, Value>) {
    for (i, it) in items.iter().enumerate() {
        match it {
            Item::If { cond, then, els } => {
                out.insert(format!("{path}~if{i}"), json!({ "cond": cond }));
                static_nodes(then, &format!("{path}~if{i}"), out);
                static_nodes(els, &format!("{path}~else{i}"), out);
            }
            Item::For { list, key, body, .. } => {
                out.insert(format!("{path}~for{i}"), json!({ "list": list, "key": key }));
                static_nodes(body, &format!("{path}~for{i}"), out);
            }
            Item::Node(n) => {
                let key = match &n.id {
                    Some(id) => format!("#{id}"),
                    None => format!("{path}~{}{i}", n.tag),
                };
                out.insert(key.clone(), json!({"tag": n.tag, "style": n.style, "props": n.props, "text": n.text, "on": n.on}));
                static_nodes(&n.children, &key, out);
            }
        }
    }
}

fn keyed<T: Serialize>(items: &[T], key: impl Fn(&T) -> String) -> BTreeMap<String, Value> {
    items.iter().map(|x| (key(x), serde_json::to_value(x).expect("value"))).collect()
}

fn diff_maps(a: &BTreeMap<String, Value>, b: &BTreeMap<String, Value>) -> Value {
    let ks: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    let (mut add, mut rem, mut chg) = (vec![], vec![], vec![]);
    for k in ks {
        match (a.get(k), b.get(k)) {
            (None, Some(_)) => add.push(k.clone()),
            (Some(_), None) => rem.push(k.clone()),
            (Some(x), Some(y)) if x != y => chg.push(k.clone()),
            _ => {}
        }
    }
    json!({"added": add, "removed": rem, "changed": chg})
}

/// What differs between two plans, by name and by id. A reviewer reads this
/// before the source diff.
pub fn plan_diff(old: &Plan, new: &Plan) -> Value {
    let nodes = |p: &Plan| {
        let mut m = BTreeMap::new();
        static_nodes(&p.root, "", &mut m);
        m
    };
    let styles = |p: &Plan| p.styles.iter().map(|(k, v)| (k.clone(), json!(v))).collect::<BTreeMap<_, _>>();
    json!({
        "state": diff_maps(&keyed(&old.state, |f| f.name.clone()), &keyed(&new.state, |f| f.name.clone())),
        "derives": diff_maps(&keyed(&old.derives, |f| f.name.clone()), &keyed(&new.derives, |f| f.name.clone())),
        "invariants": diff_maps(&keyed(&old.invariants, |f| f.src.clone()), &keyed(&new.invariants, |f| f.src.clone())),
        "timers": diff_maps(&keyed(&old.timers, |t| format!("{}ms {}", t.ms, t.action)), &keyed(&new.timers, |t| format!("{}ms {}", t.ms, t.action))),
        "styles": diff_maps(&styles(old), &styles(new)),
        "nodes": diff_maps(&nodes(old), &nodes(new)),
        "actions": diff_maps(&keyed(&old.actions, |a| a.clone()), &keyed(&new.actions, |a| a.clone())),
    })
}

fn pascal(s: &str) -> String {
    s.split('_')
        .map(|p| {
            let mut c = p.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        })
        .collect()
}

/// Rust types for the logic crate, generated from the plan so the shape of
/// the state lives in one place only.
pub fn rust_types(p: &Plan) -> String {
    fn ty(name: &str, t: &Ty, defs: &mut Vec<String>) -> String {
        match t {
            Ty::Bool => "bool".into(),
            Ty::Int => "i64".into(),
            Ty::Text => "String".into(),
            Ty::List(t) => format!("Vec<{}>", ty(name, t, defs)),
            Ty::Enum(vs) => {
                let n = pascal(name);
                let body: String = vs.iter().map(|v| format!("    #[serde(rename = \"{v}\")]\n    {},\n", pascal(v))).collect();
                defs.push(format!("#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]\npub enum {n} {{\n{body}}}\n"));
                n
            }
            Ty::Record(m) => {
                let n = pascal(name);
                let body: String = m.iter().map(|(k, t)| format!("    pub {k}: {},\n", ty(&format!("{name}_{k}"), t, defs))).collect();
                defs.push(format!("#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]\npub struct {n} {{\n{body}}}\n"));
                n
            }
        }
    }
    let mut defs = vec![];
    let fields: String = p.state.iter().map(|f| format!("    pub {}: {},\n", f.name, ty(&f.name, &f.ty, &mut defs))).collect();
    format!(
        "// Generated by `evo build` from the app's .evo files. Do not edit.\n\n{}\n#[derive(serde::Deserialize, Clone, Debug)]\npub struct State {{\n{fields}}}\n",
        defs.join("\n")
    )
}
