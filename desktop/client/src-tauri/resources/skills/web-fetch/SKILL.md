---
name: web-fetch
description: |
  Web content fetching tool. Trigger conditions:
  1. User asks to "open/visit/fetch/read" a web URL
  2. User asks to "get/view" the content of a web page
  3. User provides a URL and wants to understand its content
  4. Need to fetch information from a web page to assist in answering user questions
version: 1.0.0
author: moray
---

# Web Fetch - Web Content Fetching

Fetches the content of a specified URL, strips scripts/styles/tags, then returns plain text for LLM consumption.

Use this skill when `web_fetch` is **not** available as a direct API tool. Run via `shell` and the `moray-cli` sidecar (`$CLI` is set by the Moray shell tool).

## Environment Variable Syntax

| User System | Correct Syntax | Full Example |
|-------------|----------------|--------------|
| **macOS / Linux** | `$CLI` | `$CLI tool run web_fetch --args '{"url":"https://example.com"}'` |
| **Windows CMD** | `%CLI%` | `%CLI% tool run web_fetch --args "{\"url\":\"https://example.com\"}"` |
| **Windows PowerShell** | `& $env:CLI` | `& $env:CLI tool run web_fetch --args '{"url":"https://example.com"}'` |

## Tool: web_fetch

```bash
$CLI tool run web_fetch --args '{"url": "<HTTP or HTTPS URL>"}'
```

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `url` | string | yes | HTTP or HTTPS URL to fetch |

**Examples:**

```bash
$CLI tool run web_fetch --args '{"url": "https://example.com"}'
```

## Notes

- Only `http` and `https` URLs are supported
- Request timeout is 30 seconds
- HTML responses are converted to plain text (tags/scripts/styles removed)
