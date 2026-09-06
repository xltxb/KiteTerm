# ZMODEM(sz/rz)设计

日期:2026-09-06
状态:已确认

## 目标

在 SSH 会话中支持 lrzsz 的 `sz`(远端发、本机收)与 `rz`(远端收、本机发):远端一敲命令,
本机自动接管传输,进度显示在终端浮条与传输页,完成后自动回到终端模式。

## 非目标

- 本地终端(`kind = "local"`)暂不支持,其数据不经过 `run_session`。
- 不实现 ZSINIT、ZCOMMAND、ZCHALLENGE 等扩展帧;不做 XMODEM/YMODEM。
- 不做断点续传(ZRPOS 只用于本次传输内的纠错重传)。

## 第一部分:数据流与拦截点

1. **探测**。`ssh.rs` 的 `run_session` 收到 `ChannelMsg::Data` 时先经过 `zmodem::Detector`,它维护约 32 字节滑动窗口:
   - 匹配 `**\x18B00`(远端 `sz` 的 ZRQINIT)→ 进入接收模式;
   - 匹配 `**\x18B01`(远端 `rz` 的 ZRINIT)→ 进入发送模式。
   匹配前的字节照常送终端与录制器;从握手头起的字节全部交给状态机,不进终端也不进录制。
2. **接管**。会话循环调用 `zmodem::run(ctx)`,借出 `channel`、`ctl_rx`、`app`、会话 id;状态机自行 select 通道数据与控制消息,返回后外层循环恢复。期间用户键盘输入丢弃;`Resize` 记下并在结束后补发;`Close` 视同取消,发 ZCAN 后关闭。
3. **控制消息**。`Ctl` 新增 `SendFiles(Vec<PathBuf>)` 与 `CancelTransfer`。
4. **rz 选文件**。后端检测到 ZRINIT 后发事件 `zmodem://pick-files { sessionId }`;前端 session store 弹原生多选文件框,调用命令 `zmodem_send_files(sessionId, paths)`;取消则回传空列表,状态机发 ZCAN。等待期间吞掉远端每 10 秒重发的 ZRINIT。
5. **sz 落盘**。`AppSettings` 新增 `download_dir: String`(serde 缺省空串),空则取系统 Downloads;同名文件追加 ` (1)`、` (2)` 序号。设置页加目录选择。
6. **进度**。复用 `TransferJob` 与 `transfer://progress`,以 `running` 状态直接推入任务列表;`TransferJob` 新增 `protocol: String`(`sftp` | `zmodem`,serde 缺省 `sftp`);`sftp::retry` 与前端重试按钮对 zmodem 任务屏蔽。
7. **范围**。仅 SSH 会话。

## 第二部分:协议状态机

1. **模块** `src-tauri/src/zmodem/`:
   - `frame.rs`:帧头三种格式(hex `**\x18B`、bin16 `*\x18A`、bin32 `*\x18C`)、子包结束符 ZCRCE/ZCRCG/ZCRCQ/ZCRCW、ZDLE 转义(0x10/0x11/0x90/0x91/0x18 及 ZRUB0/ZRUB1)、CRC16(XMODEM)与 CRC32;增量 `Parser`,喂字节吐 `Event::Header { kind, arg }` / `Event::Data { bytes, end }`。
   - `receive.rs`:本机收(远端 sz)。
   - `send.rs`:本机发(远端 rz)。
   - `mod.rs`:`Detector`、`run(ctx)`、`ZChannel` trait(`read` 一块字节 / `write` 一块字节),状态机可跑在 russh 通道、内存管道或本机 lrzsz 子进程 stdio 上。
2. **接收流程**:发 ZRINIT(CANFDX|CANOVIO|CANFC32)→ 收 ZFILE 与数据子包(`name\0size mtime mode`)→ 建文件 → 发 ZRPOS(0) → 收 ZDATA 与子包写盘,ZCRCQ/ZCRCW 回 ZACK → 收 ZEOF → 发 ZRINIT 等下一文件 → 收 ZFIN 回 ZFIN,吞掉 `OO`。CRC 错误发 ZRPOS(当前偏移),连续 10 次中止。
3. **发送流程**:等前端文件列表 → 每个文件:ZFILE(ZCBIN)+ ZCRCW 子包 → 等 ZRPOS/ZSKIP → ZDATA(偏移) 后 1024 字节 ZCRCG 流式,末包 ZCRCE,再 ZEOF(总长) → 等 ZRINIT → 下一文件;全部完成发 ZFIN,收 ZFIN 后发 `OO`。流式期间每包后非阻塞查看通道,收到 ZRPOS 立即回退重发。
4. **取消**:本机取消或收到远端 ZCAN(5×0x18)时,发 5×CAN + 5×BS,排空通道 1 秒,交还终端模式。
5. **超时**:等帧头 10 秒无响应重发当前帧,10 次后中止。
6. **文件 IO**:`tokio::fs`,子包 1024 字节,读写 64 KB 缓冲;权限与 mtime 尽力设置,失败忽略。

## 第三部分:UI、错误处理、测试

1. **终端浮条** `TransferHint`:位于 `term-surface` 左下(与错误提示同位,不会同时出现),显示方向图标、文件名、百分比、速率、序号(第 n/N 个)与「取消」;数据来自 transfer store 中当前会话、`protocol === "zmodem"`、状态 `running` 的任务;取消调用命令 `zmodem_cancel(sessionId)` → `Ctl::CancelTransfer`。
2. **终端文字提示**:开始时 `emit_line`「⇣ 接收 foo …」/「⇡ 发送 N 个文件 …」;结束时「已接收 foo 到 ~/Downloads」/「传输已取消」/「传输失败: 原因」。
3. **设置项**:「下载目录」显示当前路径,「更改…」走 `pickDirectory`,留空回到系统 Downloads。
4. **传输页**:zmodem 任务正常列出;远端路径列显示「ZMODEM」;重试按钮对 zmodem 隐藏。
5. **错误处理**:选文件取消 → ZCAN +「传输已取消」;本地文件打开失败 → 对该文件 ZSKIP/ZFERR 跳过并继续,任务 failed 附原因;通道断开 → 任务 failed,会话走原有断线重连;未知帧类型回 ZNAK。
6. **测试**:
   - `frame.rs` 单元测试:hex/bin32 头编解码对照 lrzsz 已知字节;ZDLE 往返;CRC 已知向量;分片喂入解析器。
   - 回环测试:内存管道连接 `send.rs` 与 `receive.rs`,文件大小 0、1、1023、1024、1025 字节与 10 MB,校验内容与 mtime。
   - 互操作测试:`brew install lrzsz`(执行前再次征得同意),通过 `ZChannel` 接到本机 `sz`/`rz` 子进程 stdio。
   - 端到端:用户在远程主机实测。
