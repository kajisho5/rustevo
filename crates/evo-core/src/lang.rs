//! `.evo` source -> plan. The language is closed at 14 constructs:
//! app, use, state, derive, invariant, every, view, style,
//! element, if, for, on, set, do.
use crate::plan::*;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Id(String),
    Int(i64),
    Str(String),
    Sym(&'static str),
    Sep,
    Eof,
}

const SYMS: &[&str] = &[
    "==", "!=", "<=", ">=", "&&", "||", "{", "}", "(", ")", "[", "]", ":", ",", ".", "#", "=", "<", ">", "+", "-",
    "*", "/", "%", "!", "|",
];

/// Function vocabulary of `derive` and other expressions: name -> arity.
pub const FUNCS: &[(&str, usize)] =
    &[("len", 1), ("contains", 2), ("mmss", 1), ("str", 1), ("min", 2), ("max", 2), ("abs", 1)];

fn lex(src: &str) -> Result<Vec<(Tok, usize)>, String> {
    let c: Vec<char> = src.chars().collect();
    let (mut i, mut line, mut out) = (0, 1, Vec::new());
    while i < c.len() {
        let ch = c[i];
        if ch == '\n' {
            out.push((Tok::Sep, line));
            line += 1;
            i += 1;
        } else if ch == ';' {
            out.push((Tok::Sep, line));
            i += 1;
        } else if ch.is_whitespace() {
            i += 1;
        } else if ch == '/' && c.get(i + 1) == Some(&'/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
        } else if ch == '"' {
            let mut s = String::new();
            i += 1;
            loop {
                match c.get(i) {
                    None | Some('\n') => return Err(format!("line {line}: 文字列が閉じていない")),
                    Some('"') => break,
                    Some('\\') => {
                        match c.get(i + 1) {
                            Some('n') => s.push('\n'),
                            Some(&x) => s.push(x),
                            None => return Err(format!("line {line}: 文字列が閉じていない")),
                        }
                        i += 2;
                    }
                    Some(&x) => {
                        s.push(x);
                        i += 1;
                    }
                }
            }
            i += 1;
            out.push((Tok::Str(s), line));
        } else if ch.is_ascii_digit() {
            let st = i;
            while i < c.len() && c[i].is_ascii_digit() {
                i += 1;
            }
            let n: String = c[st..i].iter().collect();
            out.push((Tok::Int(n.parse().map_err(|_| format!("line {line}: 数が大きすぎる"))?), line));
        } else if ch.is_alphabetic() || ch == '_' {
            let st = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            out.push((Tok::Id(c[st..i].iter().collect()), line));
        } else {
            let rest: String = c[i..(i + 2).min(c.len())].iter().collect();
            match SYMS.iter().find(|s| rest.starts_with(**s)) {
                Some(s) => {
                    out.push((Tok::Sym(s), line));
                    i += s.len();
                }
                None => return Err(format!("line {line}: 使えない文字 '{ch}'")),
            }
        }
    }
    out.push((Tok::Eof, line));
    Ok(out)
}

struct View {
    params: Vec<String>,
    body: Vec<Item>,
}

#[derive(Default)]
struct Decls {
    app: Option<(String, String)>,
    state: Vec<Field>,
    derives: Vec<Derive>,
    invs: Vec<Inv>,
    timers: Vec<Timer>,
    views: BTreeMap<String, View>,
    styles: BTreeMap<String, BTreeMap<String, Value>>,
}

struct P {
    t: Vec<(Tok, usize)>,
    i: usize,
}

type R<T> = Result<T, String>;

impl P {
    fn peek(&self) -> &Tok {
        &self.t[self.i].0
    }
    fn peek2(&self) -> &Tok {
        &self.t[(self.i + 1).min(self.t.len() - 1)].0
    }
    fn line(&self) -> usize {
        self.t[self.i].1
    }
    fn next(&mut self) -> Tok {
        let t = self.t[self.i].0.clone();
        if self.i < self.t.len() - 1 {
            self.i += 1;
        }
        t
    }
    fn err<T>(&self, m: &str) -> R<T> {
        Err(format!("line {}: {m}", self.line()))
    }
    fn seps(&mut self) {
        while *self.peek() == Tok::Sep {
            self.i += 1;
        }
    }
    fn sym(&mut self, s: &str) -> bool {
        if matches!(self.peek(), Tok::Sym(x) if *x == s) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn want(&mut self, s: &str) -> R<()> {
        if self.sym(s) {
            Ok(())
        } else {
            self.err(&format!("'{s}' が必要"))
        }
    }
    fn kw(&mut self, s: &str) -> bool {
        if matches!(self.peek(), Tok::Id(x) if x == s) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn ident(&mut self) -> R<String> {
        match self.next() {
            Tok::Id(s) => Ok(s),
            _ => {
                self.i -= 1;
                self.err("名前が必要")
            }
        }
    }

    // ---- types and literals
    fn ty(&mut self) -> R<Ty> {
        if self.sym("{") {
            let mut m = BTreeMap::new();
            loop {
                self.seps();
                if self.sym("}") {
                    break;
                }
                let k = self.ident()?;
                self.want(":")?;
                m.insert(k, self.ty()?);
                self.sym(",");
            }
            return Ok(Ty::Record(m));
        }
        let n = self.ident()?;
        Ok(match n.as_str() {
            "bool" => Ty::Bool,
            "int" => Ty::Int,
            "text" => Ty::Text,
            "list" => {
                self.want("<")?;
                let t = self.ty()?;
                self.want(">")?;
                Ty::List(Box::new(t))
            }
            _ => {
                let mut v = vec![n];
                while self.sym("|") {
                    v.push(self.ident()?);
                }
                Ty::Enum(v)
            }
        })
    }
    fn literal(&mut self) -> R<Value> {
        Ok(match self.next() {
            Tok::Int(n) => json!(n),
            Tok::Sym("-") => match self.next() {
                Tok::Int(n) => json!(-n),
                _ => return self.err("数が必要"),
            },
            Tok::Str(s) => json!(s),
            Tok::Id(s) if s == "true" => json!(true),
            Tok::Id(s) if s == "false" => json!(false),
            Tok::Id(s) => json!(s),
            Tok::Sym("[") => {
                let mut v = vec![];
                while !self.sym("]") {
                    v.push(self.literal()?);
                    self.sym(",");
                }
                Value::Array(v)
            }
            Tok::Sym("{") => {
                let mut m = serde_json::Map::new();
                loop {
                    self.seps();
                    if self.sym("}") {
                        break;
                    }
                    let k = self.ident()?;
                    self.want(":")?;
                    m.insert(k, self.literal()?);
                    self.sym(",");
                }
                Value::Object(m)
            }
            _ => {
                self.i -= 1;
                return self.err("値が必要");
            }
        })
    }

    // ---- expressions
    fn expr(&mut self) -> R<Expr> {
        if self.kw("if") {
            let c = self.expr()?;
            if !self.kw("then") {
                return self.err("'then' が必要");
            }
            let a = self.expr()?;
            if !self.kw("else") {
                return self.err("'else' が必要");
            }
            let b = self.expr()?;
            return Ok(Expr::If(Box::new(c), Box::new(a), Box::new(b)));
        }
        self.bin(0)
    }
    fn bin(&mut self, lv: usize) -> R<Expr> {
        const LV: &[&[&str]] =
            &[&["||"], &["&&"], &["==", "!=", "<=", ">=", "<", ">"], &["+", "-"], &["*", "/", "%"]];
        if lv == LV.len() {
            return self.unary();
        }
        let mut l = self.bin(lv + 1)?;
        loop {
            let op = match self.peek() {
                Tok::Sym(s) if LV[lv].contains(s) => *s,
                _ => return Ok(l),
            };
            self.i += 1;
            let r = self.bin(lv + 1)?;
            l = Expr::Bin(op.into(), Box::new(l), Box::new(r));
        }
    }
    fn unary(&mut self) -> R<Expr> {
        if self.sym("!") {
            return Ok(Expr::Un("!".into(), Box::new(self.unary()?)));
        }
        if self.sym("-") {
            return Ok(Expr::Un("-".into(), Box::new(self.unary()?)));
        }
        let mut e = self.primary()?;
        while self.sym(".") {
            e = Expr::Field(Box::new(e), self.ident()?);
        }
        Ok(e)
    }
    fn primary(&mut self) -> R<Expr> {
        Ok(match self.next() {
            Tok::Int(n) => Expr::Lit(json!(n)),
            Tok::Str(s) => interp(&s, self.line())?,
            Tok::Id(s) if s == "true" => Expr::Lit(json!(true)),
            Tok::Id(s) if s == "false" => Expr::Lit(json!(false)),
            Tok::Id(s) => {
                if self.sym("(") {
                    let mut a = vec![];
                    while !self.sym(")") {
                        a.push(self.expr()?);
                        self.sym(",");
                    }
                    Expr::Call(s, a)
                } else {
                    Expr::Ref(s)
                }
            }
            Tok::Sym("(") => {
                let e = self.expr()?;
                self.want(")")?;
                e
            }
            Tok::Sym("[") => {
                let mut a = vec![];
                while !self.sym("]") {
                    a.push(self.expr()?);
                    self.sym(",");
                }
                Expr::List(a)
            }
            _ => {
                self.i -= 1;
                return self.err("式が必要");
            }
        })
    }

    // ---- view items
    fn block(&mut self) -> R<Vec<Item>> {
        self.want("{")?;
        let mut v = vec![];
        loop {
            self.seps();
            if self.sym("}") {
                return Ok(v);
            }
            v.push(self.item()?);
        }
    }
    fn act(&mut self) -> R<Act> {
        if self.kw("set") {
            let field = self.ident()?;
            self.want("=")?;
            Ok(Act::Set { field, expr: self.expr()? })
        } else if self.kw("do") {
            let action = self.ident()?;
            let mut args = vec![];
            if self.sym("(") {
                while !self.sym(")") {
                    args.push(self.expr()?);
                    self.sym(",");
                }
            }
            Ok(Act::Do { action, args })
        } else {
            self.err("'set' か 'do' が必要")
        }
    }
    fn item(&mut self) -> R<Item> {
        if self.kw("if") {
            let cond = self.expr()?;
            let then = self.block()?;
            let els = if self.kw("else") {
                if matches!(self.peek(), Tok::Id(s) if s == "if") {
                    vec![self.item()?]
                } else {
                    self.block()?
                }
            } else {
                vec![]
            };
            return Ok(Item::If { cond, then, els });
        }
        if self.kw("for") {
            let var = self.ident()?;
            if !self.kw("in") {
                return self.err("'in' が必要");
            }
            let list = self.expr()?;
            if !self.kw("key") {
                return self.err("'key' が必要（繰り返しにはキーが要る）");
            }
            let key = self.expr()?;
            return Ok(Item::For { var, list, key, body: self.block()? });
        }
        let is_node = matches!(self.peek(), Tok::Id(_))
            && (matches!(self.peek2(), Tok::Sym("#") | Tok::Sym("{")) || matches!(self.peek2(), Tok::Id(s) if s == "style"));
        if !is_node {
            return self.err("要素が必要（種類 #id { ... }）");
        }
        let tag = self.ident()?;
        let mut n = Node { tag, ..Default::default() };
        if self.sym("#") {
            n.id = Some(self.ident()?);
        }
        if self.kw("style") {
            n.style = Some(self.ident()?);
        }
        if self.sym("{") {
            loop {
                self.seps();
                if self.sym("}") {
                    break;
                }
                if self.kw("on") {
                    let ev = self.ident()?;
                    let a = self.act()?;
                    n.on.insert(ev, a);
                } else if matches!(self.peek(), Tok::Id(s) if s == "if" || s == "for") {
                    n.children.push(self.item()?);
                } else if matches!(self.peek(), Tok::Id(_)) && matches!(self.peek2(), Tok::Sym(":")) {
                    let k = self.ident()?;
                    self.i += 1;
                    n.props.insert(k, self.expr()?);
                } else if matches!(self.peek(), Tok::Id(_))
                    && (matches!(self.peek2(), Tok::Sym("#") | Tok::Sym("{"))
                        || matches!(self.peek2(), Tok::Id(s) if s == "style"))
                {
                    n.children.push(self.item()?);
                } else {
                    if n.text.is_some() {
                        return self.err("文字の中身は1つだけ");
                    }
                    n.text = Some(self.expr()?);
                }
            }
        }
        Ok(Item::Node(n))
    }

    // ---- declarations
    fn decls(&mut self, d: &mut Decls, dir: &Path, seen: &mut BTreeSet<String>) -> R<()> {
        loop {
            self.seps();
            let line = self.line();
            let k = match self.next() {
                Tok::Eof => return Ok(()),
                Tok::Id(k) => k,
                _ => {
                    self.i -= 1;
                    return self.err("宣言が必要");
                }
            };
            match k.as_str() {
                "app" => {
                    let name = self.ident()?;
                    self.want(":")?;
                    d.app = Some((name, self.ident()?));
                }
                "use" => {
                    let f = match self.next() {
                        Tok::Str(s) => s,
                        _ => return self.err("use \"file.evo\""),
                    };
                    if seen.insert(f.clone()) {
                        let src = std::fs::read_to_string(dir.join(&f)).map_err(|e| format!("use {f}: {e}"))?;
                        let mut p = P { t: lex(&src).map_err(|e| format!("{f}: {e}"))?, i: 0 };
                        p.decls(d, dir, seen).map_err(|e| format!("{f}: {e}"))?;
                    }
                }
                "state" => {
                    let name = self.ident()?;
                    self.want(":")?;
                    let ty = self.ty()?;
                    self.want("=")?;
                    let init = self.literal()?;
                    if !ty.check(&init) {
                        return Err(format!("line {line}: state {name} の初期値が型に合わない"));
                    }
                    d.state.push(Field { name, ty, init });
                }
                "derive" => {
                    let name = self.ident()?;
                    self.want("=")?;
                    d.derives.push(Derive { name, expr: self.expr()? });
                }
                "invariant" => {
                    let st = self.i;
                    let expr = self.expr()?;
                    d.invs.push(Inv { src: self.src(st, self.i), expr });
                }
                "every" => {
                    let n = match self.next() {
                        Tok::Int(n) => n,
                        _ => return self.err("every 1s のように書く"),
                    };
                    let ms = match self.ident()?.as_str() {
                        "ms" => n,
                        "s" => n * 1000,
                        "m" => n * 60_000,
                        _ => return self.err("単位は ms / s / m"),
                    };
                    if ms <= 0 {
                        return self.err("間隔は 1ms 以上");
                    }
                    let when = if self.kw("when") { self.expr()? } else { Expr::Lit(json!(true)) };
                    if !self.kw("do") {
                        return self.err("'do' が必要");
                    }
                    d.timers.push(Timer { ms, when, action: self.ident()? });
                }
                "view" => {
                    let name = self.ident()?;
                    let mut params = vec![];
                    if self.sym("(") {
                        while !self.sym(")") {
                            params.push(self.ident()?);
                            self.sym(",");
                        }
                    }
                    let body = self.block()?;
                    d.views.insert(name, View { params, body });
                }
                "style" => {
                    let name = self.ident()?;
                    self.want("{")?;
                    let mut m = BTreeMap::new();
                    loop {
                        self.seps();
                        if self.sym("}") {
                            break;
                        }
                        let k = self.ident()?;
                        self.want(":")?;
                        m.insert(k, self.literal()?);
                    }
                    d.styles.insert(name, m);
                }
                other => {
                    return Err(format!(
                        "line {line}: '{other}' という宣言はない。使えるのは app / use / state / derive / invariant / every / view / style"
                    ))
                }
            }
        }
    }
    fn src(&self, a: usize, b: usize) -> String {
        self.t[a..b]
            .iter()
            .map(|(t, _)| match t {
                Tok::Id(s) => s.clone(),
                Tok::Int(n) => n.to_string(),
                Tok::Str(s) => format!("{s:?}"),
                Tok::Sym(s) => s.to_string(),
                _ => String::new(),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// "done: {done}" -> Cat(["done: ", Ref(done)])
fn interp(s: &str, line: usize) -> R<Expr> {
    if !s.contains('{') {
        return Ok(Expr::Lit(json!(s)));
    }
    let (mut parts, mut rest) = (vec![], s);
    while let Some(a) = rest.find('{') {
        if a > 0 {
            parts.push(Expr::Lit(json!(rest[..a])));
        }
        let b = rest[a..].find('}').ok_or(format!("line {line}: 文字列の中の {{ が閉じていない"))? + a;
        let mut p = P { t: lex(&rest[a + 1..b]).map_err(|e| format!("line {line}: {e}"))?, i: 0 };
        parts.push(p.expr().map_err(|e| format!("line {line}: 文字列の中: {e}"))?);
        rest = &rest[b + 1..];
    }
    if !rest.is_empty() {
        parts.push(Expr::Lit(json!(rest)));
    }
    Ok(Expr::Cat(parts))
}

// ---- resolve

fn subst(e: &mut Expr, m: &BTreeMap<String, Expr>) {
    match e {
        Expr::Ref(n) => {
            if let Some(x) = m.get(n) {
                *e = x.clone();
            }
        }
        Expr::Lit(_) => {}
        Expr::Field(a, _) | Expr::Un(_, a) => subst(a, m),
        Expr::Bin(_, a, b) => {
            subst(a, m);
            subst(b, m);
        }
        Expr::If(a, b, c) => {
            subst(a, m);
            subst(b, m);
            subst(c, m);
        }
        Expr::Call(_, v) | Expr::Cat(v) | Expr::List(v) => v.iter_mut().for_each(|x| subst(x, m)),
    }
}

fn subst_act(a: &mut Act, m: &BTreeMap<String, Expr>) {
    match a {
        Act::Set { expr, .. } => subst(expr, m),
        Act::Do { args, .. } => args.iter_mut().for_each(|x| subst(x, m)),
    }
}

fn inline(items: &[Item], views: &BTreeMap<String, View>, stack: &mut Vec<String>, m: &BTreeMap<String, Expr>, pre: &str) -> R<Vec<Item>> {
    let mut out = vec![];
    for it in items {
        match it {
            Item::If { cond, then, els } => {
                let mut c = cond.clone();
                subst(&mut c, m);
                out.push(Item::If { cond: c, then: inline(then, views, stack, m, pre)?, els: inline(els, views, stack, m, pre)? });
            }
            Item::For { var, list, key, body } => {
                let (mut l, mut k) = (list.clone(), key.clone());
                subst(&mut l, m);
                let mut m2 = m.clone();
                m2.remove(var);
                subst(&mut k, &m2);
                out.push(Item::For { var: var.clone(), list: l, key: k, body: inline(body, views, stack, &m2, pre)? });
            }
            Item::Node(n) => {
                if let Some(v) = views.get(&n.tag) {
                    if stack.contains(&n.tag) {
                        return Err(format!("view {} が自分自身を呼んでいる", n.tag));
                    }
                    if n.text.is_some() || !n.children.is_empty() || !n.on.is_empty() {
                        return Err(format!("view {} の呼び出しには引数（名前: 式）だけ書ける", n.tag));
                    }
                    let mut m2 = BTreeMap::new();
                    for p in &v.params {
                        let mut a = n.props.get(p).cloned().ok_or(format!("view {} に引数 {p} が要る", n.tag))?;
                        subst(&mut a, m);
                        m2.insert(p.clone(), a);
                    }
                    if let Some(x) = n.props.keys().find(|k| !v.params.contains(k)) {
                        return Err(format!("view {} に引数 {x} はない", n.tag));
                    }
                    let pre2 = match &n.id {
                        Some(id) => format!("{pre}{id}_"),
                        None => pre.to_string(),
                    };
                    stack.push(n.tag.clone());
                    out.extend(inline(&v.body, views, stack, &m2, &pre2)?);
                    stack.pop();
                } else {
                    let mut n2 = n.clone();
                    n2.id = n.id.as_ref().map(|i| format!("{pre}{i}"));
                    n2.props.values_mut().for_each(|e| subst(e, m));
                    if let Some(t) = &mut n2.text {
                        subst(t, m);
                    }
                    n2.on.values_mut().for_each(|a| subst_act(a, m));
                    n2.children = inline(&n.children, views, stack, m, pre)?;
                    out.push(Item::Node(n2));
                }
            }
        }
    }
    Ok(out)
}

struct Cx {
    names: BTreeSet<String>,
    variants: BTreeSet<String>,
    fields: BTreeSet<String>,
}

impl Cx {
    fn fix(&self, e: &mut Expr, scope: &[String]) -> R<()> {
        match e {
            Expr::Ref(n) => {
                if scope.contains(n) || self.names.contains(n) {
                } else if self.variants.contains(n) {
                    *e = Expr::Lit(json!(n));
                } else {
                    return Err(format!("'{n}' という名前はない"));
                }
            }
            Expr::Lit(_) => {}
            Expr::Field(a, _) | Expr::Un(_, a) => self.fix(a, scope)?,
            Expr::Bin(_, a, b) => {
                self.fix(a, scope)?;
                self.fix(b, scope)?;
            }
            Expr::If(a, b, c) => {
                self.fix(a, scope)?;
                self.fix(b, scope)?;
                self.fix(c, scope)?;
            }
            Expr::Call(f, v) => {
                match FUNCS.iter().find(|(n, _)| n == f) {
                    None => {
                        return Err(format!(
                            "関数 '{f}' はない。使えるのは {}",
                            FUNCS.iter().map(|x| x.0).collect::<Vec<_>>().join(" / ")
                        ))
                    }
                    Some((_, n)) if *n != v.len() => return Err(format!("関数 {f} の引数は {n} 個")),
                    _ => {}
                }
                for x in v {
                    self.fix(x, scope)?;
                }
            }
            Expr::Cat(v) | Expr::List(v) => {
                for x in v {
                    self.fix(x, scope)?;
                }
            }
        }
        Ok(())
    }
    fn fix_act(&self, a: &mut Act, scope: &[String], actions: &mut BTreeSet<String>) -> R<()> {
        match a {
            Act::Set { field, expr } => {
                if !self.fields.contains(field) {
                    return Err(format!("set の相手 '{field}' は state ではない"));
                }
                self.fix(expr, scope)
            }
            Act::Do { action, args } => {
                actions.insert(action.clone());
                args.iter_mut().try_for_each(|x| self.fix(x, scope))
            }
        }
    }
    fn fix_items(&self, items: &mut [Item], scope: &mut Vec<String>, ids: &mut BTreeSet<String>, actions: &mut BTreeSet<String>) -> R<()> {
        for it in items {
            match it {
                Item::If { cond, then, els } => {
                    self.fix(cond, scope)?;
                    self.fix_items(then, scope, ids, actions)?;
                    self.fix_items(els, scope, ids, actions)?;
                }
                Item::For { var, list, key, body } => {
                    self.fix(list, scope)?;
                    scope.push(var.clone());
                    self.fix(key, scope)?;
                    self.fix_items(body, scope, ids, actions)?;
                    scope.pop();
                }
                Item::Node(n) => {
                    match &n.id {
                        Some(id) => {
                            if !ids.insert(id.clone()) {
                                return Err(format!("id #{id} が2回使われている"));
                            }
                        }
                        None => {
                            if !n.on.is_empty() || n.text.is_some() {
                                return Err(format!("{} に #id が要る（操作できる要素と文字を出す要素は必須）", n.tag));
                            }
                        }
                    }
                    let w = |e: String| format!("#{}: {e}", n.id.clone().unwrap_or_default());
                    for e in n.props.values_mut() {
                        self.fix(e, scope).map_err(w)?;
                    }
                    if let Some(t) = &mut n.text {
                        self.fix(t, scope).map_err(w)?;
                    }
                    for (ev, a) in n.on.iter_mut() {
                        let mut s = scope.clone();
                        if ev == "input" {
                            s.push("value".into());
                        }
                        self.fix_act(a, &s, actions).map_err(w)?;
                    }
                    self.fix_items(&mut n.children, scope, ids, actions)?;
                }
            }
        }
        Ok(())
    }
}

fn refs(e: &Expr, out: &mut BTreeSet<String>) {
    match e {
        Expr::Ref(n) => {
            out.insert(n.clone());
        }
        Expr::Lit(_) => {}
        Expr::Field(a, _) | Expr::Un(_, a) => refs(a, out),
        Expr::Bin(_, a, b) => {
            refs(a, out);
            refs(b, out);
        }
        Expr::If(a, b, c) => {
            refs(a, out);
            refs(b, out);
            refs(c, out);
        }
        Expr::Call(_, v) | Expr::Cat(v) | Expr::List(v) => v.iter().for_each(|x| refs(x, out)),
    }
}

fn variants(t: &Ty, out: &mut BTreeSet<String>) {
    match t {
        Ty::Enum(v) => out.extend(v.iter().cloned()),
        Ty::List(t) => variants(t, out),
        Ty::Record(m) => m.values().for_each(|t| variants(t, out)),
        _ => {}
    }
}

/// Compile the app rooted at `file` into a plan.
pub fn compile(file: &Path) -> R<Plan> {
    let src = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let dir = file.parent().unwrap_or(Path::new("."));
    let mut d = Decls::default();
    let mut p = P { t: lex(&src)?, i: 0 };
    p.decls(&mut d, dir, &mut BTreeSet::new())?;
    let (app, root_view) = d.app.clone().ok_or("app 宣言がない（app 名前: 最初のview）")?;

    let mut cx = Cx { names: BTreeSet::new(), variants: BTreeSet::new(), fields: BTreeSet::new() };
    for f in &d.state {
        if !cx.fields.insert(f.name.clone()) {
            return Err(format!("state {} が2回宣言されている", f.name));
        }
        variants(&f.ty, &mut cx.variants);
    }
    cx.names = cx.fields.clone();
    let dnames: BTreeSet<String> = d.derives.iter().map(|x| x.name.clone()).collect();
    for x in &d.derives {
        if cx.fields.contains(&x.name) || d.derives.iter().filter(|y| y.name == x.name).count() > 1 {
            return Err(format!("derive {} の名前が重なっている", x.name));
        }
    }
    cx.names.extend(dnames.iter().cloned());
    if let Some(v) = cx.variants.iter().find(|v| cx.names.contains(*v)) {
        return Err(format!("'{v}' が列挙の値と state / derive の名前の両方に使われている"));
    }

    // derives: resolve, then order by dependency; a cycle is an error
    for x in &mut d.derives {
        cx.fix(&mut x.expr, &[]).map_err(|e| format!("derive {}: {e}", x.name))?;
    }
    let mut ordered: Vec<Derive> = vec![];
    let mut left = d.derives.clone();
    while !left.is_empty() {
        let done: BTreeSet<String> = ordered.iter().map(|x| x.name.clone()).collect();
        let (ready, rest): (Vec<_>, Vec<_>) = left.into_iter().partition(|x| {
            let mut r = BTreeSet::new();
            refs(&x.expr, &mut r);
            r.iter().all(|n| !dnames.contains(n) || done.contains(n))
        });
        if ready.is_empty() {
            return Err(format!(
                "derive が輪になっている: {}",
                rest.iter().map(|x| x.name.clone()).collect::<Vec<_>>().join(", ")
            ));
        }
        ordered.extend(ready);
        left = rest;
    }

    let mut actions = BTreeSet::new();
    for x in &mut d.invs {
        cx.fix(&mut x.expr, &[]).map_err(|e| format!("invariant {}: {e}", x.src))?;
    }
    for t in &mut d.timers {
        cx.fix(&mut t.when, &[]).map_err(|e| format!("every: {e}"))?;
        actions.insert(t.action.clone());
    }
    let rv = d.views.get(&root_view).ok_or(format!("view {root_view} がない"))?;
    if !rv.params.is_empty() {
        return Err(format!("最初の view {root_view} は引数を取れない"));
    }
    let mut root = inline(&rv.body, &d.views, &mut vec![root_view.clone()], &BTreeMap::new(), "")?;
    let mut ids = BTreeSet::new();
    cx.fix_items(&mut root, &mut vec![], &mut ids, &mut actions)?;
    let check_style = |items: &[Item]| -> R<()> {
        fn walk(items: &[Item], s: &BTreeMap<String, BTreeMap<String, Value>>) -> R<()> {
            for it in items {
                match it {
                    Item::If { then, els, .. } => {
                        walk(then, s)?;
                        walk(els, s)?;
                    }
                    Item::For { body, .. } => walk(body, s)?,
                    Item::Node(n) => {
                        if let Some(st) = &n.style {
                            if !s.contains_key(st) {
                                return Err(format!("style {st} がない"));
                            }
                        }
                        walk(&n.children, s)?;
                    }
                }
            }
            Ok(())
        }
        walk(items, &d.styles)
    };
    check_style(&root)?;

    Ok(Plan {
        v: PLAN_VERSION,
        app,
        state: d.state,
        derives: ordered,
        invariants: d.invs,
        timers: d.timers,
        styles: d.styles,
        root,
        actions: actions.into_iter().collect(),
    })
}
