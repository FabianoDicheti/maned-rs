//! File routes: send file BYTES in, get output file bytes back — the
//! in-process twin of `curl -F data=@data.csv http://.../run`.
//!
//!     cargo run --example files
//!
//! Expected output:
//!     outputs: ["g"]
//!     8.75,3.50
//!     3.50,5.00

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/examples");
    // The script's own csv("data.csv") resolves against the process CWD.
    std::env::set_current_dir(dir)?;
    let mut script = maned::load(format!("{dir}/pipeline.mnd"))?;
    let csv_bytes = std::fs::read(format!("{dir}/data.csv"))?;

    let out = script.run_files(&[("data", csv_bytes)])?;
    println!("outputs: {:?}", out.keys().collect::<Vec<_>>());
    print!("{}", String::from_utf8_lossy(&out["g"]));
    Ok(())
}
