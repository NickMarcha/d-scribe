// Points the download links at the latest installer, and shows the SmartScreen note after a click.
(async () => {
  const links = document.querySelectorAll(".download-link");
  const notice = document.querySelector(".smartscreen");
  links.forEach((a) =>
    a.addEventListener("click", () => {
      notice.hidden = false;
    }),
  );
  try {
    const res = await fetch("https://api.github.com/repos/NickMarcha/d-scribe/releases/latest");
    if (!res.ok) return;
    const release = await res.json();
    const asset = release.assets.find((a) => a.name.endsWith("-setup.exe"));
    if (!asset) return;
    links.forEach((a) => (a.href = asset.browser_download_url));
    const size = Math.round(asset.size / 1024 ** 2);
    document.querySelector(".version").textContent = `Version ${release.tag_name.replace(/^app-v/, "")}, ${size} MB`;
  } catch {
    // Keep the link to the releases page.
  }
})();
