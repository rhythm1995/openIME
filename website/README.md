# openIME 官网（landing page）

深色声学交互风的单页官网，部署于 GitHub Pages：<https://rhythm1995.github.io/openIME/>

## 开发

```bash
pnpm install        # 在仓库根目录（pnpm workspace）
pnpm --filter openime-website dev     # http://localhost:5173/openIME/
pnpm --filter openime-website build   # 产物在 website/dist/
```

## 约定

- 设计 token 来自 `branding/VI.md`（深色模式），角色纪律：粒蓝 `#5C6AFF` 仅用于 CTA/链接/声波；SF Mono 仅用于规格与快捷键。
- 文案在 `src/i18n/locales/{zh,en}.json`，默认中文；切换持久化在 `localStorage("openime.site_lang")`。
- `vite.config.ts` 的 `base` 对应 GitHub Pages 项目页路径，换自定义域名时改为 `"/"`。
- 部署：`.github/workflows/pages.yml`（push main 且 `website/**` 变更时构建并发布）。
