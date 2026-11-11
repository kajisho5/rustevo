//! What an app's logic crate uses. An action is a plain function:
//!
//! ```ignore
//! fn tick(s: &State, _args: &[Value]) -> Outcome {
//!     Outcome::new().set("remaining", s.remaining - 1)
//! }
//! evo_logic::actions!(State; tick);
//! ```
//!
//! It reads the state and returns changes and requests as data. It cannot
//! read a clock, the network or a file: built for WebAssembly, those do not
//! exist.
pub use serde_json::{json, Value};

#[derive(Default, serde::Serialize)]
pub struct Outcome {
    pub set: serde_json::Map<String, Value>,
    pub effects: Vec<Value>,
}

impl Outcome {
    pub fn new() -> Self {
        Self::default()
    }
    /// Change one state field.
    pub fn set(mut self, field: &str, v: impl serde::Serialize) -> Self {
        self.set.insert(field.into(), serde_json::to_value(v).expect("value"));
        self
    }
    /// Ask the outside for something. The runtime does it and records the answer.
    pub fn effect(mut self, e: Value) -> Self {
        self.effects.push(e);
        self
    }
}

/// Save a value under a key.
pub fn store_put(key: &str, value: impl serde::Serialize) -> Value {
    json!({"kind": "store.put", "key": key, "value": serde_json::to_value(value).expect("value")})
}

/// Read a saved value; `then` is the action that receives it (null if absent).
pub fn store_get(key: &str, then: &str) -> Value {
    json!({"kind": "store.get", "key": key, "then": then})
}

/// A random integer in 0..max; `then` is the action that receives it.
pub fn random(max: i64, then: &str) -> Value {
    json!({"kind": "random", "key": max.to_string(), "max": max, "then": then})
}

#[doc(hidden)]
pub fn run(input: &[u8], f: impl FnOnce(&str, Value, &[Value]) -> Result<Outcome, String>) -> u64 {
    let out = (|| {
        let v: Value = serde_json::from_slice(input).map_err(|e| e.to_string())?;
        let args = v["args"].as_array().cloned().unwrap_or_default();
        f(v["action"].as_str().unwrap_or(""), v["state"].clone(), &args)
    })();
    let bytes = match out {
        Ok(o) => serde_json::to_vec(&o).expect("outcome"),
        Err(e) => serde_json::to_vec(&json!({ "error": e })).expect("error"),
    };
    let b = Box::leak(bytes.into_boxed_slice());
    ((b.as_ptr() as u64) << 32) | b.len() as u64
}

/// Export the listed actions. `$state` is the generated `State` type.
#[macro_export]
macro_rules! actions {
    ($state:ty; $($name:ident),* $(,)?) => {
        #[no_mangle]
        pub extern "C" fn evo_alloc(len: u32) -> u32 {
            let mut v = Vec::<u8>::with_capacity(len as usize);
            let p = v.as_mut_ptr();
            core::mem::forget(v);
            p as u32
        }
        #[no_mangle]
        pub extern "C" fn evo_call(ptr: u32, len: u32) -> u64 {
            let input = unsafe { Vec::from_raw_parts(ptr as *mut u8, len as usize, len as usize) };
            $crate::run(&input, |action, state, args| {
                let s: $state = $crate::__from_value(state)?;
                match action {
                    $(stringify!($name) => Ok($name(&s, args)),)*
                    other => Err(format!("action {other} はロジックにない")),
                }
            })
        }
    };
}

#[doc(hidden)]
pub fn __from_value<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| format!("state を読めない: {e}"))
}
