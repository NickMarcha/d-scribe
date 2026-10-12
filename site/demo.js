// Interactive copy of d-scribe with two made-up calls: the recordings list, then a call with
// Read, Edit, Stats and AI. Play only moves a clock until a session has an `audio` file.
(() => {
  const root = document.getElementById("demo");
  if (!root) return;

  const DAY = 86400000;
  const today = new Date();
  today.setHours(14, 2, 0, 0);
  const yesterday = new Date(today.getTime() - DAY);
  yesterday.setHours(19, 34);

  // Lines are [speaker, seconds, text] for a made-up clock, or [speaker, [startMs, endMs], text]
  // when timed against an audio file.
  function makeSegs(lines) {
    let t = 1500;
    return lines.map(([speaker, time, text]) => {
      if (Array.isArray(time)) return { speaker, text, start: time[0], end: time[1] };
      const s = { speaker, text, start: t, end: t + time * 1000 };
      t += time * 1000 + 900;
      return s;
    });
  }

  const sessions = [
    {
      id: "sprint",
      title: "Sprint planning",
      place: "Studio Nord / Meetings",
      date: today,
      saved: true,
      transcribed: true,
      audio: "audio/sprint-planning.mp3",
      durationMs: 56240,
      speakers: ["Leah", "Sam", "Jonas"],
      // Timed against the audio: whisper's line starts, snapped to the nearest pause.
      segs: makeSegs([
        ["Leah", [20, 4270], "Okay, let's start. The main thing today is the release next week and what we cut."],
        ["Sam", [5060, 8670], "I think the export can wait. Nobody has asked for Word files yet."],
        ["Jonas", [8810, 13520], "Agreed. But search has to be in, that's what people keep asking for."],
        ["Leah", [13860, 17290], "Search is mostly done. I need a day to make it fast on long recordings."],
        ["Leah", [17620, 20150], "Maybe two, if the old recordings need converting."],
        ["Sam", [20330, 24530], "What about the onboarding? New users get stuck on the Discord step."],
        ["Leah", [24900, 30070], "Yes. I want a short checklist on the first screen: connect Discord, pick a language, record a test call."],
        ["Jonas", [30370, 34680], "Can we test that with someone who hasn't seen the app?"],
        ["Sam", [34900, 38110], "My brother can try it on Thursday. He'll find every problem."],
        ["Jonas", [38250, 40620], "He found three bugs in my bike app in ten minutes."],
        ["Leah", [41040, 46550], "Perfect. So, search and onboarding are in, and export moves to the next release."],
        ["Jonas", [46990, 50460], "I'll write the release notes when the build is ready. Friday?"],
        ["Sam", [50700, 52180], "Friday works for me."],
        ["Leah", [52260, 55950], "Great. Thanks, both. Same time next week."],
      ]),
      ai: {
        Summary:
          "The team planned next week's release. Search and a first-run checklist are in; Word export moves to the next release. Sam's brother tests the onboarding on Thursday, and Jonas writes the release notes for Friday.",
        "Action points":
          "- Leah: finish search and make it fast on long recordings (1 to 2 days)\n- Leah: first-run checklist (connect Discord, pick a language, test call)\n- Sam: ask his brother to test on Thursday\n- Jonas: release notes by Friday",
      },
    },
    {
      id: "bergen",
      title: null,
      place: "Call with Sam",
      date: yesterday,
      saved: false,
      transcribed: false,
      audio: null,
      speakers: ["Leah", "Sam"],
      segs: makeSegs([
        ["Sam", 4, "Hey, did you get the train tickets for Bergen?"],
        ["Leah", 5, "Not yet. The prices went up again, so I wanted to check with you first."],
        ["Sam", 2, "How much are they now?"],
        ["Leah", 5, "About nine hundred kroner each, if we take the early train on Saturday."],
        ["Sam", 5, "That's fine. The later one gets in too late for the hike anyway."],
        ["Leah", 4, "Okay, I'll book the early one. Can you sort out where we stay?"],
        ["Sam", 5, "Yes. My cousin has a flat near the fish market, I'll ask her tonight."],
        ["Leah", 3, "Perfect. And bring a rain jacket. It's Bergen."],
        ["Sam", 4, "Ha, always. I'll message you when I hear from her."],
        ["Leah", 2, "Sounds good. Talk later."],
      ]),
      ai: {
        Summary:
          "Leah and Sam planned a trip to Bergen. They take the early train on Saturday (about 900 kroner each) so they make the hike. Sam asks his cousin about staying in her flat near the fish market.",
        "Action points": "- Leah: book two tickets for the early train on Saturday\n- Sam: ask his cousin about the flat tonight, then message Leah\n- Both: pack a rain jacket",
      },
    },
  ];

  const fmt = (ms) => {
    const s = Math.floor(ms / 1000);
    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
  };
  const esc = (s) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
  const clock = (d) => d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  const length = (s) => s.durationMs ?? s.segs[s.segs.length - 1].end + 1000;
  const minutes = (ms) => (ms < 60000 ? "< 1 min" : `${Math.round(ms / 60000)} min`);

  let open = null; // the session being shown, or null for the list
  let view = "read";
  let query = "";
  let now = 0;
  let playing = false;
  let last = 0;
  let active = -1;
  let history = [];
  let transcribing = false;
  let listQuery = "";

  const bar = `
    <div class="demo-bar">
      <span class="demo-brand">d-scribe</span>
      <span class="demo-tab demo-tab--on">Session</span><span class="demo-tab">Settings</span>
      <span class="demo-status">Discord: connected</span>
    </div>`;

  // Recordings list

  function renderList() {
    stop();
    open = null;
    const q = listQuery.toLowerCase();
    const shown = sessions.filter((s) =>
      [s.title, s.place, ...s.speakers].some((v) => v && v.toLowerCase().includes(q)),
    );
    const days = [
      ["Today", shown.filter((s) => s.date === today)],
      ["Yesterday", shown.filter((s) => s.date === yesterday)],
    ].filter(([, items]) => items.length);
    root.innerHTML = `${bar}
      <div class="demo-list">
        <h3>Recordings</h3>
        <input class="demo-search demo-list-search" type="search" placeholder="Search titles, people, servers and channels" aria-label="Search recordings" value="${esc(listQuery)}" />
        ${days.length ? "" : `<p class="demo-muted">No recordings match.</p>`}
        ${days
          .map(
            ([label, items]) => `<p class="demo-day">${label}</p><div class="demo-recs">${items
              .map(
                (s) => `<div class="demo-rec">
                  <button type="button" class="demo-rec-open" data-id="${s.id}">
                    <span class="demo-rec-time">${clock(s.date)}</span>
                    <span class="demo-rec-main">
                      <span class="demo-rec-place">${s.title ? `${esc(s.title)} <span class="demo-muted">${esc(s.place)}</span>` : esc(s.place)}</span>
                      <span class="demo-muted">${s.speakers.join(", ")}</span>
                    </span>
                    <span class="demo-rec-info">${minutes(length(s))}${s.transcribed ? "" : `<span class="demo-warn">Not transcribed</span>`}</span>
                  </button>
                  <button type="button" class="demo-rec-save${s.saved ? " on" : ""}" data-id="${s.id}">${s.saved ? "Saved" : "Save"}</button>
                </div>`,
              )
              .join("")}</div>`,
          )
          .join("")}
        <p class="demo-muted demo-small">Unsaved recordings are deleted after 10 days. Saved ones are kept.</p>
      </div>`;
    const search = root.querySelector(".demo-list-search");
    search.addEventListener("input", () => {
      listQuery = search.value;
      renderList();
      const again = root.querySelector(".demo-list-search");
      again.focus();
      again.setSelectionRange(again.value.length, again.value.length);
    });
    root.querySelectorAll(".demo-rec-open").forEach((b) =>
      b.addEventListener("click", () => openSession(sessions.find((s) => s.id === b.dataset.id))),
    );
    root.querySelectorAll(".demo-rec-save").forEach((b) =>
      b.addEventListener("click", () => {
        const s = sessions.find((x) => x.id === b.dataset.id);
        s.saved = !s.saved;
        renderList();
      }),
    );
  }

  // One call

  function openSession(s) {
    open = s;
    view = "read";
    query = "";
    now = 0;
    active = -1;
    history = [];
    root.innerHTML = `${bar}
      <div class="demo-head">
        <div class="demo-row">
          <button type="button" class="demo-link demo-back">Recordings</button>
          <span class="demo-title">${s.title ? `${esc(s.title)} <span class="demo-muted">${esc(s.place)}</span>` : esc(s.place)}</span>
          <span class="demo-btn demo-wide">English</span>
          <button type="button" class="demo-btn demo-transcribe">Transcribe</button>
          <button type="button" class="demo-btn demo-save">Save</button>
        </div>
        <div class="demo-row">
          <button type="button" class="demo-btn demo-play">Play</button>
          <input class="demo-seek" type="range" min="0" max="${length(s)}" value="0" aria-label="Seek" />
          <span class="demo-time"></span>
        </div>
        <div class="demo-progress" hidden><span></span></div>
      </div>
      <div class="demo-tabs">
        <button type="button" data-view="read" class="demo-view">Read</button>
        <button type="button" data-view="edit" class="demo-view">Edit</button>
        <button type="button" data-view="stats" class="demo-view">Stats</button>
        <button type="button" data-view="ai" class="demo-view">AI</button>
        <input class="demo-search demo-text-search" type="search" placeholder="Search transcript" aria-label="Search transcript" />
      </div>
      <div class="demo-body"></div>`;
    if (s.audio) {
      s.player = s.player || new Audio(s.audio);
    }
    root.querySelector(".demo-back").addEventListener("click", renderList);
    root.querySelector(".demo-play").addEventListener("click", () => (playing ? stop() : play()));
    root.querySelector(".demo-seek").addEventListener("input", (e) => seekTo(Number(e.target.value), true));
    root.querySelector(".demo-transcribe").addEventListener("click", transcribe);
    root.querySelector(".demo-save").addEventListener("click", () => {
      s.saved = !s.saved;
      renderHead();
    });
    root.querySelector(".demo-text-search").addEventListener("input", (e) => {
      query = e.target.value.trim();
      if (view !== "read") view = "read";
      renderBody();
      const hit = root.querySelector(".demo-seg mark");
      if (hit) scrollInside(hit.parentElement);
    });
    root.querySelectorAll(".demo-view").forEach((b) =>
      b.addEventListener("click", () => {
        view = b.dataset.view;
        renderBody();
      }),
    );
    renderHead();
    renderBody();
  }

  function renderHead() {
    const s = open;
    const save = root.querySelector(".demo-save");
    save.textContent = s.saved ? "Saved" : "Save";
    save.classList.toggle("demo-btn--on", s.saved);
    const tr = root.querySelector(".demo-transcribe");
    tr.textContent = transcribing ? "Transcribing..." : "Transcribe";
    tr.disabled = transcribing;
    root.querySelector(".demo-seek").value = now;
    root.querySelector(".demo-time").textContent = `${fmt(now)} / ${fmt(length(s))}`;
  }

  function blocks(s) {
    const out = [];
    s.segs.forEach((seg, i) => {
      const prev = out[out.length - 1];
      if (prev && prev.speaker === seg.speaker) prev.items.push(i);
      else out.push({ speaker: seg.speaker, items: [i] });
    });
    return out;
  }

  function mark(text) {
    if (!query) return esc(text);
    const at = text.toLowerCase().indexOf(query.toLowerCase());
    if (at < 0) return esc(text);
    return `${esc(text.slice(0, at))}<mark>${esc(text.slice(at, at + query.length))}</mark>${esc(text.slice(at + query.length))}`;
  }

  function renderBody() {
    const s = open;
    root.querySelectorAll(".demo-view").forEach((b) => b.classList.toggle("demo-view--on", b.dataset.view === view));
    const body = root.querySelector(".demo-body");
    active = -1;
    if (view === "read") {
      const shown = (i) => s.transcribed || s.segs[i].shown;
      body.innerHTML =
        (s.transcribed || transcribing ? "" : `<p class="demo-empty">Not transcribed yet. Click Transcribe.</p>`) +
        blocks(s)
          .map(
            (b) => `<section class="demo-block"><div class="demo-meta"><b>${esc(b.speaker)}</b><span>${fmt(s.segs[b.items[0]].start)}</span></div><p>${
              shown(b.items[0])
                ? b.items.map((i) => (shown(i) ? `<span class="demo-seg" data-i="${i}">${mark(s.segs[i].text)}</span>` : "")).join(" ")
                : `<span class="demo-seg demo-seg--empty" data-i="${b.items[0]}">${fmt(b.items.reduce((sum, i) => sum + s.segs[i].end - s.segs[i].start, 0))} of speech</span>`
            }</p></section>`,
          )
          .join("");
      body.querySelectorAll(".demo-seg").forEach((el) =>
        el.addEventListener("click", () => {
          seekTo(s.segs[Number(el.dataset.i)].start, false);
          play();
        }),
      );
      highlight(false);
    } else if (view === "edit") {
      if (!s.transcribed) {
        body.innerHTML = `<p class="demo-empty">Transcribe first, then fix words and speakers here.</p>`;
        return;
      }
      body.innerHTML =
        `<div class="demo-edit-bar"><button type="button" class="demo-btn demo-undo"${history.length ? "" : " disabled"}>Undo</button><span class="demo-muted">Fix a word, change who said it, or merge and delete lines. Read shows the changes.</span></div>` +
        s.segs
          .map(
            (seg, i) => `<div class="demo-edit-row" data-i="${i}">
              <span class="demo-time">${fmt(seg.start)}</span>
              <select class="demo-speaker" aria-label="Speaker">${s.speakers.map((n) => `<option${n === seg.speaker ? " selected" : ""}>${n}</option>`).join("")}</select>
              <textarea class="demo-text" rows="1" aria-label="Text">${esc(seg.text)}</textarea>
              <span class="demo-edit-actions">
                ${i < s.segs.length - 1 ? `<button type="button" class="demo-btn demo-merge" title="Merge with the next line">Merge</button>` : ""}
                <button type="button" class="demo-btn demo-del" title="Delete this line">Delete</button>
              </span>
            </div>`,
          )
          .join("");
      body.querySelectorAll("textarea").forEach(fit);
      const rowIndex = (el) => Number(el.closest(".demo-edit-row").dataset.i);
      body.querySelector(".demo-undo").addEventListener("click", () => {
        s.segs = history.pop();
        renderBody();
      });
      body.querySelectorAll(".demo-speaker").forEach((el) =>
        el.addEventListener("change", () => {
          edit(() => (s.segs[rowIndex(el)].speaker = el.value));
          renderBody();
        }),
      );
      body.querySelectorAll(".demo-text").forEach((el) => {
        // One undo step per line you start typing in.
        el.addEventListener("focus", () => edit(() => {}));
        el.addEventListener("input", () => {
          s.segs[rowIndex(el)].text = el.value;
          fit(el);
          body.querySelector(".demo-undo").disabled = false;
        });
      });
      body.querySelectorAll(".demo-merge").forEach((el) =>
        el.addEventListener("click", () => {
          const i = rowIndex(el);
          edit(() => {
            const [a, b] = [s.segs[i], s.segs[i + 1]];
            s.segs.splice(i, 2, { ...a, text: `${a.text} ${b.text}`, end: b.end });
          });
          renderBody();
        }),
      );
      body.querySelectorAll(".demo-del").forEach((el) =>
        el.addEventListener("click", () => {
          if (s.segs.length === 1) return;
          edit(() => s.segs.splice(rowIndex(el), 1));
          renderBody();
        }),
      );
    } else if (view === "stats") {
      const talk = {};
      s.segs.forEach((seg) => (talk[seg.speaker] = (talk[seg.speaker] || 0) + seg.end - seg.start));
      const max = Math.max(...Object.values(talk));
      const spoken = Object.values(talk).reduce((a, b) => a + b, 0);
      body.innerHTML =
        `<div class="demo-pad"><p class="demo-muted">Talk time</p>` +
        Object.entries(talk)
          .sort((a, b) => b[1] - a[1])
          .map(
            ([name, ms]) =>
              `<div class="demo-stat"><span>${esc(name)}</span><span class="demo-track"><span style="width:${(ms / max) * 100}%"></span></span><span class="demo-muted">${fmt(ms)}, ${Math.round((ms / spoken) * 100)}%</span></div>`,
          )
          .join("") +
        `<p class="demo-muted">${s.segs.length} turns in ${fmt(length(s))}</p></div>`;
    } else {
      body.innerHTML = s.transcribed
        ? `<div class="demo-pad"><div class="demo-row">
            <select class="demo-prompt" aria-label="Prompt">${Object.keys(s.ai).map((k) => `<option>${k}</option>`).join("")}</select>
            <button type="button" class="demo-btn demo-run">Run</button>
            <span class="demo-muted">Runs your own AI tool, for example Claude Code.</span>
          </div><pre class="demo-ai-out"></pre></div>`
        : `<p class="demo-empty">Transcribe first, then ask your AI for a summary.</p>`;
      const run = body.querySelector(".demo-run");
      if (run) run.addEventListener("click", () => typeOut(s.ai[body.querySelector(".demo-prompt").value]));
    }
  }

  function fit(el) {
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }

  function edit(change) {
    history.push(open.segs.map((seg) => ({ ...seg })));
    if (history.length > 50) history.shift();
    change();
  }

  let typing = 0;
  function typeOut(text) {
    const out = root.querySelector(".demo-ai-out");
    const id = ++typing;
    out.textContent = "Running...";
    let n = 0;
    setTimeout(function step() {
      if (id !== typing || !out.isConnected) return;
      n += 3;
      out.textContent = text.slice(0, n);
      if (n < text.length) setTimeout(step, 12);
    }, 500);
  }

  // Fills in the text one turn at a time, like a real transcription job.
  function transcribe() {
    const s = open;
    if (transcribing) return;
    transcribing = true;
    s.transcribed = false;
    s.segs.forEach((seg) => (seg.shown = false));
    view = "read";
    renderHead();
    renderBody();
    const progress = root.querySelector(".demo-progress");
    progress.hidden = false;
    let i = 0;
    const step = () => {
      if (open !== s) {
        transcribing = false;
        s.transcribed = true;
        return;
      }
      s.segs[i].shown = true;
      i++;
      progress.firstElementChild.style.width = `${(i / s.segs.length) * 100}%`;
      if (i < s.segs.length) {
        if (view === "read") renderBody();
        setTimeout(step, 260);
      } else {
        transcribing = false;
        s.transcribed = true;
        progress.hidden = true;
        renderHead();
        renderBody();
      }
    };
    setTimeout(step, 400);
  }

  // Playback: the real audio when the session has a file, otherwise a clock.

  function scrollInside(el) {
    const body = root.querySelector(".demo-body");
    body.scrollTo({ top: el.offsetTop - body.clientHeight / 3, behavior: "smooth" });
  }

  function highlight(scroll) {
    if (!open || view !== "read") return;
    const i = open.segs.findIndex((seg) => now >= seg.start && now < seg.end);
    if (i === active) return;
    root.querySelector(`.demo-seg[data-i="${active}"]`)?.classList.remove("on");
    active = i;
    const el = root.querySelector(`.demo-seg[data-i="${i}"]`);
    if (el) {
      el.classList.add("on");
      if (scroll) scrollInside(el);
    }
  }

  function seekTo(ms, scroll) {
    now = Math.max(0, Math.min(length(open), ms));
    if (open.player) open.player.currentTime = now / 1000;
    renderHead();
    highlight(scroll);
  }

  function play() {
    if (!open) return;
    if (now >= length(open)) seekTo(0, true);
    playing = true;
    root.querySelector(".demo-play").textContent = "Pause";
    if (open.player) open.player.play().catch(stop);
    last = performance.now();
    requestAnimationFrame(tick);
  }

  function stop() {
    playing = false;
    sessions.forEach((s) => s.player?.pause());
    const btn = root.querySelector(".demo-play");
    if (btn) btn.textContent = "Play";
  }

  function tick(ts) {
    if (!playing || !open) return;
    now = open.player ? open.player.currentTime * 1000 : Math.min(length(open), now + (ts - last));
    last = ts;
    renderHead();
    highlight(true);
    if (now >= length(open) || open.player?.ended) return stop();
    requestAnimationFrame(tick);
  }

  renderList();
})();
