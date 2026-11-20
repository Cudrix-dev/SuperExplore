# SuperExplore

SuperExplore 是 Cudrix 开发的 Windows NTFS 文件搜索工具。程序通过 Win32 枚举 MFT，使用 USN 日志更新内存中的索引，并在搜索命中后解析路径和读取文件属性。

## 使用

- 接受 UAC 提示后，应用展示 2 秒的品牌动画。若初次索引仍在运行，窗口会显示索引状态，准备完成后打开主界面。
- 使用 <kbd>Alt</kbd> + <kbd>Space</kbd> 呼出或隐藏主窗口。若组合键已被占用，可通过系统托盘菜单打开应用。
- 输入文件名关键词。空格分隔的关键词必须同时匹配，大小写不影响结果。界面最多显示 100 条结果。
- 使用 <kbd>↑</kbd> / <kbd>↓</kbd> 选择结果、<kbd>Enter</kbd> 打开、<kbd>Ctrl</kbd> + <kbd>Enter</kbd> 打开所在目录。右键菜单也支持复制完整路径。
- 通过系统托盘菜单中的「退出 SuperExplore」结束索引进程。

## Windows 本地构建

建议使用 Windows 11 x64、Node.js 24 LTS、当前稳定版 MSVC Rust，以及带有「使用 C++ 的桌面开发」工作负载和 Windows SDK 的 Visual Studio Build Tools。MSI 打包需要 Windows 的 VBScript 可选功能。若本机尚未缓存 WiX、NSIS 或依赖，构建时需要访问其下载站点。 [Tauri 构建依赖](https://v2.tauri.app/start/prerequisites/) · [Windows 安装包](https://v2.tauri.app/distribute/windows-installer/)

在 PowerShell 中确认工具链和 WebView2 Runtime。若 Rust 刚安装，请重新打开终端，以便加载 PATH。

~~~powershell
node --version
npm --version
rustc --version
rustup show active-toolchain

$edgeClients = `(
  'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients',
  'HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients',
  'HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients'
)
Get-ChildItem $edgeClients -ErrorAction SilentlyContinue |
  ForEach-Object { Get-ItemProperty $_.PSPath } |
  Where-Object name -like '*WebView2*' |
  Select-Object name, pv
~~~

若查询未显示有效版本，请安装微软的 Evergreen WebView2 Runtime。安装包可在目标机器缺少它时下载引导程序，便携版依赖目标机器已有运行时。 [微软 WebView2 Runtime 下载](https://developer.microsoft.com/microsoft-edge/webview2/#download-section)

~~~powershell
npm ci
npm run tauri build
~~~

| 产物 | 本地路径 |
| --- | --- |
| 便携 EXE | `src-tauri/target/release/superexplore.exe` |
| NSIS 安装程序 | `src-tauri/target/release/bundle/nsis/*-setup.exe` |
| MSI 安装包 | `src-tauri/target/release/bundle/msi/*.msi` |

开发时，从管理员 PowerShell 执行 `npm run tauri dev`，以便被 Cargo 启动的程序可以打开底层卷句柄。图标文件随仓库提供。编辑 `public/superexplore.svg` 后，可用 `npm run icons` 重新生成 Windows ICO。

索引和 USN 解析测试可用 `cargo test --manifest-path src-tauri/Cargo.toml --locked` 运行。

## 索引行为

- 自动发现本地 NTFS 卷，优先展示盘符路径，也支持目录挂载点和卷 GUID 路径。
- 每个卷维护 FRN 到父 FRN、文件名和类型的映射。Rayon 并行匹配文件名，只为最多 100 条命中查询完整路径和文件元数据。
- 初次扫描前记录 USN 游标，扫描结束后重放扫描期间的变更。日志回卷或重建时重新枚举受影响的卷。对 USN 日志不可用的卷，每分钟尝试重新扫描。
- 每个 FRN 只记录 MFT/USN 提供的一个名称。一个文件的多个 NTFS 硬链接名称可能不会全部出现。被删除、受权限保护或暂时不可用的结果，大小或修改日期可能显示为「—」。
- 索引仅存于内存。首次扫描时间与卷数量、文件规模和磁盘状态有关。

## GitHub Release

推送到 `main`、推送 `v*` 标签或手动运行 Windows release 工作流时，会在 `windows-latest` 构建，并把便携 EXE、NSIS 安装包和 MSI 直接上传到 GitHub Release 草稿。`main` 的草稿按提交区分，版本草稿使用推送的标签。审核产物后，可在 GitHub 的 Releases 页面发布对应草稿。
