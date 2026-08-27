use std::fs;
use std::path::Path;

const PROFILES: &[&str] = &["minimal", "standard", "high-rigor"];

#[test]
fn shipped_profile_bytes_have_no_bookends_keys() {
    for profile in PROFILES {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("data")
            .join("configs")
            .join(format!("{profile}.json"));
        let text =
            fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {path:?}: {error}"));
        assert!(
            !text.contains("bookends"),
            "{profile} shipped JSON must not contain bookends keys"
        );
        assert!(
            !text.contains("requirement_ids"),
            "{profile} shipped JSON must not contain requirement_ids"
        );
        assert!(
            !text.contains("ids-grounded"),
            "{profile} shipped JSON must not contain ids-grounded"
        );
        assert!(
            !text.contains("bypass-not-green"),
            "{profile} shipped JSON must not contain bypass-not-green"
        );
    }
}
