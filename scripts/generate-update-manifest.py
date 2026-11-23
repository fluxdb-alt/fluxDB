#!/usr/bin/env python3
"""Generate the signed-updater manifest from a published GitHub Release."""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path


PLATFORM_ASSETS = {
    "macos-aarch64": ("app.tar.gz", "macos_app_tar_gz"),
    "macos-x86_64": ("app.tar.gz", "macos_app_tar_gz"),
    "windows-x86_64": ("windows-x64-setup.exe", "windows_inno"),
    "linux-x86_64": (".deb", "linux_deb"),
}

MAX_RELEASE_HISTORY = 5
MAX_RELEASE_NOTES_BYTES = 48 * 1024


def semver_sort_key(version: str) -> tuple[int, ...]:
    numbers: list[int] = []
    for part in version.split(".")[:3]:
        match = re.fullmatch(r"(\d+)(?:[-+].*)?", part)
        if match is None:
            raise RuntimeError(f"invalid semantic version: {version!r}")
        numbers.append(int(match.group(1)))
    while len(numbers) < 3:
        numbers.append(0)
    return tuple(numbers)


def notes_size(release: dict[str, object]) -> int:
    notes = release.get("notes")
    if not isinstance(notes, dict):
        return 0
    return sum(
        len(value.encode("utf-8"))
        for value in notes.values()
        if isinstance(value, str)
    )


def trim_release_history(
    release_rows: list[dict[str, object]], current_version: str
) -> list[dict[str, object]]:
    current_key = semver_sort_key(current_version)
    eligible: list[tuple[tuple[int, ...], int, dict[str, object]]] = []
    for index, release in enumerate(release_rows):
        version = str(release.get("version", ""))
        try:
            key = semver_sort_key(version)
        except RuntimeError:
            continue
        if key <= current_key:
            eligible.append((key, index, release))

    eligible.sort(key=lambda item: (item[0], item[1]), reverse=True)
    kept: list[dict[str, object]] = []
    total_bytes = 0
    for key, _, release in eligible:
        size = notes_size(release)
        if kept and (
            len(kept) >= MAX_RELEASE_HISTORY
            or total_bytes + size > MAX_RELEASE_NOTES_BYTES
        ):
            break
        kept.append(release)
        total_bytes += size
    return kept


def api_get(url: str, token: str) -> object:
    request = urllib.request.Request(
        url,
        headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {token}",
            "User-Agent": "fluxdb-release-manifest",
            "X-GitHub-Api-Version": "2022-11-28",
        },
    )
    with urllib.request.urlopen(request, timeout=60) as response:
        return json.load(response)


def parse_notes(body: str) -> dict[str, str]:
    notes: dict[str, str] = {}
    pattern = re.compile(
        r"<details[^>]*>\s*<summary>\s*(中文|English)\s*</summary>(.*?)</details>",
        re.IGNORECASE | re.DOTALL,
    )
    for language, content in pattern.findall(body):
        key = "zh-CN" if language == "中文" else "en"
        notes[key] = content.strip()
    if not notes and body.strip():
        notes = {"zh-CN": body.strip(), "en": body.strip()}
    return notes


def note_files(notes_dir: Path, tag: str) -> dict[str, str]:
    directory = notes_dir / tag
    notes: dict[str, str] = {}
    for locale, file_name in (("zh-CN", "zh-CN.md"), ("en", "en.md")):
        path = directory / file_name
        if path.is_file():
            notes[locale] = path.read_text(encoding="utf-8").strip()
    return notes


def asset_sha256(asset: dict[str, object]) -> str:
    digest = asset.get("digest")
    if not isinstance(digest, str) or not digest.startswith("sha256:"):
        raise RuntimeError(
            f"GitHub did not return a sha256 digest for asset {asset.get('name')!r}"
        )
    value = digest.removeprefix("sha256:").lower()
    if len(value) != 64 or any(character not in "0123456789abcdef" for character in value):
        raise RuntimeError(f"invalid sha256 digest for asset {asset.get('name')!r}")
    return value


def find_platform_assets(release: dict[str, object], version: str) -> dict[str, dict[str, object]]:
    assets = release.get("assets", [])
    if not isinstance(assets, list):
        raise RuntimeError("release assets are not a list")
    by_name = {
        str(asset.get("name", "")).lower(): asset
        for asset in assets
        if isinstance(asset, dict)
    }
    result: dict[str, dict[str, object]] = {}
    expected = {
        "macos-aarch64": f"FluxDB-{version}-macos-arm64.app.tar.gz".lower(),
        "macos-x86_64": f"FluxDB-{version}-macos-x64.app.tar.gz".lower(),
        "windows-x86_64": f"FluxDB-{version}-windows-x64-setup.exe".lower(),
        "linux-x86_64": f"fluxdb_{version}_amd64.deb".lower(),
    }
    for platform, name in expected.items():
        asset = by_name.get(name)
        if asset is None:
            raise RuntimeError(f"missing updater asset {name}")
        url = asset.get("browser_download_url")
        size = asset.get("size")
        if not isinstance(url, str) or not isinstance(size, int):
            raise RuntimeError(f"invalid updater asset metadata for {name}")
        result[platform] = {
            "url": url,
            "sha256": asset_sha256(asset),
            "size": size,
            "kind": PLATFORM_ASSETS[platform][1],
        }
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--notes-dir", default="release-notes")
    parser.add_argument("--api-url", default=os.environ.get("GITHUB_API_URL", "https://api.github.com"))
    args = parser.parse_args()

    token = os.environ.get("GITHUB_TOKEN", "")
    if not token:
        raise RuntimeError("GITHUB_TOKEN is required")
    version = args.tag.removeprefix("v")
    api_url = args.api_url.rstrip("/")
    release = api_get(f"{api_url}/repos/{args.repo}/releases/tags/{args.tag}", token)
    releases_page = api_get(
        f"{api_url}/repos/{args.repo}/releases?per_page=100", token
    )
    if not isinstance(release, dict) or not isinstance(releases_page, list):
        raise RuntimeError("unexpected GitHub Releases response")

    notes_dir = Path(args.notes_dir)
    release_rows: list[dict[str, object]] = []
    seen_versions: set[str] = set()
    for row in releases_page:
        if not isinstance(row, dict) or row.get("draft") or row.get("prerelease"):
            continue
        tag = str(row.get("tag_name", ""))
        row_version = tag.removeprefix("v")
        if not row_version or row_version in seen_versions:
            continue
        body = str(row.get("body") or "")
        notes = note_files(notes_dir, tag) or parse_notes(body)
        if not notes:
            continue
        seen_versions.add(row_version)
        release_rows.append(
            {
                "version": row_version,
                "published_at": row.get("published_at"),
                "notes": notes,
            }
        )

    release_rows = trim_release_history(release_rows, version)

    current_notes = note_files(notes_dir, args.tag) or parse_notes(
        str(release.get("body") or "")
    )
    manifest = {
        "schema": 1,
        "version": version,
        "published_at": release.get("published_at"),
        "notes": current_notes,
        "releases": release_rows,
        "platforms": find_platform_assets(release, version),
    }
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    print(f"Wrote {output}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (RuntimeError, urllib.error.URLError) as error:
        print(f"generate-update-manifest: {error}", file=sys.stderr)
        raise SystemExit(1)
