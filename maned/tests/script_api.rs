//! The safe surface of §4.1–§4.4 against the real library: flags, the routed
//! refusal, the decimal channel, the byte body, and the refusal messages that
//! are the API contract.

use maned::Error;
use std::path::{Path, PathBuf};

fn corpus(name: &str) -> PathBuf {
    let dir = std::env::var("MANED_CONFORMANCE_DIR")
        .unwrap_or_else(|_| "/Users/fabianodicheti/Documents/maned/capi-freeze/conformance".into());
    PathBuf::from(dir).join(name)
}

/// A throwaway directory with one script in it. No `tempfile` dependency: the
/// crate's selling point is that it has almost none, and this is six lines.
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!(
            "maned-rs-test-{tag}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn script(&self, name: &str, source: &str) -> PathBuf {
        let p = self.dir.join(name);
        std::fs::write(&p, source).unwrap();
        p
    }

    fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A script that routes a flow to a device that is not there. 127.0.0.1:1 is
/// chosen because nothing listens on port 1 — but nothing connects here
/// either: `mnd_open` plans, it does not dial, and `into_shared` refuses on
/// the plan alone.
const ROUTED: &str = r#"mnd::quantmax=1000;
mnd::quantmin=-1000;
mnd::quantres=0;

device::rex { host = "127.0.0.1"; port = 1; user = "u"; pass = env("MANED_RS_TEST_PASS"); }

calc::lambda_flow f(x) @device(rex) {
    x x add r =
} return r;
"#;

#[test]
fn into_shared_refuses_a_routed_script_and_hands_it_back() {
    std::env::set_var("MANED_RS_TEST_PASS", "unused");
    let scratch = Scratch::new("routed");
    let path = scratch.script("routed.mnd", ROUTED);

    let script = maned::builder(&path)
        .synthesize_outputs(true)
        .open()
        .expect("a routed script still loads — mnd_open plans, it does not dial");
    assert!(script.is_routed());

    let refusal = script
        .into_shared()
        .expect_err("a routed script must not become a SharedScript");

    let message = refusal.to_string();
    assert!(message.contains("routes to a device"), "{message}");
    assert!(message.contains("single-connection"), "{message}");
    assert!(message.contains(&path.display().to_string()), "{message}");

    // The refusal did not destroy the handle: mnd_open is the expensive call,
    // and the caller keeps the script it paid for.
    let recovered = refusal.into_script();
    assert!(recovered.is_routed());
    assert_eq!(recovered.path(), path);

    // ... and it is still usable, single-threaded, which is the whole point.
    // (Handle accounting is asserted in shutdown.rs, which owns its process.)
    drop(recovered);
}

#[test]
fn a_local_script_is_not_routed_and_shares_freely() {
    let script = maned::load(corpus("01_local_matmul.mnd")).unwrap();
    assert!(!script.is_routed());
    let shared = script.into_shared().expect("not routed");
    assert!(!shared.is_routed());
    assert_eq!(
        shared
            .run(&serde_json::json!({"h0": [[3,1],[2,4]]}))
            .unwrap()
            .status(),
        200
    );
    // And back again.
    let mut script = shared.into_script();
    assert_eq!(
        script
            .run(&serde_json::json!({"h0": [[1,0],[0,1]]}))
            .unwrap()
            .status(),
        200
    );
}

// ---------------------------------------------------------------------------
// Flags
// ---------------------------------------------------------------------------

/// `MND_SYNTHESIZE_OUTPUTS` is the difference between this script loading and
/// not loading — which is a sharper test of the flag word than reading it back.
#[test]
fn synthesize_outputs_is_the_difference_between_loading_and_not() {
    let path = corpus("09_synthesized.mnd");

    let refused = maned::load(&path).expect_err("no out::/profile::/frame:: without the flag");
    match &refused {
        Error::Load { path: p, message } => {
            assert_eq!(p, &path);
            assert!(message.contains("declares no out::"), "{message}");
            // Hazard 6's clause, appended where somebody will read it.
            assert!(message.contains("working directory"), "{message}");
        }
        other => panic!("{other:?}"),
    }

    let mut script = maned::builder(&path)
        .synthesize_outputs(true)
        .open()
        .unwrap();
    let r = script
        .run(&serde_json::json!({"x": [[3,-1],[2,4]]}))
        .unwrap();
    assert_eq!(r.body(), br#"{"outputs":{"y":[[3,0],[2,4]]},"notes":[]}"#);
    assert_eq!(r.tensor("y").unwrap().data, vec![3, 0, 2, 4]);
}

#[test]
fn no_vm_fallback_reaches_mnd_open() {
    // Both bits at once, on a script that cares about neither: the assertion
    // is that the flag word is accepted and forwarded, not that it changes
    // this script's behaviour.
    let mut script = maned::builder(corpus("09_synthesized.mnd"))
        .synthesize_outputs(true)
        .no_vm_fallback(true)
        .open()
        .unwrap();
    assert_eq!(
        script
            .run(&serde_json::json!({"x": [[1,1],[1,1]]}))
            .unwrap()
            .status(),
        200
    );
}

// ---------------------------------------------------------------------------
// Response
// ---------------------------------------------------------------------------

#[test]
fn tensor_reads_the_run_json_outputs_object() {
    let mut script = maned::load(corpus("04_multi_output.mnd")).unwrap();
    let r = script
        .run(&serde_json::json!({"h0": [[3,1],[2,4]]}))
        .unwrap();

    let a = r.tensor("a").unwrap();
    assert_eq!(a.shape, vec![2, 2]);
    assert_eq!(a.data, vec![7, 9, 2, 4]);
    let t = r.tensor("t").unwrap();
    assert_eq!(t.data, vec![7, 2, 9, 4]);

    let missing = r.tensor("nope").unwrap_err().to_string();
    assert!(missing.contains("outputs are: a, t"), "{missing}");

    // json() and deserialize() are the same bytes, twice.
    #[derive(serde::Deserialize)]
    struct Run {
        outputs: std::collections::BTreeMap<String, Vec<Vec<i64>>>,
    }
    let typed: Run = r.deserialize().unwrap();
    assert_eq!(typed.outputs["a"], vec![vec![7, 9], vec![2, 4]]);
    assert_eq!(r.json().unwrap()["outputs"]["a"][0][1], 9);
    assert_eq!(r.notes(), Vec::<String>::new().as_slice());
}

/// At `quantres > 0` the engine prints `%.*f` and `tensor()` refuses, because
/// rounding those digits into an `f64` is exactly what this crate promises not
/// to do. `decimal()` hands back the printed text, trailing zeros and all.
#[test]
fn decimal_returns_the_exact_printed_text() {
    let scratch = Scratch::new("decimal");
    let path = scratch.script(
        "dec.mnd",
        "mnd::quantmax=30000;\nmnd::quantmin=-30000;\nmnd::quantres=2;\n\n\
         calc::lambda_flow f(x) {\n    x x add y =\n} return y;\n",
    );
    let mut script = maned::builder(&path)
        .synthesize_outputs(true)
        .open()
        .unwrap();
    let r = script
        .request(
            "POST",
            "/run.json",
            None,
            br#"{"x": [[1.5, 2.25],[0.5, 1.0]]}"#,
        )
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.body_str().unwrap(),
        r#"{"outputs":{"y":[[3.00,4.50],[1.00,2.00]]},"notes":[]}"#
    );

    // Verbatim: "3.00", not 3.0, and "4.50", not 4.5.
    assert_eq!(r.decimal("y").unwrap(), "[[3.00,4.50],[1.00,2.00]]");

    let refused = r.tensor("y").unwrap_err().to_string();
    assert!(refused.contains("not an integer"), "{refused}");
    assert!(refused.contains("decimal(\"y\")"), "{refused}");

    assert!(r
        .decimal("zz")
        .unwrap_err()
        .to_string()
        .contains("no output named 'zz'"));
}

/// The body is `body_len` bytes, borrowed, and nothing in the crate ever calls
/// `CStr::from_ptr` on it — a multipart response is the shape that would
/// expose a `strlen`, since its parts are concatenated payloads.
#[test]
fn body_is_borrowed_bytes_of_body_len() {
    let mut script = maned::load(corpus("04_multi_output.mnd")).unwrap();
    let boundary = "----maned-rs-test";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"h0\"\r\n\r\n3,1\r\n2,4\r\n--{boundary}--\r\n"
    );
    let r = script
        .request(
            "POST",
            "/run",
            Some(&format!("multipart/form-data; boundary={boundary}")),
            body.as_bytes(),
        )
        .unwrap();

    assert_eq!(r.status(), 200);
    assert!(
        r.content_type().starts_with("multipart/"),
        "{}",
        r.content_type()
    );

    let bytes = r.body();
    // Both outputs survived: a strlen-shaped read would have stopped at the
    // first NUL, and a CStr-shaped one would have copied to the same place.
    let text = String::from_utf8_lossy(bytes);
    assert!(text.contains("7,9"), "{text}");
    assert!(
        text.contains("name=\"t\"") || text.contains("name=t"),
        "{text}"
    );
    assert!(bytes.len() > 100, "{} bytes", bytes.len());
    // The borrow is of the C allocation, so this is the same pointer twice.
    assert_eq!(r.body().as_ptr(), bytes.as_ptr());
}

// ---------------------------------------------------------------------------
// find() and load()
// ---------------------------------------------------------------------------

#[test]
fn find_searches_manedpath_then_search_path_then_cwd_then_mnd() {
    let scratch = Scratch::new("find");
    scratch.script(
        "findme.mnd",
        "mnd::quantmax=1000;\nmnd::quantmin=-1000;\nmnd::quantres=0;\n\n\
         calc::lambda_flow f(x) {\n    x x add y =\n} return y;\n",
    );

    // Not on any path yet.
    let miss = maned::find("findme").unwrap_err().to_string();
    assert!(miss.contains("no script named 'findme'"), "{miss}");
    assert!(miss.contains("search order"), "{miss}");

    // search_path() is the process-global middle branch.
    maned::add_search_path(scratch.path());
    assert!(maned::search_path().contains(&scratch.path().to_path_buf()));
    let found = maned::builder(scratch.path().join("findme.mnd"))
        .synthesize_outputs(true)
        .open();
    assert!(found.is_ok());
    let s = maned::find("findme");
    // find() uses load(), i.e. no flags, and this script has no out:: — so it
    // was FOUND (the error is a load error naming the file, not a miss).
    match s {
        Err(Error::Load { path, message }) => {
            assert_eq!(path, scratch.path().join("findme.mnd"));
            assert!(message.contains("declares no out::"), "{message}");
        }
        other => panic!("{:?}", other.map(|_| ())),
    }
    maned::set_search_path(Vec::<PathBuf>::new());
    assert!(maned::search_path().is_empty());
}

#[test]
fn load_of_a_missing_file_names_the_path() {
    let e = maned::load("/definitely/not/here.mnd").unwrap_err();
    match e {
        Error::Load { path, message } => {
            assert_eq!(path, PathBuf::from("/definitely/not/here.mnd"));
            assert!(!message.is_empty());
        }
        other => panic!("{other:?}"),
    }
}

// ---------------------------------------------------------------------------
// The process-global knobs
// ---------------------------------------------------------------------------

#[test]
fn kernel_threads_and_version_are_process_wide_reads_and_writes() {
    // Timing, never values (`src/interp/parallel.cpp:16`).
    maned::set_kernel_threads(2);
    let mut script = maned::load(corpus("01_local_matmul.mnd")).unwrap();
    let a = script
        .run(&serde_json::json!({"h0": [[3,1],[2,4]]}))
        .unwrap();
    maned::set_kernel_threads(1);
    let b = script
        .run(&serde_json::json!({"h0": [[3,1],[2,4]]}))
        .unwrap();
    assert_eq!(a.body(), b.body());

    assert!(maned::abi_version() >= 1);
    assert!(!maned::version().is_empty());
}
