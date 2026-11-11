#!/usr/bin/env python3
"""Draws the two design screens of the public ledger as SVG, from the real ledger data."""
import json, pathlib, html, unicodedata

ROOT = pathlib.Path(__file__).resolve().parents[2]
APP = ROOT / "apps/pomodoro"
OUT = pathlib.Path(__file__).resolve().parent

C = dict(bg="#F6F3EC", card="#FFFDF8", ink="#1B1A17", sub="#5F5B52", line="#D9D3C5", soft="#ECE7DB",
         ok="#1F7A4D", okbg="#E2F0E7", ng="#B3261E", ngbg="#FBE8E5", warn="#8A5A00", warnbg="#FBF0D9", link="#2B4C9B")
SANS = "system-ui, -apple-system, 'Hiragino Sans', 'Noto Sans JP', 'Noto Sans CJK JP', sans-serif"
MONO = "ui-monospace, 'SF Mono', Menlo, Consolas, 'Noto Sans Mono CJK JP', monospace"
W = 390
PAD = 20

def wide(ch):
    return unicodedata.east_asian_width(ch) in "WFA"

def units(s):
    return sum(1.0 if wide(c) else 0.56 for c in s)

def wrap(s, size, width):
    """Break by width. Latin words stay whole; a line never starts with closing punctuation."""
    toks, cur = [], ""
    for ch in s:
        if wide(ch) or ch == " ":
            if cur: toks.append(cur); cur = ""
            toks.append(ch)
        else:
            cur += ch
    if cur: toks.append(cur)
    lines, cur = [], ""
    for t in toks:
        if units(cur + t) * size > width and cur and t not in "、。）」・":
            lines.append(cur.rstrip()); cur = t.lstrip()
        else:
            cur += t
    if cur: lines.append(cur)
    return lines

class Svg:
    def __init__(self): self.o = []; self.y = 0
    def add(self, s): self.o.append(s)
    def rect(self, x, y, w, h, fill, r=0, stroke=None, sw=1):
        st = f' stroke="{stroke}" stroke-width="{sw}"' if stroke else ""
        self.add(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" fill="{fill}"{st}/>')
    def text(self, x, y, s, size=15, fill=None, weight=400, font=SANS, anchor="start", ls=0):
        sp = f' letter-spacing="{ls}"' if ls else ""
        self.add(f'<text x="{x}" y="{y}" font-family="{font}" font-size="{size}" font-weight="{weight}" fill="{fill or C["ink"]}" text-anchor="{anchor}"{sp}>{html.escape(s)}</text>')
    def para(self, x, y, s, size=15, width=W - 2 * PAD, lh=1.55, **kw):
        for i, l in enumerate(wrap(s, size, width)):
            self.text(x, y + i * size * lh, l, size, **kw)
        return y + len(wrap(s, size, width)) * size * lh
    def line(self, x1, y1, x2, y2, stroke, sw=1, dash=None):
        d = f' stroke-dasharray="{dash}"' if dash else ""
        self.add(f'<line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" stroke="{stroke}" stroke-width="{sw}" stroke-linecap="round"{d}/>')
    def badge(self, x, y, label, kind):
        fg, bg = {"ok": (C["ok"], C["okbg"]), "ng": (C["ng"], C["ngbg"]), "warn": (C["warn"], C["warnbg"]), "plain": (C["sub"], C["soft"])}[kind]
        w = units(label) * 12 + 18
        self.rect(x, y - 15, w, 22, bg, 11)
        self.text(x + w / 2, y, label, 12, fg, 600, anchor="middle")
        return w
    def check(self, cx, cy, r=10):
        self.add(f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="{C["ok"]}"/>')
        self.add(f'<path d="M{cx-4.2} {cy+0.3} l3 3 l5.6 -6.4" fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>')
    def cross(self, cx, cy, r=10):
        self.add(f'<circle cx="{cx}" cy="{cy}" r="{r-1}" fill="{C["card"]}" stroke="{C["ng"]}" stroke-width="2"/>')
        self.add(f'<path d="M{cx-3.6} {cy-3.6} l7.2 7.2 M{cx+3.6} {cy-3.6} l-7.2 7.2" stroke="{C["ng"]}" stroke-width="2" stroke-linecap="round"/>')
    def chevron(self, x, y, color=None):
        self.add(f'<path d="M{x} {y-5} l5 5 l-5 5" fill="none" stroke="{color or C["sub"]}" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/>')
    def save(self, name, h):
        body = "\n".join(self.o)
        (OUT / name).write_text(f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {h}" width="{W}" height="{h}">\n<rect width="{W}" height="{h}" fill="{C["bg"]}"/>\n{body}\n</svg>\n')

def topbar(s, back=None):
    s.rect(0, 0, W, 52, C["bg"])
    if back:
        s.add(f'<path d="M{PAD+6} 21 l-6 6 l6 6" fill="none" stroke="{C["link"]}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>')
        s.text(PAD + 14, 32, back, 15, C["link"], 500)
    else:
        s.text(PAD, 33, "rustevo", 17, C["ink"], 700, MONO)
    s.rect(W - PAD - 92, 15, 92, 26, C["soft"], 13)
    s.rect(W - PAD - 46, 17, 44, 22, C["card"], 11)
    s.text(W - PAD - 69, 32, "EN", 12, C["sub"], 600, anchor="middle")
    s.text(W - PAD - 24, 32, "日本語", 12, C["ink"], 600, anchor="middle")
    s.line(0, 52, W, 52, C["line"])

def gens():
    out = []
    for l in (APP / "ledger.jsonl").read_text().splitlines():
        v = json.loads(l); d = APP / "generations" / f"{v['n']:04d}"
        v["verdict"] = json.loads((d / "verdict.json").read_text())
        v["gen"] = json.loads((d / "gen.json").read_text())
        out.append(v)
    return out

def index():
    s = Svg(); G = gens(); topbar(s)
    y = 84
    s.text(PAD, y, "PUBLIC LEDGER  ·  公開台帳", 11, C["sub"], 600, MONO, ls=0.6); y += 36
    s.text(PAD, y, "Pomodoro", 30, weight=700); y += 30
    y = s.para(PAD, y, "エージェントがこのアプリを12回書き換えた記録です。何が変わり、何が変わらなかったかを、すべて再演で確かめられます。", 15, fill=C["sub"]) + 6
    # verified card
    s.rect(PAD, y, W - 2 * PAD, 112, C["card"], 12, C["line"])
    s.rect(PAD, y, 4, 112, C["ok"], 2)
    s.check(PAD + 28, y + 30, 10)
    s.text(PAD + 48, y + 35, "12世代すべて検証済み", 16, weight=700)
    s.text(PAD + 20, y + 60, "x86 と ARM で同じ結果  ·  f8c023f  ·  2026-11-11", 11.5, C["sub"], font=MONO)
    s.rect(PAD + 20, y + 74, 148, 26, C["soft"], 6)
    s.text(PAD + 30, y + 92, "./scripts/check.sh", 12.5, font=MONO)
    s.text(W - PAD - 34, y + 92, "自分で確かめる", 13, C["link"], 600, anchor="end")
    s.chevron(W - PAD - 26, y + 87, C["link"])
    y += 132
    # stats
    cw = (W - 2 * PAD - 16) / 3
    ok = sum(1 for g in G if g["status"] == "sealed")
    for i, (n, lab, col) in enumerate([(len(G), "試み", C["ink"]), (ok, "合格", C["ok"]), (len(G) - ok, "不合格", C["ng"])]):
        x = PAD + i * (cw + 8)
        s.rect(x, y, cw, 68, C["card"], 12, C["line"])
        s.text(x + 14, y + 36, str(n), 26, col, 700, MONO)
        s.text(x + 14, y + 56, lab, 12, C["sub"], 500)
    y += 100
    s.text(PAD, y, "世代", 18, weight=700)
    s.text(W - PAD, y, "新しい順", 12, C["sub"], anchor="end")
    y += 20
    # timeline
    LX = PAD + 12
    rows = list(reversed(G)); top = y + 26
    ys = []
    for g in rows:
        lines = wrap(g["intent"], 15, W - (LX + 30) - PAD)[:3]
        ys.append((y, lines)); y += 58 + len(lines) * 22
    s.line(LX, top, LX, ys[-1][0] + 26, C["ok"], 2)
    for g, (ry, lines) in zip(rows, ys):
        cy = ry + 26; v = g["verdict"]; sealed = g["status"] == "sealed"
        tx = LX + 30
        if sealed:
            s.check(LX, cy, 10)
        else:
            s.line(LX, cy, LX + 4, cy, C["ng"], 2, "2 3")
            s.add(f'<circle cx="{LX}" cy="{cy}" r="3" fill="{C["bg"]}" stroke="{C["ok"]}" stroke-width="2"/>')
            s.rect(tx - 12, ry + 6, W - PAD - tx + 12, 44 + len(lines) * 22, C["ngbg"], 10)
            s.cross(LX, cy, 10)
        s.text(tx, cy + 5, f"{g['n']:04d}", 14, C["ink"], 700, MONO)
        bw = s.badge(tx + 44, cy + 5, "合格" if sealed else "不合格", "ok" if sealed else "ng")
        s.text(W - PAD - 22, cy + 5, g["hash"][:8], 11.5, C["sub"], font=MONO, anchor="end")
        s.chevron(W - PAD - 12, cy, C["sub"])
        for i, l in enumerate(lines):
            s.text(tx, cy + 30 + i * 22, l, 15, weight=500)
        my = cy + 30 + len(lines) * 22
        if not sealed:
            s.text(tx, my, "理由: " + " / ".join(v["why"]), 12, C["ng"], 600)
        elif g["gen"]["parent"] is None:
            s.text(tx, my, "最初の世代  ·  記録 3 本", 12, C["sub"])
        else:
            parts = [f"一致 {len(v['same'])}", f"相違 {len(v['diff'])}"] + ([f"引退 {len(v['retired'])}"] if v["retired"] else [])
            note = "  ·  全記録が一致" if not v["diff"] and not v["retired"] else ""
            s.text(tx, my, "  ·  ".join(parts) + note, 12, C["sub"], font=SANS)
    y += 14
    s.line(PAD, y, W - PAD, y, C["line"]); y += 30
    s.text(PAD, y, "合格", 12, C["ok"], 700); s.text(PAD + 30, y, "変わった所がすべて宣言の内側", 12, C["sub"]); y += 20
    s.text(PAD, y, "不合格", 12, C["ng"], 700); s.text(PAD + 42, y, "宣言の外が変わった、または不変条件が破れた", 12, C["sub"]); y += 34
    s.text(PAD, y, "github.com/kajisho5/rustevo", 12, C["link"], 500, MONO); y += 30
    s.save("ledger-index.svg", int(y))

def phone(s, x, y, w, title, rows, hl=()):
    h = 196
    s.rect(x, y, w, h, C["card"], 14, C["line"])
    s.text(x + w / 2, y + 20, title, 11, C["sub"], 600, MONO, anchor="middle")
    s.line(x, y + 30, x + w, y + 30, C["line"])
    cy = y + 66
    for kind, label, key in rows:
        mark = key in hl
        if kind == "big":
            if mark: s.rect(x + 8, cy - 20, w - 16, 30, C["ngbg"], 6)
            s.text(x + w / 2, cy, label, 20, C["ng"] if mark else C["ink"], 700, anchor="middle"); cy += 36
        elif kind == "btn":
            s.rect(x + 14, cy - 18, w - 28, 30, C["warnbg"] if mark else C["soft"], 8, C["warn"] if mark else None)
            s.text(x + w / 2, cy + 2, label, 12.5, C["ink"], 600, anchor="middle"); cy += 42
        else:
            s.text(x + w / 2, cy, label, 12, C["sub"], anchor="middle"); cy += 24
    return y + h

def section(s, y, title, sub=None):
    s.text(PAD, y, title, 17, weight=700)
    if sub: s.text(W - PAD, y, sub, 12, C["sub"], anchor="end")
    return y + 18

def detail():
    s = Svg(); topbar(s, "台帳")
    g = [x for x in gens() if x["n"] == 4][0]; v = g["verdict"]
    y = 86
    s.text(PAD, y, "世代 0004", 13, C["sub"], 700, MONO)
    s.badge(PAD + 84, y, "不合格", "ng"); y += 34
    y = s.para(PAD, y, g["intent"], 22, lh=1.4, weight=700) - 4
    s.text(PAD, y, "Claude Fable 5.2  ·  2026-11-11  ·  親 0003", 12, C["sub"]); y += 26
    # verdict
    s.rect(PAD, y, W - 2 * PAD, 118, C["ngbg"], 12)
    s.cross(PAD + 24, y + 28, 10)
    s.text(PAD + 44, y + 33, "宣言していない所が変わった", 16, C["ng"], 700)
    s.para(PAD + 16, y + 60, "宣言は要素 start だけ。再演すると、状態 remaining・派生値 label・要素 timer も変わっていました。", 13.5, W - 2 * PAD - 32, fill=C["ink"])
    y += 150
    # declared vs measured
    y = section(s, y, "宣言と実測", "親の記録 3 本を再演")
    s.rect(PAD, y, W - 2 * PAD, 30, C["soft"], 8)
    cols = [PAD + 14, PAD + 74, W - PAD - 116, W - PAD - 44]
    for x, t, a in zip(cols, ["種類", "名前", "宣言", "実測"], ["start", "start", "middle", "middle"]):
        s.text(x, y + 20, t, 11.5, C["sub"], 600, anchor=a)
    y += 30
    table = [("要素", "start", True, True), ("状態", "remaining", False, True), ("派生値", "label", False, True), ("要素", "timer", False, True)]
    for kind, name, dec, obs in table:
        bad = obs and not dec
        if bad: s.rect(PAD, y, W - 2 * PAD, 44, C["ngbg"])
        s.text(cols[0], y + 27, kind, 12.5, C["sub"])
        s.text(cols[1], y + 27, name, 14, C["ng"] if bad else C["ink"], 600, MONO)
        if dec: s.add(f'<circle cx="{cols[2]}" cy="{y+22}" r="5" fill="{C["ink"]}"/>')
        else: s.line(cols[2] - 5, y + 22, cols[2] + 5, y + 22, C["line"], 2)
        s.add(f'<circle cx="{cols[3]}" cy="{y+22}" r="5" fill="{C["ng"] if bad else C["ok"]}"/>')
        if bad: s.text(W - PAD - 10, y + 39, "宣言外", 9.5, C["ng"], 700, anchor="end")
        s.line(PAD, y + 44, W - PAD, y + 44, C["line"]); y += 44
    y += 40
    # screens
    y = section(s, y, "画面", "記録 basic  ·  ステップ 1")
    pw = (W - 2 * PAD - 12) / 2
    phone(s, PAD, y, pw, "親 0003", [("big", "残り 25:00", "timer"), ("btn", "Stop", "stop"), ("txt", "done: 0", "count")])
    y2 = phone(s, PAD + pw + 12, y, pw, "この世代 0004", [("big", "残り 20:00", "timer"), ("btn", "Stop", "stop"), ("txt", "done: 0", "count")], hl=("timer",))
    y = y2 + 18
    y = s.para(PAD, y + 4, "画面は論理的な木からの参考表示です。合否には使いません。", 12, fill=C["sub"]) + 26
    # journal
    y = section(s, y, "記録ごとの結果")
    for name, st in [("basic", "ステップ 1 から相違"), ("complete", "ステップ 1 から相違"), ("restart", "ステップ 1 から相違")]:
        open_ = name == "basic"
        h = 248 if open_ else 50
        s.rect(PAD, y, W - 2 * PAD, h, C["card"], 12, C["line"])
        s.text(PAD + 16, y + 31, name, 14.5, weight=700, font=MONO)
        s.badge(PAD + 100, y + 31, "相違", "warn")
        s.text(W - PAD - 34, y + 31, st, 11.5, C["sub"], anchor="end")
        if open_:
            s.add(f'<path d="M{W-PAD-24} {y+23} l5 5 l5 -5" fill="none" stroke="{C["sub"]}" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/>')
            s.line(PAD, y + 50, W - PAD, y + 50, C["line"])
            hx = [PAD + 16, PAD + 42, PAD + 150, W - PAD - 16]
            for x, t, a in zip(hx, ["#", "できごと", "remaining（親）", "この世代"], ["start", "start", "start", "end"]):
                s.text(x, y + 72, t, 11, C["sub"], 600, anchor=a)
            rows = [("0", "開始", "1500", "1500", False), ("1", "tap  start", "1500", "1200", True), ("2", "時計 +3000", "1497", "1197", True), ("3", "tap  stop", "1500", "1200", True)]
            ry = y + 84
            for n, ev, a, b, bad in rows:
                if bad: s.rect(PAD + 1, ry, W - 2 * PAD - 2, 38, C["ngbg"])
                s.text(hx[0], ry + 24, n, 13, C["sub"], font=MONO)
                s.text(hx[1], ry + 24, ev, 13, font=MONO)
                s.text(hx[2], ry + 24, a, 13, C["sub"], font=MONO)
                s.text(hx[3], ry + 24, b, 13, C["ng"] if bad else C["ink"], 700 if bad else 400, MONO, anchor="end")
                ry += 38
            s.text(PAD + 16, ry + 8, "", 1)
        else:
            s.chevron(W - PAD - 22, y + 26)
        y += h + 10
    y += 30
    # plan diff
    y = section(s, y, "plan の差分")
    s.rect(PAD, y, W - 2 * PAD, 98, C["card"], 12, C["line"])
    s.badge(PAD + 16, y + 32, "変更", "plain")
    s.text(PAD + 66, y + 32, "#start", 14, weight=600, font=MONO)
    s.text(W - PAD - 16, y + 32, "要素 1 件", 11.5, C["sub"], anchor="end")
    s.para(PAD + 16, y + 60, "構造の差はボタン1つだけ。ロジックの中の違いは plan に出ないため、再演で見つかりました。", 12.5, W - 2 * PAD - 32, fill=C["sub"])
    y += 138
    # hashes
    y = section(s, y, "ハッシュ")
    s.rect(PAD, y, W - 2 * PAD, 150, C["card"], 12, C["line"])
    for i, (k, val) in enumerate([("世代", g["hash"]), ("親", g["gen"]["parent"]), ("plan", g["gen"]["plan"]), ("ロジック", g["gen"]["logic"])]):
        ry = y + 30 + i * 32
        s.text(PAD + 16, ry, k, 12, C["sub"])
        s.text(W - PAD - 16, ry, val[:28] + "…", 11.5, font=MONO, anchor="end")
        if i < 3: s.line(PAD + 16, ry + 12, W - PAD - 16, ry + 12, C["soft"])
    y += 172
    s.rect(PAD, y, W - 2 * PAD, 44, C["ink"], 10)
    s.text(PAD + 16, y + 27, "$ evo verify apps/pomodoro", 12.5, "#F6F3EC", font=MONO)
    s.text(W - PAD - 16, y + 27, "コピー", 12, "#CFC9BB", 600, anchor="end")
    y += 68
    s.text(PAD, y, "ソースと証跡のファイルを見る", 13, C["link"], 600)
    s.chevron(PAD + 190, y - 5, C["link"]); y += 40
    s.save("ledger-generation.svg", int(y))

index(); detail()
