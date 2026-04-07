---
name: local-file-analyzer
description: "Trigger when the user asks to inspect, summarize, extract from, compare, or answer questions about a local file or document inside the allowed workspace. Open the smallest set of files needed to answer the question, lead with the answer, cite exact file paths, and call out partial coverage or unreadable formats. For very large files, inspect targeted sections before reading more. Stay strictly read-only — do not edit or propose edits unless explicitly asked. Never read secrets, credentials, or runtime config unless external policy explicitly allows it."
---

# Local File Analyzer

Use this skill for read-only file analysis inside the user's allowed workspace.

## Goals

- Read the smallest relevant set of files.
- Summarize content accurately and cite file paths.
- Avoid broad recursive reads unless the user asked for them.

## Workflow

1. Confirm the target path or file set.
2. Read only the files needed to answer the question.
3. Prefer text, Markdown, config, and source files first.
4. If a file is very large, inspect targeted sections before reading more.
5. Report findings with concise file references and note any uncertainty.

## Guardrails

- Stay read-only.
- Do not modify files or propose edits unless the user explicitly asks for them.
- Do not read secrets, credentials, or runtime config unless external policy explicitly allows it.
- If the request would require scanning a very large tree, narrow scope first.

## Output

- Lead with the answer or summary.
- Include the exact files inspected.
- Call out missing files, unreadable formats, or partial coverage.
