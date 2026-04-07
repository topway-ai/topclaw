---
name: "multi-search-engine"
description: "Trigger only when the user explicitly needs a specific search engine or engine-specific query syntax — for example Baidu, Bing CN, Google, Yandex, WolframAlpha, DuckDuckGo bangs, or operators like site:, filetype:, and tbs=qdr:. Build the engine URL and call web_fetch. Supports 18 engines with no API keys: 8 CN (Baidu, Bing CN, Bing INT, 360, Sogou, WeChat, Toutiao, Jisilu) and 10 global (Google, Google HK, DuckDuckGo, Yahoo, Startpage, Brave, Ecosia, Qwant, Yandex, WolframAlpha). Prefer safe-web-search for normal lookups when a low-risk search tool is available — only reach for this skill when engine-specific behavior is required. Never send secrets, internal hostnames, tokens, or customer data to public engines. Read skills/multi-search-engine/SKILL.md for URL templates, advanced operators, time filters, and DuckDuckGo bangs."
---

# Multi Search Engine v2.0.1

Integration of 18 search engines for web crawling without API keys.

## Search Engines

### Domestic (8)
- **Baidu**: `https://www.baidu.com/s?wd={keyword}`
- **Bing CN**: `https://cn.bing.com/search?q={keyword}&ensearch=0`
- **Bing INT**: `https://cn.bing.com/search?q={keyword}&ensearch=1`
- **360**: `https://www.so.com/s?q={keyword}`
- **Sogou**: `https://sogou.com/web?query={keyword}`
- **WeChat**: `https://wx.sogou.com/weixin?type=2&query={keyword}`
- **Toutiao**: `https://so.toutiao.com/search?keyword={keyword}`
- **Jisilu**: `https://www.jisilu.cn/explore/?keyword={keyword}`

### International (10)
- **Google**: `https://www.google.com/search?q={keyword}`
- **Google HK**: `https://www.google.com.hk/search?q={keyword}`
- **DuckDuckGo**: `https://duckduckgo.com/html/?q={keyword}`
- **Yahoo**: `https://search.yahoo.com/search?p={keyword}`
- **Startpage**: `https://www.startpage.com/sp/search?query={keyword}`
- **Brave**: `https://search.brave.com/search?q={keyword}`
- **Ecosia**: `https://www.ecosia.org/search?q={keyword}`
- **Qwant**: `https://www.qwant.com/?q={keyword}`
- **Yandex**: `https://yandex.com/search/?text={keyword}`
- **WolframAlpha**: `https://www.wolframalpha.com/input?i={keyword}`

## Quick Examples

```javascript
// Basic search
web_fetch({"url": "https://www.google.com/search?q=python+tutorial"})

// Site-specific
web_fetch({"url": "https://www.google.com/search?q=site:github.com+react"})

// File type
web_fetch({"url": "https://www.google.com/search?q=machine+learning+filetype:pdf"})

// Time filter (past week)
web_fetch({"url": "https://www.google.com/search?q=ai+news&tbs=qdr:w"})

// Privacy search
web_fetch({"url": "https://duckduckgo.com/html/?q=privacy+tools"})

// Regional search
web_fetch({"url": "https://yandex.com/search/?text=opensource+rust+tools"})

// DuckDuckGo Bangs
web_fetch({"url": "https://duckduckgo.com/html/?q=!gh+tensorflow"})

// Knowledge calculation
web_fetch({"url": "https://www.wolframalpha.com/input?i=100+USD+to+CNY"})
```

## Guardrails

- Prefer `safe-web-search` for normal web lookup when a low-risk search tool is available.
- Use this skill only when the user explicitly needs a specific search engine or engine-specific query syntax.
- Do not send secrets, internal hostnames, tokens, customer data, or other sensitive queries to public search engines.
- Keep queries task-scoped and minimal; assume third-party engines log requests.
- Respect local policy and target-site terms before using automated fetches against search result pages.

## Advanced Operators

| Operator | Example | Description |
|----------|---------|-------------|
| `site:` | `site:github.com python` | Search within site |
| `filetype:` | `filetype:pdf report` | Specific file type |
| `""` | `"machine learning"` | Exact match |
| `-` | `python -snake` | Exclude term |
| `OR` | `cat OR dog` | Either term |

## Time Filters

| Parameter | Description |
|-----------|-------------|
| `tbs=qdr:h` | Past hour |
| `tbs=qdr:d` | Past day |
| `tbs=qdr:w` | Past week |
| `tbs=qdr:m` | Past month |
| `tbs=qdr:y` | Past year |

## Privacy Engines

- **DuckDuckGo**: No tracking
- **Startpage**: Google results + privacy
- **Brave**: Independent index
- **Qwant**: EU GDPR compliant
- **Yandex**: Strong regional coverage for Russian-language and Eastern Europe queries

## Bangs Shortcuts (DuckDuckGo)

| Bang | Destination |
|------|-------------|
| `!g` | Google |
| `!gh` | GitHub |
| `!so` | Stack Overflow |
| `!w` | Wikipedia |
| `!yt` | YouTube |

## WolframAlpha Queries

- Math: `integrate x^2 dx`
- Conversion: `100 USD to CNY`
- Stocks: `AAPL stock`
- Weather: `weather in Beijing`

## Documentation

- `references/international-search.md` - International search guide
- `CHANGELOG.md` - Version history

## License

MIT
