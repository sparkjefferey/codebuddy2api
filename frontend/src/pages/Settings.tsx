import { useEffect, useState } from "react";
import { getConfig, getApiKey, toggleDesensitize, getVersion } from "../lib/tauri";

export default function Settings() {
  const [config, setConfig] = useState<Record<string, unknown>>({});
  const [apiKey, setApiKey] = useState("");
  const [revealed, setRevealed] = useState(false);
  const [desensitize, setDesensitize] = useState(false);
  const [version, setVersion] = useState("");

  useEffect(() => {
    getConfig().then((c) => {
      setConfig(c);
      setDesensitize(Boolean(c.desensitize));
    }).catch(() => {});
    getApiKey().then(setApiKey).catch(() => {});
    getVersion().then(setVersion).catch(() => {});
  }, []);

  async function onToggle(next: boolean) {
    setDesensitize(next);
    await toggleDesensitize(next);
  }

  const cardStyle = {
    background: "var(--bg-panel)",
    border: "1px solid var(--line)",
    borderRadius: "var(--radius)",
    padding: "18px 20px",
  } as const;

  return (
    <div style={{ display: "grid", gap: 16 }}>
      <h2 style={{ margin: 0 }}>设置</h2>

      <div style={cardStyle}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <div>
            <div style={{ fontSize: 13.5, fontWeight: 600 }}>Prompt 脱敏</div>
            <div style={{ color: "var(--text-mid)", fontSize: 12.5, marginTop: 2 }}>
              对 system 模板中的合规声明高频词插入零宽空格，缓解上游内容审核误伤（默认关闭）
            </div>
          </div>
          <button
            onClick={() => onToggle(!desensitize)}
            style={{
              width: 44, height: 24, borderRadius: 12,
              border: "none", position: "relative",
              background: desensitize ? "var(--accent)" : "var(--line)",
              transition: "background 0.2s",
            }}
            aria-pressed={desensitize}
            aria-label="切换脱敏"
          >
            <span style={{
              position: "absolute", top: 3, left: desensitize ? 23 : 3,
              width: 18, height: 18, borderRadius: "50%", background: "#fff",
              transition: "left 0.2s",
            }} />
          </button>
        </div>
      </div>

      <div style={cardStyle}>
        <div style={{ fontSize: 13.5, fontWeight: 600, marginBottom: 6 }}>本地 API Key</div>
        <div style={{ display: "flex", gap: 10, alignItems: "center" }}>
          <code style={{
            fontSize: 12.5, fontFamily: "var(--font-mono)", background: "var(--bg-base)",
            padding: "7px 12px", borderRadius: "var(--radius-sm)", border: "1px solid var(--line)",
            flex: 1, overflow: "auto", whiteSpace: "nowrap",
          }}>
            {revealed ? apiKey : apiKey.replace(/^(sk-buddy-.{4}).*(.{2})$/, "$1••••••••••••$2")}
          </code>
          <button
            onClick={() => setRevealed((v) => !v)}
            style={{
              padding: "6px 12px", borderRadius: "var(--radius-sm)",
              border: "1px solid var(--line)", background: "transparent",
              color: "var(--text-hi)", fontSize: 12,
            }}
          >
            {revealed ? "隐藏" : "显示"}
          </button>
          <button
            onClick={() => navigator.clipboard.writeText(apiKey)}
            style={{
              padding: "6px 12px", borderRadius: "var(--radius-sm)",
              border: "1px solid var(--line)", background: "transparent",
              color: "var(--text-hi)", fontSize: 12,
            }}
          >
            复制
          </button>
        </div>
      </div>

      <div style={cardStyle}>
        <div style={{ color: "var(--text-dim)", fontSize: 12, marginBottom: 8 }}>关于</div>
        <div style={{ fontSize: 13 }}>
          BuddyAIGateway <span style={{ color: "var(--text-mid)" }}>v{version || "1.0.0"}</span>
        </div>
        <pre style={{
          margin: "10px 0 0", fontSize: 11.5, color: "var(--text-dim)",
          fontFamily: "var(--font-mono)", whiteSpace: "pre-wrap",
        }}>
          {JSON.stringify(config, null, 2)}
        </pre>
      </div>
    </div>
  );
}