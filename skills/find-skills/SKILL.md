---
name: find-skills
description: "Trigger when the user asks how to do X, says find/is there a skill for X, asks can you do X for a specialized capability, or expresses interest in extending agent capabilities. Search the public skill index at https://skills.sh/ (using web_search if no direct index lookup is available) for the user's domain and task, then present each candidate with its name, what it does, a `topclaw skills install <skills.sh URL>` command, and the index link. Always install through TopClaw's reviewed `topclaw skills install` path so the built-in vetter and domain-trust prompts run — never recommend unattended global installs or third-party package managers. Re-vet after install with `topclaw skills vet <name> --json`. If no relevant skill is found, say so and offer to help directly or suggest using skill-creator. Read skills/find-skills/SKILL.md for the full discovery and presentation workflow."
---

# Find Skills

This skill helps you discover and install skills from public skill indexes.

## When to Use This Skill

Use this skill when the user:

- Asks "how do I do X" where X might be a common task with an existing skill
- Says "find a skill for X" or "is there a skill for X"
- Asks "can you do X" where X is a specialized capability
- Expresses interest in extending agent capabilities
- Wants to search for tools, templates, or workflows
- Mentions they wish they had help with a specific domain (design, testing, deployment, etc.)

## Discovery and Install Path

Use public skill indexes such as https://skills.sh/ to discover candidate skills, but
install them through TopClaw's reviewed workflow instead of a third-party package
manager. TopClaw's `skills install` path applies domain trust prompts and the built-in
skill vetter before the skill is added to the workspace.

**Preferred commands:**

- `topclaw skills install <source>` - Install from a reviewed local path, git URL, or `skills.sh` URL
- `topclaw skills vet <installed-skill> --json` - Re-run the full skill vetter after install
- `topclaw skills list` - Verify what is currently installed

**Browse skills at:** https://skills.sh/

## How to Help Users Find Skills

### Step 1: Understand What They Need

When a user asks for help with something, identify:

1. The domain (e.g., React, testing, design, deployment)
2. The specific task (e.g., writing tests, reviewing PRs, creating docs)
3. Whether this is a common enough task that a skill likely exists

### Step 2: Search for Skills

Search the skills index or use low-risk web search with a relevant query. Capture the
skills.sh page URL for any promising result instead of jumping straight to installation.

For example:

- User asks "how do I make my React app faster?" → search skills.sh for `react performance`
- User asks "can you help me with PR reviews?" → search skills.sh for `pr review`
- User asks "I need to create a changelog" → search skills.sh for `changelog`

The index will return results like:

```
Install with topclaw skills install https://skills.sh/<owner>/<repo>/<skill>

vercel-labs/agent-skills@vercel-react-best-practices
└ https://skills.sh/vercel-labs/agent-skills/vercel-react-best-practices
```

### Step 3: Present Options to the User

When you find relevant skills, present them to the user with:

1. The skill name and what it does
2. The safer TopClaw install command they can run
3. A link to learn more at skills.sh

Example response:

```
I found a skill that might help! The "vercel-react-best-practices" skill provides
React and Next.js performance optimization guidelines from Vercel Engineering.

To install it safely with TopClaw:
topclaw skills install https://skills.sh/vercel-labs/agent-skills/vercel-react-best-practices

Learn more: https://skills.sh/vercel-labs/agent-skills/vercel-react-best-practices
```

### Step 4: Offer to Install

If the user wants to proceed, you can install the skill for them:

```bash
topclaw skills install https://skills.sh/<owner>/<repo>/<skill>
topclaw skills vet <installed-skill-name> --json
```
Do not recommend unattended global installs or third-party package managers when the
same skill can be installed through TopClaw's audited path.

## Tips for Effective Searches

1. **Use specific keywords**: "react testing" is better than just "testing"
2. **Try alternative terms**: If "deploy" doesn't work, try "deployment" or "ci-cd"
3. **Check popular sources**: Many skills come from `vercel-labs/agent-skills` or `ComposioHQ/awesome-claude-skills`

## When No Skills Are Found

If no relevant skills exist:

1. Acknowledge that no existing skill was found
2. Offer to help with the task directly using your general capabilities
3. Suggest the user could create their own skill with the local `skill-creator` bundle

Example:

```
I searched for skills related to "xyz" but didn't find any matches.
I can still help you with this task directly! Would you like me to proceed?

If this is something you do often, you could create your own skill with:
python skills/skill-creator/scripts/init_skill.py my-xyz-skill --path ./skills
```
