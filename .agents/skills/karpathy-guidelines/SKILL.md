---
name: karpathy-guidelines
description: 编码前思考、简单优先、手术式修改、可验证完成。与 .agents/rules/karpathy-guidelines.mdc 同源。
---

# Karpathy Guidelines

实现 agents-manager 代码时遵守以下四条（详见 `.agents/rules/karpathy-guidelines.mdc`）：

1. **Think Before Coding** — 假设写清；多方案说明取舍；不懂就问。
2. **Simplicity First** — 不做超范围功能；不为单次使用抽象。
3. **Surgical Changes** — 只改任务相关行；匹配现有风格。
4. **Goal-Driven Execution** — 每步有验证；`cargo test` / `npm run build` 通过再声称完成。
