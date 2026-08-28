/**
 * 白话识别规则的用例。无需测试框架:
 *   node --experimental-strip-types src/lib/nlDetect.test.ts
 */
import { looksLikeNaturalLanguage, stripPrompt } from "./nlDetect.ts";

let failed = 0;

function expect(input: string, want: boolean, why: string) {
  const got = looksLikeNaturalLanguage(input);
  if (got !== want) {
    failed++;
    console.error(`  ✗ ${JSON.stringify(input)} → ${got},应为 ${want}(${why})`);
  }
}

function expectStrip(raw: string, want: string) {
  const got = stripPrompt(raw);
  if (got !== want) {
    failed++;
    console.error(`  ✗ stripPrompt(${JSON.stringify(raw)}) → ${JSON.stringify(got)},应为 ${JSON.stringify(want)}`);
  }
}

// —— 应判为白话 ——
expect("找出占用 8080 端口的进程", true, "中文");
expect("看看磁盘还剩多少空间", true, "中文");
expect("帮我把 nginx 重启一下", true, "中文");
expect("?how do I list docker volumes", true, "显式 ? 前缀");
expect("? restart nginx", true, "显式 ? 前缀");
expect("how to find large files", true, "英文意图词开头");
expect("show me the running containers", true, "英文意图词开头");
// find/show/start 这类词既是命令也是意图词 —— 按"命令优先"处理,想走 AI 加 ? 前缀
expect("find all log files modified today", false, "find 是已知命令,命令优先");
expect("?find all log files modified today", true, "? 前缀可覆盖命令优先");

// —— 应判为命令(不得打扰)——
expect("ls", false, "太短");
expect("ls -la", false, "已知命令");
expect("git status", false, "已知命令");
expect("git commit -m 'fix: 修复中文提交信息'", false, "已知命令,即使含中文");
expect("docker ps -a", false, "已知命令");
expect("ps aux | grep nginx", false, "含管道");
expect("cat /etc/hosts", false, "已知命令");
expect("./deploy.sh", false, "相对路径可执行");
expect("/usr/local/bin/tool run", false, "绝对路径");
expect("make build", false, "已知命令");
expect("npm run dev", false, "已知命令");
expect("kubectl get pods -n prod", false, "已知命令");
expect("FOO=bar ./run", false, "环境变量赋值");
expect("tail -f app.log", false, "已知命令");
expect("echo 你好", false, "已知命令,即使含中文参数");
expect("systemctl restart nginx", false, "已知命令,即使 restart 是意图词");
expect("find . -name '*.log'", false, "find 是已知命令");
expect("myscript.sh", false, "含扩展名");
expect("deploy prod", false, "两个词且无意图词 —— 保守当命令");
expect("rm -rf /tmp/x", false, "含短选项");
expect("curl https://x.dev", false, "已知命令");

// —— 提示符剥离 ——
expectStrip("lin@prod-web-01:~$ 找出占用端口的进程", "找出占用端口的进程");
expectStrip("root@host:/var/log# tail -f a.log", "tail -f a.log");
expectStrip("user% ls", "ls");
expectStrip("> continue", "continue");
expectStrip("没有提示符的裸文本", "没有提示符的裸文本");

if (failed === 0) {
  console.log("NL_DETECT_PASS");
} else {
  console.error(`${failed} 个用例未通过`);
  process.exit(1);
}
