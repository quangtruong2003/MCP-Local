//! Frozen Lightpanda 0.3.7 native MCP tool contract.
//!
//! The bundled snapshot in `resources/lightpanda-mcp-contract-0.3.7.json`
//! was captured from the pinned binary's `tools/list` and is the only
//! authority for native tool names and input schemas. Runtime verification
//! compares discovered tools against this snapshot; `initialize.serverInfo`
//! is never used for that gate.

use std::collections::{BTreeMap, BTreeSet};

use lc_core::error::{CoreError, ErrorCode};
use serde_json::Value;

/// Exact number of native tools advertised by the pinned Lightpanda runtime.
pub const EXPECTED_NATIVE_TOOL_COUNT: usize = 31;

/// Pinned Lightpanda binary version for this contract snapshot.
pub const PINNED_LIGHTPANDA_VERSION: &str = "0.3.7";

/// One-to-one mapping from upstream native tool names to public
/// `browser_lightpanda_*` names. Count and uniqueness are enforced by tests.
pub const NATIVE_TOOL_MAPPINGS: &[(&str, &str)] = &[
    ("goto", "browser_lightpanda_goto"),
    ("search", "browser_lightpanda_search"),
    ("markdown", "browser_lightpanda_markdown"),
    ("html", "browser_lightpanda_html"),
    ("links", "browser_lightpanda_links"),
    ("evaluate", "browser_lightpanda_evaluate"),
    ("extract", "browser_lightpanda_extract"),
    ("tree", "browser_lightpanda_tree"),
    ("nodeDetails", "browser_lightpanda_node_details"),
    ("interactiveElements", "browser_lightpanda_interactive_elements"),
    ("structuredData", "browser_lightpanda_structured_data"),
    ("detectForms", "browser_lightpanda_detect_forms"),
    ("click", "browser_lightpanda_click"),
    ("fill", "browser_lightpanda_fill"),
    ("scroll", "browser_lightpanda_scroll"),
    ("waitForSelector", "browser_lightpanda_wait_for_selector"),
    ("waitForScript", "browser_lightpanda_wait_for_script"),
    ("waitForState", "browser_lightpanda_wait_for_state"),
    ("hover", "browser_lightpanda_hover"),
    ("press", "browser_lightpanda_press"),
    ("selectOption", "browser_lightpanda_select_option"),
    ("setChecked", "browser_lightpanda_set_checked"),
    ("findElement", "browser_lightpanda_find_element"),
    ("consoleLogs", "browser_lightpanda_console_logs"),
    ("getUrl", "browser_lightpanda_get_url"),
    ("getCookies", "browser_lightpanda_get_cookies"),
    ("getEnv", "browser_lightpanda_get_env"),
    ("save", "browser_lightpanda_save"),
    ("session_new", "browser_lightpanda_session_new"),
    ("session_list", "browser_lightpanda_session_list"),
    ("session_close", "browser_lightpanda_session_close"),
];

#[derive(Debug, Clone, serde::Deserialize)]
pub struct LightpandaMcpContract {
    pub lightpanda_version: String,
    pub tool_count: usize,
    pub tools: Vec<NativeToolContract>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct NativeToolContract {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    #[serde(default, rename = "outputSchema")]
    pub output_schema: Option<Value>,
}

/// Parse and validate the bundled contract snapshot.
pub fn pinned_contract() -> Result<LightpandaMcpContract, CoreError> {
    let raw = include_str!("../../resources/lightpanda-mcp-contract-0.3.7.json");
    let contract: LightpandaMcpContract = serde_json::from_str(raw).map_err(|_| {
        CoreError::new(
            ErrorCode::BrowserRuntimeVersionMismatch,
            "bundled Lightpanda MCP contract snapshot is invalid",
        )
    })?;
    if contract.lightpanda_version != PINNED_LIGHTPANDA_VERSION
        || contract.tool_count != EXPECTED_NATIVE_TOOL_COUNT
        || contract.tools.len() != EXPECTED_NATIVE_TOOL_COUNT
    {
        return Err(CoreError::new(
            ErrorCode::BrowserRuntimeVersionMismatch,
            "bundled Lightpanda MCP contract metadata does not match the pinned runtime",
        ));
    }
    Ok(contract)
}

/// Compare discovered native tools against the pinned snapshot.
///
/// Enforces the exact 31-name set and per-tool input/output schema parity.
/// JSON object key ordering is treated as irrelevant; `required`, `enum`, and
/// `default` values are preserved in the comparison.
pub fn verify_discovered_tools(discovered: &[rmcp::model::Tool]) -> Result<(), CoreError> {
    let contract = pinned_contract()?;
    let pinned: BTreeMap<&str, &NativeToolContract> = contract
        .tools
        .iter()
        .map(|tool| (tool.name.as_str(), tool))
        .collect();

    let discovered_names: BTreeSet<&str> =
        discovered.iter().map(|tool| tool.name.as_ref()).collect();
    let expected_names: BTreeSet<&str> = pinned.keys().copied().collect();
    let missing: Vec<&str> = expected_names.difference(&discovered_names).copied().collect();
    let unexpected: Vec<&str> = discovered_names.difference(&expected_names).copied().collect();
    if !missing.is_empty() || !unexpected.is_empty() || discovered.len() != pinned.len() {
        return Err(CoreError::new(
            ErrorCode::BrowserRuntimeVersionMismatch,
            format!(
                "native Lightpanda tool set does not match the pinned contract: missing={missing:?} unexpected={unexpected:?} count={}",
                discovered.len()
            ),
        ));
    }

    for tool in discovered {
        let pinned_tool = pinned[tool.name.as_ref()];
        let expected_schema = &pinned_tool.input_schema;
        let actual_schema = Value::Object((*tool.input_schema).clone());
        if !json_semantically_equal(expected_schema, &actual_schema) {
            return Err(CoreError::new(
                ErrorCode::BrowserRuntimeVersionMismatch,
                format!(
                    "native Lightpanda tool {} input schema drifted from the pinned contract",
                    tool.name
                ),
            ));
        }
        match (&pinned_tool.output_schema, &tool.output_schema) {
            (None, Some(_)) => {
                return Err(CoreError::new(
                    ErrorCode::BrowserRuntimeVersionMismatch,
                    format!(
                        "native Lightpanda tool {} advertises an outputSchema absent from the pinned contract",
                        tool.name
                    ),
                ));
            }
            (Some(expected), Some(actual)) => {
                if !json_semantically_equal(expected, &Value::Object((**actual).clone())) {
                    return Err(CoreError::new(
                        ErrorCode::BrowserRuntimeVersionMismatch,
                        format!(
                            "native Lightpanda tool {} output schema drifted from the pinned contract",
                            tool.name
                        ),
                    ));
                }
            }
            (Some(_), None) => {
                return Err(CoreError::new(
                    ErrorCode::BrowserRuntimeVersionMismatch,
                    format!(
                        "native Lightpanda tool {} dropped an outputSchema present in the pinned contract",
                        tool.name
                    ),
                ));
            }
            (None, None) => {}
        }
    }
    Ok(())
}

/// Deep JSON equality that ignores object key ordering only.
fn json_semantically_equal(expected: &Value, actual: &Value) -> bool {
    match (expected, actual) {
        (Value::Object(expected_map), Value::Object(actual_map)) => {
            expected_map.len() == actual_map.len()
                && expected_map
                    .iter()
                    .all(|(key, value)| match actual_map.get(key) {
                        Some(actual_value) => json_semantically_equal(value, actual_value),
                        None => false,
                    })
        }
        (Value::Array(expected_items), Value::Array(actual_items)) => {
            expected_items.len() == actual_items.len()
                && expected_items
                    .iter()
                    .zip(actual_items)
                    .all(|(expected_item, actual_item)| {
                        json_semantically_equal(expected_item, actual_item)
                    })
        }
        (expected_value, actual_value) => expected_value == actual_value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn contract_as_discovered(contract: &LightpandaMcpContract) -> Vec<rmcp::model::Tool> {
        contract
            .tools
            .iter()
            .map(|tool| {
                let input_schema: serde_json::Map<String, Value> =
                    serde_json::from_value(tool.input_schema.clone()).unwrap();
                let mut discovered = rmcp::model::Tool::new(
                    tool.name.clone(),
                    tool.description.clone().unwrap_or_default(),
                    Arc::new(input_schema),
                );
                if let Some(schema) = tool.output_schema.clone() {
                    discovered = discovered.with_raw_output_schema(Arc::new(
                        serde_json::from_value(schema).unwrap(),
                    ));
                }
                discovered
            })
            .collect()
    }

    #[test]
    fn pinned_contract_has_exact_31_native_tools() {
        let c = pinned_contract().unwrap();
        assert_eq!(c.tools.len(), 31);
        let names: std::collections::BTreeSet<_> =
            c.tools.iter().map(|t| t.name.as_str()).collect();
        let expected: std::collections::BTreeSet<_> = [
            "goto", "search", "markdown", "html", "links", "evaluate", "extract",
            "tree", "nodeDetails", "interactiveElements", "structuredData", "detectForms",
            "click", "fill", "scroll", "waitForSelector", "waitForScript", "waitForState",
            "hover", "press", "selectOption", "setChecked", "findElement", "consoleLogs",
            "getUrl", "getCookies", "getEnv", "save", "session_new", "session_list",
            "session_close",
        ]
        .into_iter()
        .collect();
        assert_eq!(names, expected);
    }

    #[test]
    fn mapping_is_one_to_one_and_uses_only_browser_lightpanda_prefix() {
        assert_eq!(NATIVE_TOOL_MAPPINGS.len(), 31);
        let upstream: std::collections::BTreeSet<_> =
            NATIVE_TOOL_MAPPINGS.iter().map(|(u, _)| *u).collect();
        let public: std::collections::BTreeSet<_> =
            NATIVE_TOOL_MAPPINGS.iter().map(|(_, p)| *p).collect();
        assert_eq!(upstream.len(), 31);
        assert_eq!(public.len(), 31);
        assert!(public.iter().all(|name| name.starts_with("browser_lightpanda_")));
    }

    #[test]
    fn pinned_contract_matches_pinned_version_and_advertises_no_output_schemas() {
        let c = pinned_contract().unwrap();
        assert_eq!(c.lightpanda_version, PINNED_LIGHTPANDA_VERSION);
        assert_eq!(c.tool_count, EXPECTED_NATIVE_TOOL_COUNT);
        assert!(
            c.tools.iter().all(|tool| tool.output_schema.is_none()),
            "pinned 0.3.7 must advertise outputSchema on 0 of 31 descriptors"
        );
    }

    #[test]
    fn mapping_upstream_names_match_pinned_contract() {
        let c = pinned_contract().unwrap();
        let contract_names: BTreeSet<&str> = c.tools.iter().map(|t| t.name.as_str()).collect();
        let mapping_names: BTreeSet<&str> = NATIVE_TOOL_MAPPINGS.iter().map(|(u, _)| *u).collect();
        assert_eq!(contract_names, mapping_names);
    }

    #[test]
    fn no_upstream_schema_owns_reserved_session_id() {
        for tool in pinned_contract().unwrap().tools {
            assert!(
                tool.input_schema.pointer("/properties/session_id").is_none(),
                "upstream tool {} now owns reserved session_id",
                tool.name
            );
        }
    }

    #[test]
    fn verify_discovered_tools_accepts_exact_contract() {
        let c = pinned_contract().unwrap();
        let discovered = contract_as_discovered(&c);
        verify_discovered_tools(&discovered).unwrap();
    }

    #[test]
    fn verify_discovered_tools_rejects_missing_tool() {
        let c = pinned_contract().unwrap();
        let mut discovered = contract_as_discovered(&c);
        discovered.pop();
        let err = verify_discovered_tools(&discovered).unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserRuntimeVersionMismatch);
        assert!(err.message.contains("missing"), "{}", err.message);
    }

    #[test]
    fn verify_discovered_tools_rejects_unexpected_tool() {
        let c = pinned_contract().unwrap();
        let mut discovered = contract_as_discovered(&c);
        discovered.push(rmcp::model::Tool::new(
            "screenshot",
            "",
            Arc::new(serde_json::Map::new()),
        ));
        let err = verify_discovered_tools(&discovered).unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserRuntimeVersionMismatch);
        assert!(err.message.contains("unexpected"), "{}", err.message);
    }

    #[test]
    fn verify_discovered_tools_rejects_schema_drift() {
        let c = pinned_contract().unwrap();
        let mut discovered = contract_as_discovered(&c);
        let goto = discovered.iter_mut().find(|t| t.name == "goto").unwrap();
        let mut schema = (*goto.input_schema).clone();
        schema
            .get_mut("properties")
            .and_then(Value::as_object_mut)
            .unwrap()
            .remove("waitUntil");
        goto.input_schema = Arc::new(schema);
        let err = verify_discovered_tools(&discovered).unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserRuntimeVersionMismatch);
        assert!(err.message.contains("goto"), "{}", err.message);
    }

    #[test]
    fn verify_discovered_tools_rejects_added_output_schema() {
        let c = pinned_contract().unwrap();
        let mut discovered = contract_as_discovered(&c);
        discovered[0].output_schema = Some(Arc::new(serde_json::Map::new()));
        let err = verify_discovered_tools(&discovered).unwrap_err();
        assert_eq!(err.code, ErrorCode::BrowserRuntimeVersionMismatch);
    }

    #[test]
    fn schema_equality_ignores_key_order_but_not_values() {
        let a = serde_json::json!({"type": "object", "required": ["url"], "properties": {"url": {"type": "string"}}});
        let b = serde_json::json!({"properties": {"url": {"type": "string"}}, "type": "object", "required": ["url"]});
        assert!(json_semantically_equal(&a, &b));
        let c = serde_json::json!({"type": "object", "required": ["url"], "properties": {"url": {"type": "integer"}}});
        assert!(!json_semantically_equal(&a, &c));
        let d = serde_json::json!({"type": "object", "required": ["url", "x"], "properties": {"url": {"type": "string"}}});
        assert!(!json_semantically_equal(&a, &d));
    }
}
