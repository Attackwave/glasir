#!/usr/bin/env python3
"""Writes the Homebrew formula and the Scoop manifest for one release.

Reads the `.sha256` files the release build writes beside each archive, so the
checksums in both files are the ones the archives were published with, never
recomputed from a copy. Run by the release workflow; runnable by hand:

    python3 packaging/render.py --tag v0.1.0 --dist dist --out out

`--base-url` exists for the smoke test, which installs the formula in the
Homebrew image from archives on disk before any release carries them.
"""

import argparse
import json
import pathlib
import sys

PLATFORMS = {
    "macos-arm64": ("on_macos", "on_arm"),
    "macos-x86_64": ("on_macos", "on_intel"),
    "linux-arm64": ("on_linux", "on_arm"),
    "linux-x86_64": ("on_linux", "on_intel"),
}
DESCRIPTION = "Deterministic code intelligence graph for repositories, served over MCP"


def checksums(dist: pathlib.Path) -> dict:
    """Archive file name -> SHA-256, from every `*.sha256` under `dist`."""
    sums = {}
    for f in dist.rglob("*.sha256"):
        digest, _, name = f.read_text().strip().partition(" ")
        sums[name.strip().lstrip("*")] = digest
    return sums


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--tag", required=True, help="release tag, e.g. v0.1.0")
    ap.add_argument("--repo", default="Attackwave/glasir")
    ap.add_argument("--dist", required=True, type=pathlib.Path)
    ap.add_argument("--out", required=True, type=pathlib.Path)
    ap.add_argument("--base-url", help="where the archives live; defaults to the release")
    a = ap.parse_args()

    version = a.tag.removeprefix("v")
    base = a.base_url or f"https://github.com/{a.repo}/releases/download/{a.tag}"
    sums = checksums(a.dist)

    def archive(platform: str, ext: str) -> tuple:
        stem = f"glasir-{version}-{platform}"
        name = f"{stem}.{ext}"
        if name not in sums:
            sys.exit(f"no checksum for {name} under {a.dist}; found {sorted(sums)}")
        return stem, f"{base}/{name}", sums[name]

    # Homebrew: one formula, a block per OS and CPU. The archive holds one
    # directory, which Homebrew enters before `install` runs.
    blocks = {}
    for platform, (os_block, cpu_block) in PLATFORMS.items():
        _, url, sha = archive(platform, "tar.gz")
        blocks.setdefault(os_block, []).append(
            f'    {cpu_block} do\n      url "{url}"\n      sha256 "{sha}"\n    end\n'
        )
    formula = (
        "class Glasir < Formula\n"
        f'  desc "{DESCRIPTION}"\n'
        f'  homepage "https://github.com/{a.repo}"\n'
        f'  version "{version}"\n'
        '  license "Apache-2.0"\n\n'
        + "\n".join(f"  {os_block} do\n{''.join(b)}  end\n" for os_block, b in blocks.items())
        + "\n  def install\n"
        '    bin.install "glasir"\n'
        "  end\n\n"
        "  test do\n"
        '    assert_match "glasir", shell_output("#{bin}/glasir --version")\n'
        "  end\n"
        "end\n"
    )

    # Scoop: the manifest for this release, plus what Scoop needs to write
    # the next one itself.
    stem, url, sha = archive("windows-x86_64", "zip")
    auto = "glasir-$version-windows-x86_64"
    manifest = {
        "version": version,
        "description": DESCRIPTION,
        "homepage": f"https://github.com/{a.repo}",
        "license": "Apache-2.0",
        "architecture": {"64bit": {"url": url, "hash": sha, "extract_dir": stem}},
        "bin": "glasir.exe",
        "checkver": {"github": f"https://github.com/{a.repo}"},
        "autoupdate": {
            "architecture": {
                "64bit": {
                    "url": f"https://github.com/{a.repo}/releases/download/v$version/{auto}.zip",
                    "hash": {"url": "$url.sha256"},
                    "extract_dir": auto,
                }
            }
        },
    }

    a.out.mkdir(parents=True, exist_ok=True)
    (a.out / "glasir.rb").write_text(formula)
    (a.out / "glasir.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"wrote {a.out / 'glasir.rb'} and {a.out / 'glasir.json'} for {version}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
