#!/usr/bin/env python3
"""
Skill Initializer - Creates a new skill from template

Usage:
    init_skill.py <skill-name> --path <path>
"""

import sys
from pathlib import Path


SKILL_TEMPLATE = """---
name: {skill_name}
description: [TODO: Explain what this skill does and when to use it. Include concrete trigger situations.]
---

# {skill_title}

## Overview

[TODO: Explain what this skill enables another agent to do.]

## Workflow

[TODO: Describe the core workflow or decision tree.]

## Resources

[TODO: Reference any bundled scripts, references, or assets that matter.]

## Plugin Policy

- This is a self-added TopClaw skill/plugin.
- If installed in a TopClaw workspace, `topclaw skills list` should show it as `self-added`.
- Users can disable it with `topclaw skills disable {skill_name}` and re-enable it with `topclaw skills enable {skill_name}`.
"""

EXAMPLE_SCRIPT = '''#!/usr/bin/env python3
"""
Example helper script for {skill_name}
"""


def main():
    print("Replace this example script with real functionality or delete it.")


if __name__ == "__main__":
    main()
'''

EXAMPLE_REFERENCE = """# Reference Documentation for {skill_title}

Replace this with real documentation or delete it.
"""

EXAMPLE_ASSET = """This placeholder represents where asset files would live."""


def title_case_skill_name(skill_name):
    return " ".join(word.capitalize() for word in skill_name.split("-"))


def init_skill(skill_name, path):
    skill_dir = Path(path).resolve() / skill_name
    if skill_dir.exists():
        print(f"Error: skill directory already exists: {skill_dir}")
        return None

    try:
        skill_dir.mkdir(parents=True, exist_ok=False)
        print(f"Created skill directory: {skill_dir}")
    except Exception as err:
        print(f"Error creating directory: {err}")
        return None

    skill_title = title_case_skill_name(skill_name)
    try:
        (skill_dir / "SKILL.md").write_text(
            SKILL_TEMPLATE.format(skill_name=skill_name, skill_title=skill_title)
        )
        scripts_dir = skill_dir / "scripts"
        references_dir = skill_dir / "references"
        assets_dir = skill_dir / "assets"
        scripts_dir.mkdir(exist_ok=True)
        references_dir.mkdir(exist_ok=True)
        assets_dir.mkdir(exist_ok=True)
        example_script = scripts_dir / "example.py"
        example_script.write_text(EXAMPLE_SCRIPT.format(skill_name=skill_name))
        example_script.chmod(0o755)
        (references_dir / "reference.md").write_text(
            EXAMPLE_REFERENCE.format(skill_title=skill_title)
        )
        (assets_dir / "example_asset.txt").write_text(EXAMPLE_ASSET)
    except Exception as err:
        print(f"Error writing skill files: {err}")
        return None

    print(f"Skill '{skill_name}' initialized successfully at {skill_dir}")
    print("Next steps:")
    print("1. Edit SKILL.md")
    print("2. Replace or delete example bundled resources")
    print("3. Run quick_validate.py")
    print("4. If this lives in a TopClaw workspace, manage it with `topclaw skills list|disable|enable|remove`")
    return skill_dir


def main():
    if len(sys.argv) < 4 or sys.argv[2] != "--path":
        print("Usage: init_skill.py <skill-name> --path <path>")
        sys.exit(1)

    skill_name = sys.argv[1]
    path = sys.argv[3]
    result = init_skill(skill_name, path)
    sys.exit(0 if result else 1)


if __name__ == "__main__":
    main()
