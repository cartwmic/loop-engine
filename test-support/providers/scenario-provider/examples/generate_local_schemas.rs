use std::fs;
use std::path::PathBuf;

use scenario_provider::schema;

fn main() {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/schemas");
    fs::create_dir_all(&directory).expect("schema directory creates");
    for entry in schema::all_local_schemas() {
        let bytes = serde_json::to_vec_pretty(&entry.schema).expect("schema serializes");
        fs::write(directory.join(entry.name), bytes).expect("schema writes");
    }
}
