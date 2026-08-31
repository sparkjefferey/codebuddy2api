import { useEffect, useState } from "react";
import {
  importCredential,
  getCredentialStatus,
  removeAccount,
  setAccountEnabled,
} from "../lib/tauri";
import type { AccountInfo } from "../lib/tauri";

function accountBadge(a: AccountInfo): { text: string; color: string } {
  if (!a.enabled) return { text: "已停用", color: "var(--text-dim)" };
  const now = Date.now();
  if (a.cooldown_until && a.cooldown_until > now) {
    const secs = Math.ceil((a.cooldown_until - now) / 1000);
    return { text: `冷却中 · ${secs}s`, color: "var(--warn)" };
  }
  if ((a.consecutive_429 ?? 0) > 0) {
    return { text: `连续 429 ×${a.consecutive_429}`, color: "var(--warn)" };
  }
  return { text: "正常", color: "var(--ok)" };
}

function fmtExpiry(ms: number): string {
  if (!ms || ms <= 0) return "—";
  const d = new Date(ms);
  return d.toLocaleString("zh-CN", { hour12: false });
}

export default function Account() {
  const [accounts, setAccounts] = useState<AccountInfo[]>([]);
  const [jsonText, setJsonText] = useState("");
  const [msg, setMsg] = useState("");

  async function refresh() {
    const s = await getCredentialStatus().catch(() => null);
    setAccounts(s?.accounts ?? []);
  }
  useEffect(() => {
    refresh();
    const t = setInterval(refresh, 5000); // 轮询刷新冷却/429 状态
    return () => clearInterval(t);
  }, []);

  async function doImport() {
    setMsg("");
    try {
      const action = await importCredential(jsonText);
      setMsg(action === "updated" ? "✅ 已更新同 uid 账号的凭据" : "✅ 账号已添加");
      setJsonText("");
      refresh();
    } catch (e) {
      setMsg(`❌ ${e}`);
    }
  }

  async function doRemove(id: string) {
    await removeAccount(id).catch((e) => setMsg(`❌ ${e}`));
    refresh();
  }

  async function doToggle(a: AccountInfo) {
    await setAccountEnabled(a.id, !a.enabled).catch((e) => setMsg(`❌ ${e}`));
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

  const enabledCount = accounts.filter((a) => a.enabled).length;

  return (
    <div style={{ display: "grid", gap: 16, gridTemplateColumns: "minmax(0, 1fr)" }}>
      <h2 style={{ margin: 0 }}>账号</h2>

      <div style={cardStyle}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 4 }}>
          <div style={{ fontSize: 13.5, fontWeight: 600 }}>
            账号池{accounts.length > 0 && (
              <span style={{ color: "var(--text-mid)", fontWeight: 400, marginLeft: 8, fontSize: 12.5 }}>
                {enabledCount}/{accounts.length} 已启用 · 轮询负载均衡
              </span>
            )}
          </div>
        </div>
        <p style={{ color: "var(--text-mid)", fontSize: 12.5, margin: "0 0 12px" }}>
          请求按轮询在启用账号间均摊；某账号收到 429 时自动切换下一个，连续 429 将进入冷却
          （60s 起逐次翻倍，封顶 30 分钟），冷却期内不参与调度，成功即恢复。
        </p>
        {accounts.length === 0 ? (
          <div style={{ color: "var(--warn)", fontSize: 13 }}>尚未导入账号，请在下方添加</div>
        ) : (
          <div style={{ display: "grid", gap: 10, gridTemplateColumns: "minmax(0, 1fr)" }}>
            {accounts.map((a) => {
              const badge = accountBadge(a);
              return (
                <div
                  key={a.id}
                  style={{
                    display: "flex", alignItems: "center", gap: 14,
                    padding: "12px 14px", borderRadius: "var(--radius-sm)",
                    background: "var(--bg-base)", border: "1px solid var(--line)",
                    opacity: a.enabled ? 1 : 0.55,
                  }}
                >
                  <button
                    onClick={() => doToggle(a)}
                    style={{
                      width: 40, height: 22, borderRadius: 11, flexShrink: 0,
                      border: "none", position: "relative", padding: 0, cursor: "pointer",
                      background: a.enabled ? "var(--accent)" : "var(--line)",
                      transition: "background 0.2s",
                    }}
                    aria-pressed={a.enabled}
                    aria-label={a.enabled ? "停用该账号" : "启用该账号"}
                  >
                    <span style={{
                      position: "absolute", top: 3, left: a.enabled ? 21 : 3,
                      width: 16, height: 16, borderRadius: "50%", background: "#fff",
                      transition: "left 0.2s",
                    }} />
                  </button>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ fontSize: 13.5, fontWeight: 600 }}>
                      {a.nickname || (a.uid ? `账号 ${a.uid.slice(0, 10)}` : "未命名账号")}
                    </div>
                    <div style={{ color: "var(--text-mid)", fontSize: 12, marginTop: 2, display: "flex", gap: 12, flexWrap: "wrap" }}>
                      <span>uid: {a.uid?.slice(0, 14) || "—"}</span>
                      <span>{a.domain || "—"}</span>
                      <span>token 至 {fmtExpiry(a.expires_at)}</span>
                    </div>
                    {a.last_error && (
                      <div style={{ color: "var(--err)", fontSize: 11.5, marginTop: 2 }}>
                        {a.last_error}
                      </div>
                    )}
                  </div>
                  <span style={{ color: badge.color, fontSize: 12, flexShrink: 0 }}>
                    {badge.text}
                  </span>
                  <button
                    onClick={() => doRemove(a.id)}
                    style={{
                      padding: "5px 12px", borderRadius: "var(--radius-sm)", flexShrink: 0,
                      border: "1px solid var(--err)", background: "transparent",
                      color: "var(--err)", fontSize: 12,
                    }}
                  >
                    删除
                  </button>
                </div>
              );
            })}
          </div>
        )}
      </div>

      <div style={cardStyle}>
        <h3 style={{ margin: "0 0 6px", fontSize: 14 }}>添加账号</h3>
        <p style={{ color: "var(--text-mid)", fontSize: 12.5, margin: "0 0 12px" }}>
          粘贴完整登录态 JSON（桌面端 CodeBuddy 的 <code>auth/*.info</code> 文件内容，或单独字段）。
          与已有账号 uid 相同时将更新其凭据而不是重复添加。
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
            添加账号
          </button>
          {msg && <span style={{ fontSize: 12.5, color: msg.startsWith("❌") ? "var(--err)" : "var(--ok)" }}>{msg}</span>}
        </div>
      </div>
    </div>
  );
}
