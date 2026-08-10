# openIME 品牌视觉规范（VI）

> openIME 的视觉识别系统。所有产品界面、图标、物料均以此为唯一基准。

---

## 1. 品牌核心理念

| 维度 | 内容 |
|---|---|
| **产品** | 开源、跨平台、本地优先的语音输入法 |
| **核心动作** | 说话即输入 —— 声音转化为文字 |
| **品牌承诺** | 精确、克制、值得信赖的工具 |
| **气质** | 极简编辑风（开发者工具气质，非消费级应用） |
| **隐喻** | 声波 / 语音波形 |

---

## 2. Logo

### 2.1 标志构成

openIME 标志由两部分组成：

1. **图形标志（Mark）**：5 条对称竖条组成的声波波形，中央竖条最高、两侧递减
2. **字标（Wordmark）**：`openIME`（小写 o + 驼峰 IME）

### 2.2 App Icon（macOS squircle 形态）

```
载体：1024×1024 macOS squircle（Continuous Corner）
底色：粒蓝 #3B4FE0 实色填充
图形：5 条白色竖条，圆角 28px
几何：条宽 56，间距 36，高度 180/300/420/300/180（外→内）
```

**几何示意**：
```
        ▁▁
     ▁▁ ▁▁ ▁▁
  ▁▁ ▁▁ ▁▁ ▁▁ ▁▁     ← 5 条对称竖条
  ▁▁ ▁▁ ▁▁ ▁▁ ▁▁        中峰最高
  ▁▁ ▁▁ ▁▁ ▁▁ ▁▁
```

### 2.3 菜单栏 Icon（template image）

macOS 状态栏使用**单色 template image**（3 条粗竖条简化版），由系统随明暗模式自动反色。

```
规格：64×64（@2x，逻辑 32px）
填充：纯黑 #000000 + 透明背景
几何：3 条竖条，条宽 8，间距 4，高度 16/28/16，圆角 4
```

简化到 3 条是为了在 16px 状态栏尺寸下保持清晰。

### 2.4 Logo 源文件

| 文件 | 用途 |
|---|---|
| `branding/app-icon.svg` | App icon 矢量源（彩色 squircle） |
| `branding/menubar-icon.svg` | 菜单栏 template 矢量源（单色） |
| `branding/concepts/` | 设计过程文件（3 个概念方向 + 对比图） |
| `src-tauri/icons/icon.iconset/` | macOS 各尺寸 PNG（16~1024） |
| `src-tauri/icons/icon.icns` | macOS .icns 打包格式 |
| `src-tauri/icons/menubar-template@2x.png` | 菜单栏 template（嵌入 Rust） |

---

## 3. 色彩系统

### 3.1 主色（Brand Primary）

**粒蓝 Indigo `#3B4FE0`** —— openIME 的品牌主色，用于 logo、app icon、强调色。

| 角色 | 浅色模式 | 深色模式 |
|---|---|---|
| 主色 accent | `#3B4FE0` | `#5C6AFF`（提亮，保证暗底对比度） |
| 主色 hover | `#2F40C7` | `#4452ED` |
| 主色 soft（背景） | `rgba(59,79,224,0.10)` | `rgba(92,106,255,0.16)` |

> 深色模式主色提亮是 macOS HIG 标准做法（系统蓝也从 `#007aff`→`#0a84ff`）。

### 3.2 中性色

| 角色 | 浅色 | 深色 |
|---|---|---|
| 背景 bg | `#f5f5f7` | `#161617` |
| 卡片 card | `#ffffff` | `#1f1f21` |
| 文字主 | `#1d1d1f` | `#f5f5f7` |
| 文字次 | `#6e6e73` | `#a1a1a6` |
| 文字三 | `#aeaeb2` | `#636366` |
| 边框 | `#e6e6ea` | `#2c2c2e` |

### 3.3 功能色（语义）

| 角色 | 色值 |
|---|---|
| 成功 success | `#34c759`（系统绿） |
| 警告 warning | `#ff9500`（系统橙） |
| 危险 danger | `#ff3b30`（系统红） |

---

## 4. 字体系统

```
主字体栈：-apple-system, "SF Pro Text", "PingFang SC", "Helvetica Neue", system-ui, sans-serif
等宽（日志/数据）：SF Mono, ui-monospace, monospace
```

遵循系统字体，不引入外部字体 —— 与 macOS 原生体验一致，零加载延迟。

### 字号层级

| 用途 | 字号 | 字重 |
|---|---|---|
| 页面标题 | 26px | 700 |
| 区块标题（card-title） | 13px | 600（大写 + letter-spacing） |
| 章节头（section-head） | 15px | 600 |
| 正文 | 14px | 400 |
| 辅助说明 | 13px | 400 |
| 微提示（field-hint） | 12px | 400 |

---

## 5. 图形语言

- **圆角**：`--radius: 14px`（卡片）、`--radius-sm: 10px`（控件/小卡片）、`999px`（胶囊/胶囊形 overlay）
- **控件高度**：`--control-h: 36px`（所有 input/select/button 统一）
- **阴影**：克制，`0 1px 2px` 近距 + `0 10px 30px` 远距双层
- **毛玻璃**：侧栏 `backdrop-filter: blur(30px) saturate(180%)`，不透明度 0.72
- **图标**：lucide-react，线条 `strokeWidth: 2`，与文字基线对齐

---

## 6. 设计原则

1. **克制**：无多余渐变、无发光、无装饰。留白即设计。
2. **精确**：几何对齐，间距统一（8 的倍数体系）。
3. **原生**：遵循 macOS HIG，使用系统字体、系统色彩逻辑、毛玻璃材质。
4. **工具感**：开发者/专业工具气质，而非消费级应用的活泼亲切。
5. **缩放性**：所有图形在小尺寸（16px）下必须可辨认。

---

## 7. 落地清单（已完成）

- [x] App icon：1024 squircle + macOS iconset（16~1024 全尺寸）+ `.icns`
- [x] 菜单栏 template icon：单色 3 竖条，`icon_as_template` 自动反色
- [x] 侧栏 brand-logo：内联 SVG 声波 logo（替换原渐变方块+字母 o）
- [x] 品牌主色统一：全 UI 强调色从苹果系统蓝 `#007aff` 改为粒蓝 `#3B4FE0`
- [x] Rust 托盘：加载 template icon + `icon_as_template(true)` + 左键点菜单
- [x] 构建验证：`pnpm build` + `pnpm test`（6/6 通过）+ `cargo check`
