// Settings and Session must agree on models, languages and downloads without a restart, and
// downloads must finish visibly even if you look at another tab meanwhile.

import { describe, expect, it } from "vitest";
import { baseSettings, defaultBackend, modelPath, type Call } from "./fakeBackend";
import {
  act,
  clickIfPossible,
  clickTranscribe,
  goToTab,
  modelRow,
  openManageModels,
  openRecording,
  openSelectedModels,
  renderApp,
  screen,
  settle,
  waitFor,
  within,
} from "./ui";

/** Transcribe args without the recording, so the text is only about models and paths. */
function modelArgs(call: Call) {
  const { state: _state, ...rest } = call.args;
  return JSON.stringify(rest);
}

/** English uses large-v3, which is in the model list but not downloaded. */
function backendWithLargeV3Missing() {
  return defaultBackend({
    settings: baseSettings({
      language_slots: [
        { id: "en", label: "English", languageCode: "en", liveModel: "base.en", regularModel: "large-v3" },
      ],
    }),
  });
}

function languageRow(label: string) {
  const row = screen.getAllByRole("row").find((r) => within(r).queryByText(label, { exact: true }));
  if (!row) throw new Error(`No language row ${label}`);
  return row;
}

async function startModelDownloadInSettings(user: Awaited<ReturnType<typeof renderApp>>, backend: ReturnType<typeof defaultBackend>, name: string) {
  await goToTab(user, "Settings");
  await openManageModels(user);
  await user.click(within(modelRow(name)).getByRole("button", { name: /Download/ }));
  const call = await backend.nextCall<string>("download_model_command");
  expect(call.args.modelName).toBe(name);
  return call;
}

describe("settings reach the Session tab", () => {
  it("settings changes reach the Session tab without a restart: regular model", async () => {
    const backend = defaultBackend();
    const user = await renderApp();

    await goToTab(user, "Settings");
    await openSelectedModels(user);
    const [, regular] = within(languageRow("English")).getAllByRole("combobox");
    await user.selectOptions(regular, "small.en");
    await waitFor(() =>
      expect((backend.settings.language_slots as { regularModel: string }[])[0].regularModel).toBe("small.en"),
    );

    await goToTab(user, "Session");
    await openRecording(user, /Alpha server/);
    await clickTranscribe(user);
    const call = await backend.nextCall("transcribe_session_command");
    expect(modelArgs(call)).toContain("small.en");
    expect(modelArgs(call)).not.toContain("medium.en");
  });

  it("settings changes reach the Session tab without a restart: added language", async () => {
    const backend = defaultBackend();
    const user = await renderApp();

    await goToTab(user, "Settings");
    await openSelectedModels(user);
    await user.selectOptions(screen.getByDisplayValue("Add language..."), "Norwegian");
    await user.click(screen.getByRole("button", { name: "Add" }));
    await waitFor(() =>
      expect((backend.settings.language_slots as { id: string }[]).map((s) => s.id)).toContain("no"),
    );

    await goToTab(user, "Session");
    const picker = screen.getByRole("combobox", { name: "Language" });
    expect(within(picker).queryByRole("option", { name: "Norwegian" })).toBeInTheDocument();
  });

  it("session knows about models downloaded in Settings", async () => {
    const backend = backendWithLargeV3Missing();
    const user = await renderApp();
    await openRecording(user, /Alpha server/);
    expect(await screen.findByRole("button", { name: "Download large-v3" })).toBeInTheDocument();

    const call = await startModelDownloadInSettings(user, backend, "large-v3");
    backend.installedModels.push("large-v3");
    await act(async () => call.resolve(modelPath("large-v3")));
    await waitFor(() => expect(within(modelRow("large-v3")).getByText("Installed")).toBeInTheDocument());

    await goToTab(user, "Session");
    expect(screen.queryByRole("button", { name: "Download large-v3" })).toBeNull();
    expect(screen.getByRole("button", { name: "Transcribe" })).toBeEnabled();
  });
});

describe("downloads that finish while you are on another tab", () => {
  it("settings shows a model as installed after a download that finished while you were on another tab", async () => {
    const backend = backendWithLargeV3Missing();
    const user = await renderApp();

    const call = await startModelDownloadInSettings(user, backend, "large-v3");
    await goToTab(user, "Session");
    await goToTab(user, "Settings");
    await openManageModels(user);
    backend.installedModels.push("large-v3");
    await act(async () => call.resolve(modelPath("large-v3")));

    await waitFor(() => expect(within(modelRow("large-v3")).getByText("Installed")).toBeInTheDocument());
  });

  it("settings shows an engine as installed after a download that finished while you were on another tab", async () => {
    const backend = defaultBackend({
      engine: { cpu: false, cuda: false, vulkan: false, nvidia: false, vulkanDriver: true, active: null },
    });
    const user = await renderApp();

    await goToTab(user, "Settings");
    await openManageModels(user);
    await user.click(await screen.findByRole("button", { name: /Download GPU \(Vulkan\) engine/ }));
    const call = await backend.nextCall("download_engine_command");
    expect(call.args.kind).toBe("vulkan");

    await goToTab(user, "Session");
    await goToTab(user, "Settings");
    await openManageModels(user);
    backend.engine = { ...backend.engine, vulkan: true, active: "vulkan" };
    await act(async () => call.resolve(null));

    expect(await screen.findByRole("button", { name: /Remove GPU \(Vulkan\) engine/ })).toBeInTheDocument();
  });

  it("only engines this system can download are offered", async () => {
    defaultBackend({
      engine: { cpu: false, cuda: false, vulkan: false, nvidia: false, vulkanDriver: false, active: null, available: ["cpu"] },
    });
    const user = await renderApp();

    await goToTab(user, "Settings");
    await openManageModels(user);

    expect(await screen.findByRole("button", { name: /Download CPU engine/ })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /NVIDIA/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Vulkan/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("option", { name: /NVIDIA|Vulkan/ })).not.toBeInTheDocument();
    // No driver warnings for engines that aren't offered anyway.
    expect(screen.queryByText(/driver found/)).not.toBeInTheDocument();
  });

  it("a missing driver is named for the engines that are offered", async () => {
    defaultBackend({ engine: { nvidia: false, vulkanDriver: true } });
    const user = await renderApp();

    await goToTab(user, "Settings");
    await openManageModels(user);

    expect(await screen.findByText("No NVIDIA driver found, so the NVIDIA engine is not available.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Download NVIDIA GPU engine/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: /Download GPU \(Vulkan\) engine/ })).toBeEnabled();
  });

  it("a model download that fails while you are on another tab shows its error when you come back", async () => {
    const backend = backendWithLargeV3Missing();
    const user = await renderApp();

    const call = await startModelDownloadInSettings(user, backend, "large-v3");
    await goToTab(user, "Session");
    await act(async () => call.reject("Network unreachable while fetching large-v3"));
    await settle(200);
    await goToTab(user, "Settings");
    await openManageModels(user);

    await waitFor(() => {
      const shown = screen.queryAllByText(/Network unreachable while fetching large-v3/);
      expect(shown.find((el) => !el.closest("[hidden]"))).toBeDefined();
    });
  });

  it("an engine download that fails while you are on another tab shows its error when you come back", async () => {
    const backend = defaultBackend({
      engine: { cpu: false, cuda: false, vulkan: false, nvidia: false, vulkanDriver: true, active: null },
    });
    const user = await renderApp();

    await goToTab(user, "Settings");
    await openManageModels(user);
    await user.click(await screen.findByRole("button", { name: /Download GPU \(Vulkan\) engine/ }));
    const call = await backend.nextCall("download_engine_command");
    await goToTab(user, "Session");
    await act(async () => call.reject("Engine server returned 503"));
    await settle(200);
    await goToTab(user, "Settings");
    await openManageModels(user);

    await waitFor(() => {
      const shown = screen.queryAllByText(/Engine server returned 503/);
      expect(shown.find((el) => !el.closest("[hidden]"))).toBeDefined();
    });
  });
});

describe("settings don't overwrite choices", () => {
  it("opening Settings doesn't overwrite model choices", async () => {
    const slots = [{ id: "en", label: "English", languageCode: "en", liveModel: "medium.en", regularModel: "medium.en" }];
    const backend = defaultBackend({
      settings: baseSettings({
        remote_sources: [{ id: "src1", name: "Remote", host: "http://localhost:9000" }],
        model_registry: [
          { id: "medium.en", type: "integrated" },
          { id: "src1:whisper-1", type: "remote", sourceId: "src1", modelName: "whisper-1" },
        ],
        language_slots: slots,
      }),
      installedModels: ["medium.en"],
      control: ["list_installed_model_names_command"],
    });
    const user = await renderApp();

    await goToTab(user, "Settings");
    await screen.findByRole("heading", { name: "Settings" });
    await settle(300);
    await act(async () => backend.resolveAll("list_installed_model_names_command", ["medium.en"]));
    backend.controlled.delete("list_installed_model_names_command");
    await settle(1000);

    expect(backend.settings.language_slots).toEqual(slots);
  });
});

describe("one download per model", () => {
  it("the same model can't be downloaded twice at once from the UI", async () => {
    const backend = backendWithLargeV3Missing();
    const user = await renderApp();

    await startModelDownloadInSettings(user, backend, "large-v3");
    await goToTab(user, "Session");
    await openRecording(user, /Alpha server/);
    await clickIfPossible(user, screen.queryByRole("button", { name: /Download large-v3/ }));
    await settle(300);

    expect(backend.callsTo("download_model_command").filter((c) => c.args.modelName === "large-v3")).toHaveLength(1);
  });
});
