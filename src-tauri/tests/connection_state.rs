use local_workspace_mcp_manager::connection::{ConnectionState, ConnectionStatus};

#[test]
fn connection_status_defaults_to_stopped() {
    let status = ConnectionStatus::default();
    assert_eq!(status.state, ConnectionState::Stopped);
    assert_eq!(status.local_port, 0);
    assert!(!status.paused);
}

#[test]
fn connection_state_serializes_as_frontend_friendly_snake_case() {
    assert_eq!(
        serde_json::to_string(&ConnectionState::StartingTunnel).unwrap(),
        r#"{"state":"starting_tunnel"}"#
    );
    assert_eq!(
        serde_json::to_string(&ConnectionState::Stopping).unwrap(),
        r#"{"state":"stopping"}"#
    );
}
