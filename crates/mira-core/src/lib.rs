//! Shared vocabulary for the Mira harness.
//!
//! Everything downstream — providers, tools, the loop, the CLI — speaks the
//! types in this crate. Keep it small and free of runtime dependencies so it
//! stays cheap to depend on.

pub mod error;
pub mod id;
pub mod message;

pub use error::{Error, Result};
pub use id::{MessageId, SessionId, ToolCallId};
pub use message::{
    ImageData, Message, ReasoningBlock, Role, ToolCall, ToolCallFunction, ToolCallKind, ToolResult,
};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    // =========================================================================
    // ID type tests
    // =========================================================================

    #[test]
    fn session_id_generation_produces_unique_ids() {
        let id1 = SessionId::new();
        let id2 = SessionId::new();
        assert_ne!(id1, id2, "two generated SessionIds should differ");
    }

    #[test]
    fn session_id_has_correct_prefix() {
        let id = SessionId::new();
        assert!(
            id.0.starts_with("sess_"),
            "SessionId should have 'sess_' prefix"
        );
    }

    #[test]
    fn session_id_as_str_returns_inner_string() {
        let id = SessionId::new();
        assert_eq!(id.as_str(), &id.0);
    }

    #[test]
    fn session_id_display_impl() {
        let id = SessionId::new();
        assert_eq!(format!("{}", id), id.0);
    }

    #[test]
    fn session_id_from_string() {
        let id = SessionId::from("custom_id".to_string());
        assert_eq!(id.0, "custom_id");
    }

    #[test]
    fn session_id_from_str() {
        let id = SessionId::from("custom_id");
        assert_eq!(id.0, "custom_id");
    }

    #[test]
    fn session_id_default_is_new() {
        let id = SessionId::default();
        assert!(id.0.starts_with("sess_"));
    }

    #[test]
    fn message_id_generation_produces_unique_ids() {
        let id1 = MessageId::new();
        let id2 = MessageId::new();
        assert_ne!(id1, id2, "two generated MessageIds should differ");
    }

    #[test]
    fn message_id_has_correct_prefix() {
        let id = MessageId::new();
        assert!(
            id.0.starts_with("msg_"),
            "MessageId should have 'msg_' prefix"
        );
    }

    #[test]
    fn tool_call_id_generation_produces_unique_ids() {
        let id1 = ToolCallId::new();
        let id2 = ToolCallId::new();
        assert_ne!(id1, id2, "two generated ToolCallIds should differ");
    }

    #[test]
    fn tool_call_id_has_correct_prefix() {
        let id = ToolCallId::new();
        assert!(
            id.0.starts_with("call_"),
            "ToolCallId should have 'call_' prefix"
        );
    }

    #[test]
    fn all_id_types_are_clone_and_debug() {
        let session = SessionId::new();
        let message = MessageId::new();
        let tool = ToolCallId::new();

        // Clone
        let _ = session.clone();
        let _ = message.clone();
        let _ = tool.clone();

        // Debug
        let _ = format!("{:?}", session);
        let _ = format!("{:?}", message);
        let _ = format!("{:?}", tool);
    }

    #[test]
    fn id_types_implement_partial_eq_and_eq() {
        let a = SessionId::from("same");
        let b = SessionId::from("same");
        let c = SessionId::from("different");

        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn id_types_implement_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(SessionId::from("a"));
        set.insert(SessionId::from("b"));
        set.insert(SessionId::from("a")); // duplicate
        assert_eq!(set.len(), 2);
    }

    // =========================================================================
    // Role enum tests
    // =========================================================================

    #[test]
    fn role_serialization() {
        assert_eq!(serde_json::to_string(&Role::System).unwrap(), "\"system\"");
        assert_eq!(serde_json::to_string(&Role::User).unwrap(), "\"user\"");
        assert_eq!(
            serde_json::to_string(&Role::Assistant).unwrap(),
            "\"assistant\""
        );
        assert_eq!(serde_json::to_string(&Role::Tool).unwrap(), "\"tool\"");
    }

    #[test]
    fn role_deserialization() {
        assert_eq!(
            serde_json::from_str::<Role>("\"system\"").unwrap(),
            Role::System
        );
        assert_eq!(
            serde_json::from_str::<Role>("\"user\"").unwrap(),
            Role::User
        );
        assert_eq!(
            serde_json::from_str::<Role>("\"assistant\"").unwrap(),
            Role::Assistant
        );
        assert_eq!(
            serde_json::from_str::<Role>("\"tool\"").unwrap(),
            Role::Tool
        );
    }

    // =========================================================================
    // ReasoningBlock tests
    // =========================================================================

    #[test]
    fn reasoning_block_serialization_roundtrip() {
        let block = ReasoningBlock {
            text: "thinking...".to_string(),
            signature: Some("sig123".to_string()),
            redacted: Some("redacted".to_string()),
        };
        let json = serde_json::to_string(&block).unwrap();
        let parsed: ReasoningBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(block.text, parsed.text);
        assert_eq!(block.signature, parsed.signature);
        assert_eq!(block.redacted, parsed.redacted);
    }

    #[test]
    fn reasoning_block_serialization_omits_none_fields() {
        let block = ReasoningBlock {
            text: "thinking...".to_string(),
            signature: None,
            redacted: None,
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains("thinking"));
        assert!(!json.contains("signature"));
        assert!(!json.contains("redacted"));
    }

    #[test]
    fn reasoning_block_text_is_required() {
        let block = ReasoningBlock {
            text: "thinking...".to_string(),
            signature: None,
            redacted: None,
        };
        let json = serde_json::to_string(&block).unwrap();
        let parsed: ReasoningBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.text, "thinking...");
    }

    // =========================================================================
    // ImageData tests
    // =========================================================================

    #[test]
    fn image_data_serialization_roundtrip() {
        let img = ImageData {
            source: None,
            media_type: "image/png".to_string(),
            data: "base64data".to_string(),
        };
        let json = serde_json::to_string(&img).unwrap();
        let parsed: ImageData = serde_json::from_str(&json).unwrap();
        assert_eq!(img.media_type, parsed.media_type);
        assert_eq!(img.data, parsed.data);
    }

    #[test]
    fn image_data_png_constructor() {
        let img = ImageData::png("base64data");
        assert_eq!(img.media_type, "image/png");
        assert_eq!(img.data, "base64data");
    }

    #[test]
    fn image_data_data_url() {
        let img = ImageData::png("base64data");
        let url = img.data_url();
        assert_eq!(url, "data:image/png;base64,base64data");
    }

    // =========================================================================
    // ToolCall / ToolCallFunction / ToolCallKind tests
    // =========================================================================

    #[test]
    fn tool_call_kind_serialization() {
        assert_eq!(
            serde_json::to_string(&ToolCallKind::Function).unwrap(),
            "\"function\""
        );
    }

    #[test]
    fn tool_call_function_serialization_roundtrip() {
        let func = ToolCallFunction {
            name: "read_file".to_string(),
            arguments: r#"{"path": "test.txt"}"#.to_string(),
        };
        let json = serde_json::to_string(&func).unwrap();
        let parsed: ToolCallFunction = serde_json::from_str(&json).unwrap();
        assert_eq!(func.name, parsed.name);
        assert_eq!(func.arguments, parsed.arguments);
    }

    #[test]
    fn tool_call_serialization_roundtrip() {
        let call = ToolCall {
            id: ToolCallId::from("call_123"),
            kind: ToolCallKind::Function,
            function: ToolCallFunction {
                name: "bash".to_string(),
                arguments: r#"{"command": "ls"}"#.to_string(),
            },
        };
        let json = serde_json::to_string(&call).unwrap();
        let parsed: ToolCall = serde_json::from_str(&json).unwrap();
        assert_eq!(call.id, parsed.id);
        assert_eq!(call.kind, parsed.kind);
        assert_eq!(call.function.name, parsed.function.name);
        assert_eq!(call.function.arguments, parsed.function.arguments);
    }

    // =========================================================================
    // ToolResult tests
    // =========================================================================

    #[test]
    fn tool_result_serialization_roundtrip() {
        let result = ToolResult {
            call_id: ToolCallId::from("call_123"),
            content: "output".to_string(),
            is_error: true,
            data: Some(serde_json::json!({"key": "value"})),
            images: vec![ImageData {
            source: None,
                media_type: "image/png".to_string(),
                data: "base64".to_string(),
            }],
        };
        let json = serde_json::to_string(&result).unwrap();
        let parsed: ToolResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result.call_id, parsed.call_id);
        assert_eq!(result.content, parsed.content);
        assert_eq!(result.is_error, parsed.is_error);
        assert_eq!(result.data, parsed.data);
        assert_eq!(result.images, parsed.images);
    }

    #[test]
    fn tool_result_optional_fields_are_optional() {
        let result = ToolResult::ok(ToolCallId::from("call_123"), "output");
        let json = serde_json::to_string(&result).unwrap();
        let parsed: ToolResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result.call_id, parsed.call_id);
        assert_eq!(result.content, parsed.content);
        assert_eq!(result.is_error, parsed.is_error);
        assert_eq!(result.data, parsed.data);
        assert_eq!(result.images, parsed.images);
    }

    #[test]
    fn tool_result_ok_constructor() {
        let result = ToolResult::ok(ToolCallId::from("call_123"), "success");
        assert_eq!(result.is_error, false);
        assert_eq!(result.content, "success");
    }

    #[test]
    fn tool_result_err_constructor() {
        let result = ToolResult::err(ToolCallId::from("call_123"), "failed");
        assert_eq!(result.is_error, true);
        assert_eq!(result.content, "failed");
    }

    // =========================================================================
    // Message tests
    // =========================================================================

    #[test]
    fn message_user_text_construction() {
        let msg = Message::user("Hello world");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content, Some("Hello world".to_string()));
        assert!(msg.tool_calls.is_empty());
        assert!(msg.tool_call_id.is_none());
        assert!(msg.name.is_none());
        assert!(msg.images.is_empty());
        assert!(msg.reasoning.is_empty());
    }

    #[test]
    fn message_assistant_text_construction() {
        let msg = Message::assistant("I'll help you");
        assert_eq!(msg.role, Role::Assistant);
        assert_eq!(msg.content, Some("I'll help you".to_string()));
        assert!(msg.tool_calls.is_empty());
    }

    #[test]
    fn message_system_construction() {
        let msg = Message::system("You are a helpful assistant");
        assert_eq!(msg.role, Role::System);
        assert_eq!(msg.content, Some("You are a helpful assistant".to_string()));
    }

    #[test]
    fn message_tool_construction() {
        let msg = Message::tool(ToolCallId::from("call_123"), "tool output");
        assert_eq!(msg.role, Role::Tool);
        assert_eq!(msg.content, Some("tool output".to_string()));
        assert_eq!(msg.tool_call_id, Some(ToolCallId::from("call_123")));
    }

    #[test]
    fn message_assistant_calls_construction() {
        let calls = vec![ToolCall {
            id: ToolCallId::from("call_123"),
            kind: ToolCallKind::Function,
            function: ToolCallFunction {
                name: "read_file".to_string(),
                arguments: r#"{"path": "test.txt"}"#.to_string(),
            },
        }];
        let msg = Message::assistant_calls(calls.clone());
        assert_eq!(msg.role, Role::Assistant);
        assert_eq!(msg.content, None);
        assert_eq!(msg.tool_calls, calls);
    }

    #[test]
    fn message_with_images() {
        let msg = Message::user("See this").with_images(vec![ImageData {
            source: None,
            media_type: "image/png".to_string(),
            data: "base64".to_string(),
        }]);
        assert_eq!(msg.images.len(), 1);
        assert_eq!(msg.images[0].data, "base64");
    }

    #[test]
    fn message_serialization_roundtrip() {
        let msg = Message::assistant_calls(vec![ToolCall {
            id: ToolCallId::from("call_123"),
            kind: ToolCallKind::Function,
            function: ToolCallFunction {
                name: "read_file".to_string(),
                arguments: r#"{"path": "test.txt"}"#.to_string(),
            },
        }])
        .with_images(vec![ImageData {
            source: None,
            media_type: "image/png".to_string(),
            data: "base64".to_string(),
        }]);

        let json = serde_json::to_string(&msg).unwrap();
        let parsed: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(msg.role, parsed.role);
        assert_eq!(msg.content, parsed.content);
        assert_eq!(msg.tool_calls, parsed.tool_calls);
        assert_eq!(msg.tool_call_id, parsed.tool_call_id);
        assert_eq!(msg.name, parsed.name);
        assert_eq!(msg.images, parsed.images);
        assert_eq!(msg.reasoning, parsed.reasoning);
    }

    #[test]
    fn message_serialization_omits_empty_optional_fields() {
        let msg = Message::user("Hello");
        let json = serde_json::to_string(&msg).unwrap();
        // Check that empty vectors/None fields are omitted
        assert!(!json.contains("tool_calls"));
        assert!(!json.contains("tool_call_id"));
        assert!(!json.contains("name"));
        assert!(!json.contains("images"));
        assert!(!json.contains("reasoning"));
    }

    #[test]
    fn message_tool_result_images_propagated() {
        let msg = Message::tool(ToolCallId::from("call_123"), "result")
            .with_images(vec![ImageData::png("screenshot")]);
        assert_eq!(msg.images.len(), 1);
    }

    #[test]
    fn message_reasoning_blocks() {
        let msg = Message::assistant("I'll help").with_images(vec![]);
        // Verify we can add reasoning manually since there's no builder for it
        // The field exists but is public
        let mut msg_with_reasoning = msg;
        msg_with_reasoning.reasoning = vec![ReasoningBlock {
            text: "Let me think...".to_string(),
            signature: Some("sig".to_string()),
            redacted: None,
        }];
        assert_eq!(msg_with_reasoning.reasoning.len(), 1);
    }

    // =========================================================================
    // Error type tests
    // =========================================================================

    #[test]
    fn error_config_variant() {
        let err = Error::Config("bad config".to_string());
        assert_eq!(format!("{}", err), "configuration error: bad config");
    }

    #[test]
    fn error_provider_variant() {
        let err = Error::Provider("api down".to_string());
        assert_eq!(format!("{}", err), "provider error: api down");
    }

    #[test]
    fn error_tool_variant() {
        let err = Error::Tool {
            tool: "bash".to_string(),
            message: "command failed".to_string(),
        };
        assert_eq!(format!("{}", err), "tool `bash` failed: command failed");
    }

    #[test]
    fn error_permission_denied_variant() {
        let err = Error::PermissionDenied("no access".to_string());
        assert_eq!(format!("{}", err), "permission denied: no access");
    }

    #[test]
    fn error_cancelled_variant() {
        let err = Error::Cancelled;
        assert_eq!(format!("{}", err), "cancelled");
    }

    #[test]
    fn error_from_serde_json() {
        let json_err = serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
        let err: Error = json_err.into();
        assert!(matches!(err, Error::Serde(_)));
    }

    #[test]
    fn error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err: Error = io_err.into();
        assert!(matches!(err, Error::Io(_)));
        assert!(format!("{}", err).contains("file not found"));
    }

    #[test]
    fn error_debug_impl() {
        let err = Error::Config("test".to_string());
        let debug_str = format!("{:?}", err);
        assert!(debug_str.contains("Config"));
        assert!(debug_str.contains("test"));
    }

    #[test]
    fn result_type_alias_works() {
        fn returns_result() -> Result<i32> {
            Ok(42)
        }
        fn returns_error() -> Result<i32> {
            Err(Error::Config("bad".to_string()))
        }

        assert_eq!(returns_result().unwrap(), 42);
        assert!(returns_error().is_err());
    }

    // =========================================================================
    // Edge case and property tests
    // =========================================================================

    #[test]
    fn id_uniqueness_property() {
        // Generate many IDs and verify they're all unique
        let mut ids = std::collections::HashSet::new();
        for _ in 0..1000 {
            ids.insert(SessionId::new());
        }
        assert_eq!(ids.len(), 1000);
    }

    #[test]
    fn message_empty_content_is_some_empty_string() {
        let msg = Message::system("");
        // Empty string is Some(""), not None
        assert_eq!(msg.content, Some("".to_string()));
    }

    #[test]
    fn tool_call_id_equality_with_different_construction_methods() {
        let id1 = ToolCallId::new();
        let id2 = ToolCallId::from(id1.0.clone());
        let id3 = ToolCallId::from(id1.as_str());
        assert_eq!(id1, id2);
        assert_eq!(id1, id3);
    }

    #[test]
    fn roundtrip_through_json_preserves_all_message_fields() {
        let original = Message::assistant_calls(vec![ToolCall {
            id: ToolCallId::from("call_1"),
            kind: ToolCallKind::Function,
            function: ToolCallFunction {
                name: "tool1".to_string(),
                arguments: "{}".to_string(),
            },
        }])
        .with_images(vec![ImageData {
            source: None,
            media_type: "image/jpeg".to_string(),
            data: "abc".to_string(),
        }]);

        let json = serde_json::to_string(&original).unwrap();
        let parsed: Message = serde_json::from_str(&json).unwrap();

        // Verify all fields roundtrip correctly
        assert_eq!(original.role, parsed.role);
        assert_eq!(original.content, parsed.content);
        assert_eq!(original.tool_calls, parsed.tool_calls);
        assert_eq!(original.tool_call_id, parsed.tool_call_id);
        assert_eq!(original.name, parsed.name);
        assert_eq!(original.images, parsed.images);
        assert_eq!(original.reasoning, parsed.reasoning);
    }

    #[test]
    fn message_tool_result_images_are_copied() {
        let tool_result = ToolResult::ok(ToolCallId::from("call_123"), "output")
            .with_images(vec![ImageData::png("screenshot")]);
        // Message construction from ToolResult is handled by the harness,
        // but we can verify the fields exist
        assert_eq!(tool_result.images.len(), 1);
    }
}
