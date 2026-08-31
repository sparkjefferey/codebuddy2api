import { useEffect, useState } from "react";
import { GATEWAY, getApiKey } from "./lib/tauri";
import Sidebar from "./components/Sidebar";

import Dashboard from "./pages/Dashboard";
import Account from "./pages/Account";
import Models from "./pages/Models";
import ClaudeCode from "./pages/ClaudeCode";
import Activity from "./pages/Activity";
import Settings from "./pages/Settings";

export type PageId = "dashboard" | "account" | "models" | "claudecode" | "activity" | "settings";

const PAGES: { id: PageId; label: string }[] = [
  { id: "dashboard", label: "总览" },
  { id: "account", label: "账号" },
  { id: "models", label: "模型" },
  { id: "claudecode", label: "Claude Code" },
  { id: "activity", label: "活动" },
  { id: "settings", label: "设置" },
];

export let gatewayFetch = (path: string, init?: RequestInit) =>
  fetch(GATEWAY + path, init);

export default function App() {
  const [page, setPage] = useState<PageId>("dashboard");
  const [apiKey, setApiKey] = useState("");
  const [alive, setAlive] = useState(false);

  useEffect(() => {
    getApiKey().then(setApiKey).catch(() => {});
    // Set auth header for gateway calls
    gatewayFetch = (path: string, init?: RequestInit) => {
      const headers = new Headers(init?.headers);
      if (apiKey) headers.set("X-Api-Key", apiKey);
      return fetch(GATEWAY + path, { ...init, headers });
    };
    const t = setInterval(() => {
      fetch(GATEWAY + "/health")
        .then((r) => setAlive(r.ok))
        .catch(() => setAlive(false));
    }, 4000);
    fetch(GATEWAY + "/health")
      .then((r) => setAlive(r.ok))
      .catch(() => setAlive(false));
    return () => clearInterval(t);
  }, [apiKey]);

  return (
    <div style={{ display: "flex", height: "100vh", overflow: "hidden" }}>
      <nav
        style={{
          width: 200,
          flexShrink: 0,
          borderRight: "1px solid var(--line)",
          background: "var(--bg-panel)",
          padding: "16px 10px",
          display: "flex",
          flexDirection: "column",
          gap: 2,
        }}
      >
        <div style={{ fontWeight: 700, fontSize: 15, padding: "6px 12px 14px", letterSpacing: 0.3 }}>
          BuddyAIGateway
        </div>
        {PAGES.map((p) => (
          <button
            key={p.id}
            onClick={() => setPage(p.id)}
            style={{
              textAlign: "left",
              padding: "8px 12px",
              borderRadius: "var(--radius-sm)",
              border: "none",
              background: page === p.id ? "var(--bg-raised)" : "transparent",
              color: page === p.id ? "var(--text-hi)" : "var(--text-mid)",
              fontSize: 13.5,
            }}
          >
            {p.label}
          </button>
        ))}
      </nav>

      <main style={{ flex: 1, overflow: "auto", padding: "0" }}>
        <header
          style={{
            display: "flex",
            alignItems: "center",
            gap: 10,
            padding: "14px 24px",
            borderBottom: "1px solid var(--line)",
            background: "var(--bg-panel)",
            position: "sticky",
            top: 0,
            zIndex: 5,
          }}
        >
          <span
            className={alive ? "heartbeat" : "heartbeat err"}
            aria-label={alive ? "网关运行中" : "网关未响应"}
          />
          <span style={{ fontSize: 13, color: alive ? "var(--text-mid)" : "var(--err)" }}>
            {alive ? "127.0.0.1:9178 运行中" : "网关未响应"}
          </span>
        </header>
        <div style={{ padding: "22px 24px", maxWidth: 980 }}>
          {page === "dashboard" && <Dashboard />}
          {page === "account" && <Account />}
          {page === "models" && <Models />}
          {page === "claudecode" && <ClaudeCode />}
          {page === "activity" && <Activity />}
          {page === "settings" && <Settings />}
        </div>
      </main>
    </div>
  );
}