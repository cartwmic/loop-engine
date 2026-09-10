fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = loop_cli::monitor::cli(&args) {
        std::process::exit(code);
    }
    #[cfg(unix)]
    if let Some(code) = loop_cli::capture::cli(&args) {
        std::process::exit(code);
    }
    let execution = loop_cli::execute(args);
    print!("{}", execution.stdout);
    eprint!("{}", execution.stderr);
    std::process::exit(execution.exit_code);
}
