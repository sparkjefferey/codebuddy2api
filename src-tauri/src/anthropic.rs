/// Anthropic Messages ↔ OpenAI Chat Completions adapter
/// Port of anthropic_adapter.py — request conversion + SSE projection.
use serde_json::{json, Value};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn rand_id(prefix: &str) -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("{prefix}{}", bytes.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

fn format_evt(event_type: &str, data: Value) -> String {
    let mut payload = json!({"type": event_type});
    if let Value::Object(map) = data {
        for (k, v) in map {
            payload[k] = v;
        }
    }
    format!(
        "event: {event_type}\ndata: {}\n\n",
        serde_json::to_string(&payload).unwrap_or_default()
    )
}

fn extract_system_text(system: &Value) -> String {    match system {
        Value::String(s) => s.clone(),
        Value::Array(arr) => arr
            .iter()
            .filter_map(|b| {
                if b.get("type").and_then(|v| v.as_str()) == Some("text") {
                    b.get("text").and_then(|v| v.as_str()).map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn extract_blocks_text(blocks: &[Value]) -> String {
    blocks
        .iter()
        .filter_map(|b| {
            if b.get("type").and_then(|v| v.as_str()) == Some("text") {
                b.get("text").and_then(|v| v.as_str()).map(|s| s.to_string())
            } else {
                None
            }
        })
        .collect()
}

fn convert_anthropic_tools(tools: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    for t in tools {
        if t.get("function").is_some() {
            out.push(t.clone());
            continue;
        }
        let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let mut func = json!({"name": name});
        if let Some(d) = t.get("description") {
            func["description"] = d.clone();
        }
        if let Some(schema) = t.get("input_schema") {
            func["parameters"] = schema.clone();
        }
        out.push(json!({"type": "function", "function": func}));
    }
    out
}

fn convert_anthropic_message(msg: &Value) -> Vec<Value> {
    let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
    let content = msg.get("content");

    if let Some(Value::String(s)) = content {
        return vec![json!({"role": role, "content": s})];
    }
    let Some(Value::Array(blocks)) = content else {
        return vec![];
    };
    if blocks.is_empty() {
        return vec![];
    }

    if role == "user" {
        let mut result = Vec::new();
        let mut text_parts: Vec<String> = Vec::new();
        for block in blocks {
            let bt = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match bt {
                "text" => {
                    if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                        text_parts.push(t.to_string());
                    }
                }
                "tool_result" => {
                    let tc_id = block.get("tool_use_id").and_then(|v| v.as_str()).unwrap_or("");
                    let output = block.get("content");
                    let text = match output {
                        Some(Value::Array(arr)) => arr
                            .iter()
                            .filter_map(|b| {
                                if b.get("type").and_then(|v| v.as_str()) == Some("text") {
                                    b.get("text").and_then(|v| v.as_str()).map(|s| s.to_string())
                                } else {
                                    None
                                }
                            })
                            .collect::<String>(),
                        Some(Value::String(s)) => s.clone(),
                        _ => block
                            .get("content")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                    };
                    result.push(json!({"role": "tool", "tool_call_id": tc_id, "content": text}));
                }
                _ => {}
            }
        }
        if !text_parts.is_empty() {
            result.insert(0, json!({"role": "user", "content": text_parts.join("")}));
        }
        return result;
    }

    if role == "assistant" {
        let mut text_parts: Vec<String> = Vec::new();
        let mut tool_calls: Vec<Value> = Vec::new();
        for block in blocks {
            let bt = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match bt {
                "text" => {
                    if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                        text_parts.push(t.to_string());
                    }
                }
                "tool_use" => {
                    let id = block
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| rand_id("call_"));
                    let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let input = block.get("input").cloned().unwrap_or(json!({}));
                    tool_calls.push(json!({
                        "id": id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": serde_json::to_string(&input).unwrap_or_default()
                        }
                    }));
                }
                _ => {}
            }
        }
        let mut msg_out = json!({"role": "assistant"});
        if text_parts.is_empty() {
            msg_out["content"] = Value::Null;
        } else {
            msg_out["content"] = Value::String(text_parts.join(""));
        }
        if !tool_calls.is_empty() {
            msg_out["tool_calls"] = Value::Array(tool_calls);
        }
        return vec![msg_out];
    }

    let text = extract_blocks_text(blocks);
    if text.is_empty() {
        vec![]
    } else {
        vec![json!({"role": role, "content": text})]
    }
}

// ---------------------------------------------------------------------------
// Public: Anthropic → Chat
// ---------------------------------------------------------------------------

pub fn anthropic_to_chat(body: &Value) -> Value {
    let mut messages: Vec<Value> = Vec::new();

    if let Some(system) = body.get("system") {
        let sys_text = extract_system_text(system);
        if !sys_text.is_empty() {
            messages.push(json!({"role": "system", "content": sys_text}));
        }
    }

    if let Some(Value::Array(msgs)) = body.get("messages") {
        for m in msgs {
            messages.extend(convert_anthropic_message(m));
        }
    }

    let mut chat = json!({"messages": messages, "stream": true});

    if let Some(model) = body.get("model") {
        chat["model"] = model.clone();
    }
    if let Some(v) = body.get("max_tokens") {
        chat["max_tokens"] = v.clone();
    }
    if let Some(Value::Array(tools)) = body.get("tools") {
        chat["tools"] = Value::Array(convert_anthropic_tools(tools));
    }
    if let Some(tc) = body.get("tool_choice") {
        match tc {
            Value::Object(map) => {
                let t = map.get("type").and_then(|v| v.as_str()).unwrap_or("any");
                let name = map.get("name").and_then(|v| v.as_str()).unwrap_or("");
                chat["tool_choice"] = json!({"type": t, "function": {"name": name}});
            }
            Value::String(s) => {
                if s == "none" || s == "auto" || s == "required" {
                    chat["tool_choice"] = Value::String(s.clone());
                } else {
                    chat["tool_choice"] = json!({"type": "function", "function": {"name": s}});
                }
            }
            _ => {}
        }
    }
    for key in ["temperature", "top_p", "stop", "top_k"] {
        if let Some(v) = body.get(key) {
            chat[key] = v.clone();
        }
    }
    chat
}

/// workbuddy.ai 后端要求 messages 首条必须是 system（否则 11128
/// "first message is not system prompt"）；CN 后端带 system 开头也实测正常。
/// 客户端未提供 system 时补一条中性兜底。
pub fn ensure_system_first(chat: &mut Value) {
    let first_is_system = chat
        .get("messages")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|m| m.get("role"))
        .and_then(|r| r.as_str())
        == Some("system");
    if !first_is_system {
        if let Some(a) = chat.get_mut("messages").and_then(|v| v.as_array_mut()) {
            a.insert(
                0,
                json!({"role": "system", "content": "You are a helpful assistant."}),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// AnthropicStreamConverter — Chat SSE → Anthropic SSE
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct ToolSlot {
    id: String,
    name: String,
    args: String,
    block_idx: usize,
    open: bool,
}

pub struct AnthropicStreamConverter {
    msg_id: String,
    model: String,
    emitted_start: bool,
    text_content: String,
    text_block_open: bool,
    text_block_idx: usize,
    tool_uses: HashMap<usize, ToolSlot>,
    next_block_idx: usize,
    finish_reason: Option<String>,
    usage: Option<Value>,
}

impl AnthropicStreamConverter {
    pub fn new(model: &str) -> Self {
        Self {
            msg_id: rand_id("msg_"),
            model: model.to_string(),
            emitted_start: false,
            text_content: String::new(),
            text_block_open: false,
            text_block_idx: 0,
            tool_uses: HashMap::new(),
            next_block_idx: 0,
            finish_reason: None,
            usage: None,
        }
    }

    fn evt(&self, event_type: &str, data: Value) -> String {
        format_evt(event_type, data)
    }

    pub fn feed_line(&mut self, line: &str) -> String {
        let line = line.trim();
        if line.is_empty() || !line.starts_with("data:") {
            return String::new();
        }
        let data = line[5..].trim();
        if data == "[DONE]" {
            return String::new();
        }
        let chunk: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => return String::new(),
        };
        self.process_chunk(&chunk)
    }

    fn process_chunk(&mut self, chunk: &Value) -> String {
        let mut events = String::new();

        if let Some(m) = chunk.get("model").and_then(|v| v.as_str()) {
            self.model = m.to_string();
        }

        if !self.emitted_start {
            events.push_str(&self.evt(
                "message_start",
                json!({
                    "message": {
                        "id": self.msg_id,
                        "type": "message",
                        "role": "assistant",
                        "content": [],
                        "model": self.model,
                        "usage": {"input_tokens": 0, "output_tokens": 0}
                    }
                }),
            ));
            self.emitted_start = true;
        }

        if let Some(usage) = chunk.get("usage") {
            self.usage = Some(usage.clone());
        }

        let choices = chunk.get("choices").and_then(|v| v.as_array());
        let Some(choices) = choices else {
            return events;
        };

        for choice in choices {
            let delta = choice.get("delta");
            let finish = choice.get("finish_reason").and_then(|v| v.as_str());

            if let Some(content) = delta.and_then(|d| d.get("content")).and_then(|v| v.as_str()) {
                if !content.is_empty() {
                    self.text_content.push_str(content);
                    if !self.text_block_open {
                        self.text_block_idx = self.next_block_idx;
                        self.next_block_idx += 1;
                        events.push_str(&self.evt(
                            "content_block_start",
                            json!({"index": self.text_block_idx, "content_block": {"type": "text", "text": ""}}),
                        ));
                        self.text_block_open = true;
                    }
                    events.push_str(&self.evt(
                        "content_block_delta",
                        json!({"index": self.text_block_idx, "delta": {"type": "text_delta", "text": content}}),
                    ));
                }
            }

            if let Some(Value::Array(tool_calls)) = delta.and_then(|d| d.get("tool_calls")) {
                for tc in tool_calls {
                    let idx = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                    if !self.tool_uses.contains_key(&idx) {
                        let block_idx = self.next_block_idx;
                        self.next_block_idx += 1;
                        self.tool_uses.insert(
                            idx,
                            ToolSlot {
                                id: tc.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                                name: String::new(),
                                args: String::new(),
                                block_idx,
                                open: false,
                            },
                        );
                    }
                    let slot = self.tool_uses.get_mut(&idx).unwrap();
                    if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                        if !id.is_empty() {
                            slot.id = id.to_string();
                        }
                    }
                    if let Some(name) = tc
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|v| v.as_str())
                    {
                        if !name.is_empty() {
                            slot.name = name.to_string();
                        }
                    }
                    // Copy needed fields, release borrow before calling evt
                    let (block_idx, t_id, t_name, already_open) =
                        (slot.block_idx, slot.id.clone(), slot.name.clone(), slot.open);
                    if !already_open {
                        events.push_str(&format_evt(
                            "content_block_start",
                            json!({"index": block_idx, "content_block": {"type": "tool_use", "id": t_id, "name": t_name, "input": {}}}),
                        ));
                        slot.open = true;
                    }
                    if let Some(args) = tc
                        .get("function")
                        .and_then(|f| f.get("arguments"))
                        .and_then(|v| v.as_str())
                    {
                        if !args.is_empty() {
                            slot.args.push_str(args);
                            events.push_str(&format_evt(
                                "content_block_delta",
                                json!({"index": block_idx, "delta": {"type": "input_json_delta", "partial_json": args}}),
                            ));
                        }
                    }
                }
            }

            if let Some(finish) = finish {
                if !finish.is_empty() {
                    self.finish_reason = Some(finish.to_string());
                    if self.text_block_open {
                        events.push_str(&self.evt("content_block_stop", json!({"index": self.text_block_idx})));
                        self.text_block_open = false;
                    }
                    let indices: Vec<usize> = self.tool_uses.keys().copied().collect();
                    for idx in indices {
                        let (bi, was_open) = {
                            let slot = self.tool_uses.get_mut(&idx).unwrap();
                            (slot.block_idx, slot.open)
                        };
                        if was_open {
                            events.push_str(&format_evt("content_block_stop", json!({"index": bi})));
                            self.tool_uses.get_mut(&idx).unwrap().open = false;
                        }
                    }
                }
            }
        }

        events
    }

    pub fn finish(&mut self) -> String {
        let mut events = String::new();

        if self.text_block_open {
            events.push_str(&self.evt("content_block_stop", json!({"index": self.text_block_idx})));
            self.text_block_open = false;
        }
        let indices: Vec<usize> = self.tool_uses.keys().copied().collect();
        for idx in indices {
            let (bi, was_open) = {
                let slot = self.tool_uses.get_mut(&idx).unwrap();
                (slot.block_idx, slot.open)
            };
            if was_open {
                events.push_str(&format_evt("content_block_stop", json!({"index": bi})));
                self.tool_uses.get_mut(&idx).unwrap().open = false;
            }
        }

        let sr = self.finish_reason.as_deref().unwrap_or("stop");
        let stop_reason = match sr {
            "tool_calls" => "tool_use",
            "length" => "max_tokens",
            _ => "end_turn",
        };

        let usage = self.usage.as_ref().map(|u| {
            json!({
                "input_tokens": u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                "output_tokens": u.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0)
            })
        });

        events.push_str(&self.evt(
            "message_delta",
            json!({"delta": {"stop_reason": stop_reason, "stop_sequence": null}, "usage": usage}),
        ));
        events.push_str(&self.evt("message_stop", json!({})));
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_system_string() {
        let body = json!({"system": "you are helpful", "messages": [{"role": "user", "content": "hi"}]});
        let chat = anthropic_to_chat(&body);
        let msgs = chat["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "you are helpful");
    }
    #[test]
    fn test_tool_conversion() {
        let body = json!({
            "model": "hy3",
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"name": "bash", "description": "run", "input_schema": {"type": "object"}}]
        });
        let chat = anthropic_to_chat(&body);
        let tools = chat["tools"].as_array().unwrap();
        assert_eq!(tools[0]["function"]["name"], "bash");
    }

    #[test]
    fn ensure_system_first_prepends_when_missing() {
        let mut chat = json!({"messages": [{"role": "user", "content": "hi"}]});
        ensure_system_first(&mut chat);
        assert_eq!(chat["messages"][0]["role"], "system");
        assert_eq!(chat["messages"][1]["role"], "user");

        let mut chat2 = json!({"messages": [
            {"role": "system", "content": "s"}, {"role": "user", "content": "hi"}
        ]});
        ensure_system_first(&mut chat2);
        assert_eq!(chat2["messages"][0]["content"], "s");
    }
}
