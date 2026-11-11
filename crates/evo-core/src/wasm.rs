//! The app's Rust logic, compiled to WebAssembly. Run this way it has no
//! clock, no network and no files: the only thing it can do is take the state
//! and return changes and requests. Each call starts from a fresh instance, so
//! nothing is carried from one call to the next.
use crate::plan::sha;
use crate::runtime::{Logic, Outcome, R};
use serde_json::{json, Value};
use wasmi::{Config, Engine, Linker, Module, Store};

/// Work allowed per call; an action that loops forever stops here.
const FUEL: u64 = 200_000_000;

pub struct WasmLogic {
    engine: Engine,
    module: Module,
    hash: String,
}

impl WasmLogic {
    pub fn new(bytes: &[u8]) -> R<WasmLogic> {
        let mut cfg = Config::default();
        cfg.consume_fuel(true);
        let engine = Engine::new(&cfg);
        let module = Module::new(&engine, bytes).map_err(|e| format!("logic.wasm を読めない: {e}"))?;
        if let Some(i) = module.imports().next() {
            return Err(format!("logic.wasm が外の機能を求めている（{}.{}）。ロジックは時計・通信・ファイルを直接使えない", i.module(), i.name()));
        }
        Ok(WasmLogic { engine, module, hash: sha(bytes) })
    }
}

impl Logic for WasmLogic {
    fn call(&mut self, action: &str, state: &Value, args: &[Value]) -> R<Outcome> {
        let e = |x: wasmi::Error| format!("ロジックが止まった: {x}");
        let mut store = Store::new(&self.engine, ());
        store.set_fuel(FUEL).map_err(e)?;
        let inst = Linker::<()>::new(&self.engine).instantiate_and_start(&mut store, &self.module).map_err(e)?;
        let mem = inst.get_memory(&store, "memory").ok_or("logic.wasm に memory がない")?;
        let alloc = inst.get_typed_func::<u32, u32>(&store, "evo_alloc").map_err(e)?;
        let call = inst.get_typed_func::<(u32, u32), u64>(&store, "evo_call").map_err(e)?;
        let input = json!({"action": action, "state": state, "args": args}).to_string().into_bytes();
        let ptr = alloc.call(&mut store, input.len() as u32).map_err(e)?;
        mem.write(&mut store, ptr as usize, &input).map_err(|x| format!("{x}"))?;
        let packed = call.call(&mut store, (ptr, input.len() as u32)).map_err(e)?;
        let mut buf = vec![0u8; (packed & 0xffff_ffff) as usize];
        mem.read(&store, (packed >> 32) as usize, &mut buf).map_err(|x| format!("{x}"))?;
        let v: Value = serde_json::from_slice(&buf).map_err(|x| format!("ロジックの返事を読めない: {x}"))?;
        if let Some(err) = v.get("error").and_then(|x| x.as_str()) {
            return Err(err.into());
        }
        Ok(Outcome {
            set: v["set"].as_object().cloned().unwrap_or_default().into_iter().collect(),
            effects: v["effects"].as_array().cloned().unwrap_or_default(),
        })
    }
    fn hash(&self) -> String {
        self.hash.clone()
    }
}
