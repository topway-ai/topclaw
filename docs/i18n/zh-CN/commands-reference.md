# 命令参考（简体中文）

这是 Wave 1 首版本地化页面，用于快速定位 TopClaw CLI 命令。

英文原文：

- [../../commands-reference.md](../../commands-reference.md)

## 适用场景

- 按任务查命令（onboard / status / doctor / channel 等）
- 对比命令参数与行为边界
- 排查命令执行异常时确认预期输出

## 使用建议

- 命令名、参数名、配置键保持英文。
- 行为细节以英文原文为准。

## 最近更新

- `topclaw gateway` 新增 `--new-pairing` 参数，可清空已配对 token 并在网关启动时生成新的配对码。
- `topclaw update` 现在明确给出了安全更新路径：先执行 `topclaw update --check`，再执行 `topclaw update`，如果以后台服务运行则再执行 `topclaw service restart`。
- 另外补充了更易懂的别名入口：`topclaw init`、`chat`、`run`、`info`、`channels`、`skill`。
