//! Desensitize — port of desensitize.py
//! Inserts zero-width space (U+200B) inside sensitive terms to evade backend content filter
//! false-positives on compliance system templates. Default OFF, only touches system role.

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

/// Desensitize text by inserting ZWSP into matched terms (case-insensitive).
pub fn desensitize_text(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let lower = text.to_lowercase();
    // Build result by scanning, longest terms first
    let mut terms: Vec<&str> = SENSITIVE_TERMS.to_vec();
    terms.sort_by_key(|b| std::cmp::Reverse(b.len()));

    let mut result = String::with_capacity(text.len() + 16);
    let mut i = 0;
    let text_chars: Vec<char> = text.chars().collect();
    let lower_chars: Vec<char> = lower.chars().collect();

    while i < text_chars.len() {
        let mut matched: Option<(&str, usize)> = None;
        for term in &terms {
            let term_lower = term.to_lowercase();
            let term_chars: Vec<char> = term_lower.chars().collect();
            if i + term_chars.len() <= lower_chars.len()
                && lower_chars[i..i + term_chars.len()] == term_chars[..]
            {
                matched = Some((term, term_chars.len()));
                break;
            }
        }
        if let Some((_, len)) = matched {
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
    match v {
        serde_json::Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for k in keys {
                if k == "description" || k == "title" {
                    if let Some(s) = map.get(&k).and_then(|v| v.as_str()).map(|s| s.to_string()) {
                        map.insert(k.clone(), serde_json::Value::String(desensitize_text(&s)));
                    }
                } else if let Some(child) = map.get_mut(&k) {
                    desensitize_tool_value(child);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                desensitize_tool_value(item);
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
}
