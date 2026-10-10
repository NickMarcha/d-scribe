import { useEffect, useRef, useState } from "react";
import { DEFAULT_AI_COMMAND, DEFAULT_AI_PROMPTS, loadAiConfig, saveAiConfig, type AiPrompt } from "./aiConfig";

/** Settings section for the AI command and prompts. Saves on change. */
export function AiSettings() {
  const [expanded, setExpanded] = useState(false);
  const [command, setCommand] = useState(DEFAULT_AI_COMMAND);
  const [prompts, setPrompts] = useState<AiPrompt[]>(DEFAULT_AI_PROMPTS);
  const loaded = useRef(false);

  useEffect(() => {
    loadAiConfig()
      .then((c) => {
        setCommand(c.command);
        setPrompts(c.prompts);
      })
      .catch((e) => console.error("Failed to load AI settings:", e))
      .finally(() => {
        loaded.current = true;
      });
  }, []);

  useEffect(() => {
    if (!loaded.current) return;
    const timer = setTimeout(() => {
      saveAiConfig(command, prompts).catch((e) => console.error("Failed to save AI settings:", e));
    }, 400);
    return () => clearTimeout(timer);
  }, [command, prompts]);

  function updatePrompt(id: string, changes: Partial<AiPrompt>) {
    setPrompts((list) => list.map((p) => (p.id === id ? { ...p, ...changes } : p)));
  }

  return (
    <section className="settings-section collapsible">
      <button type="button" className="collapsible-header" onClick={() => setExpanded(!expanded)}>
        <span className="collapsible-chevron">{expanded ? "▼" : "▶"}</span>
        <span>AI</span>
      </button>
      {expanded && (
        <div className="collapsible-content">
          <p className="settings-hint">
            d-scribe runs a command-line AI tool you already have. It sends the prompt and transcript on stdin and
            shows what the tool prints. Changes here save automatically.
          </p>
          <div className="form-group">
            <label htmlFor="ai-command">Command</label>
            <input
              id="ai-command"
              type="text"
              className="ai-command-input"
              value={command}
              onChange={(e) => setCommand(e.target.value)}
              spellCheck={false}
            />
            <span className="field-hint">
              Default is Claude Code. Other examples: <code>ollama run llama3.1</code>, <code>llm -m gpt-4o-mini</code>,{" "}
              <code>gemini</code>.{" "}
              {command !== DEFAULT_AI_COMMAND && (
                <button type="button" className="link-button" onClick={() => setCommand(DEFAULT_AI_COMMAND)}>
                  Use default
                </button>
              )}
            </span>
          </div>

          <h4 className="settings-subsection">Prompts</h4>
          {prompts.map((p) => (
            <div key={p.id} className="ai-prompt-card">
              <div className="ai-prompt-head">
                <input
                  type="text"
                  value={p.name}
                  onChange={(e) => updatePrompt(p.id, { name: e.target.value })}
                  aria-label="Prompt name"
                />
                <button
                  type="button"
                  className="model-remove-btn"
                  onClick={() => setPrompts((list) => list.filter((x) => x.id !== p.id))}
                >
                  Remove
                </button>
              </div>
              <textarea
                value={p.instructions}
                onChange={(e) => updatePrompt(p.id, { instructions: e.target.value })}
                aria-label="Instructions"
              />
            </div>
          ))}
          <div className="language-slots-actions">
            <button
              type="button"
              className="add-source-btn"
              onClick={() =>
                setPrompts((list) => [...list, { id: crypto.randomUUID(), name: "New prompt", instructions: "" }])
              }
            >
              Add prompt
            </button>
            <button type="button" className="restore-defaults-btn" onClick={() => setPrompts(DEFAULT_AI_PROMPTS)}>
              Restore defaults
            </button>
          </div>
        </div>
      )}
    </section>
  );
}
