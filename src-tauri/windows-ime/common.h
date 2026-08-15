// 共通声明：协议常量、JSON 编解码、管道名。
// 协议 = JSONL（一行一条，UTF-8 无 BOM），字段驼峰，与 Rust 侧 protocol.rs 的
// serde 配方（tag=type + camelCase）互为黄金镜像（fixtures 由双方共用）。
#pragma once
#include <windows.h>
#include <string>

namespace openime {

constexpr uint32_t kProtocolVersion = 1;
// 与 Rust 侧 NFR-11.4 一致：单次提交文本上限 64 KiB（UTF-8 字节数）。
constexpr size_t kMaxTextBytes = 65536;
// 宿主等待 SubmitResult 的兜底（与宿主侧 submit 超时对齐）。
constexpr uint32_t kCommitTimeoutMs = 1000;
// 管道前缀；完整名 = 前缀-{pid}-{tid}（tid = TIP 激活线程）。
constexpr wchar_t kPipePrefix[] = L"\\\\.pipe\\OpenImeCommit";

inline std::wstring PipeNameForCurrentThread() {
  wchar_t name[96];
  swprintf_s(name, L"%s-%lu-%lu", kPipePrefix,
             static_cast<unsigned long>(GetCurrentProcessId()),
             static_cast<unsigned long>(GetCurrentThreadId()));
  return name;
}

// ── 最小 JSON 工具（NFR-11.7：手写，无第三方库）──

// 序列化转义：", \, 控制字符；非 ASCII UTF-8 原样直出（Rust serde 默认不转义非 ASCII，
// 双方 parser 均按 UTF-8 处理）。
inline std::string JsonEscape(const std::string& s) {
  std::string out;
  out.reserve(s.size() + 8);
  for (unsigned char c : s) {
    switch (c) {
      case '"': out += "\\\""; break;
      case '\\': out += "\\\\"; break;
      case '\n': out += "\\n"; break;
      case '\r': out += "\\r"; break;
      case '\t': out += "\\t"; break;
      default:
        if (c < 0x20) {
          char buf[8];
          sprintf_s(buf, "\\u%04x", c);
          out += buf;
        } else {
          out += static_cast<char>(c);
        }
    }
  }
  return out;
}

// 在扁平 JSON 对象里找 "key":"value"（顶层、字符串值）。支持 \" \\ \n \r \t \uXXXX。
// 消息结构固定为单层，这里刻意不做通用递归解析。
inline bool JsonFindString(const std::string& json, const char* key, std::string& out) {
  std::string pat = "\"";
  pat += key;
  pat += "\"";
  size_t k = json.find(pat);
  if (k == std::string::npos) return false;
  k = json.find(':', k + pat.size());
  if (k == std::string::npos) return false;
  ++k;
  while (k < json.size() && (json[k] == ' ' || json[k] == '\t')) ++k;
  if (k >= json.size() || json[k] != '"') return false;
  ++k;
  out.clear();
  while (k < json.size() && json[k] != '"') {
    char c = json[k];
    if (c == '\\' && k + 1 < json.size()) {
      char e = json[k + 1];
      k += 2;
      switch (e) {
        case '"': out += '"'; break;
        case '\\': out += '\\'; break;
        case '/': out += '/'; break;
        case 'n': out += '\n'; break;
        case 'r': out += '\r'; break;
        case 't': out += '\t'; break;
        case 'b': out += '\b'; break;
        case 'f': out += '\f'; break;
        case 'u': {
          if (k + 4 > json.size()) return false;
          unsigned cp = 0;
          for (int i = 0; i < 4; ++i) {
            char h = json[k + i];
            cp <<= 4;
            if (h >= '0' && h <= '9') cp |= static_cast<unsigned>(h - '0');
            else if (h >= 'a' && h <= 'f') cp |= static_cast<unsigned>(h - 'a' + 10);
            else if (h >= 'A' && h <= 'F') cp |= static_cast<unsigned>(h - 'A' + 10);
            else return false;
          }
          k += 4;
          // UTF-16 码位 → UTF-8（含代理对）。
          if (cp >= 0xD800 && cp <= 0xDBFF && k + 6 <= json.size() &&
              json[k] == '\\' && json[k + 1] == 'u') {
            unsigned lo = 0;
            bool ok = true;
            for (int i = 0; i < 4; ++i) {
              char h = json[k + 2 + i];
              lo <<= 4;
              if (h >= '0' && h <= '9') lo |= static_cast<unsigned>(h - '0');
              else if (h >= 'a' && h <= 'f') lo |= static_cast<unsigned>(h - 'a' + 10);
              else if (h >= 'A' && h <= 'F') lo |= static_cast<unsigned>(h - 'A' + 10);
              else { ok = false; break; }
            }
            if (ok && lo >= 0xDC00 && lo <= 0xDFFF) {
              k += 6;
              cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
            }
          }
          if (cp < 0x80) {
            out += static_cast<char>(cp);
          } else if (cp < 0x800) {
            out += static_cast<char>(0xC0 | (cp >> 6));
            out += static_cast<char>(0x80 | (cp & 0x3F));
          } else if (cp < 0x10000) {
            out += static_cast<char>(0xE0 | (cp >> 12));
            out += static_cast<char>(0x80 | ((cp >> 6) & 0x3F));
            out += static_cast<char>(0x80 | (cp & 0x3F));
          } else {
            out += static_cast<char>(0xF0 | (cp >> 18));
            out += static_cast<char>(0x80 | ((cp >> 12) & 0x3F));
            out += static_cast<char>(0x80 | ((cp >> 6) & 0x3F));
            out += static_cast<char>(0x80 | (cp & 0x3F));
          }
          break;
        }
        default: return false;  // 未知转义 → 视为坏帧
      }
    } else {
      out += c;
      ++k;
    }
  }
  return k < json.size();  // 找到收尾引号
}

// 在扁平 JSON 对象里找 "key":<number>（无符号整数）。
inline bool JsonFindNumber(const std::string& json, const char* key, uint32_t& out) {
  std::string pat = "\"";
  pat += key;
  pat += "\"";
  size_t k = json.find(pat);
  if (k == std::string::npos) return false;
  k = json.find(':', k + pat.size());
  if (k == std::string::npos) return false;
  ++k;
  while (k < json.size() && (json[k] == ' ' || json[k] == '\t')) ++k;
  if (k >= json.size() || json[k] < '0' || json[k] > '9') return false;
  uint32_t v = 0;
  while (k < json.size() && json[k] >= '0' && json[k] <= '9') {
    v = v * 10 + static_cast<uint32_t>(json[k] - '0');
    ++k;
  }
  out = v;
  return true;
}

// UTF-8 → UTF-16（SetText 用）。非法序列按 U+FFFD 替换，不失败。
inline std::wstring Utf8ToWide(const std::string& s) {
  if (s.empty()) return std::wstring();
  int n = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, s.data(),
                              static_cast<int>(s.size()), nullptr, 0);
  if (n <= 0) {
    n = MultiByteToWideChar(CP_UTF8, 0, s.data(), static_cast<int>(s.size()),
                            nullptr, 0);
  }
  std::wstring w(static_cast<size_t>(n), L'\0');
  MultiByteToWideChar(CP_UTF8, 0, s.data(), static_cast<int>(s.size()), w.data(), n);
  return w;
}

}  // namespace openime
