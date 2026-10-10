import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { buildAiInput, loadAiConfig, transcriptAsText, type AiOutput, type AiPrompt } from "./aiConfig";
import type { TranscriptData } from "./transcriptOps";
import "./AiPanel.css";

interface AiPanelProps {
  data: TranscriptData;
  outputs: AiOutput[];
  onAdd: (output: AiOutput) => void;
  onDelete: (index: number) => void;
}

/** Runs AI prompts on the transcript and lists the saved results, newest first. */
export function AiPanel({ data, outputs, onAdd, onDelete }: AiPanelProps) {
  const [prompts, setPrompts] = useState<AiPrompt[]>([]);
  const [command, setCommand] = useState("");
  const [promptId, setPromptId] = useState("");
  const [running, setRunning] = useState(false);
  const [error, setError] = useState("");
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

  const prompt = prompts.find((p) => p.id === promptId);
  const hasText = data.texts.some((t) => t?.trim());

  async function run() {
    const config = await refreshConfig();
    const current = config?.prompts.find((p) => p.id === promptId);
    if (!config || !current) return;
    setRunning(true);
    setError("");
    try {
      const output = await invoke<string>("run_ai_command", {
        command: config.command,
        input: buildAiInput(current.instructions, data),
      });
      onAdd({ prompt_name: current.name, output, created_at: Math.floor(Date.now() / 1000) });
      setOpenIndex(outputs.length);
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(false);
    }
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
