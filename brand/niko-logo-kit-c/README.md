# Niko Logo Kit — C 方案

这是一套独立候选资产，没有替换 `src/assets`、`public` 或 `src-tauri/icons` 中的正式 Logo。主标志沿用 C 方案的连续圆角 N、粉蓝—暖黄—杏桃—珊瑚渐变，并把轮廓重建为单一闭合路径，避免拼接、重叠和接缝。

## 快速选择

| 场景 | 推荐文件 |
| --- | --- |
| 默认主标志 | `logo/niko-primary-full-color.svg` |
| 只有图形的位置 | `logo/niko-mark-full-color.svg` |
| 只有名称的位置 | `logo/niko-wordmark.svg` |
| 横向页眉、官网、GitHub | `logo/niko-lockup-horizontal.svg` |
| 竖向海报、封面 | `logo/niko-lockup-stacked.svg` |
| 16–32 px 小尺寸 | `logo/niko-mark-small.svg` 或 `icons/favicon.svg` |
| 浅色背景展示稿 | `variants/niko-primary-on-light.svg` |
| 深色背景展示稿 | `variants/niko-primary-on-dark.svg` |
| 纯黑、纯白、单色 | `variants/niko-mark-black.svg`、`niko-mark-white.svg`、`niko-mark-monochrome-coral.svg` |
| App / Web 图标 | `icons/niko-app-icon.svg`、`icons/niko-web-icon.svg` |
| 社交头像 | `icons/niko-social-avatar.svg` |
| 总览预览 | `previews/niko-logo-kit-overview.png` |
| 使用规范 | `guidelines/niko-logo-guidelines.png` |

PNG 版本与同名 SVG 放在同一目录。透明主图形的常用尺寸位于 `png/mark/`，包含 16、32、48、64、128、256、512、1024 px；其中 16 和 32 px 自动使用简化轮廓。

## 色板

| 名称 | HEX | RGB | 用途 |
| --- | --- | --- | --- |
| Sky Blue | `#78C5DF` | `120, 197, 223` | 渐变起点、亲近与清爽感 |
| Warm Yellow | `#F3D47F` | `243, 212, 127` | 渐变中心、温暖感 |
| Apricot | `#F5AD78` | `245, 173, 120` | 黄与珊瑚之间的柔和过渡 |
| Coral | `#EE9288` | `238, 146, 136` | 渐变终点、识别亮点 |
| Ink | `#22304A` | `34, 48, 74` | 浅色背景字标 |
| Cream | `#FFF8F0` | `255, 248, 240` | 浅色品牌底 |
| Night | `#162038` | `22, 32, 56` | 深色品牌底 |

机器可读色板见 `guidelines/niko-color-palette.csv`。

## 安全区与最小尺寸

- 图形四周至少保留 `x = 图形视觉宽度的 1/4`。
- 默认图形推荐不小于 24 px；16–32 px 使用小尺寸简化版。
- 横向组合推荐不小于 120 px 宽。
- 印刷中图形推荐不小于 8 mm；横向组合推荐不小于 30 mm 宽。
- 小尺寸不要叠加描边、阴影或纹理；优先使用纯色、浅色或深色干净背景。

## 背景与单色使用

- 浅色背景：全彩图形配 Ink 字标。
- 深色背景：全彩图形配白色字标。
- 无法使用渐变时：优先使用全黑、全白或 Coral 单色版。
- `niko-mark-white.png` 和 `niko-primary-white.png` 是透明底白色图稿，在浅色图片查看器中可能看起来空白，这是正常现象。

## 禁止用法

- 不旋转、拉伸、压扁、裁切或重排图形。
- 不交换渐变方向，不增加任意品牌外颜色。
- 不添加描边、投影、发光、纹理或 3D 效果。
- 不把全彩版放在低对比、过于复杂的图片上。
- 不侵入安全区，也不要重新拼接轮廓。

## 目录说明

- `logo/`：主标志、图形标、字标、横向与纵向组合、小尺寸版。
- `variants/`：浅底、深底、黑、白、Coral 单色版本。
- `icons/`：favicon、Web/App 图标和社交头像；含常用 PNG 尺寸。
- `png/mark/`：透明图形标的 16–1024 px 导出。
- `guidelines/`：可视化使用规范和机器可读色板。
- `previews/`：整套资产总览图。
- `source/`：原始 C 方案的 Image2 精修参考与提示词；它是视觉参考，不是最终矢量资产。
- `tools/`：可复现全部 SVG/PNG 的本机构建脚本。

## 重新生成

先安装官网构建依赖，再从仓库根目录运行脚本：

```bash
npm --prefix website ci
node brand/niko-logo-kit-c/tools/build-logo-kit.mjs
```

## 制作说明

Image2 参考图使用编辑模式、`gpt-image-2-count` 模型、1024 × 1024 画布生成。最终 Logo 没有自动描摹位图，而是根据 C 方案和精修参考重建为一个连续闭合的 SVG 轮廓。定制 `Niko` 字标也是几何路径，不依赖外部字体文件。
