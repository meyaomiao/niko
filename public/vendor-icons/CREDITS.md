# Vendor brand assets

These assets identify providers/products in Niko's vendor lists. They are local files: server metadata can select a known icon identifier, but cannot supply a URL, SVG markup or executable content. Brand names and marks belong to their respective owners; inclusion does not imply affiliation or endorsement.

## Assets obtained from brand websites

| Local file | Brand / use | Original source | Preparation and rights |
| --- | --- | --- | --- |
| `302-official.png` | 302.AI | [302.AI website](https://302.ai/) → [official horizontal logo](https://302.ai/img/logo-horizontal-dark.png) | Cropped the existing symbol from the left 1046×1046 square of the 3717×1046 logo, then resized to 128×128 PNG. No new shape or lettering was drawn. No open-source license was stated for this website image; the mark remains the property of 302.AI. |
| `nanogpt.png` | NanoGPT | [NanoGPT website](https://nano-gpt.com/) → [image used as “Nano GPT logo”](https://media.nano-gpt.cloud/media/eb3b803660726ee36aaba2327cf585b07f490694c53f0a84e8c1b0f896701a72-w128-webp80-v1.webp) | Converted the website's 128×132 WebP into PNG without changing its geometry. No open-source license was stated for this website image; the mark remains the property of NanoGPT. |
| `sap.svg` | SAP, used to identify the SAP AI Core provider | [SAP-hosted logo SVG](https://www.sap.com/dam/application/shared/logos/sap-logo-svg.svg) | Kept the supplied paths, gradient and proportions. Converted its two CSS classes to presentation attributes and removed the style block and unused metadata. This is SAP's company mark, not a claim of a separate AI Core product mark. SAP retains its trademark/logo rights; no open-source license was supplied with the SVG. |

## Community-maintained brand SVGs from LobeHub

The following files are vendored from **`@lobehub/icons-static-svg` version `1.102.0`**, published by [LobeHub / lobe-icons](https://github.com/lobehub/lobe-icons). These are LobeHub's maintained renditions of existing brand marks, **not assets claimed to have been supplied directly by each vendor**. The upstream package declares the MIT license; its notice is reproduced below.

Only the listed files are bundled; Niko does not depend on the icon package at runtime. Original path geometry, colors and gradients are retained. Root `width`/`height` and inline layout styles are removed to let the component size the image. `currentColor` becomes `#111827` in standalone SVG files; monochrome marks use CSS inversion in dark mode. SVG scripts, event attributes, embedded HTML, external resources and DTD/entity declarations are not allowed.

| Local file | Represented brand / product | Pinned upstream asset |
| --- | --- | --- |
| `google-color.svg` | Google company mark; fallback when no recognized product icon is supplied | [Google](https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@1.102.0/icons/google-color.svg) |
| `gemini-color.svg` | Google Gemini; server identifier `Gemini.Color` | [Gemini](https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@1.102.0/icons/gemini-color.svg) |
| `vertexai-color.svg` | Google Vertex AI | [Vertex AI](https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@1.102.0/icons/vertexai-color.svg) |
| `opencode.svg` | OpenCode, including OpenCode Zen | [OpenCode](https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@1.102.0/icons/opencode.svg) |
| `deepseek-color.svg` | DeepSeek | [DeepSeek](https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@1.102.0/icons/deepseek-color.svg) |
| `xai.svg` | xAI | [xAI](https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@1.102.0/icons/xai.svg) |
| `minimax-color.svg` | MiniMax | [MiniMax](https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@1.102.0/icons/minimax-color.svg) |
| `anthropic.svg` | Anthropic company mark; fallback when no recognized product icon is supplied | [Anthropic](https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@1.102.0/icons/anthropic.svg) |
| `claude-color.svg` | Anthropic Claude; server identifier `Claude.Color` | [Claude](https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@1.102.0/icons/claude-color.svg) |
| `openai.svg` | OpenAI | [OpenAI](https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@1.102.0/icons/openai.svg) |
| `spark-color.svg` | iFlytek Spark / 讯飞星火 | [Spark](https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@1.102.0/icons/spark-color.svg) |
| `zhipu-color.svg` | Zhipu / 智谱 | [Zhipu](https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@1.102.0/icons/zhipu-color.svg) |
| `moonshot.svg` | Moonshot AI / 月之暗面; shared by “Moonshot” and “Moonshot AI” | [Moonshot](https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@1.102.0/icons/moonshot.svg) |
| `zai.svg` | Z.AI | [Z.AI](https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@1.102.0/icons/zai.svg) |
| `doubao-color.svg` | ByteDance Doubao / 字节跳动豆包 | [Doubao](https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@1.102.0/icons/doubao-color.svg) |
| `qwen-color.svg` | Alibaba Qwen / 阿里巴巴千问 | [Qwen](https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@1.102.0/icons/qwen-color.svg) |

A recognized server identifier takes priority over the vendor-name fallback. For example, `Google + Gemini.Color` displays Gemini, while `Google` without an icon displays Google's company mark. Likewise `Anthropic + Claude.Color` displays Claude rather than the company mark.

## Unverified brand: QiHang

The reviewed public MomoToken pricing catalog names **QiHang** but supplies no icon, website or description for it. The external [models.dev catalog](https://models.dev/api.json) has a `qihang-ai` entry named QiHang and points to `https://api.qhaigc.net/v1` and `https://www.qhaigc.net/docs`. However, the [current qhaigc.net website](https://www.qhaigc.net/) identifies itself as **QuanHex**. Those facts do not independently establish which current mark the MomoToken vendor intended.

Niko therefore does **not** assign a QuanHex logo, an unrelated “Q” logo, or another vendor's mark to QiHang. It displays the neutral unknown-image symbol with the tooltip “暂无可核实品牌图标”. The same explicit fallback is used for other unrecognized names/identifiers. This is the one unresolved vendor among the 19 names in the reviewed public catalog.

## LobeHub MIT notice

Source: [LobeHub license](https://github.com/lobehub/lobe-icons/blob/master/LICENSE), also available through [the repository mirror](https://cdn.jsdelivr.net/gh/lobehub/lobe-icons@master/LICENSE).

```text
MIT License

Copyright (c) 2023 LobeHub

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
