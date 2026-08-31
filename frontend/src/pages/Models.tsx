import { useEffect, useMemo, useState } from "react";
import { getApiKey, GATEWAY } from "../lib/tauri";

interface ModelItem {
  id: string;
  x_free?: boolean;
  x_credits?: number;
  context_window?: number;
  max_output_tokens?: number;
  badges?: string[];
}

export default function Models() {
  const [models, setModels] = useState<ModelItem[]>([]);
  const [query, setQuery] = useState("");
  const [onlyFree, setOnlyFree] = useState(false);
  const [loading, setLoading] = useState(true);

  async function refresh() {
    setLoading(true);
    try {
      const key = await getApiKey();
      const r = await fetch(GATEWAY + "/v1/models", {
        headers: { "X-Api-Key": key },
      }).then((r) => r.json());
      setModels(r?.data || []);
    } catch { setModels([]); }
    setLoading(false);
  }
  useEffect(() => { refresh(); }, []);

  const filtered = useMemo(
    () =>
      models.filter(
        (m) =>
          (!onlyFree || m.x_free) &&
          (!query || m.id.toLowerCase().includes(query.toLowerCase())),
      ),
    [models, query, onlyFree],
  );

  const btn = {
    padding: "6px 14px", borderRadius: "var(--radius-sm)", border: "1px solid var(--line)",
    background: "var(--bg-raised)", color: "var(--text-hi)", fontSize: 12.5,
  } as const;

  return (
    <div style={{ display: "grid", gap: 14, gridTemplateColumns: "minmax(0, 1fr)" }}>
      <h2 style={{ margin: 0 }}>模型目录</h2>
      <div style={{ display: "flex", gap: 10, alignItems: "center" }}>
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="搜索模型…"
          style={{
            flex: 1, maxWidth: 280, padding: "7px 10px",
            background: "var(--bg-base)", border: "1px solid var(--line)",
            borderRadius: "var(--radius-sm)", color: "var(--text-hi)", fontSize: 13,
          }}
        />
        <button onClick={() => setOnlyFree((v) => !v)} style={{ ...btn, ...(onlyFree ? { borderColor: "var(--accent)", color: "var(--accent)" } : {}) }}>
          仅免费
        </button>
        <button onClick={refresh} style={btn}>刷新</button>
        <span style={{ color: "var(--text-dim)", fontSize: 12 }}>{filtered.length} 个模型</span>
      </div>

      <div style={{ background: "var(--bg-panel)", border: "1px solid var(--line)", borderRadius: "var(--radius)", overflow: "hidden" }}>
        <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 13 }}>
          <thead>
            <tr style={{ color: "var(--text-dim)", fontSize: 12, textAlign: "left" }}>
              <th style={{ padding: "10px 14px", borderBottom: "1px solid var(--line)" }}>模型</th>
              <th style={{ padding: "10px 14px", borderBottom: "1px solid var(--line)" }}>计费</th>
              <th style={{ padding: "10px 14px", borderBottom: "1px solid var(--line)" }}>上下文</th>
              {onlyFree && <th style={{ padding: "10px 14px", borderBottom: "1px solid var(--line)" }}>角标</th>}
            </tr>
          </thead>
          <tbody>
            {loading ? (
              <tr><td colSpan={4} style={{ padding: 18, color: "var(--text-dim)" }}>加载中…</td></tr>
            ) : filtered.length === 0 ? (
              <tr><td colSpan={4} style={{ padding: 18, color: "var(--text-dim)" }}>无模型，请确认已导入账号并刷新目录</td></tr>
            ) : (
              filtered.map((m) => (
                <tr key={m.id} style={{ borderBottom: "1px solid var(--line)" }}>
                  <td style={{ padding: "9px 14px", fontFamily: "var(--font-mono)", fontSize: 12.5 }}>{m.id}</td>
                  <td style={{ padding: "9px 14px" }}>
                    {m.x_free ? (
                      <span style={{ color: "var(--ok)", fontSize: 12.5 }}>免费</span>
                    ) : m.x_credits !== undefined ? (
                      <span style={{ color: "var(--text-mid)", fontSize: 12.5 }}>x{m.x_credits.toFixed(2)}</span>
                    ) : (
                      <span style={{ color: "var(--text-dim)" }}>—</span>
                    )}
                  </td>
                  <td style={{ padding: "9px 14px", color: "var(--text-mid)", fontSize: 12.5 }}>
                    {m.context_window ? (m.context_window / 1000).toFixed(0) + "K" : "—"}
                  </td>
                  {onlyFree && (
                    <td style={{ padding: "9px 14px" }}>
                      {m.badges?.map((b) => (
                        <span key={b} style={{ fontSize: 11, color: "var(--warn)", marginRight: 6 }}>
                          {b.replace("badge:", "").split(":")[0]}
                        </span>
                      ))}
                    </td>
                  )}
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}