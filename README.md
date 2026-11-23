# TokenPlus 小助手 · macOS 安装

打开「终端」，粘贴下面这一行并回车：

```bash
curl -fsSL https://raw.githubusercontent.com/pengcai6/tokenplus-helper-release/main/install.sh | bash
```

脚本会自动下载最新版、安装到「应用程序」并打开，不会出现「无法验证开发者」或「已损坏」的提示。重新运行同一行命令即可升级。

目前只支持 Apple 芯片（M 系列）的 Mac。

## 手动安装

也可以在 [Releases](https://github.com/pengcai6/tokenplus-helper-release/releases/latest) 下载 DMG，把 App 拖进「应用程序」。
首次打开如果被拦，在「终端」运行：

```bash
xattr -dr com.apple.quarantine "/Applications/TokenPlus 小助手.app"
```
