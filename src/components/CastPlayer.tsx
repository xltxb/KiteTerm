// asciicast v2 回放器 — 解析 header + [t,"o",data] 事件行,按时间轴写入独立 xterm 实例
import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { Pause, Play, RotateCcw, X } from "lucide-react";
import { ipc } from "@/ipc";
import { fmtClock } from "@/lib/format";
import { terminalTheme } from "@/theme/terminal";
import type { Recording } from "@/types";

interface CastEvent {
  t: number;
  data: string;
}

interface Cast {
  cols: number;
  rows: number;
  events: CastEvent[];
  duration: number;
}

function parseCast(text: string): Cast {
  const lines = text.split("\n").filter((l) => l.trim());
  let cols = 100;
  let rows = 28;
  const events: CastEvent[] = [];
  for (const [i, line] of lines.entries()) {
    try {
      const parsed = JSON.parse(line);
      if (i === 0 && parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
        cols = parsed.width ?? cols;
        rows = parsed.height ?? rows;
      } else if (Array.isArray(parsed) && parsed[1] === "o") {
        events.push({ t: Number(parsed[0]) || 0, data: String(parsed[2] ?? "") });
      }
    } catch {
      // 跳过损坏行
    }
  }
  return { cols, rows, events, duration: events.length ? events[events.length - 1].t : 0 };
}

export function CastPlayer({ recording, onClose }: { recording: Recording; onClose: () => void }) {
  const paneRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const castRef = useRef<Cast | null>(null);
  const idxRef = useRef(0);
  const clockRef = useRef(0); // 已回放到的时间(秒)
  const [playing, setPlaying] = useState(false);
  const [speed, setSpeed] = useState(1);
  const speedRef = useRef(1);
  const [clock, setClock] = useState(0);
  const [duration, setDuration] = useState(0);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    speedRef.current = speed;
  }, [speed]);

  useEffect(() => {
    let disposed = false;
    (async () => {
      try {
        const text = await ipc.readRecording(recording.id);
        if (disposed) return;
        const cast = parseCast(text);
        castRef.current = cast;
        setDuration(cast.duration);
        const term = new Terminal({
          cols: cast.cols,
          rows: cast.rows,
          fontFamily: '"JetBrains Mono", ui-monospace, Menlo, monospace',
          fontSize: 12,
          disableStdin: true,
          cursorBlink: false,
          theme: terminalTheme(),
        });
        termRef.current = term;
        if (paneRef.current) term.open(paneRef.current);
        setPlaying(true);
      } catch (e) {
        setError(String(e));
      }
    })();
    return () => {
      disposed = true;
      termRef.current?.dispose();
    };
  }, [recording.id]);

  // 回放时钟:每 50ms 前进 speed*0.05s,写入到期事件
  useEffect(() => {
    if (!playing) return;
    const timer = setInterval(() => {
      const cast = castRef.current;
      const term = termRef.current;
      if (!cast || !term) return;
      clockRef.current += 0.05 * speedRef.current;
      while (idxRef.current < cast.events.length && cast.events[idxRef.current].t <= clockRef.current) {
        term.write(cast.events[idxRef.current].data);
        idxRef.current += 1;
      }
      setClock(clockRef.current);
      if (idxRef.current >= cast.events.length) {
        setPlaying(false);
        clockRef.current = cast.duration;
        setClock(cast.duration);
      }
    }, 50);
    return () => clearInterval(timer);
  }, [playing]);

  const restart = () => {
    termRef.current?.reset();
    idxRef.current = 0;
    clockRef.current = 0;
    setClock(0);
    setPlaying(true);
  };

  const done = duration > 0 && clock >= duration;

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div
        className="modal auto"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="sheet-head">
          <span className="fs-md w-bold t-strong">{recording.name}</span>
          <span className="fs-xs t-faint">
            {recording.sessionHost} · {recording.startedAt} · asciicast v2
          </span>
          <span className="spacer" />
          <X size={16} className="clickable t-muted" onClick={onClose} />
        </div>

        <div className="term-surface" style={{ padding: "12px 14px" }}>
          {error ? (
            <div className="fs-sm t-danger" style={{ padding: 20, maxWidth: 480 }}>
              读取录制失败:{error}
            </div>
          ) : (
            <div ref={paneRef} className="vterm" />
          )}
        </div>

        <div
          className="row gap-12 bd-top"
          style={{ padding: "10px 16px" }}
        >
          <button
            className="btn btn-sm btn-secondary square"
            onClick={() => (done ? restart() : setPlaying((p) => !p))}
          >
            {done ? <RotateCcw size={14} /> : playing ? <Pause size={14} /> : <Play size={14} />}
          </button>
          <div className="progress instant grow">
            <div style={{ width: duration > 0 ? `${Math.min(100, (clock / duration) * 100)}%` : "0%" }} />
          </div>
          <span className="mono fs-xs t-muted">
            {fmtClock(clock)} / {fmtClock(duration)}
          </span>
          <div className="seg">
            {[1, 2, 4].map((s) => (
              <div key={s} className="seg-item mono tight" data-active={speed === s} onClick={() => setSpeed(s)}>
                {s}×
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
