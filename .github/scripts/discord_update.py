"""Posts a published release to the #updates channel through a Discord webhook.

Reads RELEASE_NAME, RELEASE_URL, RELEASE_BODY and DISCORD_WEBHOOK from the environment.
With --dry-run, prints the message instead of posting it.
"""
import json
import os
import re
import sys
import urllib.request

LIMIT = 2000  # Discord message length limit


def changes(body: str) -> list[str]:
    """Turns GitHub's generated "* Title by @user in <PR url>" lines into "- Title ([#2](url))"."""
    lines = []
    for line in body.splitlines():
        m = re.match(r"^\* (.+?) by @\S+ in (https://github\.com/\S+/pull/(\d+))\s*$", line)
        if m:
            lines.append(f"- {m.group(1)} ([#{m.group(3)}](<{m.group(2)}>))")
    return lines


def message(name: str, url: str, body: str) -> str:
    head = f"**{name}**"
    tail = f"d-scribe updates itself the next time it starts. Release page: <{url}>"
    more = "- and more on the release page"
    lines = [head]
    for item in changes(body):
        if len("\n".join([*lines, item, more, "", tail])) > LIMIT:
            lines.append(more)
            break
        lines.append(item)
    return "\n".join([*lines, "", tail])


def main() -> None:
    text = message(os.environ["RELEASE_NAME"], os.environ["RELEASE_URL"], os.environ.get("RELEASE_BODY", ""))
    if "--dry-run" in sys.argv:
        print(text)
        return
    req = urllib.request.Request(
        os.environ["DISCORD_WEBHOOK"],
        data=json.dumps({"content": text, "allowed_mentions": {"parse": []}}).encode(),
        headers={"Content-Type": "application/json", "User-Agent": "d-scribe-release (github actions)"},
        method="POST",
    )
    with urllib.request.urlopen(req) as r:
        print("Posted to Discord:", r.status)


if __name__ == "__main__":
    main()
