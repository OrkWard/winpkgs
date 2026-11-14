# winpkgs 设计

winpkgs 是 Windows 上的便携软件包管理器，命令名 `wpk`。它读取 Scoop 格式的 manifest 和 bucket，但只接受其中一部分字段，文件布局与 Scoop 完全不同。

## 范围

只管理便携软件。以下软件包不在范围内：自己管理自身升级的、依赖注册表的、对自身目录结构有要求的大型 GUI 应用。

wpk 不修改任何用户环境，包括 PATH、注册表和用户环境变量。需要靠 PATH 才能工作的包直接拒绝安装；需要特定环境变量的包，安装后提示用户自己在 shell 里配置。

## 文件布局

根目录默认为 `~/wpk`。

```
~/wpk/
  bin/                      用户手动加入 PATH，wpk 不改 PATH
    <name>.exe              wpk-shim 的副本，同一子系统的所有副本字节相同
    <name>.shim             纯文本，记录目标路径和参数
  opt/
    <pkg>/
      current               纯文本，内容是当前版本号
      <ver>/                解包后的软件包内容
        .wpk/
          manifest.json     安装时使用的 manifest
          receipt.json      安装收据
    wpk/
      config.toml
      buckets/<name>/       bare、深度为 1 的 git 克隆，没有工作区
      held/<pkg>            空文件，存在即表示 hold
      launchers/<pkg>/      快捷方式专用的 shim 和图标，不在 PATH 中
      lock                  全局文件锁
      gc-stamp              上次 GC 时间
      <ver>/                wpk 自身，与普通包相同的版本目录
  var/
    <pkg>/                  persist 数据
```

缓存目录为 `$XDG_CACHE_HOME/wpk`，未设置该变量时用 `~/.cache/wpk`。缓存文件名沿用 Scoop 的规则：`<pkg>#<ver>#<url>`，url 中字母、数字、`.-_` 以外的字符换成 `_`。

当前版本记录在 `opt/<pkg>/current` 这个文本文件里，不使用 junction。和 `.shim` 一样，这个文件在任何时候都能改写。

版本目录名是版本号，其中 Windows 不允许的字符换成 `_`。包名和版本号为空、为 `.` 或 `..`、以 `.wpk` 开头、以 `.tmp` 结尾或含路径分隔符时拒绝安装，版本号为 `current` 时也拒绝。

## manifest 字段

| 处理方式 | 字段 |
|---|---|
| 支持 | `version` `description` `homepage` `license` `url` `hash` `architecture`（只认 `64bit` `arm64`） `extract_dir` `extract_to` `bin` `persist` `shortcuts` `depends` `suggest` `notes` `innosetup` |
| 只提示 | `env_set`：安装后列出需要用户自行设置的变量 |
| 拒绝安装 | `env_add_path` `pre_install` `post_install` `installer` `uninstaller` `pre_uninstall` `post_uninstall` `psmodule` |
| 忽略 | `checkver` `autoupdate` `cookie` `##` 及其他未知字段 |

细节：

- `hash` 支持 md5、sha1、sha256、sha512。
- `url` 支持 `#/<文件名>` 后缀，下载后按这个名字保存，并按这个扩展名选择解包方式。
- `bin` 支持三种写法：`"x.exe"`、`["x.exe", "别名"]`、`["x.exe", "别名", "参数"]`。目标只允许 exe、bat、cmd，其他类型拒绝安装。
- 搜索结果照常列出被拒绝的包，并标明拒绝原因（具体字段名）。
- `depends` 按依赖关系安装，被依赖的包同样要满足上面的规则。`suggest` 只在 notes 中展示。
- `bin` 和快捷方式的参数、`env_set` 的值中，`$dir` `$original_dir` 换成版本目录，`$persist_dir` 换成 `var/<pkg>`，`$version` `$app` 换成版本号和包名。
- 下载文件名、`extract_dir`、`extract_to`、`bin`、`shortcuts`、`persist` 中的路径必须是不含 `..` 的相对路径，否则拒绝安装。
- wpk 扩展字段 `"multicall": true`：用于 rustup 这类按自身文件名决定行为的程序。命令名与目标文件名不同的 bin，安装时在目标所在目录建一个以命令名命名的硬链接，shim 指向这个硬链接。例如 `["rustup.exe", "cargo"]` 生成 `cargo.exe`。

## shim

`wpk-shim` 是自己写的 Rust 程序，编出 console 和 gui 两个变体。安装 bin 时读取目标 exe 的 PE 子系统，复制对应的变体；bat 和 cmd 用 console 变体。

shim 启动时读取同目录下同名的 `.shim` 文件，格式为：

```
path = C:\Users\x\wpk\opt\fd\10.5.2\fd.exe
args = --hidden
dir = C:\Users\x\wpk\opt\fd\10.5.2
```

`args` 和 `dir` 可以省略。`args` 放在调用 shim 时的参数前面。`dir` 是子进程的工作目录，只有 launcher 写它，设为版本目录；bin 中的 shim 不写，子进程继承调用者的工作目录。

目标 exe 要求管理员权限时，CreateProcess 失败，shim 改用 ShellExecuteEx 的 `runas` 启动。

目标是 bat 或 cmd 时，cmd.exe 会按自己的规则重新解析命令行，而调用者是按 C 运行时的规则加的引号，原样转交会让参数里的 `&` 等字符变成新命令。所以 shim 先按 C 运行时规则拆分参数，再交给 Rust 标准库按 cmd 的规则重新加引号，无法安全表示的参数直接报错。

shim 用允许删除共享（FILE_SHARE_DELETE）的方式打开 `.shim`，读完立即关闭，保证升级时可以随时替换。

Windows 没有 exec，shim 只能创建子进程。console 变体的行为：

- 子进程继承标准输入输出句柄。
- shim 忽略 Ctrl+C 等控制台信号。子进程与 shim 共用同一个控制台，会直接收到这些信号，由子进程自己决定是否退出。
- 子进程放进一个设置了 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` 的 job object，shim 被杀时子进程随之结束。
- shim 等待子进程结束，返回其退出码。

gui 变体启动子进程后直接退出。

升级软件包只改写 `.shim`：先写临时文件，再改名覆盖。shim 本身出新版本时，如果旧的 exe 正在运行，先把它改名移开，再放入新文件，旧文件留给 GC 删除。

## 快捷方式

快捷方式放在 `开始菜单\Programs\wpk`，目标是 `opt/wpk/launchers/<pkg>/<name>.exe`。这个 shim 与 bin 中的 shim 机制相同，但不在 PATH 里，所以未列在 `bin` 中的 GUI 程序不会出现在命令行。升级时只改写 launcher 的 `.shim`，快捷方式本身不变，用户钉到任务栏的副本也不会失效。

快捷方式的图标是 `launchers/<pkg>/<name>.ico`，每次安装时从目标 exe 的资源里提取（manifest 指定了图标时用指定的），所以旧版本被 GC 删掉后图标依然有效。

任务栏按 AppUserModelID 分组，程序没有设 ID 时按 exe 路径匹配。快捷方式指向 shim，实际窗口属于另一个 exe，不处理的话会显示成两个图标。处理方式：

- 每个快捷方式都写入 `System.AppUserModel.ID`。manifest 的 `shortcut_aumid` 扩展字段给出了 ID 就用它，用于自己设了 ID 的程序（如 wezterm）；没有则生成 `wpk.<pkg>.<name>`。
- shim 把自身的 STARTUPINFO 原样传给子进程，其中包括 Shell 设置的 `STARTF_TITLEISAPPID` 或 `STARTF_TITLEISLINKNAME`，以及窗口显示方式。

分组效果还没有在真实任务栏上验证。

## persist

`persist` 的数据存放在 `var/<pkg>`。安装新版本时，目录在 `opt/<pkg>/<ver>` 中建 junction 指向 `var/<pkg>` 下的对应位置，文件用硬链接。每个条目按以下顺序处理：

1. `var/<pkg>` 中已有：以它为准，安装包里的同名条目改名为 `<name>.original` 保留。
2. 只有安装包里有：移动到 `var/<pkg>`。
3. 两边都没有：名字带扩展名的建为空文件，否则建为空目录。

删除版本目录时不跟随 junction，`var` 中的数据不受影响。

硬链接有一个问题：很多程序保存时先写新文件再替换旧文件，替换后链接就断了。这类程序用 wpk 扩展字段 `persist_copy`，列出要跨版本保留的文件或目录，它们不链接，而是复制：

- 安装新版本时，从当前已装的版本目录复制过来；没有已装版本时从 `var/<pkg>` 复制。两处都没有，安装包自带的文件保持原样。
- 卸载时，把当前版本里的这些文件复制到 `var/<pkg>`。
- 升级时，如果旧版本还在运行，跳过这个包并提示先关闭，否则旧进程退出时会把设置写回旧目录。

## 解包

解包方式按下载文件名决定，规则与 Scoop 相同：

| 文件名 | 方式 |
|---|---|
| `.zip` | 内置 |
| `.msi` | 系统的 `msiexec /a` |
| `.exe`，且 manifest 有 `innosetup` | `innounp` 包 |
| `.exe`，没有 `innosetup` | 不解包，原样放入 |
| `.7z` `.gz` `.tar` `.tgz` `.xz` `.zst` `.rar` `.iso` `.nupkg` 等 | `7zip` 包；`.tar.*` 先解出 tar 再解一次 |
| 其他 | 不解包，原样放入 |

`extract_dir` 和 `extract_to` 按顺序对应需要解包的文件，不计原样放入的文件。

`7zip` 和 `innounp` 由 `wpk init` 作为普通包安装，解包时从它们的当前版本目录找到可执行文件；未安装时报错并提示安装。main 中的 `7zip` 带有脚本字段，会被拒绝，所以 `wpk` bucket 提供一份不含脚本的 manifest。`innounp` 直接用 main 的。

## 安装与升级流程

1. 获取全局锁。
2. 解析包和依赖，检查字段和路径，检查 bin 和快捷方式是否与其他包冲突。有任何问题时，在下载之前报错退出。
3. 并行下载到缓存，校验 hash。
4. 把缓存文件硬链接（失败时复制）到 `opt/<pkg>/<ver>.tmp`，在其中解包，完成后改名为 `<ver>`。
5. 建立 persist，检查 bin 和快捷方式的目标文件存在，写入 `.wpk/manifest.json`。
6. 写入 bin 和 launcher 的 `.shim` 及 exe、图标和快捷方式。
7. 写入 `receipt.json` 和 `current`，删除新版本中已不存在的 shim 和快捷方式。
8. 打印 notes、`env_set` 提示和 `suggest`。
9. 如果距上次 GC 超过 `gc_interval_days`（默认 7 天），执行 GC。

第 5 步之前失败，删除新版本目录；之后失败，新版本目录保留但 `current` 不变。旧版本始终可用。

## GC

GC 删除 `opt/<pkg>/` 下除 `current` 所指版本以外的所有版本目录（含 `.wpk/receipt.json` 的目录，以及残留的 `.tmp` 目录），以及被改名移开的旧 shim。`opt/wpk` 中的状态文件不是版本目录，不会被删除。

删除前先检查目录中是否有 exe、dll 等映像正被进程加载，再把目录改名为 `.wpk-gc-<ver>`；有文件被打开时改名会失败。两种情况都跳过，留到下一次 GC。`wpk gc` 可以手动触发。

## 卸载

删除 bin 和 launcher 中属于该包的 shim、快捷方式和 `opt/<pkg>`，正被占用的版本目录留给 GC。`var/<pkg>` 默认保留，`--purge` 时一并删除。

## 收据

`receipt.json` 记录：

- 版本
- 来源 bucket
- 架构
- 安装时间
- 是否为用户主动安装（区别于作为依赖被装上）
- 生成的 bin、launcher 和快捷方式列表

卸载和清理 shim 时以收据为准，不重新解析 manifest。

## bucket

bucket 是 git 仓库，远程可以是 https 地址或本地路径，用 gix 操作，HTTP 传输走 curl。克隆为 bare、深度为 1 的仓库，manifest 直接从 git 对象中读取，不检出工作区。更新时把远程默认分支强制同步到本地分支，所以上游改写历史后也能正常更新。读取时 config 被隔离，用户的 git 配置（如 `url.insteadOf`）不会影响 bucket。

`wpk init` 默认添加三个 bucket，优先级从高到低：

| 名称 | 仓库 |
|---|---|
| `wpk` | OrkWard/wpk-bucket |
| `main` | ScoopInstaller/Main |
| `extras` | ScoopInstaller/Extras |

同名包按 config 中的 bucket 顺序取第一个，也可以用 `bucket/pkg` 显式指定。

## 配置

`opt/wpk/config.toml`：

```toml
proxy = "http://proxy.lan:9090"
gc_interval_days = 7

[[bucket]]
name = "wpk"
url = "https://github.com/OrkWard/wpk-bucket"

[[bucket]]
name = "main"
url = "https://github.com/ScoopInstaller/Main"

[[bucket]]
name = "extras"
url = "https://github.com/ScoopInstaller/Extras"
```

## 命令

| 命令 | 作用 |
|---|---|
| `init` | 建立目录结构，把自身装进 `opt/wpk/<ver>`，生成 bin shim，添加默认 bucket，安装 `7zip` 和 `innounp` |
| `update` | 更新 bucket |
| `upgrade [pkg...]` | 升级包，不指定时升级全部未 hold 的包 |
| `install <pkg...>` | 安装 |
| `uninstall <pkg...>` | 卸载 |
| `list` | 已安装的包 |
| `search <query>` | 搜索 |
| `info <pkg>` | 详情 |
| `hold` / `unhold <pkg>` | 锁定或解除锁定版本 |
| `bucket add/rm/list` | 管理 bucket |
| `cache list/rm` | 管理缓存 |
| `gc` | 手动 GC |
| `which <name>` | 显示某个 shim 的目标 |

wpk 自身作为 `wpk` bucket 中的普通包升级，流程与其他包相同。

## 安装脚本

提供 `scripts/install.ps1` 和 `scripts/install.elv`，两者做同样的事：读取 `latest.json`，下载 `wpk.zip` 到临时目录并校验 sha256，解压后执行 `wpk init`。init 结束时提示用户把 `~/wpk/bin` 加入 PATH。

## 发布

`elvish scripts/release.elv` 编译 release 版本并打包到 `target/dist/<ver>/`。加 `--publish` 时上传到 MinIO 并写入 `~/Repos/wpk-bucket/bucket/wpk.json`，提交和推送由人完成。

MinIO 上的位置：alias `prod`，bucket `winpkgs-bucket`，公开地址前缀 `https://s3.orkward.dev/winpkgs-bucket/wpk/`：

- `<ver>/wpk.zip`：`wpk.exe`、`wpk-shim.exe`、`wpk-shimw.exe`
- `latest.json`：最新版本的 url 和 sha256，供安装脚本使用
- `install.elv`、`install.ps1`

wpk 的自我升级不读 `latest.json`，只看 wpk-bucket 中的 manifest。

## 工程结构

仓库位于 `~/Repos/winpkgs`，cargo workspace：

| crate | 内容 |
|---|---|
| `wpk` | CLI，参数解析和终端输出 |
| `libwpk` | 核心逻辑：manifest、bucket、下载、解包、安装、GC |
| `wpk-shim` | shim 程序，console 变体 `wpk-shim.exe` 和 gui 变体 `wpk-shimw.exe` |

根目录可以用环境变量 `WPK_ROOT` 覆盖，快捷方式目录可以用 `WPK_START_MENU` 覆盖，主要用于测试。

主要依赖：

- gix：bucket 的 git 操作
- curl：静态链接，TLS 用 schannel
- RustCrypto 的 md5、sha1、sha2：hash 校验
- zip：内置的 zip 解包
- junction：persist 的目录链接
- windows、windows-sys：快捷方式、图标提取和 shim 的进程管理
- serde_json、toml：manifest 和配置
- jiff：收据中的时间
- clap：命令行

并发用线程池，不引入 async 运行时。

许可证为 BSD-3-Clause。代码全部新写，没有复制 libscoop 的代码。

## 其他规则

- 两个包提供同名的 bin 或快捷方式时，拒绝安装后来的那个，并报出冲突的包名。
- bucket 中的版本字符串与已安装版本不同就视为可升级，不比较大小。
- 不提供从 Scoop 或 hok 迁移的功能。
- wpk 不能卸载自己。