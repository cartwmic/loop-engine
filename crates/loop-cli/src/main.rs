use std::io::{ErrorKind, Write};

/// Write all of `text`. A reader that closes the pipe early (`| head`) is a
/// normal end of output, not a failure.
fn write_all(mut stream: impl Write, text: &str) {
    if let Err(error) = stream
        .write_all(text.as_bytes())
        .and_then(|()| stream.flush())
    {
        if error.kind() != ErrorKind::BrokenPipe {
            std::process::exit(74);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = loop_cli::monitor::cli(&args) {
        std::process::exit(code);
    }
    let execution = loop_cli::execute(args);
    write_all(std::io::stdout().lock(), &execution.stdout);
    write_all(std::io::stderr().lock(), &execution.stderr);
    std::process::exit(execution.exit_code);
}
