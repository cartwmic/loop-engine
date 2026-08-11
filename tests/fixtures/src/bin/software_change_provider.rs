fn main() {
    let response = loop_reference_fixtures::run_provider(
        loop_reference_fixtures::FixtureProvider::SoftwareChange,
    );
    std::process::exit(response);
}
