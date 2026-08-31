import { useEffect, useState } from "react";
import { buildCcswitchLink, getApiKey, openCcswitchLink, GATEWAY } from "../lib/tauri";

export default function ClaudeCode() {
  const [apiKey, setApiKey] = useState("");
  const [model, setModel] = useState("hy3");
  const [link, setLink] = useState("");

  useEffect(() => {
    getApiKey().then(setApiKey).catch(() => {});
  }, []);

  async function makeLink(launch: boolean) {
    const url = await buildCcswitchLink(
      GATEWAY, "BuddyAIGateway", apiKey || "workbuddy", model || undefined,
    );
    setLink(url);
    if (launch) openCcswitchLink(url);
  }

  async function testGateway() {
    try {
      const r = await fetch(GATEWAY + "/agents/test", {
        method: "POST",
        headers: { "Content-Type": "application/json", "X-Api-Key": apiKey },
        body: JSON.stringify({ model, prompt: "你好,请只回复两个字:OK" }),
      }).then((r) => r.json());
      alert(r?.ok ? `✅ 连通正常\n模型: ${r.model}\n响应: ${r.content_preview?.slice(0, 80)}` : `❌ ${r?.error || "失败"}`);
    } catch (e) {
      alert(`❌ ${e}`);
    }
  }

  const cardStyle = {
    background: "var(--bg-panel)",
    border: "1px solid var(--line)",
    borderRadius: "var(--radius)",
    padding: "18px 20px",
  } as const;

  const btnPrimary = {
    padding: "8px 18px", borderRadius: "var(--radius-sm)", border: "none",
    background: "var(--accent)", color: "#fff", fontSize: 13,
  } as const;
  const btnGhost = {
    padding: "8px 16px", borderRadius: "var(--radius-sm)", border: "1px solid var(--line)",
    background: "transparent", color: "var(--text-hi)", fontSize: 13,
  } as const;

  return (
    <div style={{ display: "grid", gap: 16 }}>
      <h2 style={{ margin: 0 }}>Claude Code 接入</h2>

      <div style={cardStyle}>
        <h3 style={{ margin: "0 0 4px", fontSize: 14 }}>CC Switch 一键注册</h3>
        <p style={{ color: "var(--text-mid)", fontSize: 12.5, margin: "0 0 14px" }}>
          生成 <code>ccswitch://</code> 导入链接，由 CC Switch 将本网关注册为 Claude Code 的 Anthropic 上游。
        </p>
        <div style={{ display: "flex", gap: 10, alignItems: "center", flexWrap: "wrap" }}>
          <label style={{ fontSize: 13, color: "var(--text-mid)" }}>
            默认模型
            <input
              value={model}
              onChange={(e) => setModel(e.target.value)}
              style={{
                marginLeft: 8, width: 140, padding: "6px 10px",
                background: "var(--bg-base)", border: "1px solid var(--line)",
                borderRadius: "var(--radius-sm)", color: "var(--text-hi)", fontSize: 12.5,
                fontFamily: "var(--font-mono)",
              }}
            />
          </label>
          <button onClick={() => makeLink(true)} style={btnPrimary}>注册到 CC Switch</button>
          <button onClick={() => makeLink(false)} style={btnGhost}>仅生成链接</button>
          <button onClick={testGateway} style={btnGhost}>连通测试</button>
        </div>
        {link && (
          <pre style={{
            marginTop: 14, padding: "10px 12px", background: "var(--bg-base)",
            border: "1px solid var(--line)", borderRadius: "var(--radius-sm)",
            fontSize: 11.5, overflow: "auto", color: "var(--text-mid)",
            fontFamily: "var(--font-mono)", wordBreak: "break-all",
          }}>
            {link}
          </pre>
        )}
      </div>

      <div style={cardStyle}>
        <h3 style={{ margin: "0 0 4px", fontSize: 14 }}>手动配置（环境变量）</h3>
        <p style={{ color: "var(--text-mid)", fontSize: 12.5, margin: "0 0 10px" }}>
          不使用 CC Switch 时，可手动为 Claude Code 设置：
        </p>
        <pre style={{
          margin: 0, padding: "12px 14px", background: "var(--bg-base)",
          border: "1px solid var(--line)", borderRadius: "var(--radius-sm)",
          fontSize: 12, color: "var(--text-mid)", overflow: "auto",
          fontFamily: "var(--font-mono)",
        }}>
{`export ANTHROPIC_BASE_URL=http://127.0.0.1:9178
export ANTHROPIC_API_KEY=${apiKey || "<你的网关 API key>"}
export ANTHROPIC_MODEL=${model || "hy3"}

claude`}
        </pre>
      </div>
    </div>
  );
}