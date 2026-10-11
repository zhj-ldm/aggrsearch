# aggrsearch

面向 AI 的**多聚合搜索服务**：一个 Rust 二进制，同时提供本地 HTTP 接口和命令行工具，专为 LLM / Agent 取数设计。

核心能力两件事：**搜得到**（多引擎并发 + 语义重排）和**读得全**（正文抽取 + CDN/WAF 拦截识别）。

---

## 特性

| 能力 | 说明 |
| --- | --- |
| 多引擎聚合 | `bing_intl` > `bing_cn` > `baidu` > `sogou` 并发抓取，URL 去重后合并（360 已排除） |
| 语义重排 | 词法相关度（title 权重 3 / snippet 权重 1）+ 引擎偏置排序 |
| CJK 优化 | 查询预处理删除中文字符两侧空格，避免必应分词器拆词 |
| 反爬对抗 | 百度 `wappass`/安全验证、搜狗 `antispider` 检测，per-engine 指数退避（5s 起，封顶 120s） |
| 首页握手 | 百度/搜狗先访问首页建立 Cookie 与 Referer，避免直接打搜索页被拦 |
| 搜狗 TLS | 搜狗专用客户端强制 TLS 1.2，规避无头指纹识别 |
| 正文抽取 | 剥离 `script`/`style`/标签，正文质量分 < 0.35 判定被墙 |
| 拦截缓存 | 被 CDN/WAF 拦截的域名缓存 2 小时，不再重复消耗时间 |
| 零硬编码评分 | 质量分完全由 DOM 结构/文本比例/句子数/文本量统计得出 |
| 诊断日志 | 每引擎状态码、字节数、质量分、反爬命中全部写 **stderr**；结果走 **stdout**，可安全管道 |

## 安装

macOS（Apple Silicon）：

```bash
curl -fsSL -O https://github.com/zhj-ldm/aggrsearch/releases/latest/download/aggrsearch-v0.5.0-macos-arm64.tar.gz
tar xzf aggrsearch-v0.5.0-macos-arm64.tar.gz
./aggrsearch version
```

从源码构建：

```bash
git clone https://github.com/zhj-ldm/aggrsearch.git
cd aggrsearch
cargo build --release
./target/release/aggrsearch version
```

## 命令行用法

```bash
# 启动本地 HTTP 服务（默认 127.0.0.1:8080）
aggrsearch serve
aggrsearch serve --port 9090

# 直接搜索
aggrsearch search "Rust axum 教程"
aggrsearch search "英伟达财报" --max 5 --topic news --freshness w
aggrsearch search "site docs" --include docs.rs,github.com --exclude medium.com
aggrsearch search "缓存策略" --engines bing_intl,baidu --json

# 抓取网页正文
aggrsearch fetch https://example.com/article
aggrsearch fetch https://example.com/article --json

# 健康检查 / 版本
aggrsearch health
aggrsearch version
```

### search 选项

| 选项 | 说明 |
| --- | --- |
| `--max <n>` | 返回条数，1-20，默认 10 |
| `--freshness <d\|w\|m\|y>` | 时间范围，仅作用于必应 `&tbs=qdr:` |
| `--topic <news>` | `news` 走必应新闻垂直 |
| `--include <a,b>` | 仅保留指定域名（含子域匹配） |
| `--exclude <a,b>` | 排除指定域名 |
| `--engines <a,b>` | 限定引擎：`bing_intl,bing_cn,baidu,sogou` |
| `--json` | 输出原始 JSON，便于管道交给 LLM |

> 管道友好：`aggrsearch search "关键词" --json | jq -r '.results[].url'`

## HTTP 接口

启动 `aggrsearch serve` 后监听 `127.0.0.1:8080`（可用 `PORT` 环境变量或 `--port` 修改）。

### `GET /search`

| 参数 | 取值 |
| --- | --- |
| `q` | 查询词（必填） |
| `max_results` | 1-20，默认 10 |
| `freshness` | `d` \| `w` \| `m` \| `y` |
| `topic` | `general` \| `news` |
| `include_domains` | 逗号分隔 |
| `exclude_domains` | 逗号分隔 |
| `engines` | 逗号分隔的引擎名 |

```bash
curl 'http://127.0.0.1:8080/search?q=Rust+axum&max_results=3'
```

响应结构：

```json
{
  "results": [
    {
      "title": "...",
      "url": "https://...",
      "snippet": "...",
      "content": "...",
      "_rerank_score": 0.72,
      "_source_engine": "bing_intl"
    }
  ],
  "query": "Rust axum",
  "backend": "multi",
  "_timing": {
    "total": 0.83,
    "stages": { "search": 0.71, "rerank": 0.01, "num_results_merged": 12 },
    "engine_counts": { "bing_intl": 6, "baidu": 3, "sogou": 3 }
  }
}
```

### `GET /fetch`

```bash
curl 'http://127.0.0.1:8080/fetch?url=https://example.com/article'
```

响应包含 `status`、`content_type`、`body`（纯正文）、`error` 与 `_timing.total`。
被 CDN/WAF 拦截时返回 `error` 且 `body` 为空，并把该域名加入 2 小时拦截缓存。

### `GET /health`

```json
{ "status": "ok", "name": "aggrsearch", "version": "0.5.0" }
```

## 日志

诊断日志走 stderr，结果走 stdout，因此可以直接重定向分离：

```bash
aggrsearch search "关键词" --json 2> search.log | jq .
```

典型日志：

```text
[bing_intl] status=200 bytes=48213 quality=0.61
[baidu] status=200 bytes=12044 wappass=false quality=0.58
[sogou] status=200 bytes=3310 antispider=true quality=0.19
[sogou] 进入退避 5s
```

## 作为 LLM 工具接入

服务默认只绑定回环地址，不对外暴露。Claude / Cursor / 自建 Agent 的典型接法：

1. 用 `aggrsearch fetch <url> --json` 作为取原文的手段（LLM 上下文更省）；
2. 或让 Agent 把 `http://127.0.0.1:8080/search` 当作 HTTP 工具调用。

## 开发

```bash
cargo fmt
cargo test        # 纯函数单测，不触网
cargo build --release
cargo clippy
```

## 许可证

Apache-2.0，见 [LICENSE](LICENSE)。