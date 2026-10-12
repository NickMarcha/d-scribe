import { useMemo, useState } from "react";
import "./RecordingsOverview.css";

export interface ProjectMeta {
  name: string;
  path: string;
  title: string | null;
  guild_name: string | null;
  channel_name: string | null;
  /** Discord channel type: 1=dm, 2=guild_voice, 3=group_dm */
  channel_type: number | null;
  created_at: number;
  saved: boolean;
  duration_ms: number;
  speakers: string[];
  /** Everyone else in the call when it started. */
  others: string[];
  transcribed: boolean;
  size_bytes: number;
}

interface RecordingsOverviewProps {
  projects: ProjectMeta[];
  retentionDays: number;
  maxGb: number;
  onOpen: (path: string) => void;
  onSetSaved: (path: string, saved: boolean) => void;
  onDelete: (path: string) => void;
}

const MAX_SPEAKERS_SHOWN = 4;

/** Where a recording happened: "Server / channel", "Call with Name", or the kind of call. */
export function placeLabel(p: {
  guild_name: string | null;
  channel_name: string | null;
  channel_type: number | null;
  others: string[];
}) {
  const withOthers = p.others.length > 0 ? `with ${p.others.join(", ")}` : "";
  if (p.guild_name || p.channel_type === 2) {
    return [p.guild_name ?? "Unknown server", p.channel_name].filter(Boolean).join(" / ");
  }
  if (p.channel_type === 1) return withOthers ? `Call ${withOthers}` : "Direct call";
  if (p.channel_type === 3) return p.channel_name ? `Group call: ${p.channel_name}` : `Group call ${withOthers}`.trim();
  return p.channel_name ?? "Unknown channel";
}

function formatDuration(ms: number) {
  const minutes = Math.round(ms / 60000);
  if (minutes < 1) return "< 1 min";
  if (minutes < 60) return `${minutes} min`;
  return `${Math.floor(minutes / 60)} h ${minutes % 60} min`;
}

export function formatSize(bytes: number) {
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
  return `${Math.round(bytes / 1024 ** 2)} MB`;
}

function dayKey(d: Date) {
  return `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
}

function dayLabel(d: Date) {
  const today = new Date();
  const yesterday = new Date(today);
  yesterday.setDate(today.getDate() - 1);
  if (dayKey(d) === dayKey(today)) return "Today";
  if (dayKey(d) === dayKey(yesterday)) return "Yesterday";
  return d.toLocaleDateString(undefined, {
    weekday: "long",
    day: "numeric",
    month: "long",
    year: d.getFullYear() === today.getFullYear() ? undefined : "numeric",
  });
}

function uniqueSorted(values: (string | null | undefined)[]) {
  return [...new Set(values.filter((v): v is string => !!v))].sort((a, b) => a.localeCompare(b));
}

/** All recordings grouped by day, with search and server / channel / person filters. */
export function RecordingsOverview({ projects, retentionDays, maxGb, onOpen, onSetSaved, onDelete }: RecordingsOverviewProps) {
  const [query, setQuery] = useState("");
  const [server, setServer] = useState("");
  const [channel, setChannel] = useState("");
  const [person, setPerson] = useState("");
  const [savedOnly, setSavedOnly] = useState(false);

  const servers = useMemo(() => uniqueSorted(projects.map((p) => p.guild_name)), [projects]);
  const channels = useMemo(
    () => uniqueSorted(projects.filter((p) => !server || p.guild_name === server).map((p) => p.channel_name)),
    [projects, server]
  );
  const people = useMemo(() => uniqueSorted(projects.flatMap((p) => p.speakers)), [projects]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return projects.filter((p) => {
      if (server && p.guild_name !== server) return false;
      if (channel && p.channel_name !== channel) return false;
      if (person && !p.speakers.includes(person)) return false;
      if (savedOnly && !p.saved) return false;
      if (!q) return true;
      return [p.title, p.name, p.guild_name, p.channel_name, ...p.speakers].some((v) => v?.toLowerCase().includes(q));
    });
  }, [projects, query, server, channel, person, savedOnly]);

  const days = useMemo(() => {
    const groups: { key: string; label: string; items: ProjectMeta[] }[] = [];
    for (const p of [...filtered].sort((a, b) => b.created_at - a.created_at)) {
      const d = new Date(p.created_at * 1000);
      const key = p.created_at > 0 ? dayKey(d) : "unknown";
      let group = groups[groups.length - 1];
      if (!group || group.key !== key) {
        group = { key, label: p.created_at > 0 ? dayLabel(d) : "Unknown date", items: [] };
        groups.push(group);
      }
      group.items.push(p);
    }
    return groups;
  }, [filtered]);

  const unsavedBytes = projects.filter((p) => !p.saved).reduce((sum, p) => sum + p.size_bytes, 0);
  const filtering = !!(query || server || channel || person || savedOnly);

  if (projects.length === 0) {
    return <p className="recordings-empty">No recordings yet.</p>;
  }

  return (
    <div className="recordings">
      <div className="recordings-filters">
        <input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search titles, people, servers and channels"
          aria-label="Search recordings"
        />
        <select
          value={server}
          onChange={(e) => {
            setServer(e.target.value);
            setChannel("");
          }}
          aria-label="Server"
        >
          <option value="">All servers</option>
          {servers.map((s) => (
            <option key={s} value={s}>
              {s}
            </option>
          ))}
        </select>
        <select value={channel} onChange={(e) => setChannel(e.target.value)} aria-label="Channel">
          <option value="">All channels</option>
          {channels.map((c) => (
            <option key={c} value={c}>
              {c}
            </option>
          ))}
        </select>
        <select value={person} onChange={(e) => setPerson(e.target.value)} aria-label="Person">
          <option value="">Everyone</option>
          {people.map((p) => (
            <option key={p} value={p}>
              {p}
            </option>
          ))}
        </select>
        <label className="recordings-saved-only">
          <input type="checkbox" checked={savedOnly} onChange={(e) => setSavedOnly(e.target.checked)} />
          Saved only
        </label>
        {filtering && (
          <button
            type="button"
            className="link-button"
            onClick={() => {
              setQuery("");
              setServer("");
              setChannel("");
              setPerson("");
              setSavedOnly(false);
            }}
          >
            Clear
          </button>
        )}
      </div>

      {days.length === 0 && <p className="recordings-empty">No recordings match.</p>}

      {days.map((day) => (
        <section key={day.key} className="recordings-day">
          <h4>{day.label}</h4>
          <ul>
            {day.items.map((p) => {
              const shown = p.speakers.slice(0, MAX_SPEAKERS_SHOWN);
              const more = p.speakers.length - shown.length;
              return (
                <li key={p.path} className="recording-row">
                  <button type="button" className="recording-open" onClick={() => onOpen(p.path)}>
                    <span className="recording-time">
                      {p.created_at > 0
                        ? new Date(p.created_at * 1000).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })
                        : ""}
                    </span>
                    <span className="recording-main">
                      <span className="recording-place">
                        {p.title ? (
                          <>
                            {p.title} <span className="recording-place-sub">{placeLabel(p)}</span>
                          </>
                        ) : (
                          placeLabel(p)
                        )}
                      </span>
                      <span className="recording-people">
                        {shown.length > 0 ? shown.join(", ") : "Nobody spoke"}
                        {more > 0 && ` and ${more} more`}
                      </span>
                    </span>
                    <span className="recording-info">
                      {formatDuration(p.duration_ms)}
                      {!p.transcribed && <span className="recording-tag">Not transcribed</span>}
                    </span>
                  </button>
                  <button
                    type="button"
                    className={`recording-save${p.saved ? " recording-save--saved" : ""}`}
                    onClick={() => onSetSaved(p.path, !p.saved)}
                    title={p.saved ? "Saved. Click to unsave, so it gets deleted like other unsaved recordings." : "Keep this recording until you delete it"}
                  >
                    {p.saved ? "Saved" : "Save"}
                  </button>
                  <button type="button" className="recording-delete" onClick={() => onDelete(p.path)} title="Delete recording">
                    Delete
                  </button>
                </li>
              );
            })}
          </ul>
        </section>
      ))}

      <p className="field-hint recordings-footer">
        Unsaved recordings use {formatSize(unsavedBytes)}. They are deleted after {retentionDays} days
        {maxGb > 0 ? `, or sooner when they use more than ${maxGb} GB` : ""}. Saved recordings are kept until you
        delete them.
      </p>
    </div>
  );
}
