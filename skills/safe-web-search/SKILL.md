---
name: safe-web-search
description: Search the web for current facts, documentation, announcements, or links when the user asks to look something up. Use this skill when fresh internet information is needed and the low-risk `web_search` tool is available, especially with DuckDuckGo or a trusted SearxNG instance.
---

# Safe Web Search

Use this skill for low-risk web lookup through the `web_search` tool.

## Goals

- Retrieve current information without using a browser.
- Prefer text-only search results with titles, links, and short summaries.
- Keep the query specific and minimize unnecessary external requests.

## Workflow

1. Confirm the information really needs current web data.
2. Prefer the `web_search` tool over browser automation.
3. Use short, specific queries and refine only if needed.
4. Summarize the top results instead of dumping raw output.
5. Include the most relevant links in the answer.

## Guardrails

- Use this skill only when the `web_search` tool is enabled.
- Prefer `duckduckgo` or a trusted self-hosted `searxng` endpoint for the lowest-risk setup.
- Keep to search-result retrieval only. Do not switch to browser automation, form submission, login flows, or cookie-backed sessions unless the user explicitly asks and external policy allows it.
- If the result quality is poor, say so clearly instead of inventing certainty.
- If the request would touch sensitive domains or secrets, stop and follow external policy.

## Output

- Lead with the answer.
- Cite the query used when it matters.
- List the best links and note any uncertainty or staleness risk.
