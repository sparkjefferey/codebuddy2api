import { useEffect, useState } from "react";
import {
  clearCredential,
  getCredentialStatus,
  importCredential,
} from "../lib/tauri";
import type { CredentialStatus } from "../lib/tauri";

export default function Account() {
  const [cred, setCred] = useState<CredentialStatus | null>(null);
  const [jsonText, setJsonText] = useState("");
  const [msg, setMsg] = useState("");

  async function refresh() {
    setCred(await getCredentialStatus().catch(() => null));
  }
  useEffect(() => { refresh(); }, []);

  async function doImport() {
    setMsg("");
    try {
      await importCredential(jsonText);
      setMsg("✅ 导入成功");
      setJsonText("");
      refresh();
    } catch (e) {
      setMsg(`❌ ${e}`);
    }
  }

  async function doClear() {
    await clearCredential();
    setMsg("已清除本地凭据");
    refresh();
  }

  const cardStyle = {
    background: "var(--bg-panel)",
    border: "1px solid var(--line)",
    borderRadius: "var(--radius)",
    padding: "18px 20px",
  } as const;
  const inputStyle = {
    width: "100%",
    padding: "8px 10px",
    background: "var(--bg-base)",
    border: "1px solid var(--line)",
    borderRadius: "var(--radius-sm)",
    color: "var(--text-hi)",
    fontSize: 13,
    fontFamily: "var(--font-mono)",
  } as const;

  return (
    <div style={{ display: "grid", gap: 16 }}>
      <h2 style={{ margin: 0 }}>账号</h2>

      <div style={cardStyle}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <div>
            <div style={{ color: "var(--text-dim)", fontSize: 12 }}>当前账号</div>
            {cred?.configured ? (
              <div style={{ marginTop: 6 }}>
                <strong>{cred.nickname || "已登录"}</strong>
                <span style={{ color: "var(--text-mid)", marginLeft: 10, fontSize: 12.5 }}>
                  {cred.domain}
                </span>
              </div>
            ) : (
              <div style={{ marginTop: 6, color: "var(--warn)" }}>未导入</div>
            )}
          </div>
          {cred?.configured && (
            <button
              onClick={doClear}
              style={{
                padding: "6px 14px", borderRadius: "var(--radius-sm)",
                border: "1px solid var(--err)", background: "transparent",
                color: "var(--err)", fontSize: 12.5,
              }}
            >
              清除凭据
            </button>
          )}
        </div>
      </div>

      <div style={cardStyle}>
        <h3 style={{ margin: "0 0 6px", fontSize: 14 }}>导入登录态</h3>
        <p style={{ color: "var(--text-mid)", fontSize: 12.5, margin: "0 0 12px" }}>
          粘贴完整登录态 JSON（桌面端 <code>auth/*.info</code> 文件内容，或单独字段）。导入后网关将用于刷新与调用上游。
        </p>
        <textarea
          value={jsonText}
          onChange={(e) => setJsonText(e.target.value)}
          rows={9}
          placeholder={`{"auth": {"accessToken": "...", "refreshToken": "...", "expiresAt": 0, "domain": "www.codebuddy.cn"}, "account": {"uid": "...", "nickname": "..."}}`}
          style={{ ...inputStyle, resize: "vertical" }}
        />
        <div style={{ display: "flex", gap: 10, marginTop: 12, alignItems: "center" }}>
          <button
            onClick={doImport}
            disabled={!jsonText.trim()}
            style={{
              padding: "7px 18px", borderRadius: "var(--radius-sm)", border: "none",
              background: "var(--accent)", color: "#fff", fontSize: 13,
              opacity: jsonText.trim() ? 1 : 0.5,
            }}
          >
            导入
          </button>
          {msg && <span style={{ fontSize: 12.5, color: msg.startsWith("❌") ? "var(--err)" : "var(--ok)" }}>{msg}</span>}
        </div>
      </div>
    </div>
  );
}