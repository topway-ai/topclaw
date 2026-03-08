# International Search Engine Deep Search Guide

## Google

### Basic advanced operators

| Operator | Function | Example | URL |
|----------|----------|---------|-----|
| `""` | Exact match | `"machine learning"` | `https://www.google.com/search?q=%22machine+learning%22` |
| `-` | Exclude term | `python -snake` | `https://www.google.com/search?q=python+-snake` |
| `OR` | Either term | `machine learning OR deep learning` | `https://www.google.com/search?q=machine+learning+OR+deep+learning` |
| `*` | Wildcard | `machine * algorithms` | `https://www.google.com/search?q=machine+*+algorithms` |
| `()` | Grouping | `(apple OR microsoft) phones` | `https://www.google.com/search?q=(apple+OR+microsoft)+phones` |
| `..` | Numeric range | `laptop $500..$1000` | `https://www.google.com/search?q=laptop+%24500..%241000` |

### Site and file search

| Operator | Function | Example |
|----------|----------|---------|
| `site:` | Search within a site | `site:github.com python projects` |
| `filetype:` | File type | `filetype:pdf annual report` |
| `inurl:` | URL contains | `inurl:login admin` |
| `intitle:` | Title contains | `intitle:"index of" mp3` |
| `intext:` | Body contains | `intext:password filetype:txt` |
| `cache:` | Cached page | `cache:example.com` |
| `related:` | Related sites | `related:github.com` |
| `info:` | Site info | `info:example.com` |

### Time filters

| Parameter | Meaning | URL example |
|-----------|---------|-------------|
| `tbs=qdr:h` | Past hour | `https://www.google.com/search?q=news&tbs=qdr:h` |
| `tbs=qdr:d` | Past 24 hours | `https://www.google.com/search?q=news&tbs=qdr:d` |
| `tbs=qdr:w` | Past week | `https://www.google.com/search?q=news&tbs=qdr:w` |
| `tbs=qdr:m` | Past month | `https://www.google.com/search?q=news&tbs=qdr:m` |
| `tbs=qdr:y` | Past year | `https://www.google.com/search?q=news&tbs=qdr:y` |

## DuckDuckGo

### Useful bangs

| Bang | Destination | Example |
|------|-------------|---------|
| `!g` | Google | `!g python tutorial` |
| `!gh` | GitHub | `!gh tensorflow` |
| `!so` | Stack Overflow | `!so javascript error` |
| `!w` | Wikipedia | `!w machine learning` |
| `!yt` | YouTube | `!yt python tutorial` |

### Parameters

| Parameter | Function | Example |
|-----------|----------|---------|
| `kp=1` | Strict safe search | `https://duckduckgo.com/html/?q=test&kp=1` |
| `kp=-1` | Disable safe search | `https://duckduckgo.com/html/?q=test&kp=-1` |
| `kl=us-en` | Region | `https://duckduckgo.com/html/?q=news&kl=us-en` |
| `ia=web` | Web results | `https://duckduckgo.com/?q=test&ia=web` |
| `ia=images` | Image results | `https://duckduckgo.com/?q=test&ia=images` |
| `ia=news` | News results | `https://duckduckgo.com/?q=test&ia=news` |

## Brave Search

| Parameter | Function | Example |
|-----------|----------|---------|
| `tf=pw` | This week | `https://search.brave.com/search?q=news&tf=pw` |
| `tf=pm` | This month | `https://search.brave.com/search?q=tech&tf=pm` |
| `tf=py` | This year | `https://search.brave.com/search?q=AI&tf=py` |
| `safesearch=strict` | Strict safe search | `https://search.brave.com/search?q=test&safesearch=strict` |
| `source=news` | News vertical | `https://search.brave.com/search?q=tech&source=news` |
| `source=images` | Image vertical | `https://search.brave.com/search?q=cat&source=images` |
| `source=videos` | Video vertical | `https://search.brave.com/search?q=music&source=videos` |

## Yandex

Yandex is useful for Russian-language, Eastern Europe, and Cyrillic-heavy queries where Western-focused engines can miss local sources.

### Query patterns

| Pattern | Function | Example |
|---------|----------|---------|
| Basic query | Standard search | `https://yandex.com/search/?text=rust+async+runtime` |
| Site search | Limit to site | `https://yandex.com/search/?text=site%3Agithub.com+tokio` |
| Exact phrase | Exact match | `https://yandex.com/search/?text=%22memory+safety%22` |
| File type | Target file extension | `https://yandex.com/search/?text=filetype%3Apdf+distributed+systems` |
| Region-heavy query | Local-language search | `https://yandex.com/search/?text=%D1%80%D0%B0%D0%B7%D1%80%D0%B0%D0%B1%D0%BE%D1%82%D0%BA%D0%B0+rust` |

### Examples

```javascript
// General search
web_fetch({"url": "https://yandex.com/search/?text=rust+cli+tools"})

// Site-specific search
web_fetch({"url": "https://yandex.com/search/?text=site:docs.rs+serde"})

// Russian-language query
web_fetch({"url": "https://yandex.com/search/?text=%D0%B0%D1%81%D0%B8%D0%BD%D1%85%D1%80%D0%BE%D0%BD%D0%BD%D1%8B%D0%B9+rust"})
```

## WolframAlpha

- Math: `integrate x^2 dx`
- Conversion: `100 USD to CNY`
- Stocks: `AAPL stock`
- Weather: `weather in Beijing`
