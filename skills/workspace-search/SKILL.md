---
name: workspace-search
description: Search the local workspace when the user wants to find code, config, docs, symbols, strings, TODOs, references, or repeated patterns. Use this skill for fast read-only retrieval across files before deeper analysis or edits.
---

# Workspace Search

Use this skill for targeted read-only search across the allowed workspace.

## Goals

- Find the smallest set of matching files or lines that answer the question.
- Prefer precise search terms over broad scans.
- Return actionable search results, not raw dumps.

## Workflow

1. Choose a narrow query from the user's wording.
2. Search filenames first when the target sounds structural.
3. Search file contents when the target sounds semantic.
4. Refine the query if the first pass is too broad or too narrow.
5. Open only the top matches needed for confirmation.

## Guardrails

- Stay inside allowed roots.
- Stay read-only.
- Avoid dumping large irrelevant excerpts; summarize and quote only the needed lines.
- If search results suggest sensitive files, stop and follow external policy.

## Output

- State what was searched.
- List the best matches with file paths.
- Explain why each match matters.
