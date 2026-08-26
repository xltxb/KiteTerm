//! 服务器监控:复用活跃 SSH 会话,每 5s 采样 CPU/内存/磁盘/负载/网络(Linux,无需 agent)

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter};

use crate::models::MonitorSample;
use crate::ssh::{self, SessionManager};

const SAMPLE_CMD: &str = concat!(
    "grep -m1 '^cpu ' /proc/stat; ",
    "awk 'NR>2{gsub(\":\",\" \"); r+=$2; t+=$10} END{print \"NETX\", r, t}' /proc/net/dev; ",
    "sleep 1; ",
    "grep -m1 '^cpu ' /proc/stat; ",
    "awk 'NR>2{gsub(\":\",\" \"); r+=$2; t+=$10} END{print \"NETX\", r, t}' /proc/net/dev; ",
    "free -b | sed -n 2p; ",
    "df -P / | sed -n 2p; ",
    "cat /proc/loadavg; ",
    "uptime"
);

#[derive(Default)]
pub struct MonitorState {
    pub samples: Mutex<Vec<MonitorSample>>,
}

fn fmt_rate(bytes_per_sec: f64) -> String {
    if bytes_per_sec >= 1024.0 * 1024.0 {
        format!("{:.1} MB/s", bytes_per_sec / 1024.0 / 1024.0)
    } else {
        format!("{:.1} KB/s", bytes_per_sec / 1024.0)
    }
}

fn parse_sample(host_id: &str, host_name: &str, out: &str) -> MonitorSample {
    let mut cpu_lines: Vec<Vec<u64>> = Vec::new();
    let mut net_lines: Vec<(f64, f64)> = Vec::new();
    let mut mem = 0u8;
    let mut disk = 0u8;
    let mut load = String::new();
    let mut uptime = String::new();

    for line in out.lines() {
        let l = line.trim();
        if l.starts_with("cpu ") {
            let nums: Vec<u64> = l.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
            cpu_lines.push(nums);
        } else if l.starts_with("NETX") {
            let p: Vec<f64> = l.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
            if p.len() >= 2 {
                net_lines.push((p[0], p[1]));
            }
        } else if l.starts_with("Mem") {
            let p: Vec<f64> = l.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
            if p.len() >= 2 && p[0] > 0.0 {
                mem = ((p[1] / p[0]) * 100.0).round() as u8;
            }
        } else if l.contains('%') && l.split_whitespace().count() >= 5 {
            if let Some(cap) = l.split_whitespace().find(|f| f.ends_with('%')) {
                disk = cap.trim_end_matches('%').parse().unwrap_or(0);
            }
        } else if l.contains(" up ") && (l.contains("load average") || l.contains("user")) {
            if let Some(pos) = l.find(" up ") {
                let rest = &l[pos + 1..];
                let cut = rest.find(',').unwrap_or(rest.len());
                uptime = rest[..cut].trim().to_string();
            }
        } else if load.is_empty() && l.split_whitespace().count() == 5 {
            if let Some(first) = l.split_whitespace().next() {
                if first.parse::<f64>().is_ok() {
                    load = format!("load {first}");
                }
            }
        }
    }

    let cpu = if cpu_lines.len() >= 2 && cpu_lines[0].len() >= 4 && cpu_lines[1].len() >= 4 {
        let total0: u64 = cpu_lines[0].iter().sum();
        let total1: u64 = cpu_lines[1].iter().sum();
        let idle0 = cpu_lines[0][3] + cpu_lines[0].get(4).copied().unwrap_or(0);
        let idle1 = cpu_lines[1][3] + cpu_lines[1].get(4).copied().unwrap_or(0);
        let dt = total1.saturating_sub(total0);
        let di = idle1.saturating_sub(idle0);
        if dt > 0 { (100.0 * (dt - di.min(dt)) as f64 / dt as f64).round() as u8 } else { 0 }
    } else {
        0
    };

    let (net_up, net_down) = if net_lines.len() >= 2 {
        let rx = (net_lines[1].0 - net_lines[0].0).max(0.0);
        let tx = (net_lines[1].1 - net_lines[0].1).max(0.0);
        (format!("↑ {}", fmt_rate(tx)), format!("↓ {}", fmt_rate(rx)))
    } else {
        ("↑ —".into(), "↓ —".into())
    };

    MonitorSample {
        host_id: host_id.to_string(),
        host_name: host_name.to_string(),
        online: true,
        uptime,
        cpu,
        mem,
        disk,
        load,
        net_up,
        net_down,
    }
}

/// 启动全局监控循环:对每台有活跃会话的主机采样并推送
pub fn spawn_monitor_loop(app: AppHandle, sessions: Arc<SessionManager>, state: Arc<MonitorState>) {
    tokio::spawn(async move {
        loop {
            let targets: HashMap<String, (String, Arc<russh::client::Handle<crate::ssh::ClientHandler>>)> = {
                let s = sessions.sessions.lock().unwrap();
                s.values()
                    .filter(|e| e.info.state == "connected")
                    .map(|e| (e.info.host_id.clone(), (e.info.host_name.clone(), e.handle.clone())))
                    .collect()
            };
            let mut samples = Vec::new();
            for (host_id, (host_name, handle)) in targets {
                match ssh::exec_capture(&handle, SAMPLE_CMD).await {
                    Ok(out) => samples.push(parse_sample(&host_id, &host_name, &out)),
                    Err(_) => samples.push(MonitorSample {
                        host_id,
                        host_name,
                        online: false,
                        uptime: String::new(),
                        cpu: 0,
                        mem: 0,
                        disk: 0,
                        load: String::new(),
                        net_up: "↑ —".into(),
                        net_down: "↓ —".into(),
                    }),
                }
            }
            *state.samples.lock().unwrap() = samples.clone();
            let _ = app.emit("monitor://samples", samples);
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });
}
