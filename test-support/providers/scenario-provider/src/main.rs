use std::io::{self, Write};

use scenario_provider::{config, handler};

fn main() {
    if let Err(error) = real_main() {
        let _ = writeln!(io::stderr(), "{error}");
        std::process::exit(1);
    }
}

fn real_main() -> Result<(), Box<dyn std::error::Error>> {
    let config = config::parse_config()?;
    handler::run(&config)?;
    Ok(())
}
