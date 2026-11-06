# 中文桥 · Herdr ZH Bridge

使用中文与 Herdr 中的 LLM CLI 对话：独立 Codex 翻译任务把中文转为英文，发送到现有目标终端，再把本轮回复翻译为中文。每个标签独立绑定一个终端，支持并行会话、拖动分栏和模型 / Effort 选择。

A Chinese ↔ English translation companion for existing LLM CLI sessions in Herdr, with independent terminal tabs and drag-to-split conversations.

当前版本 **0.7（预览版）**。Windows 已完成本地自动化测试；macOS 共用 Python/Tk 核心并提供安装入口，尚未实机验证。本项目为独立社区项目。

## 安装前准备

- Python 3.10+，包含 Tk；运行 `python -m tkinter`（macOS 使用 `python3`）可检查是否能打开测试窗口。
- 已安装并登录的 Codex CLI，用作翻译 agent。
- 已运行的 Herdr，以及可调用的 `hc`。目标终端必须能报告 Herdr 的 idle / working 状态。
- 应用本身只使用 Python 标准库，无需额外 pip 依赖。

克隆仓库，或从 GitHub 的 **Code → Download ZIP** 下载并解压。下面的命令均在项目目录执行：

```sh
git clone https://github.com/soqt/herdr-zh-bridge.git
cd herdr-zh-bridge
```

### Windows

直接运行：

```powershell
python panel.py
```

也可双击 `launch.pyw`。需要安装为独立应用时运行 `python install_portable.py --launch`，代码保存到 `%APPDATA%/HerdrBridge/app`。

需要在 Herdr 内添加启动入口时，在 PowerShell 运行：

```powershell
.\install.ps1
```

安装器会先运行 Herdr mod 检查，再安装到 `mods/zh-bridge`。如果自动查找失败，可用 `-HerdrHome` 指定 Herdr 配置目录、用 `-HerdrExe` 指定程序路径。

## 0.7 界面

- 每个标签固定 184 × 36 逻辑单位，字号/DPI 缩放时统一调整；选中、悬停、等待和报错不会改变尺寸。
- 状态使用固定位置的指示点，完整状态与名称在悬停提示和会话菜单中查看。长名称省略，不挤压相邻标签。
- **+** 紧跟最后一个标签，点击后在该栏新建会话；设置和会话列表在标签栏右侧 **⋯** 菜单中。
- 标签过多时才显示左右滚动按钮，也可使用鼠标滚轮。拖动至标签栏边缘会自动滚动。标签内有关闭按钮；处理中关闭按钮不可用。
- 标签拖到右侧内容区，出现预览后松开即可分栏；拖入另一栏的标签栏即可合并，或在标签栏内拖动排序。Esc 或移到窗口外松开会取消。空分栏自动收起，窗口尺寸不因分栏改变。
- 切换内容使用固定容器，窗口和分栏尺寸不依赖当前标签的内容大小。
- 中性深灰、细边框和紧凑输入卡片；活动日志仍在每轮消息内默认折叠。
- 输入框上方按 **模型 → Effort → ↻** 靠左排列，菜单固定靠右；模型框最多 224 逻辑单位，窄分栏自动收缩，控件保持同高。模型显示简短名称，完整 ID 可悬停查看，同名模型仍用 ID 区分。

## 使用

1. 点击最后一个标签右侧的 **+**，按 Herdr 的机器 / 工作区 / 标签页层级选择目标终端。
2. 每个会话标签固定绑定一个目标。输入框上方选择翻译模型和 Effort，输入中文，点击 **发送** 或按 **Ctrl+Enter**。
3. 多个会话可以同时工作。切换标签不停止翻译；后台完成的标签显示 **新回复**，异常显示 **需处理**。
4. 有两个或更多标签时，把其中一个拖到右侧内容区，松开建立分栏。拖回另一侧标签栏会作为普通标签合并；拖动中间分隔线调整宽度。对话、草稿和正在运行的任务保留。
5. 每轮消息中的 **活动日志** 默认折叠，点击展开查看发送的英文、终端原文、提取正文及错误记录。完成后直接显示中文回复，不显示重复成功提示。

Effort 右侧的 **↻** 刷新模型列表；当前会话处理中禁用刷新。输入栏的 **⋯** 菜单提供选择终端、回译最新回答、仅回译输入、重试回译、结束跟踪。路径和字号在标签栏 **⋯ → 设置** 中统一配置，也可按 Ctrl+,（macOS 为 ⌘+,）。

选择已绑定的终端会跳转到已有标签。为已绑定的会话选择另一个终端会打开新标签，避免把两个终端的历史混在一起。同一工作区不允许重复绑定同一个终端。

标签栏 **⋯** 菜单可以定位任意标签。Windows 快捷键：Ctrl+Shift+T 新建，Ctrl+Tab 切换，Ctrl+Shift+W 关闭当前标签，Ctrl+Enter 发送。macOS 新建、关闭、发送使用 ⌘ 替代 Ctrl；Ctrl+Tab 切换会话。标签栏还支持左右方向键。

运行中的标签不能关闭；需要放弃待回译内容时，等当前操作结束后从 **··· → 结束跟踪** 释放。此操作不会停止原终端中的任务。某个标签失败不会阻止其他标签发送。

## 更新

关闭旧中文面板，下载新版源码或更新本地仓库。已安装 Herdr mod 入口的 Windows 用户在项目目录运行：

```powershell
.\update.ps1
```

脚本备份旧代码、复制 8 个模块并打开新版，Herdr 本身无需重启。已有配置会自动迁移。配置保存各标签的终端、模型、Effort 和分栏归属；不保存草稿、聊天内容或凭据。重启恢复绑定，不自动恢复发送或任务跟踪。

独立应用用户重新运行 `python install_portable.py --launch`；macOS 用户重新运行 `sh install-macos.command`。直接从源码启动的用户更新源码后重启即可。

## macOS 安装与运行

在 Mac 下载或克隆源码。安装 Python 3.10+（含 Tk；可使用 python.org 的安装包），并配置好 Codex 与 Herdr。在项目目录运行：

```sh
sh install-macos.command
```

安装器会创建 `~/Applications/HerdrBridge.app` 并打开。应用代码放在 `~/Library/Application Support/HerdrBridge/app`，以后可从 Finder 启动；再次安装会备份旧代码。这个 `.app` 是调用本机 Python 的启动包装，不包含 Python 运行时，也不是签名/公证过的独立发行版。

无需安装也可运行 `sh launch.command` 或 `python3 panel.py`。Windows 源码版可运行 `python panel.py` 或双击 `launch.pyw`。

两套系统共用翻译引擎与界面代码，仅字体、快捷键、配置路径和 CLI 查找属于平台适配。macOS 从 Finder 启动时会额外查找 `~/.local/bin`、`~/.cargo/bin`、`/opt/homebrew/bin`、`/usr/local/bin` 中的程序；仍可在设置中指定完整路径。

配置目录：Windows 为 `%APPDATA%/HerdrBridge`，macOS 为 `~/Library/Application Support/HerdrBridge`。首次运行会读取旧安装目录中的 `.preferences.json`，保留旧文件。可用 `HERDR_BRIDGE_HOME` 指定其他目录。

## 翻译与回复提取

中文输入 → 独立 Codex 翻译任务 B → hc send 粘贴英文并提交到现有终端 A → 等待本轮完成 → B 标记最终回答的原文行号 → 本地核对问题及位置 → B 将正文翻译为中文。

- A 一直是现有会话，不启动新 A，也不通过 A 的启动参数传递问题。B 使用 codex exec 和本机 Codex 登录配置。
- 模型目录通过 Codex app-server 的 model/list 查询。Effort 使用所选模型返回的 supportedReasoningEfforts；未读取目录时提供常用选项，由本次 Codex 请求确认可用性。“默认”不覆盖已有 Codex 设置。
- 模型和 Effort 按会话独立设置，用于中译英、回答提取、英译中。正在运行的任务不会被另一个标签的模型刷新改动。
- 发送成功后清空对应标签的输入框。失败、结果未知或发送期间修改过草稿时保留内容。
- 只回译经过位置校验的回答正文。无法确认本轮关系时暂停，不退回整屏翻译；可从原终端复制回答到输入框，选择“仅回译输入”。

## 当前边界

- 自动发送要求目标报告 idle 且输入框为空；无状态 CLI 尚不支持自动转发。
- 回答识别仍依赖终端文本及模型判断，未直接订阅 Codex/Claude/Grok 的结构化会话事件。原文位置校验不是语义正确性的证明，问题滚出或截断时可能要求手动回译。
- 并行会话会各自调用 Codex，时间和额度使用随任务数量增加；每轮通常包含两次翻译和一次提取。
- 同一目标的去重范围是当前工作区；请勿用多个独立中文桥进程同时控制同一目标。
- 当前是独立的 Python/Tk 窗口，Herdr mod 提供启动入口。支持原生高 DPI 字体绘制；真实桌面截图、跨显示器移动和中文输入法仍需用户环境验证。
- 附件和图片未实现。关闭整个窗口会停止后续跟踪，已经发送到原终端的任务不会撤回。

## 文件与验证

- panel.py：标签、分栏、共享设置及工作区配置。
- tab_group.py：固定尺寸标签、拖动、行内新建、状态指示、滚动和关闭。
- platform_support.py / install_portable.py：平台边界和独立应用安装。
- conversation.py：单个终端的对话、队列、翻译及跟踪。
- bridge.py / replies.py：转发、身份校验、翻译和回答提取。
- catalog.py / widgets.py：真实目录、模型目录、高 DPI、终端选择器和折叠日志。
- mod/：Herdr 原生侧栏启动入口。
- install.ps1：验证后首次安装；update.ps1：备份并更新。

```powershell
python -m unittest -v
python panel.py --smoke-test
```

macOS 使用 `python3`。包含界面的测试需要可用桌面。`python preview.py` 可以查看不会调用 CLI 的演示界面；`python package_release.py` 生成包含 MIT License 的源码压缩包。

测试覆盖并发发送和反向完成、回复及草稿隔离、同目标去重、鼠标事件驱动的分栏与合并、标签排序、取消拖动、溢出滚动、空分栏收起、模型刷新和隔离、高倍率缩放布局、日志折叠、配置迁移、发送失败恢复。模拟协议测试不替代真实多终端验收。

## License

[MIT License](LICENSE) · Copyright (c) 2026 Yvmeng Wang。

参考：
- https://herdr.dev/core/
- https://github.com/herdrdev/herdr-core/blob/master/docs/commands.md
- https://github.com/herdrdev/herdr-app/blob/master/defaults/AGENTS.md
- https://learn.chatgpt.com/docs/app-server
- https://learn.chatgpt.com/docs/non-interactive-mode
- https://learn.chatgpt.com/docs/developer-settings
- https://docs.python.org/3/library/tkinter.html
- https://www.python.org/download/mac/tcltk/
