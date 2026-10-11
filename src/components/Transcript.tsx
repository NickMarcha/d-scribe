import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type MutableRefObject, type ReactNode } from "react";
import {
  deleteSegment,
  formatMs,
  groupBlocks,
  mergeWithNext,
  setSpeaker,
  setText,
  splitSegment,
  type TranscriptData,
} from "./transcriptOps";
import "./Transcript.css";

export interface Speaker {
  id: string;
  name: string;
}

interface TranscriptProps {
  data: TranscriptData;
  activeIndex: number | null;
  /** False while recording: read view only. */
  editable: boolean;
  speakers: Speaker[];
  onSeek: (index: number) => void;
  /** `coalesceKey` groups consecutive edits (typing in one segment) into one undo step. */
  onEdit: (next: TranscriptData, coalesceKey?: string) => void;
  canUndo: boolean;
  canRedo: boolean;
  onUndo: () => void;
  onRedo: () => void;
  models: { id: string; label: string }[];
  retranscribeModelId: string;
  onRetranscribeModelChange: (id: string) => void;
  onRetranscribe: (index: number) => void;
  retranscribingIndex: number | null;
  /** More tabs after Read and Edit (e.g. Stats, AI). Only shown when editable. */
  extraViews?: { id: string; label: string; render: () => ReactNode }[];
}

export function Transcript(props: TranscriptProps) {
  const { data, activeIndex, editable } = props;
  const [mode, setMode] = useState<string>("read");
  const [query, setQuery] = useState("");
  const view = editable ? mode : "read";
  const extra = props.extraViews?.find((v) => v.id === view);
  const rowRefs = useRef(new Map<number, HTMLElement>());
  const bodyRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (activeIndex === null) return;
    // Don't move the view while someone is typing in it.
    if (bodyRef.current?.contains(document.activeElement) && document.activeElement?.tagName === "TEXTAREA") return;
    rowRefs.current.get(activeIndex)?.scrollIntoView({ block: "nearest" });
  }, [activeIndex, view]);

  const setRowRef = (i: number) => (el: HTMLElement | null) => {
    if (el) rowRefs.current.set(i, el);
    else rowRefs.current.delete(i);
  };

  function onKeyDown(e: KeyboardEvent) {
    if (view !== "edit" || !(e.ctrlKey || e.metaKey)) return;
    const key = e.key.toLowerCase();
    if (key === "z" && !e.shiftKey) {
      e.preventDefault();
      props.onUndo();
    } else if (key === "y" || (key === "z" && e.shiftKey)) {
      e.preventDefault();
      props.onRedo();
    }
  }

  return (
    <div className="transcript" onKeyDown={onKeyDown}>
      <div className="transcript-toolbar">
        {editable && (
          <div className="transcript-tabs">
            <button className={view === "read" ? "active" : ""} onClick={() => setMode("read")}>
              Read
            </button>
            <button className={view === "edit" ? "active" : ""} onClick={() => setMode("edit")}>
              Edit
            </button>
            {props.extraViews?.map((v) => (
              <button key={v.id} className={view === v.id ? "active" : ""} onClick={() => setMode(v.id)}>
                {v.label}
              </button>
            ))}
          </div>
        )}
        {extra ? null : view === "read" ? (
          <ReaderSearch data={data} rowRefs={rowRefs} query={query} setQuery={setQuery} />
        ) : (
          <div className="transcript-tools">
            <button onClick={props.onUndo} disabled={!props.canUndo} title="Ctrl+Z">
              Undo
            </button>
            <button onClick={props.onRedo} disabled={!props.canRedo} title="Ctrl+Y">
              Redo
            </button>
            {props.models.length > 0 && (
              <label className="transcript-model">
                Re-transcribe with
                <select value={props.retranscribeModelId} onChange={(e) => props.onRetranscribeModelChange(e.target.value)}>
                  {props.models.map((m) => (
                    <option key={m.id} value={m.id}>
                      {m.label}
                    </option>
                  ))}
                </select>
              </label>
            )}
          </div>
        )}
      </div>

      <div className="transcript-body" ref={bodyRef}>
        {extra ? (
          <div className="transcript-extra">{extra.render()}</div>
        ) : data.segments.length === 0 ? (
          <p className="transcript-empty">No segments yet.</p>
        ) : view === "read" ? (
          <Reader data={data} activeIndex={activeIndex} onSeek={props.onSeek} setRowRef={setRowRef} query={query.trim()} />
        ) : (
          <Editor {...props} setRowRef={setRowRef} />
        )}
      </div>
    </div>
  );
}

function ReaderSearch({
  data,
  rowRefs,
  query,
  setQuery,
}: {
  data: TranscriptData;
  rowRefs: MutableRefObject<Map<number, HTMLElement>>;
  query: string;
  setQuery: (q: string) => void;
}) {
  const [current, setCurrent] = useState(0);
  const matches = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return [];
    return data.segments.map((_, i) => i).filter((i) => (data.texts[i] ?? "").toLowerCase().includes(q));
  }, [query, data]);

  function jump(delta: number) {
    if (matches.length === 0) return;
    const next = (current + delta + matches.length) % matches.length;
    setCurrent(next);
    rowRefs.current.get(matches[next])?.scrollIntoView({ block: "center" });
  }

  return (
    <div className="transcript-search">
      <input
        type="search"
        placeholder="Search transcript"
        value={query}
        onChange={(e) => {
          setQuery(e.target.value);
          setCurrent(0);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            jump(e.shiftKey ? -1 : 1);
          }
        }}
      />
      {query.trim() && (
        <span className="transcript-search-count">
          {matches.length === 0 ? "No matches" : `${current + 1} of ${matches.length}`}
        </span>
      )}
    </div>
  );
}

function highlight(text: string, query: string): ReactNode {
  if (!query) return text;
  const lower = text.toLowerCase();
  const q = query.toLowerCase();
  const parts: ReactNode[] = [];
  let from = 0;
  let at = lower.indexOf(q);
  while (at !== -1) {
    if (at > from) parts.push(text.slice(from, at));
    parts.push(<mark key={at}>{text.slice(at, at + q.length)}</mark>);
    from = at + q.length;
    at = lower.indexOf(q, from);
  }
  parts.push(text.slice(from));
  return parts;
}

/** In the reader, a pause longer than this starts a new paragraph. */
const PARAGRAPH_GAP_MS = 20_000;

function Reader({
  data,
  activeIndex,
  onSeek,
  setRowRef,
  query,
}: {
  data: TranscriptData;
  activeIndex: number | null;
  onSeek: (i: number) => void;
  setRowRef: (i: number) => (el: HTMLElement | null) => void;
  query: string;
}) {
  // Once there is text, leave out segments without any (silence, noise), so the same speaker's
  // lines join up. Before transcription, show each turn once.
  const hasText = data.texts.some((t) => t?.trim());
  const blocks = useMemo(() => {
    const keep = data.segments.map((_, i) => i).filter((i) => !hasText || (data.texts[i] ?? "").trim());
    const grouped = groupBlocks(keep.map((i) => data.segments[i])).map((b) => ({ ...b, indices: b.indices.map((j) => keep[j]) }));
    // Start a new paragraph after a long pause, so a speaker's lines don't run together for minutes.
    return grouped.flatMap((b) => {
      const parts = [{ ...b, indices: [b.indices[0]] }];
      for (const i of b.indices.slice(1)) {
        const last = parts[parts.length - 1];
        const prev = data.segments[last.indices[last.indices.length - 1]];
        if (data.segments[i].start_ms - prev.end_ms > PARAGRAPH_GAP_MS) parts.push({ ...b, startMs: data.segments[i].start_ms, indices: [i] });
        else last.indices.push(i);
      }
      return parts;
    });
  }, [data.segments, data.texts, hasText]);
  return (
    <>
      {!hasText && <p className="transcript-empty">Not transcribed yet. Pick a language and click Transcribe.</p>}
      {blocks.map((block) => (
        <section key={block.indices[0]} className="reader-block">
          <div className="reader-meta">
            <span className="reader-speaker">{block.speaker}</span>
            <span className="reader-time">{formatMs(block.startMs)}</span>
          </div>
          <p className="reader-text">
            {!hasText && (
              <span ref={setRowRef(block.indices[0])} className="reader-seg empty" onClick={() => onSeek(block.indices[0])}>
                {formatMs(block.indices.reduce((sum, i) => sum + data.segments[i].end_ms - data.segments[i].start_ms, 0))} of speech
              </span>
            )}
            {hasText && block.indices.map((i) => {
              const text = (data.texts[i] ?? "").trim();
              return (
                <span
                  key={i}
                  ref={setRowRef(i)}
                  className={`reader-seg${activeIndex === i ? " active" : ""}${text ? "" : " empty"}`}
                  onClick={() => onSeek(i)}
                  title={`${formatMs(data.segments[i].start_ms)} to ${formatMs(data.segments[i].end_ms)}`}
                >
                  {highlight(text, query)}{" "}
                </span>
              );
            })}
          </p>
        </section>
      ))}
    </>
  );
}

function Editor(props: TranscriptProps & { setRowRef: (i: number) => (el: HTMLElement | null) => void }) {
  const { data, activeIndex, speakers, onEdit, setRowRef, retranscribingIndex } = props;
  const carets = useRef(new Map<number, number>());
  const [, forceRender] = useState(0);

  function trackCaret(i: number, el: HTMLTextAreaElement) {
    carets.current.set(i, el.selectionStart);
    forceRender((n) => n + 1);
  }

  function split(i: number) {
    const at = carets.current.get(i) ?? 0;
    onEdit(splitSegment(data, i, at));
    carets.current.clear();
  }

  return (
    <>
      {data.segments.map((seg, i) => {
        const text = data.texts[i] ?? "";
        const caret = carets.current.get(i) ?? 0;
        const canSplit = caret > 0 && caret < text.length;
        const busy = retranscribingIndex !== null;
        return (
          <div key={i} ref={setRowRef(i)} className={`editor-row${activeIndex === i ? " active" : ""}`}>
            <div className="editor-row-head">
              <button className="editor-time" onClick={() => props.onSeek(i)} title="Play from here">
                {formatMs(seg.start_ms)}
              </button>
              <select
                className="editor-speaker"
                value={seg.user_id}
                onChange={(e) => {
                  const speaker = speakers.find((s) => s.id === e.target.value);
                  onEdit(setSpeaker(data, i, e.target.value, speaker?.name ?? null));
                }}
              >
                {speakers.map((s) => (
                  <option key={s.id} value={s.id}>
                    {s.name}
                  </option>
                ))}
              </select>
              <div className="editor-actions">
                <button onClick={() => split(i)} disabled={busy || !canSplit} title="Split at the cursor (Ctrl+Enter)">
                  Split
                </button>
                <button
                  onClick={() => onEdit(mergeWithNext(data, i))}
                  disabled={busy || i === data.segments.length - 1}
                  title="Join with the next segment"
                >
                  Merge down
                </button>
                <button onClick={() => props.onRetranscribe(i)} disabled={busy}>
                  {retranscribingIndex === i ? "Transcribing..." : "Re-transcribe"}
                </button>
                <button className="editor-delete" onClick={() => onEdit(deleteSegment(data, i))} disabled={busy}>
                  Delete
                </button>
              </div>
            </div>
            <textarea
              className="editor-text"
              value={text}
              placeholder="No text yet"
              rows={1}
              onChange={(e) => {
                onEdit(setText(data, i, e.target.value), `text-${i}`);
                trackCaret(i, e.target);
              }}
              onSelect={(e) => trackCaret(i, e.currentTarget)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
                  e.preventDefault();
                  trackCaret(i, e.currentTarget);
                  split(i);
                }
              }}
            />
          </div>
        );
      })}
    </>
  );
}
