#!/usr/bin/env python3
"""
Skill Packager - Creates a distributable .skill file from a skill folder.
"""

import sys
import zipfile
from pathlib import Path

from quick_validate import BLOCKED_DIR_NAMES, validate_skill


def package_skill(skill_path, output_dir=None):
    skill_path = Path(skill_path).resolve()
    if not skill_path.exists():
        print(f"Error: skill folder not found: {skill_path}")
        return None
    if not skill_path.is_dir():
        print(f"Error: path is not a directory: {skill_path}")
        return None
    if not (skill_path / "SKILL.md").exists():
        print(f"Error: SKILL.md not found in {skill_path}")
        return None

    valid, message = validate_skill(skill_path)
    if not valid:
        print(f"Validation failed: {message}")
        return None

    if output_dir:
        output_path = Path(output_dir).resolve()
        output_path.mkdir(parents=True, exist_ok=True)
    else:
        output_path = Path.cwd()

    archive_path = output_path / f"{skill_path.name}.skill"

    try:
        with zipfile.ZipFile(archive_path, "w", zipfile.ZIP_DEFLATED) as archive:
            for file_path in skill_path.rglob("*"):
                rel = file_path.relative_to(skill_path)
                if any(part in BLOCKED_DIR_NAMES for part in rel.parts):
                    continue
                if file_path.is_file():
                    archive.write(file_path, file_path.relative_to(skill_path.parent))
        print(f"Successfully packaged skill to: {archive_path}")
        return archive_path
    except Exception as err:
        print(f"Error creating .skill file: {err}")
        return None


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: package_skill.py <path/to/skill-folder> [output-directory]")
        sys.exit(1)

    result = package_skill(sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else None)
    sys.exit(0 if result else 1)
