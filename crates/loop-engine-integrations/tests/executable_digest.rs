use loop_engine_core::capabilities::digest::DigestComputer;
use loop_engine_core::capabilities::provider_catalog::{ProviderConfig, ResolvedProviderConfig};
use loop_engine_core::model::ids::{ProviderHandle, RegistrationId};
use loop_engine_core::model::provider::DigestObservation;
use loop_engine_integrations::sha256_digest::{Sha256DigestComputer, sha256_label};

fn config(path: &std::path::Path) -> ResolvedProviderConfig {
    ResolvedProviderConfig::new(
        RegistrationId::parse("provider-1").unwrap(),
        ProviderHandle::parse("provider").unwrap(),
        1,
        ProviderConfig::new(
            path.to_str().unwrap(),
            Vec::new(),
            path.parent().unwrap().to_str().unwrap(),
            5,
        )
        .unwrap(),
    )
    .unwrap()
}

fn observed(value: DigestObservation) -> String {
    match value {
        DigestObservation::Observed(value) => value.as_str().to_owned(),
        DigestObservation::Unavailable => panic!("digest unexpectedly unavailable"),
    }
}

#[test]
fn readable_and_replaced_executable_bytes_change_observation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("provider");
    std::fs::write(&path, b"first").unwrap();
    let provider = config(&path);
    let first = observed(Sha256DigestComputer.executable_digest(&provider).unwrap());
    assert_eq!(first, sha256_label(b"first"));
    std::fs::write(&path, b"second").unwrap();
    let second = observed(Sha256DigestComputer.executable_digest(&provider).unwrap());
    assert_eq!(second, sha256_label(b"second"));
    assert_ne!(first, second);
}

#[test]
fn interpreted_script_hashes_script_and_non_regular_locator_is_explicit() {
    let directory = tempfile::tempdir().unwrap();
    let script = directory.path().join("provider.sh");
    let bytes = b"#!/bin/sh\necho '{}'";
    std::fs::write(&script, bytes).unwrap();
    assert_eq!(
        observed(
            Sha256DigestComputer
                .executable_digest(&config(&script))
                .unwrap()
        ),
        sha256_label(bytes)
    );
    assert!(matches!(
        Sha256DigestComputer
            .executable_digest(&config(directory.path()))
            .unwrap(),
        DigestObservation::Unavailable
    ));

    assert!(matches!(
        Sha256DigestComputer
            .executable_digest(&config(std::path::Path::new("/dev/null")))
            .unwrap(),
        DigestObservation::Unavailable
    ));

    let fifo = directory.path().join("provider.fifo");
    assert!(
        std::process::Command::new("/usr/bin/mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let started = std::time::Instant::now();
    assert!(matches!(
        Sha256DigestComputer
            .executable_digest(&config(&fifo))
            .unwrap(),
        DigestObservation::Unavailable
    ));
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}
