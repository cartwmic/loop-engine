fn main() {
    let response = loop_reference_fixtures::run_provider(
        loop_reference_fixtures::FixtureProvider::ModelBinding,
    );
    std::process::exit(response);
}
