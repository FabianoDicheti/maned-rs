//! Routing from inside your process: the flow carries @device(vm1) with
//! host = "local", so this run spawns the local Bark VM — the worker's
//! REAL compute core — exactly as `maned-run` would.
//!
//! Requires the maned toolchain installed (it provides `bark-vm` on PATH):
//!     curl -fsSL https://fabianodicheti.github.io/maned/install.sh | sh
//!
//!     cargo run --example routed
//!
//! Expected output:
//!     r = shape [2, 2], data [19, 22, 43, 50]

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/routed.mnd");
    std::env::set_current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/examples"))?;
    let mut script = maned::load(path)?;
    let result = script.run(&serde_json::json!({}))?;
    let r = result.tensor("r")?;
    println!("r = shape {:?}, data {:?}", r.shape, r.data);
    Ok(())
}
