# 中文桥 · Herdr ZH Bridge

使用中文与 Herdr 中的 LLM CLI 对话：独立 Codex 翻译任务把中文转为英文，发送到现有目标终端，再把本轮回复翻译为中文。左侧自动同步 Herdr 的机器与空间结构，每个空间显示其中终端的会话，可在同一屏幕左右、上下分栏排列；每个标签独立绑定一个终端，支持并行会话和模型 / Effort 选择。

A Chinese ↔ English translation companion for existing LLM CLI sessions in Herdr, with a sidebar that mirrors Herdr's machines and spaces, independent terminal tabs, and drag-to-split panes in both directions. Native app written in Rust with [GPUI](https://gpui.rs) and [gpui-component](https://github.com/longbridge/gpui-kit), for macOS and Windows.

当前版本 **0.8（预览版）**：由 Python/Tk 重写为 Rust + GPUI 原生应用，单一可执行文件，不再需要 Python。macOS 已完成本地构建、自动化测试、离屏界面渲染，以及真实 `hc` / Herdr 布局 / Codex 模型目录的只读联调；Windows 代码可通过交叉类型检查，尚未实机验证。本项目为独立社区项目。Python 0.7 版本保留在 git 历史中。

## 安装前准备

- 已安装并登录的 Codex CLI，用作翻译 agent。
- 已运行的 Herdr，以及可调用的 `hc`。目标终端不必报告 Herdr 状态。
- 从源码构建时需要 [Rust](https://rustup.rs)。仓库的 `rust-toolchain.toml` 固定 Rust 1.99.1，rustup 会自动安装，不影响系统默认版本。
  - macOS：Xcode 或 Command Line Tools。
  - Windows：Visual Studio Build Tools（“使用 C++ 的桌面开发”，含 MSVC 和 Windows SDK）。发行版构建会用 Windows SDK 中的 `fxc.exe` 编译着色器；找不到时可用 `GPUI_FXC_PATH` 指定。

克隆仓库，或从 GitHub 的 **Code → Download ZIP** 下载并解压。下面的命令均在项目目录执行：

```sh
git clone https://github.com/soqt/herdr-zh-bridge.git
cd herdr-zh-bridge
cargo run --release
```

`cargo run --release -- --preview` 打开不会调用任何 CLI 的演示界面；`--target <终端地址>` 打开或定位绑定该终端的标签。

### macOS

```sh
sh install-macos.command
```

安装器会使用目录中的发行版可执行文件，或在源码目录中先运行 `cargo build --release`，再创建 `~/Applications/HerdrBridge.app` 并打开。检测到 Herdr 配置目录时，会先运行 `herdr-app check`，通过后把启动入口安装到 `~/.config/herdr-app/mods/zh-bridge`。再次运行即为更新；旧应用和旧入口移动到 `~/Library/Application Support/HerdrBridge/backups`。不会覆盖无关的同名应用。这个 `.app` 只做了本机临时签名，不是公证过的发行版。

### Windows

在 PowerShell 运行：

```powershell
.\install.ps1
```

安装器会使用目录中的 `herdr-zh-bridge.exe`，或在源码目录中先运行 `cargo build --release`。随后运行 Herdr mod 检查，通过后把程序和启动入口安装到 `mods/zh-bridge`，并添加开始菜单项。如果自动查找失败，可用 `-HerdrHome` 指定 Herdr 配置目录、`-HerdrExe` 指定 `herdr-app.exe`、`-Binary` 指定程序路径。

更新已安装的版本：

```powershell
.\update.ps1
```

脚本先验证新程序能启动，再把旧程序（以及 0.7 的 Python 模块）移入备份目录、复制新版并打开，Herdr 本身无需重启。旧窗口仍开着时也可以更新。

## 0.8 界面

- macOS 上窗口没有标题栏：红绿灯按钮位于左侧栏顶部，旁边是应用名称和收起按钮。拖动侧栏顶部或标签栏空白处可移动窗口，双击可缩放。Windows 保留系统标题栏。
- 左侧空间列表自动同步 Herdr 窗口（`herdr-app ls`，启动时读取，之后约每 4 秒刷新）：按机器分组（本机、各远程机器），组内顺序、名称与 Herdr 一致，在 Herdr 中新建、重命名或关闭空间后自动更新。点击机器标题可折叠该组，折叠状态会保存。不在 Herdr 窗口中的 hc 终端归入“其他”。
- 每个会话固定位于其终端所在的 Herdr 空间；终端在 Herdr 中移到别的空间时，会话随之移动。空间行右侧的 **+** 在该空间新建会话，只列出该空间的终端；最右侧的状态点汇总该空间会话的状态（需处理 > 处理中 > 等待 > 新回复 > 离线）。没有会话的空间会直接列出其中的终端，点一下即可开始会话。
- ⌘1–⌘9 / Ctrl+1–9 切换到列表中的第 1–9 个空间。右键空间行可新建会话或关闭该空间的全部会话。Herdr 中已关闭的空间若仍有会话，会以灰色保留，直到会话全部关闭。Herdr 窗口暂时无法读取时，列表保持上次的状态，侧栏底部显示提示。
- 侧栏顶部的面板按钮或 ⌘B / Ctrl+B 可把列表收起为窄栏，只显示首字和状态点，给分栏留出更多宽度；收起状态会保存。切换空间会回到该空间上次聚焦的会话；后台空间里的发送和跟踪照常进行。
- **+** 紧跟最后一个标签，点击后在该栏新建会话；当前空间的会话列表、分栏和设置在标签栏右侧 **⋯** 菜单中，也可在标签栏点右键。空间列表底部也有 **设置**。
- 标签过多时才显示左右滚动按钮，也可使用鼠标滚轮。拖动至标签栏边缘会自动滚动。标签内有关闭按钮；发送中或仍在等待回复时关闭按钮不可用。
- 同一空间内可以左右、上下任意分栏，在一个屏幕上同时显示多个终端会话。把标签拖到某个分栏内容区的左、右、上、下边缘，出现预览后松开即在该侧拆分；拖到中间则移入该分栏，拖到标签栏可合并或排序。分栏太小（宽不足 2 × 300、高不足 2 × 260 逻辑单位）时不会再拆分。Esc 或在窗口外松开会取消。空分栏自动收起，窗口尺寸不因分栏改变。拖动分隔线调整大小，双击恢复等分。
- 标签栏 **⋯** 菜单可“向右分栏新建”“向下分栏新建”；标签右键菜单可向右 / 向下拆分或关闭会话。
- 中性深灰、细边框和紧凑输入卡片。按钮、菜单项、标签和列表行都有清晰的悬停和按下状态。
- 活动日志在每轮消息内默认展开，高度固定；新记录在框内滚动并始终显示滚动条，不会撑高对话。停在末尾时自动跟随最新记录，向上翻看时保持不动。滚轮先滚动日志，到顶或到底后再滚动对话。
- 终端快照等超过 80 行的记录只显示标题和最后 80 行，并注明省略的行数；完整历史仍在原终端中。对话和日志都可以选择复制，长文本分段排版，拖选和长对话都保持流畅。
- 输入框上方按 **模型 → Effort → ↻** 靠左排列，菜单固定靠右，控件同高；模型框最多 224 逻辑单位，窄分栏自动收缩。模型可从列表选择或直接输入 ID，完整 ID 可悬停查看，同名模型仍用 ID 区分。
- 输入框支持系统中文输入法；输入法组字时按回车只确认候选，不会发送。

## 使用

1. 点击左侧某个空间右侧的 **+**（或在空白空间中直接点击终端），选择该空间中的目标终端。
2. 每个会话标签固定绑定一个目标。输入框上方选择翻译模型和 Effort，输入中文，按 **回车** 或点击 **发送**；**⇧↩**（Windows 为 **Shift+Enter**）换行。输入框立即清空，可以接着输入下一条，不必等终端回复。
3. 每个会话有独立的发送和跟踪两个后台任务：上一条还在等待回复时可以继续发送，新消息按顺序送达，回复按顺序回译到各自那一轮的日志中；跟踪轮询不会推迟发送。多个会话可以同时工作。切换标签不停止翻译；后台完成的标签显示 **新回复**，异常显示 **需处理**。
4. 把标签拖到内容区边缘建立分栏，可反复拆分成多列、多行；拖回另一栏的标签栏会作为普通标签合并。对话、草稿和正在运行的任务都保留。
5. 每轮消息中的 **活动日志** 默认展开，显示发送的英文、终端原文（最后 80 行）、提取正文及错误记录；点击标题可折叠，再次展开时回到最新记录。完成后直接显示中文回复，不显示重复成功提示。

Effort 右侧的 **↻** 刷新模型列表，不受正在发送或跟踪的任务影响。模型、Effort 和设置中的路径随时可改，已排队的任务沿用排队时的设置。输入栏的 **⋯** 菜单提供选择终端（重新读取终端列表）、回译最新回答、仅回译输入、重试回译、结束跟踪。路径和字号在标签栏 **⋯ → 设置** 中统一配置，也可按 ⌘,（Windows 为 Ctrl+,）。

选择已绑定的终端会跳转到已有标签（必要时切换空间）。为已绑定的会话选择另一个终端会在同一分栏打开新标签，避免把两个终端的历史混在一起。同一窗口的所有空间中，一个终端只能绑定一个标签。

快捷键：⌘⇧T / Ctrl+Shift+T 新建会话，Ctrl+Tab 在当前空间内切换，⌘⇧W / Ctrl+Shift+W 关闭当前标签，⌘1–⌘9 / Ctrl+1–9 切换空间，⌘B / Ctrl+B 收起或展开空间栏，回车（或 ⌘↩ / Ctrl+Enter）发送，⇧↩ / Shift+Enter 换行，Esc 取消拖动。macOS 菜单栏的“会话”菜单提供会话和空间栏操作。

发送中或仍有待回译内容的标签不能关闭，含有这类标签的空间也不能“关闭全部会话”；需要放弃时从 **⋯ → 结束跟踪** 释放全部待回译的轮次，此操作不会停止原终端中的任务。关闭空间中最后一个会话后，该空间改为列出其中的终端。某个标签失败不会阻止其他标签发送；某条消息翻译失败也不会暂停同一标签的回复跟踪。

## 配置与升级

配置目录：Windows 为 `%APPDATA%/HerdrBridge`，macOS 为 `~/Library/Application Support/HerdrBridge`。可用 `HERDR_BRIDGE_HOME` 指定其他目录。

配置文件为 `preferences.json`，版本 9，保存上次看到的 Herdr 空间（启动时立即显示，读到 Herdr 后更新）、当前空间、折叠的机器、空间栏是否收起、每个空间的分栏树与比例，以及各标签的终端、模型、Effort 和字号。版本 7 和版本 8（早期 0.8 自建空间）的文件会合并为一个空间，读到 Herdr 布局后各会话自动移到其终端所在的空间，未绑定终端的空白会话不再保留；新版本仍会写入版本 7 的标签列表，换回 0.7 时终端绑定不会丢失（空间和分栏除外）。不保存草稿、聊天内容或凭据。重启恢复绑定，不自动恢复发送或任务跟踪。没有配置文件时，会读取程序旁边旧版的 `.preferences.json`，保留旧文件。

从 Finder 或开始菜单启动时，除了 PATH，还会在 `~/.local/bin`、`~/.cargo/bin`、`/opt/homebrew/bin`、`/usr/local/bin`（Windows 为 `%LOCALAPPDATA%/Programs/herdr-app`、`%APPDATA%/npm` 等）中查找 `hc`、`codex` 和 `herdr-app`，并把这些目录加入子进程的 PATH；仍可在设置中指定完整路径。

## 翻译与回复提取

中文输入 → 独立 Codex 翻译任务 B → hc send 粘贴英文并提交到现有终端 A → 等待本轮完成 → B 标记最终回答的原文行号 → 本地核对问题及位置 → B 将正文翻译为中文。

- A 一直是现有会话，不启动新 A，也不通过 A 的启动参数传递问题。B 使用 `codex exec --ephemeral --sandbox read-only` 和本机 Codex 登录配置；所有参数直接传给程序，不经过 shell。
- 模型目录通过 Codex app-server 的 model/list 查询。Effort 使用所选模型返回的 supportedReasoningEfforts；未读取目录时提供常用选项，由本次 Codex 请求确认可用性。“默认”不覆盖已有 Codex 设置。
- 模型和 Effort 按会话独立设置，用于中译英、回答提取、英译中。每个任务使用排队时的模型，之后的修改或模型刷新只影响后续任务。
- 发送时立即清空输入框。翻译或发送前的检查失败时，若输入框仍为空，原文会放回输入框；发送结果未知时不放回，以免重复发送，该轮保留在跟踪队列中，需要“重试回译”或“结束跟踪”。
- 只回译经过位置校验的回答正文。无法确认本轮关系时暂停，不退回整屏翻译；可从原终端复制回答到输入框，选择“仅回译输入”。

## 当前边界

- 发送不检查目标状态。目标报告 Herdr 状态（OSC 7501）时按 idle / done 判断本轮完成；不报告时，以可见屏幕连续三次读取不变视为完成，回答中途停顿较久时可能提前回译，可用 ⋯ 菜单中的“回译最新回答”补译。中文桥自己所在的终端不会出现在终端列表中，也不能作为目标。
- 上一轮未完成时发送的消息如何处理（排队或打断）取决于目标程序本身。不报告状态的目标连续处理多条消息时，屏幕要到全部完成后才稳定，各轮回复届时依次回译。
- 回答识别仍依赖终端文本及模型判断，未直接订阅 Codex/Claude/Grok 的结构化会话事件。原文位置校验不是语义正确性的证明，问题滚出或截断时可能要求手动回译。
- 并行会话会各自调用 Codex，时间和额度使用随任务数量增加；每轮通常包含两次翻译和一次提取。
- 同一目标的去重范围是当前窗口；请勿用多个独立中文桥进程同时控制同一目标。
- Windows 版本尚需实机验证：构建、高 DPI、跨显示器移动、中文输入法和安装脚本。macOS 上真实中文输入法组字也需在桌面环境中确认。
- 附件和图片未实现。关闭整个窗口会停止后续跟踪，已经发送到原终端的任务不会撤回。

## 代码结构与验证

- `src/core/`：与界面无关的引擎。`herdr.rs`（hc 适配与身份校验）、`session.rs`（发送、等待、提取、回译；发送与跟踪共享的待回复队列）、`replies.rs`（回答位置校验）、`translator.rs`（Codex 翻译任务）、`catalog.rs`（终端层级与模型目录）、`prefs.rs`、`platform.rs`、`process.rs`。
- `src/ui/`：GPUI 界面。`workspace.rs`（同步 Herdr 的空间列表、标签、分栏、拖动、窗口拖动区域、快捷键、配置保存）、`conversation.rs`（单个终端的对话、日志、输入卡片，以及发送、跟踪两个后台任务）、`picker.rs`（终端选择器）、`settings.rs`、`layout.rs` 与 `models.rs`（可单测的空间分栏树与模型状态）、`theme.rs`。
- `mod/`：Herdr 原生侧栏启动入口，启动同目录中的程序。
- `install-macos.command`、`install.ps1`、`update.ps1`：安装与更新；`test-installer.ps1` 检查 Windows 脚本。

```sh
cargo test                       # 引擎、分栏与界面测试（无界面测试平台）
cargo run -- --smoke-test        # 构建完整工作区后退出
cargo run --example screenshot   # macOS：离屏渲染各个界面状态到 /tmp/herdr-zh-bridge-shots
```

测试覆盖：发送不受目标状态限制、无状态目标的屏幕稳定判断、结果未知时禁止重发、换程序检测、本轮回答校验、模型与 Effort 参数传递、模型目录分页协议、终端层级、配置迁移与隐私；界面测试在真实组件上模拟鼠标拖动分栏（左右与上下）、合并、排序和取消，拖动分隔线与双击复位，分栏空间不足时拒绝拆分，空间切换与焦点记忆、按 Herdr 布局归入空间并随终端移动、空白空间列出终端与空间 + 只提供本空间终端、机器折叠与 Herdr 改名、Herdr 关闭空间后保留至会话关闭、空间栏收起、忙碌空间禁止关闭全部会话、v7 / v8 配置迁移，以及并发发送和反向完成、回复及草稿隔离、同目标去重、离线终端、运行中禁止关闭。模拟协议测试不替代真实多终端验收。

## License

[MIT License](LICENSE) · Copyright (c) 2026 Yvmeng Wang。

参考：
- https://herdr.dev/core/
- https://github.com/herdrdev/herdr-core/blob/master/docs/commands.md
- https://github.com/herdrdev/herdr-app/blob/master/defaults/AGENTS.md
- https://learn.chatgpt.com/docs/app-server
- https://learn.chatgpt.com/docs/non-interactive-mode
- https://learn.chatgpt.com/docs/developer-settings
- https://gpui.rs
- https://github.com/longbridge/gpui-kit
