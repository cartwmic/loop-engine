fn main() {
    let response = loop_reference_fixtures::run_provider(
        loop_reference_fixtures::FixtureProvider::PolicyDocument,
    );
    std::process::exit(response);
}
