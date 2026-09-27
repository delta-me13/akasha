---
name: structured-logging
description: "Use when: emit structured, queryable logs with the right levels, context, and no sensitive data."
---

Goal: logs that are machine-queryable and useful during incidents.

Use for:
- adding or cleaning up application logging
- making logs searchable and correlatable
- fixing noisy, unstructured, or leaky logs

Workflow:
1. Log structured key-value events, not free-form strings.
2. Attach context: request ID, user ID, and trace ID.
3. Use levels deliberately: error, warn, info, debug.
4. Log at boundaries and on state changes, not every line.
5. Redact secrets and PII before they are written.
6. Verify logs answer "what happened to this request".

Practices:
- one event per significant action, with fields
- consistent field names across services
- correlation IDs to stitch a request together
- sampling for high-volume debug logs

Rules:
- never log secrets, tokens, or PII
- use levels consistently; do not log everything as info
- prefer structured fields over string interpolation
- make every error log actionable, with context
