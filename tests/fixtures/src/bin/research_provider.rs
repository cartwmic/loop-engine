fn main() {
    let response =
        loop_reference_fixtures::run_provider(loop_reference_fixtures::FixtureProvider::Research);
    std::process::exit(response);
}
