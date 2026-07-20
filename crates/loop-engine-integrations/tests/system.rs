use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use loop_engine_core::capabilities::digest::DigestComputer;
use loop_engine_core::capabilities::id_generator::IdGenerator;
use loop_engine_core::capabilities::time::TimeSource;
use loop_engine_integrations::sha256_digest::{Sha256DigestComputer, sha256_label};
use loop_engine_integrations::system_clock::SystemTimeSource;
use loop_engine_integrations::uuid_ids::UuidV7Generator;

#[test]
fn utc_clock_emits_core_accepted_timestamp() {
    let timestamp = SystemTimeSource.now().unwrap();
    let timestamp = timestamp.as_timestamp().to_string();
    assert!(timestamp.ends_with('Z'));
    assert!(timestamp.contains('T'));
}

#[test]
fn uuid_v7_ids_are_unique_under_concurrency() {
    let ids = Arc::new(Mutex::new(BTreeSet::new()));
    let threads = (0..8)
        .map(|_| {
            let ids = Arc::clone(&ids);
            std::thread::spawn(move || {
                for _ in 0..1_000 {
                    ids.lock()
                        .unwrap()
                        .insert(UuidV7Generator.request_id().unwrap().to_string());
                }
            })
        })
        .collect::<Vec<_>>();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(ids.lock().unwrap().len(), 8_000);
}

#[test]
fn sha256_known_vector_and_graph_type_stay_distinct() {
    assert_eq!(
        sha256_label(b"abc"),
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let _computer: &dyn DigestComputer<
        Error = loop_engine_integrations::sha256_digest::DigestError,
    > = &Sha256DigestComputer;
}
