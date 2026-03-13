# 本地化桥接文档：One Click Bootstrap

这是增强型 bridge 页面。它提供该主题的定位、原文章节导览和执行提示，帮助你在不丢失英文规范语义的情况下快速落地。

英文原文:

- [../../one-click-bootstrap.md](../../one-click-bootstrap.md)

## 主题定位

- 类别：运行与接入
- 深度：增强 bridge（章节导览 + 执行提示）
- 适用：先理解结构，再按英文规范逐条执行。

## 原文导览

- 直接以英文原文中的实际标题为准进行导航。
- 如果桥接页中的中文说明与英文标题结构不一致，请优先阅读英文原文。

## 操作建议

- 对于已有安装，推荐的安全更新顺序是：先运行 `topclaw update --check`，再运行 `topclaw update`，如果 TopClaw 以后台服务方式运行，再执行 `topclaw service restart`。
- 托管 one-line installer 现在会先尝试最新兼容 release 二进制，只有在需要回退到源码构建时才会 clone 仓库；验证本地代码改动时，请在 checkout 中使用 `./bootstrap.sh --force-source-build`。
- 先通读原文目录，再聚焦与你当前变更直接相关的小节。
- 命令名、配置键、API 路径和代码标识保持英文。
- 发生语义歧义或行为冲突时，以英文原文为准。

## 相关入口

- [README.md](README.md)
- [SUMMARY.md](SUMMARY.md)
- [docs-inventory.md](docs-inventory.md)
