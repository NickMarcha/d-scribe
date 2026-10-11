import { useTasks, type Task } from "../tasks";
import "./TaskBar.css";

function formatMb(bytes: number) {
  return `${Math.round(bytes / 1024 / 1024)} MB`;
}

function progressText(t: Task) {
  if (t.unit === "bytes") return t.total ? `${formatMb(t.done)} of ${formatMb(t.total)}` : formatMb(t.done);
  return t.total ? `${t.done} of ${t.total}` : "";
}

/** Fixed to the bottom of the window: one row per running task. */
export function TaskBar() {
  const tasks = useTasks();
  if (tasks.length === 0) return null;
  return (
    <div className="task-bar" role="status">
      {tasks.map((t) => {
        const pct = t.total ? Math.min(100, (t.done / t.total) * 100) : null;
        return (
          <div key={t.id} className="task-row">
            <span className="task-label">{t.label}</span>
            <div className="task-track">
              <div
                className={`task-fill${pct == null ? " task-fill--indeterminate" : ""}`}
                style={pct == null ? undefined : { width: `${pct}%` }}
              />
            </div>
            <span className="task-text">
              {progressText(t)}
              {pct != null && ` (${Math.floor(pct)}%)`}
            </span>
          </div>
        );
      })}
    </div>
  );
}
