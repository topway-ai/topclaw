#!/usr/bin/env python3
"""Ensure a pushed release tag has a publish run, or dispatch the manual fallback."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time


WORKFLOW_NAME = "Pub Release"
WORKFLOW_FILE = "pub-release.yml"


def run_gh(args: list[str]) -> str:
    proc = subprocess.run(
        ["gh", *args],
        text=True,
        capture_output=True,
        check=False,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"gh {' '.join(args)} failed ({proc.returncode}): {proc.stderr.strip()}")
    return proc.stdout


def parse_repository(repository: str, origin_url: str) -> str:
    if repository:
        return repository

    candidate = origin_url.strip()
    if candidate.endswith(".git"):
        candidate = candidate[:-4]

    for marker in ("github.com:", "github.com/"):
        if marker in candidate:
            slug = candidate.split(marker, 1)[1].strip("/")
            if slug.count("/") == 1:
                return slug

    raise ValueError(
        "unable to infer GitHub repository slug from origin URL; pass --repository or a GitHub origin URL"
    )


def find_push_run(runs: dict, release_tag: str) -> dict | None:
    for run in runs.get("workflow_runs", []):
        if (
            run.get("name") == WORKFLOW_NAME
            and run.get("event") == "push"
            and run.get("head_branch") == release_tag
        ):
            return run
    return None


def fetch_push_run(repository: str, head_sha: str, release_tag: str) -> dict | None:
    raw = run_gh(["api", f"repos/{repository}/actions/runs?head_sha={head_sha}&per_page=100"])
    return find_push_run(json.loads(raw), release_tag)


def dispatch_fallback(release_tag: str) -> None:
    run_gh(
        [
            "workflow",
            "run",
            WORKFLOW_FILE,
            "-f",
            f"release_ref={release_tag}",
            "-f",
            "publish_release=true",
            "-f",
            f"release_tag={release_tag}",
            "-f",
            "draft=false",
        ]
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", default="")
    parser.add_argument("--origin-url", default="")
    parser.add_argument("--release-tag", required=True)
    parser.add_argument("--head-sha", required=True)
    parser.add_argument("--wait-seconds", type=int, default=30)
    parser.add_argument("--poll-interval-seconds", type=int, default=5)
    args = parser.parse_args()

    try:
        repository = parse_repository(args.repository, args.origin_url)
        deadline = time.monotonic() + max(args.wait_seconds, 0)

        while True:
            run = fetch_push_run(repository, args.head_sha, args.release_tag)
            if run is not None:
                print(
                    f"Detected push-triggered {WORKFLOW_NAME} run for {args.release_tag}: "
                    f"{run.get('html_url', '')}".rstrip()
                )
                return 0

            if time.monotonic() >= deadline:
                break

            time.sleep(max(args.poll_interval_seconds, 0))

        print(
            f"No push-triggered {WORKFLOW_NAME} run detected within {max(args.wait_seconds, 0)}s "
            f"for {args.release_tag}. Dispatching manual publish fallback."
        )
        dispatch_fallback(args.release_tag)
        print(
            f"Dispatched manual publish fallback for {args.release_tag} via {WORKFLOW_FILE}."
        )
        return 0
    except (RuntimeError, ValueError, json.JSONDecodeError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
