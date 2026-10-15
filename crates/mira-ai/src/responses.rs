//! Native OpenAI compaction and replay through Responses. Compatibility endpoints
//! keep Chat Completions; only the official endpoint is probed for compaction.
use eventsource_stream::Eventsource;
use futures::{stream::BoxStream, StreamExt};
use mira_core::{message::ProviderContext, Message, Role, ToolCall, ToolCallId};
use serde_json::{json, Value};
use tokio_stream::wrappers::ReceiverStream;

use crate::{
    openai::OpenAiCompatible, ChatEvent, ChatRequest, FinishReason, ProviderError, TokenUsage,
};

fn official(base: &str) -> bool {
    base.trim_end_matches('/') == "https://api.openai.com/v1"
}

fn input(request: &ChatRequest, owner: &str) -> Result<Vec<Value>, ProviderError> {
    let mut items = Vec::new();
    let mut images = Vec::new();
    for m in &request.messages {
        if m.role != Role::Tool && !images.is_empty() {
            items.push(json!({"role":"user","content":std::mem::take(&mut images)}));
        }
        if let Some(context) = &m.provider_context {
            if context.owner != owner {
                return Err(ProviderError::Config("Native checkpoint belongs to another provider; resume with its original provider".into()));
            }
            items.extend(context.items.clone());
            continue;
        }
        if m.role == Role::Tool {
            items.push(json!({"type":"function_call_output", "call_id":m.tool_call_id, "output":m.content.as_deref().unwrap_or("")}));
            images.extend(m.images.iter().map(|i| json!({"type":"input_image","image_url":format!("data:{};base64,{}",i.media_type,i.data)})));
        } else {
            let role = match m.role {
                Role::System => "system",
                Role::User => "user",
                _ => "assistant",
            };
            let mut content = Vec::new();
            if let Some(text) = m.content.as_ref().filter(|text| !text.is_empty()) {
                content.push(json!({"type": if m.role == Role::Assistant {"output_text"} else {"input_text"},"text":text}));
            }
            content.extend(m.images.iter().map(|i| json!({"type":"input_image","image_url":format!("data:{};base64,{}",i.media_type,i.data)})));
            if !content.is_empty() {
                items.push(json!({"role":role,"content":content}));
            }
            for call in &m.tool_calls {
                items.push(json!({"type":"function_call","call_id":call.id,"name":call.function.name,"arguments":call.function.arguments}));
            }
        }
    }
    if !images.is_empty() {
        items.push(json!({"role":"user","content":images}));
    }
    Ok(items)
}

async fn checked(response: reqwest::Response) -> Result<reqwest::Response, ProviderError> {
    if response.status().is_success() {
        return Ok(response);
    }
    Err(ProviderError::Status {
        status: response.status().as_u16(),
        retry_after: crate::provider::retry_after(response.headers()),
        body: response.text().await.unwrap_or_default(),
    })
}

pub(super) async fn compact(
    p: &OpenAiCompatible,
    request: ChatRequest,
) -> Result<Option<Message>, ProviderError> {
    if !official(&p.cfg.base_url) {
        return Ok(None);
    }
    let base = p.cfg.base_url.trim_end_matches('/');
    let owner = p.context_owner();
    let response = checked(
        p.http
            .post(format!("{base}/responses/compact"))
            .headers(p.headers()?)
            .json(&json!({"model":request.model,"input":input(&request, &owner)?}))
            .send()
            .await?,
    )
    .await?;
    let value: Value = response.json().await?;
    checkpoint(value, &owner, &request.model).map(Some)
}

fn checkpoint(value: Value, owner: &str, model: &str) -> Result<Message, ProviderError> {
    let items = value["output"]
        .as_array()
        .filter(|items| {
            items.iter().any(|i| {
                i["type"] == "compaction"
                    && i["encrypted_content"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty())
            })
        })
        .ok_or_else(|| {
            ProviderError::Decode("Compaction returned no encrypted checkpoint".into())
        })?;
    let mut message = Message::user("Provider-native conversation checkpoint");
    message.provider_context = Some(ProviderContext {
        owner: owner.into(),
        model: model.into(),
        items: items.clone(),
        tokens: value["usage"]["output_tokens"].as_u64().unwrap_or(8_000),
    });
    Ok(message)
}

fn body(request: &ChatRequest, owner: &str) -> Result<Value, ProviderError> {
    let mut body = json!({"model":request.model,"input":input(request,owner)?,"stream":true,"store":false,"include":["reasoning.encrypted_content"]});
    if !request.tools.is_empty() {
        body["tools"] = json!(request.tools.iter().map(|t| json!({"type":"function","name":t.name,"description":t.description,"parameters":t.parameters})).collect::<Vec<_>>());
    }
    if let Some(tokens) = request.max_tokens {
        body["max_output_tokens"] = json!(tokens);
    }
    if let Some(temperature) = request.temperature {
        body["temperature"] = json!(temperature);
    }
    if let Some(effort) = &request.reasoning_effort {
        body["reasoning"] = json!({"effort":effort});
    }
    if let Some(tier) = &request.service_tier {
        body["service_tier"] = json!(tier);
    }
    if let Some(format) = &request.response_format {
        body["text"] = match format {
            crate::ResponseFormat::JsonObject => json!({"format":{"type":"json_object"}}),
            crate::ResponseFormat::JsonSchema {
                name,
                schema,
                strict,
            } => {
                json!({"format":{"type":"json_schema","name":name,"schema":schema,"strict":strict}})
            }
        };
    }
    Ok(body)
}

pub(super) async fn stream(
    p: &OpenAiCompatible,
    request: ChatRequest,
) -> Result<BoxStream<'static, Result<ChatEvent, ProviderError>>, ProviderError> {
    let base = p.cfg.base_url.trim_end_matches('/');
    let owner = p.context_owner();
    if !official(base) {
        return Err(ProviderError::Config(
            "This endpoint cannot replay OpenAI native checkpoints".into(),
        ));
    }
    let response = checked(
        p.http
            .post(format!("{base}/responses"))
            .headers(p.headers()?)
            .json(&body(&request, &owner)?)
            .send()
            .await?,
    )
    .await?;
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    let model = request.model.clone();
    tokio::spawn(async move {
        let mut sse = Box::pin(response.bytes_stream().eventsource());
        let mut ended = false;
        while let Some(event) = sse.next().await {
            let events = match event {
                Ok(event) => serde_json::from_str::<Value>(&event.data)
                    .map_err(|e| ProviderError::Decode(e.to_string()))
                    .and_then(|v| decode(&v)),
                Err(e) => Err(ProviderError::Decode(e.to_string())),
            };
            match events {
                Ok(events) => {
                    for mut event in events {
                        if let ChatEvent::ProviderContext(context) = &mut event {
                            context.owner = owner.clone();
                            context.model = model.clone();
                        }
                        ended |= matches!(event, ChatEvent::Done(_));
                        if tx.send(Ok(event)).await.is_err() {
                            return;
                        }
                    }
                }
                Err(error) => {
                    let _ = tx.send(Err(error)).await;
                    return;
                }
            }
            if ended {
                return;
            }
        }
        if !ended {
            let _ = tx
                .send(Err(ProviderError::Decode(
                    "Responses stream ended before completion".into(),
                )))
                .await;
        }
    });
    Ok(ReceiverStream::new(rx).boxed())
}

fn decode(v: &Value) -> Result<Vec<ChatEvent>, ProviderError> {
    let mut events = Vec::new();
    match v["type"].as_str().unwrap_or("") {
        "response.output_text.delta" => {
            if let Some(text) = v["delta"].as_str() {
                events.push(ChatEvent::TextDelta(text.into()));
            }
        }
        "response.reasoning_summary_text.delta" => {
            if let Some(text) = v["delta"].as_str() {
                events.push(ChatEvent::ReasoningDelta(text.into()));
            }
        }
        "response.completed" | "response.incomplete" => {
            let response = &v["response"];
            let calls: Vec<ToolCall> = response["output"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|i| v["type"] == "response.completed" && i["type"] == "function_call")
                .map(|i| {
                    let id = i["call_id"]
                        .as_str()
                        .ok_or_else(|| ProviderError::Decode("Missing function call id".into()))?;
                    let name = i["name"]
                        .as_str()
                        .ok_or_else(|| ProviderError::Decode("Missing function name".into()))?;
                    let arguments = i["arguments"].as_str().ok_or_else(|| {
                        ProviderError::Decode("Missing function arguments".into())
                    })?;
                    Ok(ToolCall {
                        id: ToolCallId::from(id),
                        kind: mira_core::message::ToolCallKind::Function,
                        function: mira_core::message::ToolCallFunction {
                            name: name.into(),
                            arguments: arguments.into(),
                        },
                    })
                })
                .collect::<Result<_, ProviderError>>()?;
            let reason = if v["type"] == "response.incomplete" {
                FinishReason::Length
            } else if calls.is_empty() {
                FinishReason::Stop
            } else {
                FinishReason::ToolCalls
            };
            if !calls.is_empty() {
                events.push(ChatEvent::ToolCalls(calls));
            }
            if let Some(usage) = response.get("usage") {
                events.push(ChatEvent::Usage(TokenUsage {
                    prompt_tokens: usage["input_tokens"].as_u64().unwrap_or(0) as u32,
                    completion_tokens: usage["output_tokens"].as_u64().unwrap_or(0) as u32,
                    cached_input_tokens: usage["input_tokens_details"]["cached_tokens"]
                        .as_u64()
                        .unwrap_or(0) as u32,
                    cache_write_tokens: 0,
                }));
            }
            if v["type"] == "response.completed" {
                let items = response["output"].as_array().cloned().unwrap_or_default();
                events.push(ChatEvent::ProviderContext(ProviderContext {
                    owner: String::new(),
                    model: String::new(),
                    items,
                    tokens: response["usage"]["output_tokens"].as_u64().unwrap_or(0),
                }));
            }
            events.push(ChatEvent::Done(reason));
        }
        "error" | "response.failed" => return Err(ProviderError::Decode(v.to_string())),
        _ => {}
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(messages: Vec<Message>) -> ChatRequest {
        ChatRequest {
            model: "gpt-5".into(),
            messages,
            tools: vec![],
            temperature: None,
            max_tokens: None,
            reasoning_effort: None,
            service_tier: None,
            response_format: None,
        }
    }
    #[test]
    fn checkpoint_ownership_changes_with_credentials_or_endpoint() {
        let owner = crate::provider::context_owner("https://api.openai.com/v1/", "key-a");
        assert_eq!(
            owner,
            crate::provider::context_owner("https://api.openai.com/v1", "key-a")
        );
        assert_ne!(
            owner,
            crate::provider::context_owner("https://api.openai.com/v1", "key-b")
        );
        assert_ne!(
            owner,
            crate::provider::context_owner("https://other/v1", "key-a")
        );
        assert!(!owner.contains("key-a"));
    }
    #[test]
    fn native_checkpoint_roundtrips_without_textifying_encrypted_items() {
        let output = json!({"output":[{"type":"compaction","encrypted_content":"opaque"}]});
        let m = checkpoint(output.clone(), "https://api.openai.com/v1", "gpt-5").unwrap();
        let saved = serde_json::to_string(&m).unwrap();
        let restored = serde_json::from_str(&saved).unwrap();
        assert_eq!(
            input(
                &request(vec![restored, Message::user("next")]),
                "https://api.openai.com/v1"
            )
            .unwrap()[0],
            output["output"][0]
        );
        assert!(input(&request(vec![m]), "other").is_err());
        assert!(checkpoint(json!({"output":[]}), "owner", "model").is_err());
    }
    #[test]
    fn completed_tool_calls_and_usage_are_not_lost() {
        let events = decode(&json!({"type":"response.completed","response":{"output":[{"type":"reasoning","id":"rs1","encrypted_content":"opaque-thought","summary":[]},{"type":"function_call","call_id":"c1","name":"read_file","arguments":"{}"}],"usage":{"input_tokens":120,"output_tokens":20,"input_tokens_details":{"cached_tokens":100}}}})).unwrap();
        assert!(matches!(&events[0],ChatEvent::ToolCalls(calls) if calls.len()==1));
        assert!(matches!(
            events[1],
            ChatEvent::Usage(TokenUsage {
                prompt_tokens: 120,
                ..
            })
        ));
        assert!(matches!(
            events[3],
            ChatEvent::Done(FinishReason::ToolCalls)
        ));
        assert!(
            matches!(&events[2], ChatEvent::ProviderContext(c) if c.items[0]["encrypted_content"]=="opaque-thought")
        );
        assert!(decode(&json!({"type":"response.failed","response":{"error":"failed"}})).is_err());
    }
}
