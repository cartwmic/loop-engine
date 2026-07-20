fn main() {
    println!(
        "{}",
        serde_json::to_string_pretty(&loop_engine_integrations::trace::trace_event_schema())
            .expect("trace schema serializes")
    );
}
