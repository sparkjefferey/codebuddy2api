import { useEffect, useState } from "react";

interface LogEntry {
  ts: string;
  level: string;
  text: string;
}

export default function Activity() {
  const [entries, setEntries] = useState<LogEntry[]>([]);
  const [note, setNote] = useState("v1.0.0 日志暂存于内存，重启后清空");

  useEffect(() => {
    // v1: gateway logs live in stderr/file on Rust side; UI polls nothing yet.
    // Placeholder: show a static explainer + keep the page shape for future streaming logs.
    setEntries([]);
  }, []);

  return (
    <div style={{ display: "grid", gap: 16, gridTemplateColumns: "minmax(0, 1fr)" }}>
      <h2 style={{ margin: 0 }}>活动</h2>
      <div style={{
        background: "var(--bg-panel)", border: "1px solid var(--line)",
        borderRadius: "var(--radius)", padding: "18px 20px",
      }}>
        <div style={{ color: "var(--text-dim)", fontSize: 12, marginBottom: 10 }}>请求日志</div>
        {entries.length === 0 ? (
          <div style={{ padding: "28px 0", textAlign: "center", color: "var(--text-dim)", fontSize: 13 }}>
            {note}
            <br />
            <span style={{ fontSize: 12 }}>
              完整日志输出在应用控制台；后续版本将提供实时日志流与诊断导出。
            </span>
          </div>
        ) : (
          entries.map((e, i) => (
            <div key={i} style={{ fontSize: 12.5, fontFamily: "var(--font-mono)", color: "var(--text-mid)" }}>
              [{e.ts}] {e.text}
            </div>
          ))
        )}
      </div>
    </div>
  );
}