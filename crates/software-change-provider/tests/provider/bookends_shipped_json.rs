use std::fs;

const PROFILES: &[&str] = &["minimal", "standard", "high-rigor"];

#[test]
fn shipped_profile_bytes_have_no_bookends_keys() {
    for profile in PROFILES {
        let path = workspace_integration::package_root("software-change-provider")
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
