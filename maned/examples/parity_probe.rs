//! Gate 4 probe: print 01_local_matmul's /run.json body, exactly.
use std::io::Write;
fn main() {
    let dir = std::env::var("MANED_CONFORMANCE_DIR")
        .unwrap_or_else(|_| "../capi-freeze/conformance".into());
    let mut s = maned::load(format!("{dir}/01_local_matmul.mnd")).unwrap();
    let r = s
        .request("POST", "/run.json", None, br#"{"h0": [[3,1],[2,4]]}"#)
        .unwrap();
    std::io::stdout().write_all(r.body()).unwrap();
}
