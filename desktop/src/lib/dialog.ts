/** 确认框与提示框。
 *
 *  **不要用 `window.confirm` / `window.alert`** —— 本运行时的 WKWebView 没有实现
 *  `runJavaScriptConfirmPanel` / `runJavaScriptAlertPanel`(wry 的 UIDelegate 只做了
 *  文件选择、媒体权限、新窗口三项),按 WKWebView 的语义,这类调用**不弹窗、直接返回
 *  false / undefined**。后果是确认框静默返回 false:`if (confirm()) doIt()` 永远不执行,
 *  `if (!confirm()) return` 则永远拦下。两种极性都错,且没有任何报错。
 *
 *  这里统一走 tauri-plugin-dialog 的原生对话框(Rust 侧 rfd 实现,与 WebView 无关)。
 *  代价是异步 —— 调用点需要 await。 */
import { confirm as tauriConfirm, message as tauriMessage, open as tauriOpen } from "@tauri-apps/plugin-dialog";

/** 危险操作确认。确认返回 true,取消返回 false。 */
export async function confirmAction(text: string, title = "请确认"): Promise<boolean> {
  return tauriConfirm(text, { title, kind: "warning" });
}

/** 错误提示 */
export async function showError(text: string, title = "出错了"): Promise<void> {
  await tauriMessage(text, { title, kind: "error" });
}

/** 选择一个本地目录。取消返回 null。 */
export async function pickDirectory(defaultPath?: string): Promise<string | null> {
  const picked = await tauriOpen({ directory: true, multiple: false, defaultPath });
  return typeof picked === "string" ? picked : null;
}
