// Pure edit operations on a transcript. Segments, texts and the optional live texts
// are parallel arrays, so every structural edit applies to all three.

export interface Segment {
  start_ms: number;
  end_ms: number;
  user_id: string;
  speaker_name: string | null;
}

export interface TranscriptData {
  segments: Segment[];
  texts: string[];
  liveTexts?: string[];
}

function textAt(arr: string[] | undefined, i: number): string {
  return arr?.[i] ?? "";
}

function joinText(a: string, b: string): string {
  return [a.trim(), b.trim()].filter(Boolean).join(" ");
}

function padTexts(texts: string[], length: number): string[] {
  const out = texts.slice(0, length);
  while (out.length < length) out.push("");
  return out;
}

export function setText(data: TranscriptData, i: number, text: string): TranscriptData {
  const texts = padTexts(data.texts, data.segments.length);
  texts[i] = text;
  return { ...data, texts };
}

export function setSpeaker(data: TranscriptData, i: number, userId: string, name: string | null): TranscriptData {
  const segments = data.segments.map((s, j) => (j === i ? { ...s, user_id: userId, speaker_name: name } : s));
  return { ...data, segments };
}

export function deleteSegment(data: TranscriptData, i: number): TranscriptData {
  const drop = <T,>(arr: T[]) => arr.filter((_, j) => j !== i);
  return {
    segments: drop(data.segments),
    texts: drop(padTexts(data.texts, data.segments.length)),
    liveTexts: data.liveTexts && drop(padTexts(data.liveTexts, data.segments.length)),
  };
}

/** Merge segment i with segment i + 1. The result keeps segment i's speaker. */
export function mergeWithNext(data: TranscriptData, i: number): TranscriptData {
  if (i < 0 || i >= data.segments.length - 1) return data;
  const a = data.segments[i];
  const b = data.segments[i + 1];
  const merged: Segment = { ...a, start_ms: Math.min(a.start_ms, b.start_ms), end_ms: Math.max(a.end_ms, b.end_ms) };
  const mergeArr = (arr: string[]) => {
    const out = padTexts(arr, data.segments.length);
    out.splice(i, 2, joinText(out[i], out[i + 1]));
    return out;
  };
  const segments = [...data.segments];
  segments.splice(i, 2, merged);
  return {
    segments,
    texts: mergeArr(data.texts),
    liveTexts: data.liveTexts && mergeArr(data.liveTexts),
  };
}

/**
 * Split segment i at character `at` of its text. The split time is proportional
 * to the character position, since there are no word timestamps.
 */
export function splitSegment(data: TranscriptData, i: number, at: number): TranscriptData {
  const seg = data.segments[i];
  const text = textAt(data.texts, i);
  if (!seg || at <= 0 || at >= text.length) return data;
  const ratio = at / text.length;
  const splitMs = Math.round(seg.start_ms + (seg.end_ms - seg.start_ms) * ratio);
  const first: Segment = { ...seg, end_ms: splitMs };
  const second: Segment = { ...seg, start_ms: splitMs };
  const segments = [...data.segments];
  segments.splice(i, 1, first, second);
  const texts = padTexts(data.texts, data.segments.length);
  texts.splice(i, 1, text.slice(0, at).trim(), text.slice(at).trim());
  let liveTexts = data.liveTexts;
  if (liveTexts) {
    // Live text has no matching caret position; keep it on the first half.
    liveTexts = padTexts(liveTexts, data.segments.length);
    liveTexts.splice(i + 1, 0, "");
  }
  return { segments, texts, liveTexts };
}

export interface Block {
  userId: string;
  speaker: string;
  startMs: number;
  indices: number[];
}

/** Group consecutive segments by the same speaker into reading blocks. */
export function groupBlocks(segments: Segment[]): Block[] {
  const blocks: Block[] = [];
  segments.forEach((seg, i) => {
    const last = blocks[blocks.length - 1];
    if (last && last.userId === seg.user_id) {
      last.indices.push(i);
    } else {
      blocks.push({ userId: seg.user_id, speaker: seg.speaker_name ?? seg.user_id, startMs: seg.start_ms, indices: [i] });
    }
  });
  return blocks;
}

export function formatMs(ms: number): string {
  const s = Math.floor(ms / 1000);
  const m = Math.floor(s / 60);
  const h = Math.floor(m / 60);
  const ss = (s % 60).toString().padStart(2, "0");
  const mm = m % 60;
  if (h > 0) return `${h}:${mm.toString().padStart(2, "0")}:${ss}`;
  return `${mm}:${ss}`;
}
