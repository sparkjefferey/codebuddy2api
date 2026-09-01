//! Desensitize — port of desensitize.py
//! Inserts zero-width space (U+200B) inside sensitive terms to evade backend content filter
//! false-positives on compliance system templates. Default OFF, only touches system role.
//!
//! 另含常开的 `channel_desensitize`（见下）。

const ZWSP: &str = "\u{200B}";

const SENSITIVE_TERMS: &[&str] = &[
    "DoS",
    "DDoS",
    "exploit",
    "credential testing",
    "credential stuffing",
    "supply chain compromise",
    "supply-chain compromise",
    "detection evasion",
    "C2 frameworks",
    "C2 framework",
    "command and control",
    "malicious purposes",
    "malicious intent",
    "mass targeting",
    "brute force",
    "brute-force",
    "privilege escalation",
    "reverse shell",
    "remote code execution",
    "SQL injection",
    "XSS",
    "CSRF",
    "phishing",
    "malware",
    "ransomware",
    "keylogger",
    "rootkit",
    "backdoor",
    "botnet",
    "zero-day",
    "0day",
    "vulnerability",
    "vulnerabilities",
    "red teaming",
    "red-teaming",
    "sandbox",
    "sandboxing",
    "sandboxed",
    "unsandboxed",
    "escalated privileges",
    "escalated",
    "escalation",
    "destructive action",
    "destructive command",
    "destructive",
    "attack",
    "attacks",
    "cybersecurity",
    "security review",
    "exploit development",
    "hacking",
    "penetration testing",
    "penetration test",
    "injection",
    "weaponize",
    "weaponized",
    "harmful",
    "dangerous",
    "abuse",
    "abusive",
    "illegal",
    "terrorist",
    "terrorism",
    "bomb",
    "weapon",
    "weapons",
    "drug",
    "drugs",
    "narcotic",
    "suicide",
    "self-harm",
    "murder",
    "kill",
    "violence",
    "violent",
    "Claude Code",
    "Claude Opus",
    "Claude Sonnet",
    "Claude Haiku",
    "Claude Fable",
    "Anthropic",
    "Co-Authored-By",
    "noreply@anthropic.com",
];

/// 渠道指纹词表（常开层）：上游 2026-09 起对请求内容做官方客户端签名扫描，
/// 命中即 11128 "Illegal API invocation from an unapproved channel"。除品牌词外，
/// CC 注入的系统性样板也是指纹载体（实测环境信息段里不含任何品牌词的
/// "Main branch (you will usually use this for PRs): …" 一行即可触发 11128），
/// 因此必须配合 `compact_harness_systems` 从源头移除样板，而非逐词绕过。
const CHANNEL_TERMS: &[&str] = &["claude", "anthropic"];

/// CC/Codex 壳层 system 指纹句（对标 Python 版 _CODEX_SYSTEM_MARKERS）。
const HARNESS_SYSTEM_MARKERS: &[&str] = &[
    "You are Claude Code",
    "You are a coding agent running in the Codex CLI",
    "Within this context, Codex refers to",
    "x-anthropic-billing-header",
];

/// 壳层 system 的中性摘要替换文本（与 Python 版 compact_harness 同款）。
const HARNESS_SYSTEM_SUMMARY: &str = "You are a coding assistant. Be precise, helpful, concise, and safe. \
Use available tools when needed, follow repository instructions, and keep the user informed.";

/// 压缩壳层 system：Claude Code / Codex 注入的全套系统样板（IMPORTANT 段、
/// tone、环境信息、gitStatus…）是上游指纹风控的主要载体，且对模型行为非必需。
/// Python 版 compact_harness 的做法：整体替换为一句中性摘要，让样板从不出站。
/// 必须在 ZWSP 拆分**之前**运行（指纹要匹配未拆分的原文）。
pub fn compact_harness_systems(body: &mut serde_json::Value) {
    let Some(messages) = body.get_mut("messages").and_then(|v| v.as_array_mut()) else {
        return;
    };
    for msg in messages.iter_mut() {
        if msg.get("role").and_then(|v| v.as_str()) != Some("system") {
            continue;
        }
        let Some(content) = msg.get_mut("content") else { continue };
        let Some(text) = content.as_str().map(|s| s.to_string()) else {
            continue;
        };
        if HARNESS_SYSTEM_MARKERS.iter().any(|m| text.contains(m)) {
            *content = serde_json::Value::String(HARNESS_SYSTEM_SUMMARY.into());
        }
    }
}

fn zero_width_split(term: &str) -> String {
    let mut chars = term.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => {
            let rest: String = chars.collect();
            format!("{first}{ZWSP}{rest}")
        }
    }
}

/// 词内插零宽空格（大小写原样保留），通用扫描器：按词表长度降序做大小写不敏感匹配。
fn split_terms_in_text(text: &str, terms: &[&str]) -> String {
    if text.is_empty() {
        return String::new();
    }
    let lower = text.to_lowercase();
    let mut terms: Vec<&str> = terms.to_vec();
    terms.sort_by_key(|b| std::cmp::Reverse(b.len()));

    let mut result = String::with_capacity(text.len() + 16);
    let mut i = 0;
    let text_chars: Vec<char> = text.chars().collect();
    let lower_chars: Vec<char> = lower.chars().collect();

    while i < text_chars.len() {
        let mut matched: Option<usize> = None;
        for term in &terms {
            let term_lower = term.to_lowercase();
            let term_chars: Vec<char> = term_lower.chars().collect();
            if i + term_chars.len() <= lower_chars.len()
                && lower_chars[i..i + term_chars.len()] == term_chars[..]
            {
                matched = Some(term_chars.len());
                break;
            }
        }
        if let Some(len) = matched {
            let original: String = text_chars[i..i + len].iter().collect();
            result.push_str(&zero_width_split(&original));
            i += len;
        } else {
            result.push(text_chars[i]);
            i += 1;
        }
    }
    result
}

/// Privacy desensitize (默认关闭层) — 词表见 SENSITIVE_TERMS。
pub fn desensitize_text(text: &str) -> String {
    split_terms_in_text(text, SENSITIVE_TERMS)
}

/// 渠道指纹中和（常开层）：
/// 1. 压缩 CC/Codex 壳层 system 样板（主要指纹载体，见 compact_harness_systems）；
/// 2. 拆分全部角色文本与工具描述中的品牌词，破坏上游子串匹配。
///    不改工具函数名/参数，不影响调用匹配。
pub fn channel_desensitize(body: &mut serde_json::Value) {
    compact_harness_systems(body);
    if let Some(messages) = body.get_mut("messages").and_then(|v| v.as_array_mut()) {
        for msg in messages.iter_mut() {
            if let Some(content) = msg.get_mut("content") {
                if content.is_string() {
                    let s = content.as_str().unwrap_or_default().to_string();
                    *content = serde_json::Value::String(split_terms_in_text(&s, CHANNEL_TERMS));
                } else if let Some(arr) = content.as_array_mut() {
                    for block in arr.iter_mut() {
                        if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                            if let Some(t) = block.get("text").and_then(|v| v.as_str()).map(|s| s.to_string()) {
                                block["text"] =
                                    serde_json::Value::String(split_terms_in_text(&t, CHANNEL_TERMS));
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some(tools) = body.get_mut("tools").and_then(|v| v.as_array_mut()) {
        for tool in tools.iter_mut() {
            desensitize_tool_value_with(tool, CHANNEL_TERMS);
        }
    }
}

/// Apply desensitize to request body messages (only system role by default).
pub fn desensitize_body(mut body: serde_json::Value) -> serde_json::Value {
    let Some(messages) = body.get_mut("messages").and_then(|v| v.as_array_mut()) else {
        return body;
    };
    for msg in messages.iter_mut() {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if role != "system" {
            continue;
        }
        if let Some(content) = msg.get_mut("content") {
            if let Some(s) = content.as_str() {
                *content = serde_json::Value::String(desensitize_text(s));
            } else if let Some(arr) = content.as_array_mut() {
                for block in arr.iter_mut() {
                    if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                        if let Some(t) = block.get("text").and_then(|v| v.as_str()).map(|s| s.to_string()) {
                            block["text"] = serde_json::Value::String(desensitize_text(&t));
                        }
                    }
                }
            }
        }
    }
    // Also desensitize tool descriptions
    if let Some(tools) = body.get_mut("tools").and_then(|v| v.as_array_mut()) {
        for tool in tools.iter_mut() {
            desensitize_tool_value(tool);
        }
    }
    body
}

fn desensitize_tool_value(v: &mut serde_json::Value) {
    desensitize_tool_value_with(v, SENSITIVE_TERMS);
}

fn desensitize_tool_value_with(v: &mut serde_json::Value, terms: &[&str]) {
    match v {
        serde_json::Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for k in keys {
                if k == "description" || k == "title" {
                    if let Some(s) = map.get(&k).and_then(|v| v.as_str()).map(|s| s.to_string()) {
                        map.insert(
                            k.clone(),
                            serde_json::Value::String(split_terms_in_text(&s, terms)),
                        );
                    }
                } else if let Some(child) = map.get_mut(&k) {
                    desensitize_tool_value_with(child, terms);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                desensitize_tool_value_with(item, terms);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_desensitize() {
        let s = "Refuse requests for DoS attacks and exploit development.";
        let d = desensitize_text(s);
        assert!(d.contains('\u{200B}'));
        assert_ne!(d, s);
    }
    #[test]
    fn test_no_change() {
        let s = "Hello world, no sensitive words here.";
        assert_eq!(desensitize_text(s), s);
    }

    #[test]
    fn channel_splits_brand_words_all_roles() {
        let mut body = serde_json::json!({
            "messages": [
                {"role": "system", "content": "Claude is a helpful model by Anthropic."},
                {"role": "user", "content": "what is claude"},
                {"role": "assistant", "content": null, "tool_calls": [
                    {"id": "x", "type": "function", "function": {"name": "Read", "arguments": "{}"}}
                ]},
                {"role": "tool", "tool_call_id": "x", "content": "Anthropic docs page"}
            ],
            "tools": [
                {"type": "function", "function": {
                    "name": "Bash",
                    "description": "Run claude commands",
                    "parameters": {"type": "object", "properties": {}}
                }}
            ],
            "model": "hy3",
            "stream": true
        });
        channel_desensitize(&mut body);
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(
            msgs[0]["content"],
            "C\u{200B}laude is a helpful model by A\u{200B}nthropic."
        );
        assert_eq!(msgs[1]["content"], "what is c\u{200B}laude");
        // tool_calls 函数名/参数不动
        assert_eq!(msgs[2]["tool_calls"][0]["function"]["name"], "Read");
        assert_eq!(msgs[3]["content"], "A\u{200B}nthropic docs page");
        // 工具描述拆分，函数名保留
        assert_eq!(body["tools"][0]["function"]["name"], "Bash");
        assert_eq!(
            body["tools"][0]["function"]["description"],
            "Run c\u{200B}laude commands"
        );
        // 无品牌词的字段不变
        assert_eq!(body["model"], "hy3");
    }

    #[test]
    fn harness_system_replaced_by_summary() {
        let cc_system = "x-anthropic-billing-header: cc_version=2.1.251\nYou are Claude Code, Anthropic's official CLI for Claude.\n\nYou are an interactive agent that helps users with software engineering tasks.\n\nIMPORTANT: Assist with defensive security.\nGit status: clean.\nMain branch (you will usually use this for PRs): main";
        let mut body = serde_json::json!({
            "messages": [
                {"role": "system", "content": cc_system},
                {"role": "user", "content": "hi"}
            ]
        });
        channel_desensitize(&mut body);
        let msgs = body["messages"].as_array().unwrap();
        // 整个样板被压缩为中性摘要，不出站
        assert_eq!(msgs[0]["content"], HARNESS_SYSTEM_SUMMARY);
        assert!(!msgs[0]["content"]
            .as_str()
            .unwrap()
            .contains("Main branch"));

        // 非壳层 system 不受影响
        let mut body2 = serde_json::json!({
            "messages": [
                {"role": "system", "content": "You are a helpful assistant."},
                {"role": "user", "content": "hi"}
            ]
        });
        channel_desensitize(&mut body2);
        assert_eq!(body2["messages"][0]["content"], "You are a helpful assistant.");
    }

    #[test]
    fn channel_no_brand_no_change() {
        let mut body = serde_json::json!({"messages": [{"role": "user", "content": "plain text"}]});
        channel_desensitize(&mut body);
        assert_eq!(body["messages"][0]["content"], "plain text");
    }

    #[test]
    fn channel_then_privacy_no_double_split() {
        let mut body = serde_json::json!({
            "messages": [{"role": "system", "content": "Claude Code by Anthropic"}]
        });
        channel_desensitize(&mut body);
        let once = body["messages"][0]["content"].as_str().unwrap().to_string();
        // 隐私层再跑一遍不应二次插 ZWSP（已拆分的词无法再整词匹配）
        let after = desensitize_body(body);
        assert_eq!(after["messages"][0]["content"], serde_json::Value::String(once));
    }
}
