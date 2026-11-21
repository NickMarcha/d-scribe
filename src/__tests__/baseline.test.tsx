// The happy paths behind the regression tests: if these fail, the fake backend or the UI
// helpers are broken, not the behavior under test.

import { describe, expect, it } from "vitest";
import { baseSettings, defaultBackend, modelPath, PATH_A, PATH_B, recordingA, recordingB, type SessionState } from "./fakeBackend";
import {
  act,
  clickTranscribe,
  goToTab,
  modelRow,
  openManageModels,
  openRecording,
  renderApp,
  screen,
  settle,
  showTranscriptTab,
  waitFor,
  within,
} from "./ui";

const NEW_TEXTS = ["Hello from Alice.", "Bob replies here.", "Alice again later."];

describe("baseline", () => {
  it("transcribing and staying on the recording saves the texts", async () => {
    const backend = defaultBackend();
    const user = await renderApp();
    await openRecording(user, /Alpha server/);
    await clickTranscribe(user);
    const call = await backend.nextCall<SessionState>("transcribe_session_command");
    expect(call.args.modelPath).toBe(modelPath("medium.en"));
    await act(async () => call.resolve({ ...call.args.state, transcript_texts: NEW_TEXTS }));

    await waitFor(() => expect(backend.project(PATH_A)?.transcript_texts).toEqual(NEW_TEXTS));
    expect(screen.getByText("Hello from Alice.", { exact: false })).toBeInTheDocument();
  });

  it("an AI prompt run while staying on the recording is saved", async () => {
    const backend = defaultBackend({
      projects: { [PATH_A]: recordingA({ transcript_texts: NEW_TEXTS }), [PATH_B]: recordingB() },
    });
    const user = await renderApp();
    await openRecording(user, /Alpha server/);
    await showTranscriptTab(user, "AI");
    const run = await screen.findByRole("button", { name: "Run" });
    await waitFor(() => expect(run).toBeEnabled());
    await user.click(run);
    const call = await backend.nextCall<string>("run_ai_command");
    expect(call.args.command).toBe("fake-ai --print");
    await act(async () => call.resolve("AI SUMMARY OUTPUT"));

    await waitFor(() => expect(backend.project(PATH_A)?.ai_outputs?.map((o) => o.output)).toEqual(["AI SUMMARY OUTPUT"]));
  });

  it("a model downloaded while staying in Settings shows as installed", async () => {
    const backend = defaultBackend();
    const user = await renderApp();
    await goToTab(user, "Settings");
    await openManageModels(user);
    expect(within(modelRow("large-v3")).getByText("Not installed")).toBeInTheDocument();
    await user.click(within(modelRow("large-v3")).getByRole("button", { name: "Download" }));
    const call = await backend.nextCall<string>("download_model_command");
    backend.emit("download-progress", { modelName: "large-v3", bytesDownloaded: 50 * 1024 ** 2, totalBytes: 100 * 1024 ** 2 });
    expect(await screen.findByText(/50 MB of 100 MB/)).toBeInTheDocument();
    backend.installedModels.push("large-v3");
    await act(async () => call.resolve(modelPath("large-v3")));

    await waitFor(() => expect(within(modelRow("large-v3")).getByText("Installed")).toBeInTheDocument());
  });

  it("an engine downloaded while staying in Settings shows as installed", async () => {
    const backend = defaultBackend({
      engine: { cpu: false, cuda: false, vulkan: false, nvidia: false, vulkanDriver: true, active: null },
    });
    const user = await renderApp();
    await goToTab(user, "Settings");
    await openManageModels(user);
    await user.click(await screen.findByRole("button", { name: /Download GPU \(Vulkan\) engine/ }));
    const call = await backend.nextCall("download_engine_command");
    backend.engine = { ...backend.engine, vulkan: true, active: "vulkan" };
    await act(async () => call.resolve(null));

    expect(await screen.findByRole("button", { name: /Remove GPU \(Vulkan\) engine/ })).toBeInTheDocument();
  });

  it("a failed model download shows its error while staying in Settings", async () => {
    const backend = defaultBackend();
    const user = await renderApp();
    await goToTab(user, "Settings");
    await openManageModels(user);
    await user.click(within(modelRow("large-v3")).getByRole("button", { name: "Download" }));
    const call = await backend.nextCall("download_model_command");
    await act(async () => call.reject("Network unreachable while fetching large-v3"));

    expect(await screen.findByText(/Network unreachable while fetching large-v3/)).toBeVisible();
  });

  it("settings saved before startup are used by Session", async () => {
    const backend = defaultBackend({
      settings: baseSettings({
        language_slots: [
          { id: "en", label: "English", languageCode: "en", liveModel: "base.en", regularModel: "small.en" },
          { id: "no", label: "Norwegian", languageCode: "no", liveModel: "base.en", regularModel: "small.en" },
        ],
      }),
    });
    const user = await renderApp();
    const picker = screen.getByRole("combobox", { name: "Language" });
    expect(within(picker).getByRole("option", { name: "Norwegian" })).toBeInTheDocument();
    await openRecording(user, /Alpha server/);
    await clickTranscribe(user);
    const call = await backend.nextCall("transcribe_session_command");
    expect(call.args.modelPath).toBe(modelPath("small.en"));
    await settle(100);
  });
});
