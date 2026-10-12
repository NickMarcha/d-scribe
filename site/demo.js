// Interactive copy of the d-scribe transcript view, with made-up data. No sound: Play only moves the clock.
(() => {
  const root = document.getElementById("demo");
  if (!root) return;

  const lines = [
    ["Leah", 5, "Okay, let's start. The main thing today is the release next week and what we cut."],
    ["Sam", 6, "I think the export can wait. Nobody has asked for Word files yet."],
    ["Jonas", 5, "Agreed. But search has to be in, that's what people keep asking for."],
    ["Leah", 6, "Search is mostly done. I need a day to make it fast on long recordings."],
    ["Leah", 3, "Maybe two, if the old recordings need converting."],
    ["Sam", 5, "What about the onboarding? New users get stuck on the Discord step."],
    ["Leah", 7, "Yes. I want a short checklist on the first screen: connect Discord, pick a language, record a test call."],
    ["Jonas", 4, "Can we test that with someone who hasn't seen the app?"],
    ["Sam", 4, "My brother can try it on Thursday. He'll find every problem."],
    ["Jonas", 3, "He found three bugs in my bike app in ten minutes."],
    ["Leah", 6, "Perfect. So, search and onboarding are in, and export moves to the next release."],
    ["Jonas", 5, "I'll write the release notes when the build is ready. Friday?"],
    ["Sam", 3, "Friday works for me."],
    ["Leah", 4, "Great. Thanks, both. Same time next week."],
  ];
  let t = 1500;
  const segs = lines.map(([speaker, secs, text]) => {
    const s = { speaker, text, start: t, end: t + secs * 1000 };
    t += secs * 1000 + 900;
    return s;
  });
  const total = t + 1000;

  const ai = {
    Summary:
      "The team planned next week's release. Search and a first-run checklist are in; Word export moves to the next release. Sam's brother tests the onboarding on Thursday, and Jonas writes the release notes for Friday.",
    "Action points":
      "- Leah: finish search and make it fast on long recordings (1 to 2 days)\n- Leah: first-run checklist (connect Discord, pick a language, test call)\n- Sam: ask his brother to test on Thursday\n- Jonas: release notes by Friday",
  };

  const fmt = (ms) => {
    const s = Math.floor(ms / 1000);
    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
  };
  const esc = (s) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

  // Consecutive turns by the same speaker form one block, like the app's reader.
  const blocks = [];
  segs.forEach((s, i) => {
    const last = blocks[blocks.length - 1];
    if (last && last.speaker === s.speaker) last.items.push(i);
    else blocks.push({ speaker: s.speaker, items: [i] });
  });

  root.innerHTML = `
    <div class="demo-bar">
      <span class="demo-brand">d-scribe</span>
      <span class="demo-tab demo-tab--on">Session</span><span class="demo-tab">Settings</span>
      <span class="demo-status">Discord: in voice</span>
    </div>
    <div class="demo-head">
      <div class="demo-row">
        <span class="demo-link">Recordings</span>
        <span class="demo-title">Sprint planning <span class="demo-muted">Studio Nord / Meetings</span></span>
        <span class="demo-btn">English</span><span class="demo-btn">Transcribe</span><span class="demo-btn demo-btn--on">Saved</span>
      </div>
      <div class="demo-row">
        <button type="button" class="demo-btn demo-play">Play</button>
        <input class="demo-seek" type="range" min="0" max="${total}" value="0" aria-label="Seek" />
        <span class="demo-time">0:00 / ${fmt(total)}</span>
      </div>
    </div>
    <div class="demo-tabs">
      <button type="button" data-view="read" class="demo-view demo-view--on">Read</button>
      <button type="button" data-view="stats" class="demo-view">Stats</button>
      <button type="button" data-view="ai" class="demo-view">AI</button>
      <input class="demo-search" type="search" placeholder="Search transcript" aria-label="Search transcript" />
    </div>
    <div class="demo-body">
      <div class="demo-pane" data-pane="read">${blocks
        .map(
          (b) => `<section class="demo-block"><div class="demo-meta"><b>${b.speaker}</b><span>${fmt(segs[b.items[0]].start)}</span></div><p>${b.items
            .map((i) => `<span class="demo-seg" data-i="${i}">${esc(segs[i].text)}</span>`)
            .join(" ")}</p></section>`,
        )
        .join("")}</div>
      <div class="demo-pane" data-pane="stats" hidden></div>
      <div class="demo-pane" data-pane="ai" hidden>
        <div class="demo-row">
          <select class="demo-prompt" aria-label="Prompt">${Object.keys(ai).map((k) => `<option>${k}</option>`).join("")}</select>
          <button type="button" class="demo-btn demo-run">Run</button>
          <span class="demo-muted">Runs your own AI tool, for example Claude Code.</span>
        </div>
        <pre class="demo-ai-out"></pre>
      </div>
    </div>`;

  const $ = (sel) => root.querySelector(sel);
  const segEls = [...root.querySelectorAll(".demo-seg")];
  const body = $(".demo-body");
  const playBtn = $(".demo-play");
  const seek = $(".demo-seek");
  const time = $(".demo-time");
  let now = 0;
  let playing = false;
  let last = 0;
  let active = -1;

  // Scrolls inside the demo only, never the page. offsetTop is relative to .demo-body (position: relative).
  function scrollTo(el) {
    body.scrollTo({ top: el.offsetTop - body.clientHeight / 3, behavior: "smooth" });
  }

  function render(scroll) {
    seek.value = now;
    time.textContent = `${fmt(now)} / ${fmt(total)}`;
    const i = segs.findIndex((s) => now >= s.start && now < s.end);
    if (i !== active) {
      segEls[active]?.classList.remove("on");
      active = i;
      if (i >= 0) {
        segEls[i].classList.add("on");
        if (scroll) scrollTo(segEls[i]);
      }
    }
  }

  function tick(ts) {
    if (!playing) return;
    now = Math.min(total, now + (ts - last));
    last = ts;
    render(true);
    if (now >= total) return setPlaying(false);
    requestAnimationFrame(tick);
  }

  function setPlaying(on) {
    playing = on;
    playBtn.textContent = on ? "Pause" : "Play";
    if (on) {
      if (now >= total) now = 0;
      last = performance.now();
      requestAnimationFrame(tick);
    }
  }

  playBtn.addEventListener("click", () => setPlaying(!playing));
  seek.addEventListener("input", () => {
    now = Number(seek.value);
    render(true);
  });
  segEls.forEach((el) =>
    el.addEventListener("click", () => {
      now = segs[Number(el.dataset.i)].start;
      render(false);
      setPlaying(true);
    }),
  );

  $(".demo-search").addEventListener("input", (e) => {
    const q = e.target.value.trim().toLowerCase();
    segEls.forEach((el, i) => {
      const text = segs[i].text;
      const at = q ? text.toLowerCase().indexOf(q) : -1;
      el.innerHTML =
        at < 0 ? esc(text) : `${esc(text.slice(0, at))}<mark>${esc(text.slice(at, at + q.length))}</mark>${esc(text.slice(at + q.length))}`;
    });
    showView("read");
    const hit = root.querySelector(".demo-seg mark");
    if (hit) scrollTo(hit.parentElement);
  });

  // Stats: talk time per speaker.
  const talk = {};
  segs.forEach((s) => (talk[s.speaker] = (talk[s.speaker] || 0) + s.end - s.start));
  const max = Math.max(...Object.values(talk));
  const spoken = Object.values(talk).reduce((a, b) => a + b, 0);
  $('[data-pane="stats"]').innerHTML =
    `<p class="demo-muted">Talk time</p>` +
    Object.entries(talk)
      .sort((a, b) => b[1] - a[1])
      .map(
        ([name, ms]) =>
          `<div class="demo-stat"><span>${name}</span><span class="demo-track"><span style="width:${(ms / max) * 100}%"></span></span><span class="demo-muted">${fmt(ms)}, ${Math.round((ms / spoken) * 100)}%</span></div>`,
      )
      .join("") +
    `<p class="demo-muted">${segs.length} turns in ${fmt(total)}</p>`;

  function showView(view) {
    root.querySelectorAll(".demo-view").forEach((b) => b.classList.toggle("demo-view--on", b.dataset.view === view));
    root.querySelectorAll(".demo-pane").forEach((p) => (p.hidden = p.dataset.pane !== view));
  }
  root.querySelectorAll(".demo-view").forEach((b) => b.addEventListener("click", () => showView(b.dataset.view)));

  // AI: type out a canned answer.
  const out = $(".demo-ai-out");
  let typing = 0;
  $(".demo-run").addEventListener("click", () => {
    const text = ai[$(".demo-prompt").value];
    const id = ++typing;
    out.textContent = "Running...";
    let n = 0;
    setTimeout(function step() {
      if (id !== typing) return;
      n += 3;
      out.textContent = text.slice(0, n);
      if (n < text.length) setTimeout(step, 12);
    }, 500);
  });

  render(false);
})();
