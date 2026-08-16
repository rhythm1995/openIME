import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// base 对应 GitHub Pages 项目页路径；换自定义域名时改为 "/"。
export default defineConfig({
  plugins: [react()],
  base: "/openIME/",
});
