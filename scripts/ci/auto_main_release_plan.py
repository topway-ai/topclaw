#!/usr/bin/env python3
"""Plan auto-main-release tagging and version bump behavior."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import re
import subprocess
import sys
from pathlib import Path

SEMVER_RE = re.compile(r"^(?P<major>\d+)\.(?P<minor>\d+)\.(?P<patch>\d+)$")


def parse_csv(raw: str) -> list[str]:
    return [item.strip().lower() for item in raw.split(",") if item.strip()]


def resolve_allowed_actors(raw: str, repository_owner: str) -> list[str]:
    explicit = parse_csv(raw)
    if explicit:
        return explicit

    fallback: list[str] = []
    owner = repository_owner.strip().lower()
    if owner:
        fallback.append(owner)
    if "github-actions[bot]" not in fallback:
        fallback.append("github-actions[bot]")
    return fallback


def parse_cargo_version(cargo_toml: Path) -> str:
    in_package = False
    for raw_line in cargo_toml.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if line == "[package]":
            in_package = True
            continue
        if in_package and line.startswith("["):
            in_package = False
        if in_package and line.startswith("version = "):
            parts = line.split('"')
            if len(parts) >= 2:
                return parts[1]
    raise ValueError(f"Failed to resolve package version from {cargo_toml}")


def compute_next_version(current: str) -> str:
    match = SEMVER_RE.fullmatch(current)
    if not match:
        raise ValueError(f"Cargo.toml version must be strict semver X.Y.Z (found: {current})")

    major = int(match.group("major"))
    minor = int(match.group("minor"))
    patch = int(match.group("patch")) + 1
    return f"{major}.{minor}.{patch}"


def tag_exists(repo_root: Path, tag: str) -> bool:
    proc = subprocess.run(
        ["git", "ls-remote", "--exit-code", "--tags", "origin", f"refs/tags/{tag}"],
        cwd=str(repo_root),
        text=True,
        capture_output=True,
        check=False,
    )
    return proc.returncode == 0


def build_markdown(report: dict) -> str:
    lines = [
        "# Auto Main Release Plan",
        "",
        f"- Generated at: `{report['generated_at']}`",
        f"- Actor: `{report['actor']}`",
        f"- Actor authorized: `{report['actor_authorized']}`",
        f"- Current version: `{report.get('current_version')}`",
        f"- Next version: `{report.get('next_version')}`",
        f"- Release tag: `{report.get('release_tag')}`",
        f"- Tag exists on origin: `{report.get('tag_exists_on_origin')}`",
        f"- Mode: `{report.get('mode')}`",
        f"- Create tag: `{report.get('should_create_tag')}`",
        f"- Bump version: `{report.get('should_bump_version')}`",
        "",
        "## Allowed Actors",
    ]
    allowed_actors = report.get("allowed_actors", [])
    if allowed_actors:
        lines.extend(f"- `{item}`" for item in allowed_actors)
    else:
        lines.append("- none")
    if report["violations"]:
        lines.append("")
        lines.append("## Violations")
        lines.extend(f"- {item}" for item in report["violations"])
    return "\n".join(lines).rstrip() + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description="Plan auto-main-release tagging and bump behavior.")
    parser.add_argument("--repo-root", default=".")
    parser.add_argument("--actor", required=True)
    parser.add_argument("--repository-owner", required=True)
    parser.add_argument("--authorized-actors", default="")
    parser.add_argument("--output-json", required=True)
    parser.add_argument("--output-md", required=True)
    parser.add_argument("--fail-on-violation", action="store_true")
    args = parser.parse_args()

    repo_root = Path(args.repo_root).resolve()
    out_json = Path(args.output_json)
    out_md = Path(args.output_md)

    violations: list[str] = []
    actor = args.actor.strip()
    actor_normalized = actor.lower()
    allowed_actors = resolve_allowed_actors(args.authorized_actors, args.repository_owner)
    actor_authorized = actor_normalized in allowed_actors
    if not actor_authorized:
        violations.append(
            f"Only maintainer actors ({', '.join(allowed_actors)}) can trigger main release tagging. "
            f"Actor: {actor}"
        )

    current_version: str | None = None
    next_version: str | None = None
    release_tag: str | None = None
    exists_on_origin = False

    try:
        current_version = parse_cargo_version(repo_root / "Cargo.toml")
        next_version = compute_next_version(current_version)
        release_tag = f"v{current_version}"
        exists_on_origin = tag_exists(repo_root, release_tag)
    except ValueError as exc:
        violations.append(str(exc))

    should_bump_version = not violations and next_version is not None
    should_create_tag = should_bump_version and not exists_on_origin
    mode = "blocked"
    if should_bump_version:
        mode = "tag_and_bump" if should_create_tag else "bump_only"

    report = {
        "schema_version": "topclaw.auto-main-release-plan.v1",
        "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "actor": actor,
        "actor_authorized": actor_authorized,
        "allowed_actors": allowed_actors,
        "current_version": current_version,
        "next_version": next_version,
        "release_tag": release_tag,
        "tag_exists_on_origin": exists_on_origin,
        "should_create_tag": should_create_tag,
        "should_bump_version": should_bump_version,
        "mode": mode,
        "violations": violations,
    }

    out_json.parent.mkdir(parents=True, exist_ok=True)
    out_md.parent.mkdir(parents=True, exist_ok=True)
    out_json.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    out_md.write_text(build_markdown(report), encoding="utf-8")

    if args.fail_on_violation and violations:
        print("auto-main-release plan violations found:", file=sys.stderr)
        for item in violations:
            print(f"- {item}", file=sys.stderr)
        return 3
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
