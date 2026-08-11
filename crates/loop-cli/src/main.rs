fn main() {
    let execution = loop_cli::execute(std::env::args().skip(1));
    print!("{}", execution.stdout);
    eprint!("{}", execution.stderr);
    std::process::exit(execution.exit_code);
}
