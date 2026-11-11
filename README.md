# rustevo

**The Rust framework for apps that agents evolve, with a replayable record of every change.**

日本語: [README.ja.md](README.ja.md)

After an agent has changed your app fifty times, how do you know what actually changed?
rustevo answers that with evidence instead of trust:

1. **Every run is a record.** Input, time and every answer from the outside go through one journal. The same journal on the same build gives the same state and the same UI tree, byte for byte.
2. **Changes are declared, then measured.** Before a change, the agent writes what will differ. The previous generation's journals are replayed on the new build. Anything that differs outside the declaration fails the generation.
3. **Every change is a generation.** Source, plan, logic and evidence, linked to the parent by hash. Rejected attempts stay in the ledger too.

Status: v0.1, headless. No rendering yet. Nothing here is stable.

## See it

The demo app is a pomodoro timer, evolved through 12 attempts. Ten were sealed and two were rejected:

```
$ evo ledger apps/pomodoro
0001 sealed    first version: a 25-minute focus timer
0002 sealed    tidy the logic, no change in behaviour      <- every journal identical
0003 sealed    prefix the remaining time                    <- differs only where declared
0004 rejected  translate the start button                   <- also changed the focus length
0005 sealed    translate the start button (redo of 4)
0006 sealed    a 5-minute rest after each focus
0007 sealed    save the completed count
0008 sealed    a button that resets the count
0009 rejected  a "skip 5 minutes" button                    <- remaining time went below zero
0010 sealed    skip only during rest (redo of 9)
0011 sealed    name the task you are working on
0012 sealed    keep a list of finished sessions
```

(The intents in the repository are written in Japanese; these are translations.)

Generation 4 is the point of the project. The agent declared one change, the text of a button.
The plan diff agreed: only `#start` differs. But the logic had also changed the focus length from 25 to 20 minutes.
Replaying the parent's journals showed `remaining` differing from step 1, which was not declared, so the generation was rejected:

```json
"undeclared": { "fields": ["remaining"], "derives": ["label"], "nodes": ["timer"] }
```

Check all of it yourself. Nothing is taken on trust:

```
./scripts/check.sh
```

It re-derives every generation from the stored files, then shows that a changed journal, a flipped verdict, a removed ledger line and a swapped logic file are each caught.

## How an app is written

The UI and the shape of the state are declared in `.evo`. Logic is Rust.

```
app Pomodoro: Main

state phase: idle | focus | rest = idle
state remaining: int = 1500
state done: int = 0

derive label = "left " + mmss(remaining)
derive running = phase != idle

invariant remaining >= 0

every 1s when running do tick

view Main {
  column #root {
    text #timer { label }
    if running {
      button #stop { "Stop"; on tap do stop }
    } else {
      button #start { "Start"; on tap do start }
    }
    text #count { "done: {done}" }
  }
}
```

```rust
fn tick(s: &State, _: &[Value]) -> Outcome {
    if s.remaining > 1 {
        Outcome::new().set("remaining", s.remaining - 1)
    } else {
        Outcome::new().set("phase", Phase::Rest).set("done", s.done + 1)
            .effect(evo_logic::store_put("done", s.done + 1))
    }
}
evo_logic::actions!(State; start, stop, tick);
```

The language has 14 constructs and no more: `app` `use` `state` `derive` `invariant` `every` `view` `style`, elements, `if` `for` `on` `set` `do`.

An action reads the state and returns changes and requests as data. It is compiled to WebAssembly and run with no imports, so it cannot read a clock, the network or a file. The runtime performs the requests and records the answers; a replay feeds the recorded answers back and touches nothing outside.

## The loop

```
evo build apps/pomodoro                 # .evo -> plan, logic -> wasm
evo run   apps/pomodoro s1              # start a recorded session
evo tap   apps/pomodoro s1 start        # every input is a journal line
evo clock apps/pomodoro s1 +60000       # time belongs to the driver
evo state apps/pomodoro s1
# ... edit the app, then write change.json:
#   { "intent": "...", "fields": [], "derives": [], "nodes": ["start"], "effects": [] }
evo carry  apps/pomodoro                # dry run of the verdict
evo seal   apps/pomodoro --by "agent"   # judge, write the generation
evo verify apps/pomodoro                # check the whole ledger
```

Driver operations: `tree` `state` `journal` (read) and `tap` `type` `clock` `respond` (write, always recorded).

## Next to Exact

[Exact](https://github.com/expo/exact) is faster, draws real native UI on several platforms and lets you write logic in TypeScript. If that is what you need, use Exact. rustevo does not compete on speed or reach.

rustevo is about one question Exact's documentation does not address today: across many agent-made changes, proving which behaviour changed and which did not, and keeping that proof where anyone can re-check it.

## What it does not do yet

- No rendering and no screenshots. The UI is a logical tree.
- `evo verify` proves the stored plan and logic behave as recorded. It does not yet rebuild the logic from source to prove the `.wasm` came from it.
- The ledger is a hash chain, not a signed log. It catches edits; someone who rewrites every later generation consistently is only caught if an earlier hash was published elsewhere (git history does this).
- The carry compares nodes by id. A reordering of siblings is not reported.
- Types in expressions are checked when they run, not when they compile.
- Out of scope before 1.0: Android, Windows, device APIs, a visual editor, a JavaScript logic layer, compatibility promises.

Design notes (Japanese): [docs/DESIGN.md](docs/DESIGN.md).

## License

MIT
