import { useEffect, useRef, useMemo } from "react";
import * as d3 from "d3";
import "./StatsPanel.css";

interface SessionSegment {
  start_ms: number;
  end_ms: number;
  user_id: string;
  speaker_name: string | null;
}

interface ParticipantStats {
  user_id: string;
  name: string;
  words: number;
  speech_ms: number;
  wpm: number;
  avgWordLength: number;
}

function computeStats(
  segments: SessionSegment[],
  texts: string[]
): ParticipantStats[] {
  const byUser = new Map<
    string,
    { words: number; chars: number; speech_ms: number; name: string }
  >();
  for (let i = 0; i < segments.length; i++) {
    const seg = segments[i];
    const text = texts[i] ?? "";
    const words = text.split(/\s+/).filter(Boolean).length;
    const speechMs = seg.end_ms - seg.start_ms;
    const entry = byUser.get(seg.user_id) ?? {
      words: 0,
      chars: 0,
      speech_ms: 0,
      name: seg.speaker_name ?? seg.user_id,
    };
    entry.words += words;
    entry.chars += text.replace(/\s/g, "").length;
    entry.speech_ms += speechMs;
    if (seg.speaker_name) entry.name = seg.speaker_name;
    byUser.set(seg.user_id, entry);
  }
  return Array.from(byUser.entries()).map(([user_id, data]) => ({
    user_id,
    name: data.name,
    words: data.words,
    speech_ms: data.speech_ms,
    wpm: data.speech_ms > 0 ? data.words / (data.speech_ms / 60000) : 0,
    avgWordLength: data.words > 0 ? data.chars / data.words : 0,
  }));
}

function formatMs(ms: number): string {
  const s = Math.floor(ms / 1000);
  const m = Math.floor(s / 60);
  const ss = s % 60;
  const mm = m % 60;
  return `${mm}:${ss.toString().padStart(2, "0")}`;
}

interface StatsPanelProps {
  segments: SessionSegment[];
  texts: string[];
  collapsed: boolean;
  onToggleCollapsed: () => void;
}

export function StatsPanel({ segments, texts, collapsed, onToggleCollapsed }: StatsPanelProps) {
  const chartRef = useRef<SVGSVGElement>(null);
  const stats = useMemo(() => computeStats(segments, texts), [segments, texts]);

  const timeSeriesData = useMemo(() => {
    const byUser = new Map<string, { time: number; words: number; speech_ms: number }[]>();
    const cumByUser = new Map<string, { words: number; speech_ms: number }>();
    for (let i = 0; i < segments.length; i++) {
      const seg = segments[i];
      const text = texts[i] ?? "";
      const words = text.split(/\s+/).filter(Boolean).length;
      const speechMs = seg.end_ms - seg.start_ms;
      const cum = cumByUser.get(seg.user_id) ?? { words: 0, speech_ms: 0 };
      cum.words += words;
      cum.speech_ms += speechMs;
      cumByUser.set(seg.user_id, cum);
      const arr = byUser.get(seg.user_id) ?? [];
      arr.push({ time: seg.end_ms, words: cum.words, speech_ms: cum.speech_ms });
      byUser.set(seg.user_id, arr);
    }
    return byUser;
  }, [segments, texts]);

  useEffect(() => {
    if (!chartRef.current || collapsed || segments.length === 0) return;
    const svg = d3.select(chartRef.current);
    svg.selectAll("*").remove();
    const width = chartRef.current.clientWidth || 300;
    const height = 120;
    const margin = { top: 10, right: 10, bottom: 25, left: 40 };
    const innerWidth = width - margin.left - margin.right;
    const innerHeight = height - margin.top - margin.bottom;

    const maxTime = Math.max(...segments.map((s) => s.end_ms), 1);
    const maxWords = Math.max(...stats.map((s) => s.words), 1);

    const xScale = d3.scaleLinear().domain([0, maxTime]).range([0, innerWidth]);
    const yScale = d3.scaleLinear().domain([0, maxWords]).range([innerHeight, 0]);

    const g = svg
      .attr("width", width)
      .attr("height", height)
      .append("g")
      .attr("transform", `translate(${margin.left},${margin.top})`);

    g.append("g")
      .attr("transform", `translate(0,${innerHeight})`)
      .call(d3.axisBottom(xScale).ticks(5).tickFormat((d) => `${Math.round(Number(d) / 1000)}s`));
    g.append("g").call(d3.axisLeft(yScale).ticks(4));

    const color = d3.scaleOrdinal(d3.schemeCategory10);
    let idx = 0;
    timeSeriesData.forEach((points) => {
      if (points.length === 0) return;
      const line = d3
        .line<{ time: number; words: number; speech_ms: number }>()
        .x((d) => xScale(d.time))
        .y((d) => yScale(d.words));
      g.append("path")
        .datum(points)
        .attr("fill", "none")
        .attr("stroke", color(String(idx)))
        .attr("stroke-width", 2)
        .attr("d", line);
      idx++;
    });
  }, [segments, texts, stats, timeSeriesData, collapsed]);

  return (
    <div className={`stats-panel ${collapsed ? "stats-panel-collapsed" : ""}`}>
      <button type="button" className="stats-panel-toggle" onClick={onToggleCollapsed}>
        {collapsed ? "Show stats" : "Hide stats"}
      </button>
      {!collapsed && (
        <div className="stats-panel-content">
          <div className="stats-summary">
            {stats.map((s) => (
              <div key={s.user_id} className="stats-row">
                <span className="stats-name">{s.name}</span>
                <span>{s.words} words</span>
                <span>{formatMs(s.speech_ms)}</span>
                <span>{Math.round(s.wpm)} wpm</span>
                <span>avg {s.avgWordLength.toFixed(1)} ch/word</span>
              </div>
            ))}
          </div>
          {segments.length > 0 && (
            <div className="stats-chart">
              <svg ref={chartRef} />
            </div>
          )}
        </div>
      )}
    </div>
  );
}
