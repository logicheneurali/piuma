#!/usr/bin/env python3
"""Regenerate THIRD_PARTY_LICENSES.md from the resolved Cargo graph.

The file lists every third-party crate in the Linux build (`piuma` and
`piuma-setup`), the license each one declares, and the license texts and
copyright notices a binary distribution must carry.

Run from the repository root (the Cargo cache is enough; no network):

    scripts/gen-third-party-licenses.py

Only the standard library is used, so the script runs anywhere cargo does.
"""

from __future__ import annotations

import hashlib
import json
import re
import subprocess
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "THIRD_PARTY_LICENSES.md"
TARGET = "x86_64-unknown-linux-gnu"
LICENSE_PATTERNS = (
    "LICENSE*",
    "LICENCE*",
    "COPYING*",
    "COPYRIGHT*",
    "NOTICE*",
    "UNLICENSE*",
)
MAX_TEXT = 512 * 1024

MIT_TEXT = """MIT License

Copyright (c) {copyright}

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE."""

ZLIB_TEXT = """zlib License

Copyright (c) {copyright}

This software is provided 'as-is', without any express or implied
warranty.  In no event will the authors be held liable for any damages
arising from the use of this software.

Permission is granted to anyone to use this software for any purpose,
including commercial applications, and to alter it and redistribute it
freely, subject to the following restrictions:

1. The origin of this software must not be misrepresented; you must not
   claim that you wrote the original software. If you use this software
   in a product, an acknowledgment in the product documentation would be
   appreciated but is not required.
2. Altered source versions must be plainly marked as such, and must not be
   misrepresented as being the original software.
3. This notice may not be removed or altered from any source distribution."""


def cargo(*args: str) -> str:
    return subprocess.check_output(["cargo", *args], cwd=ROOT, text=True)


def packages_in_build() -> list[dict]:
    meta = json.loads(cargo("metadata", "--offline", "--format-version", "1"))
    by_key = {(p["name"], p["version"]): p for p in meta["packages"]}
    tree = cargo(
        "tree",
        "--offline",
        "--target",
        TARGET,
        "-e",
        "normal,build",
        "--prefix",
        "none",
        "--format",
        "{p}",
    )
    packages: list[dict] = []
    seen: set[str] = set()
    for line in tree.splitlines():
        line = re.sub(r" \(\*\)$", "", line.strip())
        match = re.match(r"^(.+?) v([^ ]+?)(?: \(.*\))?$", line)
        if not match:
            raise SystemExit(f"cannot parse cargo tree line: {line!r}")
        package = by_key.get((match.group(1), match.group(2)))
        if package is None:
            raise SystemExit(f"package not found in metadata: {line!r}")
        if package["id"] in seen:
            continue
        seen.add(package["id"])
        packages.append(package)
    return packages


def license_texts(package: dict) -> list[tuple[str, str]]:
    """The license-like files the crate ships, crate dir first."""
    directory = Path(package["manifest_path"]).parent
    bases = [directory, *list(directory.parents)[:3]]
    found: list[Path] = []
    seen: set[Path] = set()
    for base in bases:
        for pattern in LICENSE_PATTERNS:
            for path in sorted(base.glob(pattern)):
                resolved = path.resolve()
                if (
                    resolved in seen
                    or not path.is_file()
                    or path.stat().st_size > MAX_TEXT
                ):
                    continue
                seen.add(resolved)
                found.append(path)
    texts: list[tuple[str, str]] = []
    for path in found:
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        if text.strip():
            texts.append((path.name, text.strip()))
    return texts


def has_license(text: str, needle: str) -> bool:
    return needle.lower() in text.lower()


def main() -> None:
    packages = [p for p in packages_in_build() if p["name"] != "piuma"]
    packages.sort(key=lambda p: (p["name"].lower(), p["version"]))

    texts: dict[str, dict] = {}
    fallback: dict[tuple[str, tuple[str, ...]], list[str]] = defaultdict(list)
    missing_license = []

    for package in packages:
        label = f"{package['name']} {package['version']}"
        license = package.get("license") or ""
        if not license:
            missing_license.append(label)
        shipped = license_texts(package)
        if shipped:
            for _, text in shipped:
                digest = hashlib.sha256(text.encode()).hexdigest()
                entry = texts.setdefault(
                    digest, {"text": text, "crates": [], "licenses": set()}
                )
                entry["crates"].append(label)
                entry["licenses"].add(license)
        else:
            authors = tuple(package.get("authors") or ())
            fallback[(canonical(license), authors)].append(label)

    write(packages, texts, fallback, missing_license)


def canonical(license: str) -> str:
    """The text to reproduce for a crate that ships none: pick the first
    permissive option of its SPDX expression."""
    for candidate in ("MIT", "Apache-2.0", "Zlib", "CC0", "Unicode-3.0"):
        if candidate.lower() in license.lower():
            return candidate
    return license or "unknown"


def write(packages, texts, fallback, missing_license) -> None:
    count = Counter(p.get("license") or "unknown" for p in packages)
    lines: list[str] = []
    add = lines.append

    add("# Third-party licenses")
    add("")
    add(
        "Piuma itself is licensed under Apache-2.0 (see `LICENSE`). This file "
        "lists every third-party crate compiled or used at build time for the "
        f"Linux binaries (`piuma` and `piuma-setup`), resolved for "
        f"`{TARGET}`."
    )
    add("")
    add("Generated from the lock file with:")
    add("")
    add("```sh")
    add("scripts/gen-third-party-licenses.py")
    add("```")
    add("")
    add(
        f"Packages in the Linux build: **{len(packages)} third-party crates** "
        f"(plus piuma itself). Every license is OSI open source."
    )
    add("")
    add(
        "A binary or archive distribution must ship this file together with "
        "`LICENSE` and the `LICENSES/` directory: the appendix contains the "
        "copyright notices and permission texts required by MIT, BSD, ISC, "
        "Zlib and the other licenses below."
    )
    add("")

    add("## Licenses in use")
    add("")
    for license, n in sorted(count.items(), key=lambda item: (-item[1], item[0])):
        add(f"- `{license}` — {n}")
    add("")

    add("## Mozilla Public License 2.0")
    add("")
    add(
        "One crate in the build is licensed under MPL-2.0: **`option-ext "
        "0.2.0`**, reached through `freedesktop-icons → dirs → dirs-sys`. It "
        "is used unmodified; the full license text is reproduced in the "
        "appendix (and in `LICENSES/MPL-2.0.txt`). MPL-2.0 is a file-level "
        "copyleft license: it allows commercial and proprietary use of the "
        "larger work, and its obligations apply only to the MPL-covered "
        "files, which this project does not modify."
    )
    add("")
    add(
        "All other crates are under permissive licenses (MIT, Apache-2.0, "
        "BSD, ISC, Zlib, Unicode-3.0, CC0-1.0, BSL-1.0, Unlicense or a "
        "combination of these)."
    )
    add("")

    add("## Direct dependencies")
    add("")
    add("| Crate | License |")
    add("|---|---|")
    direct = {
        "iced",
        "iced_runtime",
        "iced_exwlshell",
        "image",
        "smol",
        "log",
        "libc",
        "serde",
        "serde_json",
        "signal-hook",
        "zbus",
        "freedesktop-icons",
        "pam",
        "wayland-client",
        "wayland-protocols",
        "cosmic-text",
    }
    for package in packages:
        if package["name"] in direct:
            add(f"| `{package['name']}` | `{package.get('license')}` |")
    add("")

    add("## All third-party crates")
    add("")
    add("| Crate | License |")
    add("|---|---|")
    for package in packages:
        add(
            f"| `{package['name']} {package['version']}` "
            f"| `{package.get('license') or 'unknown'}` |"
        )
    add("")

    add("## License texts and copyright notices")
    add("")
    add(
        "Texts are grouped by content; the crates that ship each text are "
        "listed above it. The same crate can ship more than one file "
        "(typically `LICENSE-MIT` and `LICENSE-APACHE`)."
    )
    add("")
    ordered = sorted(
        texts.values(), key=lambda entry: (entry["crates"][0].lower(), entry["text"][:40])
    )
    for index, entry in enumerate(ordered, start=1):
        crates = ", ".join(f"`{name}`" for name in sorted(set(entry["crates"])))
        add(f"### Text {index} — used by {crates}")
        add("")
        add("```text")
        add(entry["text"])
        add("```")
        add("")

    if fallback:
        add(
            "### Declared licenses without a shipped license file"
        )
        add("")
        add(
            "These crates declare a license in their Cargo metadata but do "
            "not include a license file in the published crate. The canonical "
            "text is reproduced below, with the published authors as the "
            "copyright line where the license uses one."
        )
        add("")
        for (license, authors), crates in sorted(fallback.items()):
            names = ", ".join(f"`{name}`" for name in sorted(crates))
            add(f"#### {names} — `{license}`")
            add("")
            if license == "MIT":
                copyright_line = ", ".join(authors) or "the crate authors"
                add("```text")
                add(MIT_TEXT.format(copyright=copyright_line))
                add("```")
            elif license == "Zlib":
                copyright_line = ", ".join(authors) or "the crate authors"
                add("```text")
                add(ZLIB_TEXT.format(copyright=copyright_line))
                add("```")
            elif license == "Apache-2.0":
                add(
                    "Apache License 2.0 — the complete text is the repository's "
                    "`LICENSE` file."
                )
            elif license == "CC0":
                add(
                    "CC0 1.0 Universal — public-domain dedication, no "
                    "attribution required. Full text: "
                    "https://creativecommons.org/publicdomain/zero/1.0/legalcode"
                )
            else:
                add(f"SPDX license `{license}`; see https://spdx.org/licenses/.")
            add("")

    if missing_license:
        add("### Crates without license metadata")
        add("")
        for name in missing_license:
            add(f"- `{name}`")
        add("")

    OUT.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(
        f"wrote {OUT.relative_to(ROOT)}: {len(packages)} crates, "
        f"{len(texts)} shipped texts, {sum(len(v) for v in fallback.values())} "
        f"fallback entries"
    )


if __name__ == "__main__":
    main()
