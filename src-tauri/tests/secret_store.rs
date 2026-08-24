use local_workspace_mcp_manager::secret_store::{SecretStore, SecretSummary};

#[test]
fn summary_exposes_presence_only() {
    let dir = tempfile::tempdir().unwrap();
    let store = SecretStore::for_test_file(dir.path().join("secure-store.test"));
    store
        .save(Some("runtime-secret-value"), Some("admin-secret-value"))
        .unwrap();
    assert_eq!(
        store.summary().unwrap(),
        SecretSummary {
            has_runtime_key: true,
            has_admin_key: true,
        }
    );
    let serialized = serde_json::to_string(&store.summary().unwrap()).unwrap();
    assert!(!serialized.contains("runtime-secret-value"));
    assert!(!serialized.contains("admin-secret-value"));
}

#[test]
fn replacing_only_runtime_key_preserves_admin_key() {
    let dir = tempfile::tempdir().unwrap();
    let store = SecretStore::for_test_file(dir.path().join("secure-store.test"));
    store.save(Some("runtime-1"), Some("admin-1")).unwrap();
    store.save(Some("runtime-2"), None).unwrap();
    assert_eq!(store.load_runtime_key().unwrap().as_deref(), Some("runtime-2"));
    assert_eq!(store.load_admin_key().unwrap().as_deref(), Some("admin-1"));
}

#[test]
fn blank_replacements_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let store = SecretStore::for_test_file(dir.path().join("secure-store.test"));
    store.save(Some("runtime-1"), Some("admin-1")).unwrap();
    store.save(Some("  "), Some("")).unwrap();
    assert_eq!(store.load_runtime_key().unwrap().as_deref(), Some("runtime-1"));
    assert_eq!(store.load_admin_key().unwrap().as_deref(), Some("admin-1"));
}
