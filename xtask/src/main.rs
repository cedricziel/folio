use anyhow::{bail, Result};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("tokens") => {
            let check = args.next().as_deref() == Some("--check");
            xtask::run(&xtask::repo_root(), check)
        }
        _ => bail!("usage: cargo run -p xtask -- tokens [--check]"),
    }
}
