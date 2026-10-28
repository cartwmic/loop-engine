use std::io::Read;

fn main() {
    // `commission < full-show.json` prints the eligibility the checked event applies
    // and the owner approval view; with no arguments it serves provider operations.
    if std::env::args().nth(1).as_deref() == Some("commission") {
        let mut input = String::new();
        let result = std::io::stdin()
            .read_to_string(&mut input)
            .map_err(|error| error.to_string())
            .and_then(|_| serde_json::from_str(&input).map_err(|error| error.to_string()))
            .and_then(|shown| {
                loop_reference_fixtures::model_binding_commission(&shown)
                    .map_err(|error| error.to_string())
            });
        match result {
            Ok(value) => {
                println!("{value}");
                std::process::exit(0)
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1)
            }
        }
    }
    let response = loop_reference_fixtures::run_provider(
        loop_reference_fixtures::FixtureProvider::ModelBinding,
    );
    std::process::exit(response);
}
