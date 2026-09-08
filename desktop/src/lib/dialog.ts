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
import { cursorPosition, getCurrentWindow, PhysicalPosition } from "@tauri-apps/api/window";

/** 弹原生对话框前把鼠标指针唤回来。
 *
 *  Windows 的「打字时隐藏指针」(以及 Chromium 自己那套同名行为)会在敲键盘时藏起指针,
 *  约定是下一次鼠标移动由**藏它的那个窗口**恢复。而 rfd 的 Windows 后端把 `IFileDialog::Show`
 *  放在新起的线程上跑(见 rfd `win_cid::ThreadFuture`),对话框窗口与主窗口分属两个输入队列:
 *  框一弹出来就抢走焦点,之后的鼠标移动全进了对话框那侧,WebView 那侧再没有恢复的时机 ——
 *  指针就一直不显形。ZMODEM 的 sz/rz 框正踩在这个点上:它是用户敲完命令回车的瞬间自动弹的。
 *
 *  故在弹框前、指针还落在 WebView 上时,原地注入一次真实的鼠标移动:先挪 1px 再挪回来
 *  (同点 SetCursorPos 不保证产生移动事件,故必须真的换一次坐标),让藏指针的那一侧收到
 *  移动并把它显示回来。tao 的 `set_cursor_position` 收的是相对窗口客户区的坐标,
 *  而 `cursorPosition()` 给的是屏幕坐标,所以要减去 `innerPosition()`。
 *
 *  纯属显示层的补救,失败了不该拦下对话框本身,故整段吞掉异常。 */
async function wakeCursor(): Promise<void> {
  try {
    const win = getCurrentWindow();
    const [cur, origin] = await Promise.all([cursorPosition(), win.innerPosition()]);
    const x = cur.x - origin.x;
    const y = cur.y - origin.y;
    await win.setCursorPosition(new PhysicalPosition(x + 1, y));
    await win.setCursorPosition(new PhysicalPosition(x, y));
  } catch {
    /* 拿不到坐标或没权限:放弃唤回,照常弹框 */
  }
}

/** 危险操作确认。确认返回 true,取消返回 false。 */
export async function confirmAction(text: string, title = "请确认"): Promise<boolean> {
  await wakeCursor();
  return tauriConfirm(text, { title, kind: "warning" });
}

/** 错误提示 */
export async function showError(text: string, title = "出错了"): Promise<void> {
  await wakeCursor();
  await tauriMessage(text, { title, kind: "error" });
}

/** 选择一个本地目录。取消返回 null。 */
export async function pickDirectory(defaultPath?: string): Promise<string | null> {
  await wakeCursor();
  const picked = await tauriOpen({ directory: true, multiple: false, defaultPath });
  return typeof picked === "string" ? picked : null;
}

/** 多选本地文件(ZMODEM 上传用)。取消返回空数组。 */
export async function pickFiles(): Promise<string[]> {
  await wakeCursor();
  const picked = await tauriOpen({ directory: false, multiple: true });
  if (Array.isArray(picked)) return picked;
  return typeof picked === "string" ? [picked] : [];
}
