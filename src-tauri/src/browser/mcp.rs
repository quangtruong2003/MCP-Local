use lc_core::error::{CoreError, ErrorCode};
use serde::{Deserialize, Serialize};

/// Native Lightpanda load/wait states, exactly as advertised upstream.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LightpandaLoadState {
    Load,
    DomContentLoaded,
    NetworkAlmostIdle,
    NetworkIdle,
    Done,
}

fn default_checked_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaGotoArgs {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(default, rename = "waitUntil", skip_serializing_if = "Option::is_none")]
    pub wait_until: Option<LightpandaLoadState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaSearchArgs {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaMarkdownArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(
        default,
        rename = "backendNodeId",
        skip_serializing_if = "Option::is_none"
    )]
    pub backend_node_id: Option<i64>,
    #[serde(default, rename = "maxBytes", skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaHtmlArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(
        default,
        rename = "backendNodeId",
        skip_serializing_if = "Option::is_none"
    )]
    pub backend_node_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaLinksArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaEvaluateArgs {
    pub script: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaExtractArgs {
    pub schema: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaTreeArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(
        default,
        rename = "backendNodeId",
        skip_serializing_if = "Option::is_none"
    )]
    pub backend_node_id: Option<i64>,
    #[serde(default, rename = "maxDepth", skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaNodeDetailsArgs {
    #[serde(rename = "backendNodeId")]
    pub backend_node_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaInteractiveElementsArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaStructuredDataArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaDetectFormsArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaClickArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(
        default,
        rename = "backendNodeId",
        skip_serializing_if = "Option::is_none"
    )]
    pub backend_node_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaFillArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(
        default,
        rename = "backendNodeId",
        skip_serializing_if = "Option::is_none"
    )]
    pub backend_node_id: Option<i64>,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaScrollArgs {
    #[serde(
        default,
        rename = "backendNodeId",
        skip_serializing_if = "Option::is_none"
    )]
    pub backend_node_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaWaitForSelectorArgs {
    pub selector: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaWaitForScriptArgs {
    pub script: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaWaitForStateArgs {
    pub state: LightpandaLoadState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaHoverArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(
        default,
        rename = "backendNodeId",
        skip_serializing_if = "Option::is_none"
    )]
    pub backend_node_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaPressArgs {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(
        default,
        rename = "backendNodeId",
        skip_serializing_if = "Option::is_none"
    )]
    pub backend_node_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaSelectOptionArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(
        default,
        rename = "backendNodeId",
        skip_serializing_if = "Option::is_none"
    )]
    pub backend_node_id: Option<i64>,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// `setChecked` preserves the unusual upstream contract exactly: `checked` is
/// advertised as required AND as defaulting to true. schemars' derive cannot
/// express both at once (a serde default makes the field optional in the
/// generated schema), so the schema is emitted manually and the parity test
/// is authoritative.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LightpandaSetCheckedArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(
        default,
        rename = "backendNodeId",
        skip_serializing_if = "Option::is_none"
    )]
    pub backend_node_id: Option<i64>,
    #[serde(default = "default_checked_true")]
    pub checked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

impl schemars::JsonSchema for LightpandaSetCheckedArgs {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "LightpandaSetCheckedArgs".into()
    }

    fn json_schema(_generator: &mut schemars::generate::SchemaGenerator) -> schemars::Schema {
        let mut properties = serde_json::Map::new();
        properties.insert(
            "selector".into(),
            serde_json::json!({"type": ["string", "null"]}),
        );
        properties.insert(
            "backendNodeId".into(),
            serde_json::json!({"type": ["integer", "null"], "format": "int64"}),
        );
        properties.insert(
            "checked".into(),
            serde_json::json!({"type": "boolean", "default": true}),
        );
        properties.insert(
            "session_id".into(),
            serde_json::json!({"type": ["string", "null"]}),
        );
        let mut schema = serde_json::Map::new();
        schema.insert("type".into(), serde_json::json!("object"));
        schema.insert("properties".into(), serde_json::Value::Object(properties));
        schema.insert("required".into(), serde_json::json!(["checked"]));
        schema.into()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaFindElementArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaConsoleLogsArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaGetUrlArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaGetCookiesArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default)]
    pub all: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaGetEnvArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaSaveArgs {
    pub path: String,
    pub script: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaSessionNewArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaSessionListArgs {}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LightpandaSessionCloseArgs {
    pub id: String,
}

/// Splits typed public arguments into the Local Workspaces routing metadata
/// (`session_id`) and the untouched native argument map. Nothing else is
/// converted: native field names, enum values, and payload strings cross
/// unchanged.
pub fn split_session_route<T: Serialize>(
    args: &T,
) -> Result<(Option<String>, serde_json::Map<String, serde_json::Value>), CoreError> {
    let mut obj = serde_json::to_value(args)
        .map_err(|_| CoreError::new(ErrorCode::InvalidArgument, "invalid browser arguments"))?
        .as_object()
        .cloned()
        .ok_or_else(|| {
            CoreError::new(
                ErrorCode::InvalidArgument,
                "browser arguments must be an object",
            )
        })?;
    let session_id = obj
        .remove("session_id")
        .and_then(|v| v.as_str().map(str::to_string));
    Ok((session_id, obj))
}

/// Applies the Local Workspaces navigation URL policy to the `url` argument
/// of native tools whose URL actually causes navigation. `getCookies.url`
/// only scopes cookie output and is deliberately not gated here.
pub fn validate_native_navigation(
    tool: &str,
    args: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), CoreError> {
    if !super::policy::NATIVE_NAVIGATION_URL_TOOLS.contains(&tool) {
        return Ok(());
    }
    let Some(url) = args.get("url").and_then(serde_json::Value::as_str) else {
        return Ok(());
    };
    super::policy::validate_navigation_url(url)?;
    Ok(())
}

/// Builds content-free audit metadata for one native tool call. Records only
/// safe facts: origins (never full URLs), byte lengths, presence flags,
/// booleans, integers, and fixed enum values. Never records raw queries,
/// fill values, scripts, extraction schemas, cookie/env results, page or
/// console content, save paths/scripts, or raw session ids/names.
pub fn safe_native_audit_detail(
    tool: &str,
    args: &serde_json::Map<String, serde_json::Value>,
    session_id: Option<&str>,
) -> serde_json::Value {
    use serde_json::{Map, Value};

    fn byte_len(detail: &mut Map<String, Value>, key: &str, args: &Map<String, Value>, arg: &str) {
        if let Some(value) = args.get(arg).and_then(Value::as_str) {
            detail.insert(key.into(), Value::Number(value.len().into()));
        }
    }
    fn present(detail: &mut Map<String, Value>, key: &str, args: &Map<String, Value>, arg: &str) {
        detail.insert(key.into(), Value::Bool(args.contains_key(arg)));
    }
    fn copy_bool(detail: &mut Map<String, Value>, key: &str, args: &Map<String, Value>, arg: &str) {
        if let Some(value) = args.get(arg).and_then(Value::as_bool) {
            detail.insert(key.into(), Value::Bool(value));
        }
    }
    fn copy_int(detail: &mut Map<String, Value>, key: &str, args: &Map<String, Value>, arg: &str) {
        if let Some(value) = args.get(arg).and_then(Value::as_i64) {
            detail.insert(key.into(), Value::Number(value.into()));
        }
    }
    fn copy_enum(detail: &mut Map<String, Value>, key: &str, args: &Map<String, Value>, arg: &str) {
        if let Some(value) = args.get(arg).and_then(Value::as_str) {
            detail.insert(key.into(), Value::String(value.to_string()));
        }
    }
    fn origin(detail: &mut Map<String, Value>, args: &Map<String, Value>) {
        if let Some(url) = args.get("url").and_then(Value::as_str) {
            if let Ok(origin) = super::policy::redacted_origin(url) {
                detail.insert("origin".into(), Value::String(origin));
            }
        }
    }
    fn target_kind(args: &Map<String, Value>) -> &'static str {
        if args.contains_key("backendNodeId") {
            "backend_node_id"
        } else if args.contains_key("selector") {
            "selector"
        } else {
            "none"
        }
    }

    let mut detail = Map::new();
    match tool {
        "goto" => {
            origin(&mut detail, args);
            copy_enum(&mut detail, "wait_until", args, "waitUntil");
            present(&mut detail, "timeout_present", args, "timeout");
        }
        "search" => byte_len(&mut detail, "query_bytes", args, "query"),
        "markdown" => {
            origin(&mut detail, args);
            present(&mut detail, "selector_present", args, "selector");
            present(
                &mut detail,
                "backend_node_id_present",
                args,
                "backendNodeId",
            );
            copy_int(&mut detail, "max_bytes", args, "maxBytes");
            present(&mut detail, "timeout_present", args, "timeout");
        }
        "html" => {
            origin(&mut detail, args);
            present(&mut detail, "selector_present", args, "selector");
            present(
                &mut detail,
                "backend_node_id_present",
                args,
                "backendNodeId",
            );
            present(&mut detail, "timeout_present", args, "timeout");
        }
        "links" | "interactiveElements" | "structuredData" | "detectForms" => {
            origin(&mut detail, args);
            present(&mut detail, "timeout_present", args, "timeout");
        }
        "tree" => {
            origin(&mut detail, args);
            present(
                &mut detail,
                "backend_node_id_present",
                args,
                "backendNodeId",
            );
            copy_int(&mut detail, "max_depth", args, "maxDepth");
            present(&mut detail, "timeout_present", args, "timeout");
        }
        "nodeDetails" => copy_int(&mut detail, "backend_node_id", args, "backendNodeId"),
        "evaluate" => {
            byte_len(&mut detail, "script_bytes", args, "script");
            origin(&mut detail, args);
            present(&mut detail, "save_present", args, "save");
            present(&mut detail, "timeout_present", args, "timeout");
        }
        "extract" => {
            byte_len(&mut detail, "schema_bytes", args, "schema");
            present(&mut detail, "save_present", args, "save");
        }
        "click" | "hover" => {
            detail.insert("target".into(), serde_json::json!(target_kind(args)));
        }
        "fill" | "selectOption" => {
            detail.insert("target".into(), serde_json::json!(target_kind(args)));
            byte_len(&mut detail, "value_bytes", args, "value");
        }
        "scroll" => {
            present(
                &mut detail,
                "backend_node_id_present",
                args,
                "backendNodeId",
            );
            present(&mut detail, "x_present", args, "x");
            present(&mut detail, "y_present", args, "y");
        }
        "waitForSelector" => {
            byte_len(&mut detail, "selector_bytes", args, "selector");
            present(&mut detail, "timeout_present", args, "timeout");
        }
        "waitForScript" => {
            byte_len(&mut detail, "script_bytes", args, "script");
            present(&mut detail, "timeout_present", args, "timeout");
        }
        "waitForState" => {
            copy_enum(&mut detail, "state", args, "state");
            present(&mut detail, "timeout_present", args, "timeout");
        }
        "press" => {
            copy_enum(&mut detail, "key", args, "key");
            detail.insert("target".into(), serde_json::json!(target_kind(args)));
        }
        "setChecked" => {
            detail.insert("target".into(), serde_json::json!(target_kind(args)));
            copy_bool(&mut detail, "checked", args, "checked");
        }
        "findElement" => {
            present(&mut detail, "role_present", args, "role");
            present(&mut detail, "name_present", args, "name");
        }
        "getCookies" => {
            copy_bool(&mut detail, "all", args, "all");
            origin(&mut detail, args);
        }
        "getEnv" => present(&mut detail, "name_present", args, "name"),
        "save" => {
            byte_len(&mut detail, "path_bytes", args, "path");
            byte_len(&mut detail, "script_bytes", args, "script");
        }
        "session_new" => present(&mut detail, "name_present", args, "name"),
        "session_close" => byte_len(&mut detail, "id_bytes", args, "id"),
        // consoleLogs, getUrl, session_list: session fingerprint only.
        _ => {}
    }
    detail.insert("session_present".into(), Value::Bool(session_id.is_some()));
    if let Some(id) = session_id {
        detail.insert("session_bytes".into(), Value::Number(id.len().into()));
    }
    Value::Object(detail)
}

#[cfg(test)]
mod schema_tests {
    //! Schema parity: every public `browser_lightpanda_*` argument schema,
    //! minus the Local Workspaces-only `session_id`, must equal the pinned
    //! Lightpanda 0.3.7 upstream inputSchema (property names/types, required
    //! sets, enum arrays, defaults).

    use super::*;
    use schemars::generate::SchemaSettings;
    use serde_json::Value;
    use std::collections::BTreeSet;

    fn public_schema<T: schemars::JsonSchema>() -> Value {
        // Same settings rmcp uses when publishing tool inputSchemas.
        let generator = SchemaSettings::draft2020_12().into_generator();
        let schema = generator.into_root_schema_for::<T>();
        serde_json::to_value(schema).unwrap()
    }

    /// One entry per pinned native tool, keyed by the upstream name, in
    /// contract mapping order. A missing public args type fails the lookup.
    fn native_public_arg_schemas() -> Vec<(&'static str, Value)> {
        let mut schemas: std::collections::HashMap<&'static str, Value> =
            std::collections::HashMap::from([
                ("goto", public_schema::<LightpandaGotoArgs>()),
                ("search", public_schema::<LightpandaSearchArgs>()),
                ("markdown", public_schema::<LightpandaMarkdownArgs>()),
                ("html", public_schema::<LightpandaHtmlArgs>()),
                ("links", public_schema::<LightpandaLinksArgs>()),
                ("evaluate", public_schema::<LightpandaEvaluateArgs>()),
                ("extract", public_schema::<LightpandaExtractArgs>()),
                ("tree", public_schema::<LightpandaTreeArgs>()),
                ("nodeDetails", public_schema::<LightpandaNodeDetailsArgs>()),
                (
                    "interactiveElements",
                    public_schema::<LightpandaInteractiveElementsArgs>(),
                ),
                (
                    "structuredData",
                    public_schema::<LightpandaStructuredDataArgs>(),
                ),
                ("detectForms", public_schema::<LightpandaDetectFormsArgs>()),
                ("click", public_schema::<LightpandaClickArgs>()),
                ("fill", public_schema::<LightpandaFillArgs>()),
                ("scroll", public_schema::<LightpandaScrollArgs>()),
                (
                    "waitForSelector",
                    public_schema::<LightpandaWaitForSelectorArgs>(),
                ),
                (
                    "waitForScript",
                    public_schema::<LightpandaWaitForScriptArgs>(),
                ),
                (
                    "waitForState",
                    public_schema::<LightpandaWaitForStateArgs>(),
                ),
                ("hover", public_schema::<LightpandaHoverArgs>()),
                ("press", public_schema::<LightpandaPressArgs>()),
                (
                    "selectOption",
                    public_schema::<LightpandaSelectOptionArgs>(),
                ),
                ("setChecked", public_schema::<LightpandaSetCheckedArgs>()),
                ("findElement", public_schema::<LightpandaFindElementArgs>()),
                ("consoleLogs", public_schema::<LightpandaConsoleLogsArgs>()),
                ("getUrl", public_schema::<LightpandaGetUrlArgs>()),
                ("getCookies", public_schema::<LightpandaGetCookiesArgs>()),
                ("getEnv", public_schema::<LightpandaGetEnvArgs>()),
                ("save", public_schema::<LightpandaSaveArgs>()),
                ("session_new", public_schema::<LightpandaSessionNewArgs>()),
                ("session_list", public_schema::<LightpandaSessionListArgs>()),
                (
                    "session_close",
                    public_schema::<LightpandaSessionCloseArgs>(),
                ),
            ]);
        super::super::contract::NATIVE_TOOL_MAPPINGS
            .iter()
            .map(|(native, _public)| {
                let schema = schemas
                    .remove(*native)
                    .unwrap_or_else(|| panic!("missing public argument type for {native}"));
                (*native, schema)
            })
            .collect()
    }

    /// Resolves one local `$ref` against the schema root.
    fn resolve_ref(schema: &Value, root: &Value) -> Value {
        if let Some(ref_path) = schema.get("$ref").and_then(Value::as_str) {
            if let Some(pointer) = ref_path.strip_prefix('#') {
                if let Some(resolved) = root.pointer(pointer) {
                    return resolved.clone();
                }
            }
        }
        schema.clone()
    }

    /// Reduces a property schema to its effective non-null branch, resolving
    /// `$ref`s and `anyOf: [X, null]` unions so schemars' nullable encoding
    /// compares against upstream's plain types.
    fn effective(schema: &Value, root: &Value) -> Value {
        let mut current = resolve_ref(schema, root);
        if let Some(branches) = current.get("anyOf").and_then(Value::as_array).cloned() {
            let non_null: Vec<Value> = branches
                .into_iter()
                .filter(|branch| branch.get("type").and_then(Value::as_str) != Some("null"))
                .map(|branch| resolve_ref(&branch, root))
                .collect();
            if non_null.len() == 1 {
                current = non_null.into_iter().next().unwrap();
            }
        }
        current
    }

    fn base_type(schema: &Value) -> Option<String> {
        match schema.get("type") {
            Some(Value::String(t)) => Some(t.clone()),
            Some(Value::Array(types)) => types
                .iter()
                .filter_map(Value::as_str)
                .find(|t| *t != "null")
                .map(str::to_string),
            _ => None,
        }
    }

    fn enum_values(schema: &Value) -> Option<BTreeSet<String>> {
        schema.get("enum").and_then(Value::as_array).map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
    }

    fn required_set(schema: &Value) -> BTreeSet<String> {
        schema
            .get("required")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn assert_parity(tool: &str, public: &Value, pinned: &Value) {
        let empty = serde_json::Map::new();
        let public_props = public
            .get("properties")
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        let pinned_props = pinned
            .get("properties")
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        let public_names: BTreeSet<&String> = public_props.keys().collect();
        let pinned_names: BTreeSet<&String> = pinned_props.keys().collect();
        assert_eq!(public_names, pinned_names, "{tool}: property names differ");
        for name in &pinned_names {
            let public_prop = effective(&public_props[*name], public);
            let pinned_prop = effective(&pinned_props[*name], pinned);
            assert_eq!(
                base_type(&public_prop),
                base_type(&pinned_prop),
                "{tool}.{name}: type differs"
            );
            assert_eq!(
                enum_values(&public_prop),
                enum_values(&pinned_prop),
                "{tool}.{name}: enum values differ"
            );
            assert_eq!(
                public_prop.get("default"),
                pinned_prop.get("default"),
                "{tool}.{name}: default differs"
            );
        }
        assert_eq!(
            required_set(public),
            required_set(pinned),
            "{tool}: required set differs"
        );
    }

    #[test]
    fn public_schemas_match_pinned_native_contract() {
        let pinned: std::collections::HashMap<String, Value> =
            super::super::contract::pinned_contract()
                .unwrap()
                .tools
                .into_iter()
                .map(|tool| (tool.name, tool.input_schema))
                .collect();
        let mut checked = 0usize;
        for (tool, mut public) in native_public_arg_schemas() {
            let pinned_schema = pinned
                .get(tool)
                .unwrap_or_else(|| panic!("pinned contract has no schema for {tool}"));
            // session_id is Local Workspaces routing metadata, never part of
            // the native contract.
            if let Some(props) = public.get_mut("properties").and_then(Value::as_object_mut) {
                props.remove("session_id");
            }
            assert_parity(tool, &public, pinned_schema);
            checked += 1;
        }
        assert_eq!(checked, super::super::contract::EXPECTED_NATIVE_TOOL_COUNT);
    }

    #[test]
    fn upstream_contract_does_not_own_session_id() {
        for tool in super::super::contract::pinned_contract().unwrap().tools {
            assert!(
                tool.input_schema
                    .pointer("/properties/session_id")
                    .is_none(),
                "upstream tool {} now owns reserved session_id",
                tool.name
            );
        }
    }

    #[test]
    fn session_id_is_present_exactly_where_allowed() {
        for (tool, schema) in native_public_arg_schemas() {
            let is_control = matches!(tool, "session_new" | "session_list" | "session_close");
            let has = schema.pointer("/properties/session_id").is_some();
            assert_eq!(has, !is_control, "{tool}: session_id placement wrong");
            assert!(
                !required_set(&schema).contains("session_id"),
                "{tool}: session_id must stay optional"
            );
        }
    }

    #[test]
    fn split_session_route_strips_only_session_id() {
        let args = LightpandaGotoArgs {
            url: "https://example.com/".into(),
            timeout: Some(5000),
            wait_until: Some(LightpandaLoadState::NetworkIdle),
            session_id: Some("alpha".into()),
        };
        let (session_id, native) = split_session_route(&args).unwrap();
        assert_eq!(session_id.as_deref(), Some("alpha"));
        assert_eq!(native["url"], "https://example.com/");
        assert_eq!(native["timeout"], 5000);
        assert_eq!(native["waitUntil"], "networkidle");
        assert!(!native.contains_key("session_id"));

        let omitted = LightpandaGetUrlArgs { session_id: None };
        let (session_id, native) = split_session_route(&omitted).unwrap();
        assert_eq!(session_id, None);
        assert!(native.is_empty());
    }

    #[test]
    fn native_navigation_policy_gates_only_navigating_urls() {
        use serde_json::json;
        // Navigating tools are gated.
        let err = validate_native_navigation(
            "goto",
            json!({"url": "file:///etc/passwd"}).as_object().unwrap(),
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument);
        assert!(validate_native_navigation(
            "goto",
            json!({"url": "https://example.com/"}).as_object().unwrap()
        )
        .is_ok());
        // Optional url absent: upstream schema requirement owns it.
        assert!(validate_native_navigation("markdown", json!({}).as_object().unwrap()).is_ok());
        // getCookies.url scopes output, never navigates.
        assert!(validate_native_navigation(
            "getCookies",
            json!({"url": "file:///etc/passwd"}).as_object().unwrap()
        )
        .is_ok());
        // Non-URL tools pass untouched.
        assert!(validate_native_navigation(
            "click",
            json!({"selector": "#x"}).as_object().unwrap()
        )
        .is_ok());
    }

    #[test]
    fn audit_detail_keeps_metadata_and_drops_raw_content() {
        use serde_json::json;
        let args = json!({
            "script": "SECRET_SCRIPT_BODY",
            "url": "https://example.com/private/path?token=abc",
            "save": "job.js"
        });
        let detail =
            safe_native_audit_detail("evaluate", args.as_object().unwrap(), Some("my-session"));
        let serialized = serde_json::to_string(&detail).unwrap();
        assert!(!serialized.contains("SECRET_SCRIPT_BODY"));
        assert!(!serialized.contains("my-session"));
        assert!(!serialized.contains("private/path"));
        assert!(!serialized.contains("token=abc"));
        assert_eq!(detail["script_bytes"], 18);
        assert_eq!(detail["origin"], "https://example.com");
        assert_eq!(detail["save_present"], true);
        assert_eq!(detail["session_present"], true);
        assert_eq!(detail["session_bytes"], 10);

        let fill = json!({"backendNodeId": 7, "value": "SECRET_VALUE"});
        let detail = safe_native_audit_detail("fill", fill.as_object().unwrap(), None);
        let serialized = serde_json::to_string(&detail).unwrap();
        assert!(!serialized.contains("SECRET_VALUE"));
        assert_eq!(detail["target"], "backend_node_id");
        assert_eq!(detail["value_bytes"], 12);
        assert_eq!(detail["session_present"], false);

        let close = json!({"id": "secret-session-name"});
        let detail = safe_native_audit_detail("session_close", close.as_object().unwrap(), None);
        let serialized = serde_json::to_string(&detail).unwrap();
        assert!(!serialized.contains("secret-session-name"));
        assert_eq!(detail["id_bytes"], 19);
    }
}

