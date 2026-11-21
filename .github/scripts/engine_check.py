"""Smoke test for the whisper.cpp engines the app downloads (see src-tauri/src/transcription/engine.rs).

  python engine_check.py --kind cpu --work DIR   download the engine from the URL in engine.rs,
                                                 check its SHA256 against engine.rs, run it
  python engine_check.py --zip FILE --work DIR   run a local engine zip (before uploading it)
  python engine_check.py --kind cuda --hash-only check the download only (no NVIDIA GPU in CI)

Running means: start whisper-server.exe the way the app does, transcribe whisper.cpp's jfk.wav
sample with language "auto" and response_format "verbose_json", and check the fields the app
reads (text, segments[].no_speech_prob / avg_logprob, language_probabilities.en).
Exits non-zero on any failure, after printing what it found.
"""
import argparse
import hashlib
import json
import os
import re
import socket
import subprocess
import sys
import time
import urllib.request
import uuid
import zipfile

ENGINE_RS = os.path.join(os.path.dirname(__file__), "..", "..", "src-tauri", "src", "transcription", "engine.rs")
MODEL_URL = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin"
KINDS = {"cpu": "Cpu", "cuda": "Cuda", "vulkan": "Vulkan"}


def engine_rs():
    src = open(ENGINE_RS, encoding="utf8").read()
    release = re.search(r'const RELEASE: &str = "([^"]+)";', src).group(1)
    # The Windows builds: the first `fn asset` (the one under #[cfg(windows)]).
    asset_fn = src[src.index("fn asset(kind"):]
    asset_fn = asset_fn[: asset_fn.index("\n}\n")]
    entries = re.findall(r'EngineKind::(\w+) => \(\s*format!\("([^"]+)"\),\s*"([0-9a-f]+)",', asset_fn)
    urls = {k: u.replace("{RELEASE}", release) for k, u, _ in entries}
    hashes = {k: h for k, _, h in entries}
    return release, urls, hashes


def fetch(url, path):
    if not os.path.exists(path):
        print(f"Downloading {url}")
        urllib.request.urlretrieve(url, path + ".part")
        os.replace(path + ".part", path)
    return path


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def unpack(zip_path, out):
    """Same as the app: flatten folders, keep the exes and DLLs."""
    os.makedirs(out, exist_ok=True)
    with zipfile.ZipFile(zip_path) as z:
        for info in z.infolist():
            name = os.path.basename(info.filename)
            low = name.lower()
            if info.is_dir() or not (low.endswith(".dll") or low in ("whisper-server.exe", "whisper-cli.exe")):
                continue
            with z.open(info) as src, open(os.path.join(out, name), "wb") as dst:
                dst.write(src.read())
    return out


def multipart(fields, file_name, file_bytes):
    boundary = uuid.uuid4().hex
    parts = []
    for k, v in fields.items():
        parts.append(f'--{boundary}\r\nContent-Disposition: form-data; name="{k}"\r\n\r\n{v}\r\n'.encode())
    parts.append(
        f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="{file_name}"\r\n'
        f"Content-Type: audio/wav\r\n\r\n".encode() + file_bytes + b"\r\n"
    )
    parts.append(f"--{boundary}--\r\n".encode())
    return b"".join(parts), f"multipart/form-data; boundary={boundary}"


def run(engine_dir, model, wav):
    """Returns a list of problems (empty if the engine works)."""
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    log_path = os.path.join(engine_dir, "server.log")
    log = open(log_path, "w")
    # Same arguments as WhisperServer::start in engine.rs.
    proc = subprocess.Popen(
        [os.path.join(engine_dir, "whisper-server.exe"), "--host", "127.0.0.1", "--port", str(port), "-bs", "5", "-m", model],
        cwd=engine_dir, stdout=log, stderr=subprocess.STDOUT,
    )
    base = f"http://127.0.0.1:{port}"
    problems = []
    try:
        deadline = time.time() + 180
        while True:
            if proc.poll() is not None:
                problems.append(f"whisper-server exited with {proc.returncode} while loading")
                return problems
            try:
                urllib.request.urlopen(base, timeout=2)
                break
            except Exception:
                if time.time() > deadline:
                    problems.append("whisper-server did not start within 3 minutes")
                    return problems
                time.sleep(0.5)
        body, ctype = multipart({"language": "auto", "response_format": "verbose_json"}, "jfk.wav", open(wav, "rb").read())
        req = urllib.request.Request(base + "/inference", data=body, headers={"Content-Type": ctype})
        resp = json.loads(urllib.request.urlopen(req, timeout=300).read())
        print("Text:", resp.get("text", "").strip())
        if "country" not in resp.get("text", "").lower():
            problems.append("text doesn't contain 'country' (expected the JFK quote)")
        segs = resp.get("segments")
        if not segs:
            problems.append("no segments in the response")
        for seg in segs or []:
            for field in ("text", "no_speech_prob", "avg_logprob"):
                if field not in seg:
                    problems.append(f"segment has no '{field}'")
        en = (resp.get("language_probabilities") or {}).get("en")
        print("English probability:", en)
        if not isinstance(en, (int, float)) or en < 0.5:
            problems.append(f"language_probabilities.en is {en!r}, expected >= 0.5")
    except Exception as e:
        problems.append(f"request failed: {e}")
    finally:
        proc.kill()
        proc.wait()
        log.close()
        lines = open(log_path, errors="replace").read().splitlines()
        backends = [l for l in lines if re.search(r"vulkan|ggml_vk|load_backend|CUDA|using device|no GPU", l, re.I)]
        print("Backend lines from the server log:\n  " + "\n  ".join(backends[:15] or ["(none)"]))
        if problems:
            print("Last lines of the server log:\n  " + "\n  ".join(lines[-40:]))
    return problems


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--kind", choices=KINDS)
    ap.add_argument("--zip")
    ap.add_argument("--work", default="engine-check")
    ap.add_argument("--hash-only", action="store_true")
    args = ap.parse_args()
    args.work = os.path.abspath(args.work)
    os.makedirs(args.work, exist_ok=True)
    release, urls, hashes = engine_rs()
    problems = []

    if args.zip:
        zip_path, label = args.zip, os.path.basename(args.zip)
        print(f"{label}: SHA256 {sha256(zip_path)}")
    else:
        variant = KINDS[args.kind]
        label = f"{args.kind} engine for {release}"
        zip_path = fetch(urls[variant], os.path.join(args.work, f"{args.kind}-{release}.zip"))
        actual, expected = sha256(zip_path), hashes.get(variant)
        print(f"{label}: SHA256 {actual}")
        if actual != expected:
            problems.append(f"SHA256 is {actual}, engine.rs says {expected}. Put the new value in `asset` in engine.rs.")
        if args.hash_only:
            return finish(label, problems)

    model = fetch(MODEL_URL, os.path.join(args.work, "ggml-tiny.bin"))
    wav = fetch(f"https://raw.githubusercontent.com/ggml-org/whisper.cpp/{release}/samples/jfk.wav", os.path.join(args.work, "jfk.wav"))
    engine_dir = unpack(zip_path, os.path.join(args.work, "engine-" + os.path.basename(zip_path).removesuffix(".zip")))
    problems += run(engine_dir, model, wav)
    finish(label, problems)


def finish(label, problems):
    if problems:
        print(f"FAILED: {label}\n  - " + "\n  - ".join(problems))
        sys.exit(1)
    print(f"OK: {label}")


if __name__ == "__main__":
    main()
