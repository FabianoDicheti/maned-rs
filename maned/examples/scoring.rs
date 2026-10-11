//! The quickstart example:
//!
//!     MANED_LIB_DIR=... MANED_INCLUDE_DIR=... cargo run --example scoring
//!
//! Expected output:
//!     y = shape [2, 2], data [2, 4, 0, 2]
//!     engine 0.4.0

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/scoring.mnd");
    let mut script = maned::load(path)?;
    let result = script.run(&serde_json::json!({"x": [[2, 0], [0, 2]]}))?;
    let y = result.tensor("y")?;
    println!("y = shape {:?}, data {:?}", y.shape, y.data);
    println!("engine {}", maned::version());
    Ok(())
}
