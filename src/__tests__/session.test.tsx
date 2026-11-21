// Long jobs started from an open recording (transcription, re-transcription, AI prompts) must
// land on the recording they were started on, whatever the user does in the meantime.

import { describe, expect, it } from "vitest";
import {
  defaultBackend,
  PATH_A,
  PATH_B,
  recordingA,
  recordingB,
  recordingC,
  recordingPath,
  type PendingCall,
  type SessionState,
} from "./fakeBackend";
import {
  act,
  backToList,
  clickIfPossible,
  clickTranscribe,
  openRecording,
  renderApp,
  screen,
  settle,
  showTranscriptTab,
  waitFor,
} from "./ui";

const NEW_TEXTS = ["Hello from Alice.", "Bob replies here.", "Alice again later."];

type TranscribeCall = PendingCall<SessionState> & { args: { state: SessionState; onlyIndices: number[] | null } };

/** What the backend returns for a transcription: the state it was given, with new texts. */
async function finishTranscription(call: TranscribeCall, texts: string[]) {
  await act(async () => {
    call.resolve({ ...call.args.state, transcript_texts: texts });
  });
}

/** What the backend returns for a single-segment re-transcription. */
async function finishSegment(call: TranscribeCall, text: string) {
  const texts = [...call.args.state.transcript_texts];
  for (const i of call.args.onlyIndices ?? []) texts[i] = text;
  await finishTranscription(call, texts);
}

async function startTranscribingA() {
  const backend = defaultBackend();
  const user = await renderApp();
  await openRecording(user, /Alpha server/);
  await clickTranscribe(user);
  const call = (await backend.nextCall("transcribe_session_command")) as TranscribeCall;
  expect(call.args.state.session_id).toBe(recordingA().session_id);
  return { backend, user, call };
}

describe("transcription", () => {
  it("transcription result goes to the recording it was started on", async () => {
    const { backend, user, call } = await startTranscribingA();
    const originalB = backend.project(PATH_B);

    await backToList(user);
    await openRecording(user, /Beta server/);
    await finishTranscription(call, NEW_TEXTS);
    await settle();

    // B is untouched, on disk and on screen.
    expect(backend.project(PATH_B)).toEqual(originalB);
    for (const text of NEW_TEXTS) expect(screen.queryByText(text, { exact: false })).toBeNull();
    // A got the texts.
    await waitFor(() => expect(backend.project(PATH_A)?.transcript_texts).toEqual(NEW_TEXTS));
    expect(backend.project(PATH_A)?.segments).toEqual(recordingA().segments);
  });

  it("transcription result isn't lost when you leave the recording", async () => {
    const { backend, user, call } = await startTranscribingA();

    await backToList(user);
    await finishTranscription(call, NEW_TEXTS);
    await settle();

    await waitFor(() => expect(backend.project(PATH_A)?.transcript_texts).toEqual(NEW_TEXTS));
  });

  it("edits made during transcription survive", async () => {
    const { backend, user, call } = await startTranscribingA();

    // Give the first turn (Alice) to Bob while the transcription runs.
    await showTranscriptTab(user, "Edit");
    const firstSpeaker = screen.getAllByDisplayValue("Alice")[0];
    await user.selectOptions(firstSpeaker, "Bob");
    await finishTranscription(call, NEW_TEXTS);
    await settle();

    await waitFor(() => expect(backend.project(PATH_A)?.transcript_texts).toEqual(NEW_TEXTS));
    const saved = backend.project(PATH_A)!;
    expect(saved.segments[0]).toMatchObject({ user_id: "u2", speaker_name: "Bob" });
    expect(saved.segments.slice(1)).toEqual(recordingA().segments.slice(1));
  });

  it("recording started during transcription doesn't receive the result", async () => {
    const { backend, user, call } = await startTranscribingA();
    const c = recordingC();
    const pathC = recordingPath(c.session_id);
    backend.nextRecording = c;

    await backToList(user);
    await user.click(screen.getByRole("button", { name: "Start recording" }));
    await user.click(await screen.findByRole("button", { name: "Stop recording" }));
    await waitFor(() => expect(backend.project(pathC)).toBeDefined());

    await finishTranscription(call, NEW_TEXTS);
    await settle();

    const savedC = backend.project(pathC)!;
    expect(savedC.segments).toEqual(c.segments);
    expect(savedC.transcript_texts).toEqual(c.transcript_texts);
    for (const state of backend.savesTo(pathC)) {
      expect(state.segments).toEqual(c.segments);
      expect(state.transcript_texts.some((t) => NEW_TEXTS.includes(t))).toBe(false);
    }
  });

  it("no second whisper run while a full transcription is running", async () => {
    const { backend, user } = await startTranscribingA();

    await showTranscriptTab(user, "Edit");
    const retranscribe = screen.queryAllByRole("button", { name: "Re-transcribe" })[0];
    await clickIfPossible(user, retranscribe);
    await settle(300);

    expect(backend.callsTo("transcribe_session_command")).toHaveLength(1);
  });
});

describe("re-transcribing one segment", () => {
  async function startSegment(index: number) {
    const backend = defaultBackend();
    const user = await renderApp();
    await openRecording(user, /Alpha server/);
    await showTranscriptTab(user, "Edit");
    await user.click(screen.getAllByRole("button", { name: "Re-transcribe" })[index]);
    const call = (await backend.nextCall("transcribe_session_command")) as TranscribeCall;
    expect(call.args.onlyIndices).toEqual([index]);
    return { backend, user, call };
  }

  it("re-transcribing one segment updates that segment, not another recording", async () => {
    const { backend, user, call } = await startSegment(1);
    const originalB = backend.project(PATH_B);

    await backToList(user);
    await openRecording(user, /Beta server/);
    await finishSegment(call, "Fresh Bob text.");
    await settle();

    expect(backend.project(PATH_B)).toEqual(originalB);
    expect(screen.queryByText("Fresh Bob text.", { exact: false })).toBeNull();
    await waitFor(() => expect(backend.project(PATH_A)?.transcript_texts[1]).toBe("Fresh Bob text."));
  });

  it("re-transcribing one segment updates that segment after a merge above it", async () => {
    // Segment 2 starts at 30 s. Merging 0 and 1 while it runs moves it to index 1.
    const { backend, user, call } = await startSegment(2);

    const merged = await clickIfPossible(user, screen.getAllByRole("button", { name: "Merge down" })[0]);
    await finishSegment(call, "Fresh third turn.");
    await settle();

    await waitFor(() => expect(backend.project(PATH_A)?.transcript_texts).toContain("Fresh third turn."));
    const saved = backend.project(PATH_A)!;
    expect(saved.segments).toHaveLength(merged ? 2 : 3);
    saved.segments.forEach((seg, i) => {
      if (seg.start_ms === 30000) expect(saved.transcript_texts[i]).toBe("Fresh third turn.");
      else expect(saved.transcript_texts[i] ?? "").not.toContain("Fresh third turn.");
    });
  });
});

describe("AI prompts", () => {
  const TRANSCRIBED_A = recordingA({ transcript_texts: NEW_TEXTS });

  async function runPromptOnA() {
    const backend = defaultBackend({
      projects: { [PATH_A]: TRANSCRIBED_A, [PATH_B]: recordingB() },
    });
    const user = await renderApp();
    await openRecording(user, /Alpha server/);
    await showTranscriptTab(user, "AI");
    const run = await screen.findByRole("button", { name: "Run" });
    await waitFor(() => expect(run).toBeEnabled());
    await user.click(run);
    const call = await backend.nextCall<string>("run_ai_command");
    expect(call.args.input).toContain("Bob replies here.");
    return { backend, user, call };
  }

  it("AI prompt output is saved to the recording it ran on", async () => {
    const { backend, user, call } = await runPromptOnA();
    const originalB = backend.project(PATH_B);

    await backToList(user);
    await openRecording(user, /Beta server/);
    await act(async () => call.resolve("AI SUMMARY OUTPUT"));
    await settle();

    expect(backend.project(PATH_B)).toEqual(originalB);
    expect(screen.queryByText("AI SUMMARY OUTPUT")).toBeNull();
    await waitFor(() =>
      expect(backend.project(PATH_A)?.ai_outputs?.map((o) => o.output)).toEqual(["AI SUMMARY OUTPUT"]),
    );
  });

  it("the same AI prompt can't be started twice by switching tabs while it runs", async () => {
    const { backend, user } = await runPromptOnA();

    await showTranscriptTab(user, "Read");
    await showTranscriptTab(user, "AI");
    const again = screen.queryByRole("button", { name: /^(Run|Running\.\.\.)$/ });
    await clickIfPossible(user, again);
    await settle(300);

    expect(backend.callsTo("run_ai_command")).toHaveLength(1);
  });
});

