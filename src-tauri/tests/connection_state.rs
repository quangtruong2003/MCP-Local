use local_workspace_mcp_manager::connection::{ConnectionSnapshot, ConnectionState};

#[test]
fn connection_snapshot_starts_stopped_with_local_mcp_url() {
    let snapshot = ConnectionSnapshot::new(43123);
    assert_eq!(snapshot.state, ConnectionState::Stopped);
    assert_eq!(snapshot.local_mcp_url, "http://127.0.0.1:43123/mcp");
    assert_eq!(snapshot.tunnel_id, None);
    assert_eq!(snapshot.health_url, None);
}

#[test]
fn connection_state_serializes_as_frontend_friendly_snake_case() {
    assert_eq!(
        serde_json::to_string(&ConnectionState::CheckingHealth).unwrap(),
        "\"checking_health\""
    );
    assert_eq!(
        serde_json::to_string(&ConnectionState::NeedsSetup).unwrap(),
        "\"needs_setup\""
    );
}
