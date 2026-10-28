//! Load the built collect_core plugin and call it through the real C ABI.
//!
//! Why this exists: `cargo test` exercises the logic crate, and `nix build`
//! proves the plugin links — but neither shows a method actually crossing the
//! ABI the generated scaffold publishes. Basecamp would, except that loading
//! is on demand and needs a click.
//!
//! No auth token is forged. `logos_module_accept_token` authenticates a
//! module's *outbound* calls; inbound `logos_module_dispatch` is not
//! token-gated, so a local dispatch is legitimate. Nothing here can reach
//! Storage or Delivery, which is exactly why it is safe — and why it cannot
//! replace a real load.
//!
//! Usage: abi-check <path-to-collect_core_plugin.dylib>

use libloading::{Library, Symbol};
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};

type Install = unsafe extern "Rust" fn();
type Dispatch = unsafe extern "C" fn(*const c_char, *const c_char) -> *mut c_char;
type GetMethods = unsafe extern "C" fn() -> *mut c_char;
type ProtocolVersion = unsafe extern "C" fn() -> *const c_char;
type SetContext = unsafe extern "C" fn(*const c_char, *const c_char, *const c_char) -> c_int;
type StringFree = unsafe extern "C" fn(*mut c_char);

struct Module {
    lib: Library,
}

impl Module {
    unsafe fn open(path: &str) -> Result<Self, String> {
        let lib = Library::new(path).map_err(|e| format!("dlopen failed: {e}"))?;
        Ok(Module { lib })
    }

    unsafe fn sym<T>(&self, name: &[u8]) -> Result<Symbol<'_, T>, String> {
        self.lib
            .get::<T>(name)
            .map_err(|e| format!("missing symbol {}: {e}", String::from_utf8_lossy(name)))
    }

    /// Call a method and take ownership of the returned string, freeing it
    /// through the module's own allocator rather than ours.
    unsafe fn call(&self, method: &str, args_json: &str) -> Result<String, String> {
        let dispatch: Symbol<Dispatch> = self.sym(b"logos_module_dispatch")?;
        let free: Symbol<StringFree> = self.sym(b"logos_module_string_free")?;

        let m = CString::new(method).map_err(|e| e.to_string())?;
        let a = CString::new(args_json).map_err(|e| e.to_string())?;
        let raw = dispatch(m.as_ptr(), a.as_ptr());
        if raw.is_null() {
            return Err(format!("{method} returned null"));
        }
        let out = CStr::from_ptr(raw).to_string_lossy().into_owned();
        free(raw);
        Ok(out)
    }
}

fn main() {
    let path = match std::env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("usage: abi-check <path-to-collect_core_plugin.dylib>");
            std::process::exit(2);
        }
    };

    let mut failures = 0usize;
    let mut check = |name: &str, outcome: Result<String, String>, want: &dyn Fn(&str) -> bool| {
        match outcome {
            Ok(text) if want(&text) => println!("ok   {name}\n       {text}"),
            Ok(text) => {
                failures += 1;
                println!("FAIL {name}\n       {text}");
            }
            Err(e) => {
                failures += 1;
                println!("FAIL {name}\n       {e}");
            }
        }
    };

    unsafe {
        let module = match Module::open(&path) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("could not load the plugin: {e}");
                std::process::exit(1);
            }
        };
        println!("loaded {path}\n");

        // The handshake, in the order a host performs it.
        match module.sym::<ProtocolVersion>(b"logos_module_get_protocol_version") {
            Ok(f) => println!("ok   protocol version\n       {}", CStr::from_ptr(f()).to_string_lossy()),
            Err(e) => { failures += 1; println!("FAIL protocol version\n       {e}"); }
        }

        match module.sym::<Install>(b"logos_module_install") {
            Ok(f) => { f(); println!("ok   logos_module_install"); }
            Err(e) => { failures += 1; println!("FAIL logos_module_install\n       {e}"); }
        }

        // Stamp a context, which is what makes on_context_ready fire and
        // gives the module its persistence path (SPEC Q8).
        match module.sym::<SetContext>(b"logos_module_set_context") {
            Ok(f) => {
                let p = CString::new("/tmp/abi-check/collect_core").unwrap();
                let id = CString::new("abi-check-0001").unwrap();
                let persist = CString::new("/tmp/abi-check/persist").unwrap();
                let rc = f(p.as_ptr(), id.as_ptr(), persist.as_ptr());
                println!("ok   logos_module_set_context (rc={rc})");
            }
            Err(e) => { failures += 1; println!("FAIL logos_module_set_context\n       {e}"); }
        }

        // The published method table the host reads.
        match module.sym::<GetMethods>(b"logos_module_get_methods") {
            Ok(f) => {
                let raw = f();
                let text = CStr::from_ptr(raw).to_string_lossy().into_owned();
                if let Ok(free) = module.sym::<StringFree>(b"logos_module_string_free") {
                    free(raw);
                }
                let parsed: Result<serde_json::Value, _> = serde_json::from_str(&text);
                match parsed {
                    Ok(serde_json::Value::Array(items)) => {
                        let names: Vec<String> = items
                            .iter()
                            .filter_map(|i| i.get("name").and_then(|n| n.as_str()).map(str::to_string))
                            .collect();
                        println!("ok   method table: {} entries", names.len());
                        println!("       {}", names.join(", "));
                    }
                    _ => { failures += 1; println!("FAIL method table is not a JSON array\n       {text}"); }
                }
            }
            Err(e) => { failures += 1; println!("FAIL logos_module_get_methods\n       {e}"); }
        }

        println!();

        // Now the real thing: methods crossing the ABI.
        check(
            "session_capabilities",
            module.call("session_capabilities", "[]"),
            &|t| t.contains("\"ok\":true") && t.contains("\"storage\":false"),
        );

        check(
            "geo_round -> cell centre",
            module.call("geo_round", r#"["{\"lat\":12.97712,\"lon\":77.59581}"]"#),
            &|t| t.contains("12.975") && t.contains("77.595"),
        );

        check(
            "experiments_parse_link",
            module.call(
                "experiments_parse_link",
                r#"["{\"link\":\"logos-collect://exp/q7k2m3x4w3b5n6tz?m=logos:zb2rhManifestV4\"}"]"#,
            ),
            &|t| t.contains("/collect/1/exp-q7k2m3x4w3b5n6tz/json"),
        );

        let submission = include_str!("../../../examples/submission.json");
        let request = serde_json::json!({ "document": serde_json::from_str::<serde_json::Value>(submission).unwrap() });
        check(
            "documents_validate (embedded contract)",
            module.call("documents_validate", &serde_json::Value::Array(vec![request.to_string().into()]).to_string()),
            &|t| t.contains("\"valid\":true"),
        );

        check(
            "unknown method is reported, not ignored",
            module.call("call", r#"["experiments.teleport","{}"]"#),
            &|t| t.contains("unknown_method"),
        );
    }

    println!();
    if failures == 0 {
        println!("ALL PASSED — methods crossed the real C ABI");
    } else {
        println!("{failures} FAILURE(S)");
        std::process::exit(1);
    }
}
