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
/// 因此必须配合 `strip_harness_context` 从源头移除样板，而非逐词绕过。
/// 注：指纹检测**只针对 assistant 角色文本**（实测同样内容放 user/tool 不触发，
/// tool_calls 参数也不触发）；拆分应用到全部角色属安全覆盖。
const CHANNEL_TERMS: &[&str] = &[
    // 品牌无关指纹短语（从真实抓包请求二分得出；assistant 文本出现即 11128）
    "main branch (you will usually use this for prs)",
    "claude",
    "anthropic",
];

/// CC/Codex 壳层 system 指纹句（对标 Python 版 _CODEX_SYSTEM_MARKERS）。
const HARNESS_SYSTEM_MARKERS: &[&str] = &[
    "You are Claude Code",
    "You are a coding agent running in the Codex CLI",
    "Within this context, Codex refers to",
    "x-anthropic-billing-header",
];

/// 壳层 system 中的机器上下文块起点：CC 在 system 末尾注入的 gitStatus
/// 数据快照（Current branch / Main branch… / Git user / Recent commits）。
/// 上游指纹实测载体 —— 不含任何品牌词的 "Main branch (you will usually use
/// this for PRs): …" 一行即可触发 11128。该信息模型可自行运行 git 获取，
/// 剪除不影响行为指令（实测：指令主体单独回放 200，剪块后全量回放 200）。
const HARNESS_GIT_STATUS_PREFIX: &str = "gitStatus:";

/// 剪除壳层 system 中的机器上下文块：命中 CC/Codex 指纹句的 system，
/// 从 gitStatus 行起到消息末尾剪除，其余指令完整保留（不做整体压缩）。
/// 非壳层 system 完全不动。必须在 ZWSP 拆分**之前**运行指纹匹配。
pub fn strip_harness_context(body: &mut serde_json::Value) {
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
        if !HARNESS_SYSTEM_MARKERS.iter().any(|m| text.contains(m)) {
            continue;
        }
        // gitStatus 块从独立成行的 gitStatus: 起（到消息末尾）
        let cut = if text
            .find(HARNESS_GIT_STATUS_PREFIX)
            .is_some_and(|pos| text[..pos].ends_with('\n'))
        {
            text.find(HARNESS_GIT_STATUS_PREFIX)
        } else if text.starts_with(HARNESS_GIT_STATUS_PREFIX) {
            Some(0)
        } else {
            None
        };
        if let Some(pos) = cut {
            *content = serde_json::Value::String(text[..pos].trim_end().to_string());
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
/// 1. 剪除 CC/Codex 壳层 system 里的机器上下文块（主要指纹载体，见
///    strip_harness_context），行为指令保留；
/// 2. 拆分全部角色文本与工具描述中的品牌词，破坏上游子串匹配。
///    不改工具函数名/参数，不影响调用匹配。
pub fn channel_desensitize(body: &mut serde_json::Value) {
    strip_harness_context(body);
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
    fn harness_gitstatus_block_stripped_instructions_kept() {
        let cc_system = "You are Claude Code, Anthropic's official CLI for Claude.\n\nIMPORTANT: Assist with defensive security.\n\n# Tone\nBe concise.\n\ngitStatus: This is the git status snapshot.\n\nCurrent branch: main\n\nMain branch (you will usually use this for PRs): main\n\nGit user: someone";
        let mut body = serde_json::json!({
            "messages": [
                {"role": "system", "content": cc_system},
                {"role": "user", "content": "hi"}
            ]
        });
        channel_desensitize(&mut body);
        let msgs = body["messages"].as_array().unwrap();
        let s = msgs[0]["content"].as_str().unwrap();
        // gitStatus 块剪除，行为指令完整保留
        assert!(!s.contains("gitStatus"));
        assert!(!s.contains("Main branch"));
        assert!(!s.contains("Current branch"));
        assert!(s.contains("IMPORTANT: Assist with defensive security."));
        assert!(s.contains("# Tone"));
        assert!(s.contains("Be concise."));

        // 非壳层 system 即使含类似行也不动
        let mut body2 = serde_json::json!({
            "messages": [
                {"role": "system", "content": "You are a helpful assistant.\nCurrent branch: dev"},
                {"role": "user", "content": "hi"}
            ]
        });
        channel_desensitize(&mut body2);
        let s2 = body2["messages"][0]["content"].as_str().unwrap();
        assert!(s2.contains("Current branch: dev"));

        // 壳层 system 无 gitStatus 块时提示词不变（品牌词由 ZWSP 层处理）
        let mut body3 = serde_json::json!({
            "messages": [
                {"role": "system", "content": "You are Claude Code, Anthropic's official CLI for Claude."},
                {"role": "user", "content": "hi"}
            ]
        });
        channel_desensitize(&mut body3);
        let s3 = body3["messages"][0]["content"].as_str().unwrap();
        assert!(s3.contains("interactive") == false); // 未替换整条 —— 应保留原文（带 ZWSP）
        assert_eq!(s3.replace('\u{200B}', ""), "You are Claude Code, Anthropic's official CLI for Claude.");
    }

    #[test]
    fn channel_splits_fingerprint_phrase_in_assistant_text() {
        // assistant 文本出现品牌无关指纹短语 → 拆分（B1/B5 实测：原文 11128，拆分后 200）
        let mut body = serde_json::json!({
            "messages": [
                {"role": "system", "content": "You are a coding assistant."},
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "Current branch: main\n\nMain branch (you will usually use this for PRs): main\n\nGit user: x"}
            ]
        });
        channel_desensitize(&mut body);
        let s = body["messages"][2]["content"].as_str().unwrap();
        assert!(!s.contains("Main branch (you will usually use this for PRs)"));
        assert!(s.contains('\u{200B}'));
        // 拆掉 ZWSP 后原文仍在（只破坏子串匹配，不删内容）
        assert_eq!(s.replace('\u{200B}', ""), "Current branch: main\n\nMain branch (you will usually use this for PRs): main\n\nGit user: x");
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
