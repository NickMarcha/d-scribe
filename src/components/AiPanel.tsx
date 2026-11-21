import { useEffect, useRef, useState } from "react";
import { buildAiInput, loadAiConfig, transcriptAsText, type AiOutput, type AiPrompt } from "./aiConfig";
import type { TranscriptData } from "./transcriptOps";
import "./AiPanel.css";

interface AiPanelProps {
  data: TranscriptData;
  outputs: AiOutput[];
  /** Whether a prompt is running on this recording. Kept by the parent so it survives this panel closing. */
  running: boolean;
  /** The last run's error, if it failed. */
  error: string;
  /** Runs the command and saves the output to the recording, even if this panel closes meanwhile. */
  onRun: (command: string, input: string, promptName: string) => void;
  onDelete: (index: number) => void;
}

/** Runs AI prompts on the transcript and lists the saved results, newest first. */
export function AiPanel({ data, outputs, running, error: runError, onRun, onDelete }: AiPanelProps) {
  const [prompts, setPrompts] = useState<AiPrompt[]>([]);
  const [command, setCommand] = useState("");
  const [promptId, setPromptId] = useState("");
  const [configError, setError] = useState("");
  const error = configError || runError;
  const [copied, setCopied] = useState("");
  const [openIndex, setOpenIndex] = useState<number | null>(outputs.length ? outputs.length - 1 : null);

  // Settings can change while this panel stays mounted, so reload before use.
  function refreshConfig() {
    return loadAiConfig()
      .then((c) => {
        setCommand(c.command);
        setPrompts(c.prompts);
        setPromptId((id) => (c.prompts.some((p) => p.id === id) ? id : c.prompts[0]?.id || ""));
        return c;
      })
      .catch((e) => {
        setError(String(e));
        return null;
      });
  }

  useEffect(() => {
    refreshConfig();
  }, []);

  // Open the newest output when one is added.
  const count = outputs.length;
  const seen = useRef(count);
  useEffect(() => {
    if (count > seen.current) setOpenIndex(count - 1);
    seen.current = count;
  }, [count]);

  const prompt = prompts.find((p) => p.id === promptId);
  const hasText = data.texts.some((t) => t?.trim());

  async function run() {
    const config = await refreshConfig();
    const current = config?.prompts.find((p) => p.id === promptId);
    if (!config || !current) return;
    setError("");
    onRun(config.command, buildAiInput(current.instructions, data), current.name);
  }

  async function copy(text: string, what: string) {
    await navigator.clipboard.writeText(text);
    setCopied(what);
    setTimeout(() => setCopied(""), 1500);
  }

  return (
    <div className="ai-panel">
      <div className="ai-panel-bar">
        <select value={promptId} onChange={(e) => setPromptId(e.target.value)} onFocus={refreshConfig} disabled={running || prompts.length === 0}>
          {prompts.length === 0 && <option value="">No prompts. Add one in Settings.</option>}
          {prompts.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </select>
        <button onClick={run} disabled={running || !prompt || !hasText} title={hasText ? command : "Transcribe first"}>
          {running ? "Running..." : "Run"}
        </button>
        <button className="ai-panel-copy" onClick={() => copy(transcriptAsText(data), "transcript")} disabled={!hasText}>
          {copied === "transcript" ? "Copied" : "Copy transcript"}
        </button>
      </div>
      {error && <p className="ai-panel-error">{error}</p>}
      {outputs
        .map((o, i) => ({ o, i }))
        .reverse()
        .map(({ o, i }) => (
          <div key={`${o.created_at}-${i}`} className="ai-output">
            <button className="ai-output-head" onClick={() => setOpenIndex(openIndex === i ? null : i)}>
              <span className="collapsible-chevron">{openIndex === i ? "▼" : "▶"}</span>
              <span className="ai-output-name">{o.prompt_name}</span>
              <span className="ai-output-time">{new Date(o.created_at * 1000).toLocaleString()}</span>
            </button>
            {openIndex === i && (
              <div className="ai-output-body">
                <div className="ai-output-text">{o.output}</div>
                <div className="ai-output-actions">
                  <button onClick={() => copy(o.output, `out-${i}`)}>{copied === `out-${i}` ? "Copied" : "Copy"}</button>
                  <button className="ai-output-delete" onClick={() => onDelete(i)}>
                    Delete
                  </button>
                </div>
              </div>
            )}
          </div>
        ))}
    </div>
  );
}
