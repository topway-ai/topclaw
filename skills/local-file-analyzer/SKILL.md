---
name: local-file-analyzer
description: Read and summarize local files inside the allowed workspace when the user asks to inspect a document, code file, notes folder, transcript, or project artifact. Use this skill for read-only file understanding tasks such as summarizing, extracting key points, comparing text, or answering questions about local content.
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
