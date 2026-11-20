<p align="center">
  <img src="docs/readme-hero.png" alt="aggrsearch — 面向 LLM 与 Agent 的多引擎聚合搜索">
</p>

<p align="center">
  <a href="https://github.com/zhj-ldm/aggrsearch/releases/latest"><img alt="release" src="https://img.shields.io/github/v/release/zhj-ldm/aggrsearch?style=flat-square&color=41e1bd"></a>
  <img alt="rust" src="https://img.shields.io/badge/Rust-2024-000000?style=flat-square&logo=rust">
  <img alt="license" src="https://img.shields.io/badge/license-Apache--2.0-blue?style=flat-square">
  <img alt="local" src="https://img.shields.io/badge/server-127.0.0.1-41e1bd?style=flat-square">
</p>

<p align="center">
  <b>一个为 LLM / Agent 设计的多引擎搜索与正文抽取服务。</b><br>
  一个 Rust 二进制，同时提供本地 HTTP API 与 CLI。
</p>

> 核心目标只有两个：**搜得到**（多引擎并发 + 语义重排）和 **读得全**（正文抽取 + CDN/WAF 拦截识别）。<br>
> 不需要 API Key，不依赖浏览器，不把搜索结果困在某个 SaaS 里。

## 为什么会有 aggrsearch

Agent 的搜索工具通常只有两种选择：昂贵的官方搜索 API，或者直接爬搜索引擎 HTML、拿回一堆导航和广告。

aggrsearch 走第三条路：

- **多个引擎一起搜**，URL 去重后合并，不把结果押在一个来源上。
- **用结构化规则重排**，标题权重高于摘要，再叠加引擎先验；不引入额外模型。
- **把网页正文读出来**，剥离脚本、导航和模板，让 LLM 拿到可用的上下文。
- **识别反爬状态**，把被 CDN/WAF 拦下的域名缓存起来，避免反复浪费时间。
- **只在本机监听**，CLI 与 HTTP 走同一套核心逻辑。

## 能力总览

| 能力 | 实现 |
| --- | --- |
| 多引擎聚合 | `bing_intl` > `bing_cn` > `baidu` > `sogou` 并发抓取，URL 去重合并 |
| 语义重排 | 本地 EmbeddingGemma-300M 余弦相似度（对齐 SearchPIN），模型不可用时回退词法相关度 |
| CJK 优化 | 删除中文字符两侧空格，避免中文查询被分词器拆碎 |
| 反爬对抗 | 百度 `wappass`、搜狗 `antispider` 检测；每引擎指数退避 5s → 120s |
| 首页握手 | 百度 / 搜狗先访问首页建立 Cookie 与 Referer |
| TLS 适配 | 搜狗专用客户端强制定向 TLS 1.2 |
| 正文抽取 | 移除 `script` / `style` / 标签，按文本结构计算质量分 |
| 拦截缓存 | 被 WAF 拦截的域名缓存 2 小时，避免重复消耗 |
| 双入口 | HTTP 服务 + CLI，JSON 输出管道友好 |
| 可观测 | 每引擎状态码 / 字节数 / 质量分 / 退避全部写 stderr |

## 快速开始

### macOS 发布包

```bash
curl -fsSL -O https://github.com/zhj-ldm/aggrsearch/releases/latest/download/aggrsearch-v0.5.0-macos-arm64.tar.gz
tar xzf aggrsearch-v0.5.0-macos-arm64.tar.gz
cd aggrsearch-v0.5.0-macos-arm64
./aggrsearch version
```

### 从源码构建

```bash
git clone https://github.com/zhj-ldm/aggrsearch.git
cd aggrsearch
cargo build --release
./target/release/aggrsearch version
```

Rust 版本要求：`1.85+`（edition 2024）。

### Embedding 重排

默认使用 `EmbeddingGemma-300M`（ONNX int8，约 309MB），首次搜索时自动下载并缓存，之后完全本地推理。ONNX Runtime 已静态链接进单文件 exe，Windows 版无需额外 DLL。

| 环境变量 | 说明 |
| --- | --- |
| `AGGRSEARCH_EMBED=off` | 关闭 embedding，仅用词法重排 |
| `AGGRSEARCH_EMBED_MODEL` | `gemma`（默认）/ `gemma-q4`（更小更快）/ `minilm`（测试用） |
| `AGGRSEARCH_EMBED_CACHE` | 模型缓存目录（默认 Windows `%LOCALAPPDATA%\aggrsearch\models`，macOS `~/Library/Caches/aggrsearch/models`） |
| `AGGRSEARCH_EMBED_THREADS` | ONNX Runtime 推理线程数，默认用满所有核心 |
| `HF_ENDPOINT` | Hugging Face 镜像，例如 `https://hf-mirror.com` |

## 两种使用方式

### 1. 本地 HTTP 服务

```bash
# 默认监听 127.0.0.1:8080
aggrsearch serve
aggrsearch serve --port 9090
```

```bash
curl 'http://127.0.0.1:8080/search?q=Rust+axum&max_results=3'
curl 'http://127.0.0.1:8080/fetch?url=https://example.com/article'
curl 'http://127.0.0.1:8080/health'
```

### 2. 命令行直接使用

```bash
aggrsearch search "Rust axum 教程"
aggrsearch search "英伟达财报" --max 5 --topic news --freshness w
aggrsearch search "site docs" --include docs.rs,github.com --exclude medium.com
aggrsearch search "缓存策略" --engines bing_intl,baidu --json

aggrsearch fetch https://example.com/article
aggrsearch fetch https://example.com/article --json
```

管道交给 LLM：

```bash
aggrsearch search "向量数据库选型" --json | jq -r '.results[].url'
```

## CLI 参数

### `search`

| 参数 | 说明 |
| --- | --- |
| `--max <n>` | 返回条数，1–20，默认 10 |
| `--freshness <d\|w\|m\|y>` | 时间范围，作用于必应 `&tbs=qdr:` |
| `--topic <news>` | `news` 走必应新闻垂直 |
| `--include <a,b>` | 仅保留指定域名（含子域） |
| `--exclude <a,b>` | 排除指定域名 |
| `--engines <a,b>` | 限定引擎：`bing_intl,bing_cn,baidu,sogou` |
| `--json` | 输出原始 JSON，便于管道处理 |

### `fetch`

抓取网页并抽取正文。被 CDN/WAF 拦截时，返回 `error`，`body` 为空，并把域名加入 2 小时拦截缓存。

## HTTP 响应

`GET /search` 的典型响应：

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

`GET /fetch` 返回 `status`、`content_type`、`body`（纯正文）、`error` 与 `_timing.total`。

## 工作方式

```text
                    ┌──────────────┐
                    │  query + opts │
                    └──────┬───────┘
                           │
         ┌─────────────────┼─────────────────┐
         ▼                 ▼                 ▼
   bing_intl           bing_cn            baidu / sogou
         │                 │                 │
         └─────────────────┼─────────────────┘
                           ▼
                  URL 去重 / 过滤 / 清洗
                           ▼
                  相关度 + 引擎偏置重排
                           ▼
                       JSON / Text
```

`fetch` 路径：

```text
URL → HTTP 抓取 → 拦截识别 → 剥离 script/style → 正文质量分 → 纯文本 / JSON
```

### 设计取舍

- **不调用搜索 API**：免 Key、免账号，代价是依赖引擎 HTML 结构，需要持续维护。
- **不引入向量模型**：重排完全可解释，启动没有额外模型加载和显存占用。
- **质量分不是魔法**：由 DOM 结构、文本比例、句子数、文本量统计得出，阈值调参可复现。
- **先保结果，再报错误**：单引擎失败不影响整体；只有全部引擎失败才返回错误。

## 可观测性

诊断日志写 **stderr**，结果写 **stdout**，不会污染 JSON 管道：

```bash
aggrsearch search "关键词" --json 2> search.log | jq .
```

```text
[bing_intl] status=200 bytes=48213 quality=0.61
[baidu] status=200 bytes=12044 wappass=false quality=0.58
[sogou] status=200 bytes=3310 antispider=true quality=0.19
[sogou] 进入退避 5s
```

## 作为 LLM / Agent 工具

- 把 `http://127.0.0.1:8080/search` 注册为 HTTP 工具。
- 或者用 `aggrsearch fetch <url> --json` 单独获取原文，减少上下文里的导航噪声。
- 默认只绑定回环地址，不会暴露到局域网。

## 开发

```bash
cargo fmt
cargo test        # 纯函数单测，不触网
cargo clippy
cargo build --release
```

## 已知边界

- 搜索引擎页面结构变化会导致解析失效，需要更新选择器。
- 抓取频率过高会触发验证码或临时封禁；请控制请求频率并遵守目标站点条款。
- 本项目不绕过登录、付费墙或验证码，只做公开页面的读取。
- 适合个人 Agent、内部工具和检索实验，不建议直接作为面向公网的高并发搜索后端。

## License

[Apache License 2.0](LICENSE)
