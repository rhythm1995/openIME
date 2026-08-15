// i18n 守护：两个 locale 的 key 集合必须一致，防止新增 key 只写一侧
//（英文用户会看到 key 原文渲染出来）。
import { describe, expect, it } from "vitest";
import en from "./locales/en.json";
import zh from "./locales/zh.json";

type Json = string | number | boolean | null | Json[] | { [k: string]: Json };

function flatten(obj: Json, prefix = ""): string[] {
  if (Array.isArray(obj)) {
    return obj.flatMap((v) => flatten(v, `${prefix}[]`));
  }
  if (obj !== null && typeof obj === "object") {
    return Object.entries(obj).flatMap(([k, v]) => flatten(v, prefix ? `${prefix}.${k}` : k));
  }
  return [prefix];
}

describe("i18n locales", () => {
  it("en 与 zh 的 key 集合完全一致", () => {
    const enKeys = new Set(flatten(en as Json));
    const zhKeys = new Set(flatten(zh as Json));
    const onlyEn = [...enKeys].filter((k) => !zhKeys.has(k));
    const onlyZh = [...zhKeys].filter((k) => !enKeys.has(k));
    expect(onlyEn, "仅 en 存在（zh 缺失或 en 残留死键）").toEqual([]);
    expect(onlyZh, "仅 zh 存在（en 缺失）").toEqual([]);
  });

  it("Settings 静态回退清单引用的 ASR 模型 key 在两个 locale 都存在", () => {
    // JSON 字面量类型无索引签名，断言前转 Record。
    const enModels = en.settings.localAsr.models as Record<
      string,
      { title: string; desc: string } | undefined
    >;
    const zhModels = zh.settings.localAsr.models as Record<
      string,
      { title: string; desc: string } | undefined
    >;
    for (const id of ["sensevoice", "funasr_nano_int8", "funasr_nano_fp16"]) {
      expect(enModels[id], `en 缺 ${id}`).toBeTruthy();
      expect(zhModels[id], `zh 缺 ${id}`).toBeTruthy();
    }
  });
});
