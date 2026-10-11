//! Exact decimals + async: a non-routed script promotes to SharedScript,
//! and the `tokio` feature gives it `run_async` on the blocking pool.
//!
//!     cargo run --example decimals_tokio --features tokio
//!
//! `quantres=2` makes the integer 1999 mean 19.99; `result.decimal(name)`
//! hands back the decoded decimal as a STRING — never a float.
//!
//! Expected output:
//!     total = [55.48]  mean = [18.49]
//!     4 concurrent runs, byte-identical: true

use std::sync::Arc;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/decimal.mnd");
    std::env::set_current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/examples"))?;
    let shared = Arc::new(maned::load(path)?.into_shared().expect("non-routed"));

    let result = Arc::clone(&shared).run_async(serde_json::json!({})).await?;
    println!(
        "total = {}  mean = {}",
        result.decimal("total")?,
        result.decimal("mean")?
    );

    let reference = result.body().to_vec();
    let mut tasks = Vec::new();
    for _ in 0..4 {
        let s = Arc::clone(&shared);
        tasks.push(tokio::spawn(s.run_async(serde_json::json!({}))));
    }
    let mut identical = true;
    for t in tasks {
        identical &= t.await??.body() == reference.as_slice();
    }
    println!("4 concurrent runs, byte-identical: {identical}");
    Ok(())
}
