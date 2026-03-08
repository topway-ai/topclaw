#!/usr/bin/env python3
"""
Quick validation script for skills.
"""

import re
import sys
from pathlib import Path

import yaml


ALLOWED_PROPERTIES = {"name", "description", "license", "allowed-tools", "metadata"}
ARCHIVE_SUFFIXES = (
    ".zip",
    ".tar",
    ".tgz",
    ".gz",
    ".xz",
    ".bz2",
    ".7z",
    ".rar",
    ".jar",
)
SECRET_FILENAMES = {
    ".env",
    ".env.local",
    ".env.production",
    "id_rsa",
    "id_ed25519",
    "credentials",
    "credentials.json",
    "token",
    "token.txt",
    "secret",
    "secret.txt",
    "private.key",
}
BLOCKED_DIR_NAMES = {"__pycache__", ".git"}
BLOCKED_FILE_SUFFIXES = (".pyc", ".pyo", ".pem", ".key", ".p12", ".pfx")


def find_disallowed_files(skill_path):
    problems = []
    for file_path in skill_path.rglob("*"):
        rel = file_path.relative_to(skill_path)
        if any(part in BLOCKED_DIR_NAMES for part in rel.parts):
            problems.append(f"Blocked generated or VCS path present: {rel}")
            continue
        if file_path.is_symlink():
            problems.append(f"Symlinks are not allowed in packaged skills: {rel}")
            continue
        if not file_path.is_file():
            continue

        name = file_path.name.lower()
        rel_lower = str(rel).lower()
        if name in SECRET_FILENAMES or name.endswith(BLOCKED_FILE_SUFFIXES):
            problems.append(f"Secret-like file must not be packaged: {rel}")
        if rel_lower.endswith(ARCHIVE_SUFFIXES):
            problems.append(f"Nested archive must not be packaged: {rel}")
    return problems


def validate_skill(skill_path):
    skill_path = Path(skill_path).resolve()
    skill_md = skill_path / "SKILL.md"
    if not skill_md.exists():
        return False, "SKILL.md not found"

    content = skill_md.read_text()
    if not content.startswith("---"):
        return False, "No YAML frontmatter found"

    match = re.match(r"^---\n(.*?)\n---", content, re.DOTALL)
    if not match:
        return False, "Invalid frontmatter format"

    try:
        frontmatter = yaml.safe_load(match.group(1))
    except yaml.YAMLError as err:
        return False, f"Invalid YAML in frontmatter: {err}"

    if not isinstance(frontmatter, dict):
        return False, "Frontmatter must be a YAML dictionary"

    unexpected_keys = set(frontmatter.keys()) - ALLOWED_PROPERTIES
    if unexpected_keys:
        return (
            False,
            "Unexpected key(s) in SKILL.md frontmatter: "
            + ", ".join(sorted(unexpected_keys)),
        )

    name = frontmatter.get("name")
    description = frontmatter.get("description")
    if not isinstance(name, str) or not name.strip():
        return False, "Missing or invalid 'name' in frontmatter"
    if not isinstance(description, str) or not description.strip():
        return False, "Missing or invalid 'description' in frontmatter"

    normalized_name = name.strip()
    if not re.match(r"^[a-z0-9-]+$", normalized_name):
        return False, "Name must be hyphen-case"
    if normalized_name.startswith("-") or normalized_name.endswith("-") or "--" in normalized_name:
        return False, "Name cannot start/end with hyphen or contain consecutive hyphens"
    if len(normalized_name) > 64:
        return False, "Name exceeds the 64 character limit"

    normalized_description = description.strip()
    if "<" in normalized_description or ">" in normalized_description:
        return False, "Description cannot contain angle brackets"
    if len(normalized_description) > 1024:
        return False, "Description exceeds the 1024 character limit"

    problems = find_disallowed_files(skill_path)
    if problems:
        return False, "; ".join(problems)

    return True, "Skill is valid"


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print("Usage: quick_validate.py <skill_directory>")
        sys.exit(1)

    valid, message = validate_skill(sys.argv[1])
    print(message)
    sys.exit(0 if valid else 1)
