import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./app/App";
import { inDesktopShell } from "./ipc/api";
import "./app.css";

const root = ReactDOM.createRoot(document.getElementById("root")!);

// KiteTerm 只发布 macOS / Windows 桌面端。直接用浏览器打开开发服务器时给出明确提示,
// 而不是渲染一个所有 IPC 都会失败的空壳。
if (!inDesktopShell()) {
  root.render(
    <div
      style={{
        height: "100vh",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        gap: 10,
        fontFamily: "var(--font-sans)",
        color: "var(--text-muted)",
        textAlign: "center",
        padding: 24,
      }}
    >
      <div
        style={{
          fontFamily: "var(--font-display)",
          fontSize: 20,
          fontWeight: 700,
          color: "var(--text-strong)",
        }}
      >
        请在桌面应用中打开 KiteTerm
      </div>
      <div style={{ fontSize: 13.5, lineHeight: 1.7, maxWidth: 460 }}>
        KiteTerm 是 macOS / Windows 桌面应用,SSH、SFTP、密钥与同步能力均由本地 Rust 核心提供,
        没有 Web 版本。
        <br />
        开发请运行 <span style={{ fontFamily: "var(--font-mono)", color: "var(--accent-text)" }}>pnpm tauri dev</span>,
        发布请运行 <span style={{ fontFamily: "var(--font-mono)", color: "var(--accent-text)" }}>pnpm tauri build</span>。
      </div>
    </div>,
  );
} else {
  root.render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}
