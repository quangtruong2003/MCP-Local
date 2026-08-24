//! Connection state machine types serialized to the frontend as tagged
//! snake_case JSON. `endpoint` is only ever emitted in the `Online` variant.

use serde::{Deserialize, Serialize};

/// Lifecycle state of the local server / tunnel pair. Defaults to `Stopped`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ConnectionState {
    #[default]
    Stopped,
    StartingServer,
    StartingTunnel,
    NeedsLogin {
        login_url: String,
    },
    NeedsFunnelPermission {
        message: String,
    },
    Verifying {
        fqdn: String,
    },
    Online {
        fqdn: String,
        endpoint: String,
    },
    Degraded {
        fqdn: Option<String>,
        message: String,
    },
    Stopping,
    Error {
        code: String,
        message: String,
    },
}

/// Full status snapshot pushed to (and polled by) the frontend. Defaults to
/// `Stopped` / port 0 / not paused.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ConnectionStatus {
    pub state: ConnectionState,
    pub local_port: u16,
    pub paused: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn to_value(state: &ConnectionState) -> Value {
        serde_json::to_value(state).unwrap()
    }

    #[test]
    fn unit_variants_use_snake_case_state_tag() {
        for (state, expected_tag) in [
            (ConnectionState::Stopped, "stopped"),
            (ConnectionState::StartingServer, "starting_server"),
            (ConnectionState::StartingTunnel, "starting_tunnel"),
            (ConnectionState::Stopping, "stopping"),
        ] {
            let v = to_value(&state);
            assert_eq!(v["state"], expected_tag);
            assert_eq!(
                v.as_object().unwrap().len(),
                1,
                "{expected_tag} should carry no payload fields"
            );
        }
    }

    #[test]
    fn needs_login_carries_login_url() {
        let v = to_value(&ConnectionState::NeedsLogin {
            login_url: "https://login.example.com".into(),
        });
        assert_eq!(v["state"], "needs_login");
        assert_eq!(v["login_url"], "https://login.example.com");
    }

    #[test]
    fn online_carries_fqdn_and_endpoint() {
        let v = to_value(&ConnectionState::Online {
            fqdn: "box.tailnet.ts.net".into(),
            endpoint: "https://box.tailnet.ts.net".into(),
        });
        assert_eq!(v["state"], "online");
        assert_eq!(v["fqdn"], "box.tailnet.ts.net");
        assert_eq!(v["endpoint"], "https://box.tailnet.ts.net");
    }

    #[test]
    fn only_online_emits_endpoint_key() {
        assert!(to_value(&ConnectionState::Verifying {
            fqdn: "box.tailnet.ts.net".into()
        })
        .get("endpoint")
        .is_none());

        assert!(to_value(&ConnectionState::Degraded {
            fqdn: Some("box.tailnet.ts.net".into()),
            message: "tunnel flapping".into(),
        })
        .get("endpoint")
        .is_none());
        assert!(to_value(&ConnectionState::Degraded {
            fqdn: None,
            message: "server gone".into(),
        })
        .get("endpoint")
        .is_none());
        assert!(to_value(&ConnectionState::Error {
            code: "funnel_denied".into(),
            message: "permission missing".into(),
        })
        .get("endpoint")
        .is_none());

        // Sanity: Online does have it.
        assert!(to_value(&ConnectionState::Online {
            fqdn: "f".into(),
            endpoint: "e".into(),
        })
        .get("endpoint")
        .is_some());
    }

    #[test]
    fn connection_status_serializes_top_level_shape() {
        let status = json!(ConnectionStatus {
            state: ConnectionState::Online {
                fqdn: "f".into(),
                endpoint: "e".into(),
            },
            local_port: 8787,
            paused: false,
        });
        let keys = status.as_object().unwrap();
        assert_eq!(keys.len(), 3);
        assert!(keys.contains_key("state"));
        assert!(keys.contains_key("local_port"));
        assert!(keys.contains_key("paused"));
        assert_eq!(status["local_port"], 8787);
        assert_eq!(status["paused"], false);
        assert_eq!(status["state"]["state"], "online");

        // Default matches spec.
        let d = ConnectionStatus::default();
        assert_eq!(d.state, ConnectionState::Stopped);
        assert_eq!(d.local_port, 0);
        assert!(!d.paused);

        // Deserialize round-trips (frontend pushes status back).
        let parsed: ConnectionStatus = serde_json::from_value(status).unwrap();
        assert_eq!(
            parsed.state,
            ConnectionState::Online {
                fqdn: "f".into(),
                endpoint: "e".into(),
            }
        );
    }
}
