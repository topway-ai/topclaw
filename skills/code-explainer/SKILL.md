---
name: code-explainer
description: "Trigger when the user asks what a function, file, test, module, or subsystem does, or wants an architecture walkthrough or onboarding tour of existing code. Read the smallest relevant files first, trace entrypoints before helpers, and explain control flow, important types, and side effects with concrete file references. Stay grounded in the current repository state — do not speculate about behavior you have not verified from code. Distinguish confirmed behavior from inference, name relevant config surfaces when behavior depends on them, and mention tests or missing tests when they affect confidence."
---

# Code Explainer

Use this skill to explain existing code without changing it.

## Goals

- Explain behavior in the language of the codebase.
- Clarify module boundaries, inputs, outputs, and side effects.
- Make the explanation easy for a contributor to act on.

## Workflow

1. Read the smallest relevant files first.
2. Trace entrypoints before diving into helper details.
3. Distinguish confirmed behavior from inference.
4. Prefer concrete references over vague architectural claims.
5. Mention tests or missing tests when they affect confidence.

## Guardrails

- Stay grounded in the current repository state.
- Do not speculate about behavior you did not verify from code.
- Keep explanations concise unless the user asks for depth.
- If behavior depends on config, name the relevant config surface.

## Output

- Start with the high-level purpose.
- Then describe control flow, important types, and side effects.
- Include file references for the critical paths.
