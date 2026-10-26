use anyhow::Result;
use lfmclean::env::Env;

fn main() -> Result<()> {
    let _env = Env::from_system()?;
    println!("lfmclean {}", env!("CARGO_PKG_VERSION"));
    Ok(())
}
