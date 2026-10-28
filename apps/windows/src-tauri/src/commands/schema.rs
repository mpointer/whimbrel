//! The JSON Schema bundle the UI's TypeScript types are generated from (plan
//! Decision 11): schemars 1, pinned to draft-07 for `json-schema-to-typescript`.
//!
//! `cargo run --bin gen-schema` writes it to `ui/src/generated/schema.json`, and
//! `npm run gen:types` turns that into `types.ts`. The test below fails when a DTO
//! changes without regenerating, so a shape change breaks the build, not the runtime.

use schemars::generate::SchemaSettings;
use serde_json::{json, Map, Value};

use super::AppStatus;
use crate::error::CommandError;
use crate::settings::RoutingSettings;

/// Where the committed bundle lives, relative to `src-tauri/`.
pub const SCHEMA_PATH: &str = "../ui/src/generated/schema.json";

/// Every DTO the UI sees, as one draft-07 schema whose `definitions` hold each type.
pub fn schema_bundle() -> Value {
    let mut generator = SchemaSettings::draft07().into_generator();
    generator.subschema_for::<AppStatus>();
    generator.subschema_for::<CommandError>();
    generator.subschema_for::<RoutingSettings>();
    generator.subschema_for::<crate::capture::service::CaptureStatus>();
    generator.subschema_for::<super::library::LibraryRow>();
    generator.subschema_for::<super::library::CaptureDetail>();
    generator.subschema_for::<super::library::MeetingDetail>();
    generator.subschema_for::<super::library::LibrarySettings>();
    generator.subschema_for::<crate::syncing::controller::SyncView>();
    generator.subschema_for::<crate::syncing::controller::DeviceView>();
    generator.subschema_for::<super::ai::AiSettings>();
    generator.subschema_for::<super::ai::ProviderKey>();
    generator.subschema_for::<super::capture::CaptureSettings>();
    generator.subschema_for::<super::models::ModelStatus>();
    generator.subschema_for::<super::models::DownloadView>();
    generator.subschema_for::<super::models::MeetingModelsStatus>();
    generator.subschema_for::<crate::settings::PromptSurface>();
    generator.subschema_for::<crate::chat::ChatEvent>();
    generator.subschema_for::<super::chat::ConversationRow>();
    generator.subschema_for::<super::chat::ConversationView>();
    generator.subschema_for::<super::chat::StarterPromptView>();
    generator.subschema_for::<super::chat::ChatSpend>();
    generator.subschema_for::<super::today::TodayOverview>();
    generator.subschema_for::<super::today::TodayRecap>();
    generator.subschema_for::<super::schedule::ScheduleView>();
    generator.subschema_for::<super::schedule::ScheduleEvent>();
    generator.subschema_for::<crate::tray::TrayStatus>();
    generator.subschema_for::<crate::meetings::controller::MeetingRecordingView>();
    generator.subschema_for::<crate::meetings::pipeline::Processing>();
    generator.subschema_for::<crate::meetings::pipeline::PipelineFailure>();
    generator.subschema_for::<super::workspace::WorkspaceView>();
    generator.subschema_for::<super::workspace::TranscriptView>();
    generator.subschema_for::<super::workspace::ActionItemView>();
    generator.subschema_for::<super::meetings::MeetingSettings>();
    generator.subschema_for::<crate::meetings::service::AlertView>();
    generator.subschema_for::<super::connectors::CalendarAccounts>();
    generator.subschema_for::<super::mcp::McpServerView>();
    generator.subschema_for::<super::mcp::McpClientView>();
    generator.subschema_for::<super::mcp::McpNewClient>();
    generator.subschema_for::<super::connectors::GmailAccounts>();
    generator.subschema_for::<super::connectors::GmailAccount>();
    generator.subschema_for::<crate::retention::RetentionSettings>();
    generator.subschema_for::<super::retention::RetentionView>();
    generator.subschema_for::<crate::retention::RetentionPreview>();
    generator.subschema_for::<super::general::GeneralSettings>();
    generator.subschema_for::<super::prep::NextMeetingView>();
    generator.subschema_for::<super::prep::PrepBriefView>();
    generator.subschema_for::<super::prep::PrepAttendee>();
    generator.subschema_for::<super::prep::PrepClaim>();
    generator.subschema_for::<super::prep::PrepSourceView>();
    generator.subschema_for::<super::profile::ProfileView>();
    generator.subschema_for::<super::profile::ProfileFields>();
    generator.subschema_for::<super::profile::ProfileSwitches>();
    generator.subschema_for::<super::profile::SuggestionView>();
    generator.subschema_for::<super::routines::RoutineView>();
    generator.subschema_for::<super::routines::RunView>();
    generator.subschema_for::<super::routines::HostView>();
    generator.subschema_for::<super::routines::HostOption>();
    generator.subschema_for::<super::approvals::ApprovalView>();
    generator.subschema_for::<super::approvals::ApproveResult>();
    generator.subschema_for::<super::approvals::ActionView>();
    let mut definitions: Map<String, Value> = generator.definitions().clone().into_iter().collect();
    for schema in definitions.values_mut() {
        ref_siblings_to_all_of(schema);
    }
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "WhimbrelDtos",
        "description": "Generated from apps/windows/src-tauri's command DTOs. Do not edit; run `cargo run --bin gen-schema`.",
        "type": "object",
        "additionalProperties": false,
        "definitions": definitions,
    })
}

/// Draft-07 ignores every keyword next to a `$ref`. schemars puts an internally tagged
/// enum's `kind` property beside each newtype variant's `$ref`, so a draft-07 consumer
/// (`json-schema-to-typescript`) keeps the `kind` and drops the referenced struct's
/// fields. Rewrites `{ "$ref": R, ...rest }` as `{ "allOf": [{ "$ref": R }, { ...rest }] }`,
/// which means the same thing and which draft-07 tools read correctly.
fn ref_siblings_to_all_of(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for child in map.values_mut() {
                ref_siblings_to_all_of(child);
            }
            if map.contains_key("$ref") && map.len() > 1 {
                let reference = map.remove("$ref").unwrap();
                let rest = std::mem::take(map);
                map.insert("allOf".into(), json!([{ "$ref": reference }, Value::Object(rest)]));
            }
        }
        Value::Array(items) => items.iter_mut().for_each(ref_siblings_to_all_of),
        _ => {}
    }
}

pub fn schema_bundle_text() -> String {
    serde_json::to_string_pretty(&schema_bundle()).unwrap() + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_schema_is_current() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(SCHEMA_PATH);
        let committed = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            committed.replace("\r\n", "\n") == schema_bundle_text(),
            "{} is stale: run `cargo run --bin gen-schema` then `npm run gen:types` in apps/windows/ui",
            path.display()
        );
    }

    #[test]
    fn bundle_is_draft_07_with_every_dto() {
        let bundle = schema_bundle();
        assert_eq!(bundle["$schema"], "http://json-schema.org/draft-07/schema#");
        for name in ["AppStatus", "CommandError", "RoutingSettings", "QueueWaitStats", "Provider", "SpendScope", "CaptureStatus", "LibraryRow", "CaptureDetail", "MeetingDetail", "EpisodeRow", "DeviceTag", "LibrarySettings", "SyncView", "SyncState", "DeviceView"] {
            assert!(bundle["definitions"].get(name).is_some(), "missing {name}");
        }
        assert!(!schema_bundle_text().contains("$defs"), "draft-07 uses `definitions`, not `$defs`");
    }

    fn assert_no_ref_siblings(value: &Value, path: &str) {
        match value {
            Value::Object(map) => {
                assert!(!(map.contains_key("$ref") && map.len() > 1), "{path} has keywords next to $ref: {map:?}");
                for (k, v) in map {
                    assert_no_ref_siblings(v, &format!("{path}/{k}"));
                }
            }
            Value::Array(items) => items.iter().enumerate().for_each(|(i, v)| assert_no_ref_siblings(v, &format!("{path}/{i}"))),
            _ => {}
        }
    }

    #[test]
    fn no_ref_carries_sibling_keywords_draft_07_would_ignore() {
        assert_no_ref_siblings(&schema_bundle(), "#");
    }

    #[test]
    fn ref_siblings_become_all_of() {
        let mut v = json!({ "$ref": "#/definitions/CaptureRow", "properties": { "kind": { "const": "capture" } }, "required": ["kind"] });
        ref_siblings_to_all_of(&mut v);
        assert_eq!(
            v,
            json!({ "allOf": [
                { "$ref": "#/definitions/CaptureRow" },
                { "properties": { "kind": { "const": "capture" } }, "required": ["kind"] }
            ] })
        );
        let mut lone = json!({ "$ref": "#/definitions/X" });
        ref_siblings_to_all_of(&mut lone);
        assert_eq!(lone, json!({ "$ref": "#/definitions/X" }), "a bare $ref is left alone");
    }
}
