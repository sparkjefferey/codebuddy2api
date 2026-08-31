import { useEffect, useState } from "react";
import { getCredentialStatus, getApiKey, GATEWAY } from "../lib/tauri";
import type { CredentialStatus } from "../lib/tauri";

export default function Dashboard() {
  const [cred, setCred] = useState<CredentialStatus | null>(null);
  const [credits, setCredits] = useState<string>("—");
  const [health, setHealth] = useState<Record<string, unknown> | null>(null);

  async function refresh() {
    setCred(await getCredentialStatus().catch(() => null));
    try {
      const h = await fetch(GATEWAY + "/health").then((r) => r.json());
      setHealth(h);
    } catch { /* offline */ }
    try {
      const key = await getApiKey();
      const r = await fetch(GATEWAY + "/credits", {
        headers: { "X-Api-Key": key },
      }).then((r) => r.json());
      if (r?.credits_remaining !== undefined) {
        setCredits(String(r.credits_remaining));
      } else if (r?.error) {
        setCredits("不可用");
      }
    } catch { setCredits("不可用"); }
  }

  useEffect(() => { refresh(); }, []);

  const accounts = cred?.accounts ?? [];
  const enabledCount = accounts.filter((a) => a.enabled).length;

  const cardStyle = {
    background: "var(--bg-panel)",
    border: "1px solid var(--line)",
    borderRadius: "var(--radius)",
    padding: "18px 20px",
  } as const;

  return (
    <div style={{ display: "grid", gap: 16, gridTemplateColumns: "minmax(0, 1fr)" }}>
      <h2 style={{ margin: 0 }}>总览</h2>
      <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(min(220px, 100%), 1fr))", gap: 14 }}>
        <div style={cardStyle}>
          <div style={{ color: "var(--text-dim)", fontSize: 12, marginBottom: 6 }}>网关状态</div>
          <div style={{ fontSize: 22, fontWeight: 700, fontFamily: "var(--font-mono)" }}>
            127.0.0.1:9178
          </div>
          <div style={{ color: "var(--text-mid)", fontSize: 12.5, marginTop: 4 }}>
            {health?.status === "ok" ? "就绪" : health?.status === "degraded" ? "未配置账号" : "启动中…"}
          </div>
        </div>
        <div style={cardStyle}>
          <div style={{ color: "var(--text-dim)", fontSize: 12, marginBottom: 6 }}>积分余额</div>
          <div style={{ fontSize: 22, fontWeight: 700 }}>{credits}</div>
          <div style={{ color: "var(--text-mid)", fontSize: 12.5, marginTop: 4 }}>
            {accounts.length > 0
              ? `${enabledCount}/${accounts.length} 账号合计 · 轮询`
              : "CN"}
          </div>
        </div>
        <div style={cardStyle}>
          <div style={{ color: "var(--text-dim)", fontSize: 12, marginBottom: 6 }}>账号池</div>
          {accounts.length > 0 ? (
            <>
              <div style={{ fontSize: 22, fontWeight: 700 }}>
                {enabledCount}
                <span style={{ fontSize: 14, fontWeight: 400, color: "var(--text-mid)" }}>
                  {" "}/ {accounts.length}
                </span>
              </div>
              <div style={{ color: "var(--text-mid)", fontSize: 12.5, marginTop: 4 }}>
                {enabledCount > 0
                  ? accounts
                      .filter((a) => a.enabled)
                      .map((a) => a.nickname || a.uid?.slice(0, 8) || "账号")
                      .join(" · ")
                  : "全部已停用"}
              </div>
            </>
          ) : (
            <>
              <div style={{ fontSize: 15, fontWeight: 600, color: "var(--warn)" }}>未配置</div>
              <div style={{ color: "var(--text-mid)", fontSize: 12.5, marginTop: 4 }}>前往「账号」导入凭据</div>
            </>
          )}
        </div>
      </div>
      <div style={cardStyle}>
        <div style={{ color: "var(--text-dim)", fontSize: 12, marginBottom: 8 }}>健康详情</div>
        <pre style={{
          margin: 0, fontSize: 12, color: "var(--text-mid)", overflow: "auto",
          maxWidth: "100%", minWidth: 0,
          fontFamily: "var(--font-mono)",
        }}>
          {health ? JSON.stringify(health, null, 2) : "等待响应…"}
        </pre>
      </div>
    </div>
  );
}
