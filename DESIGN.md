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
      buckets/<name>/       git 仓库
      held/<pkg>            空文件，存在即表示 hold
      launchers/<pkg>/      快捷方式专用的 shim，不在 PATH 中
      lock                  全局文件锁
      gc-stamp              上次 GC 时间
      <ver>/                wpk 自身，与普通包相同的版本目录
  var/
    <pkg>/                  persist 数据
```

缓存目录为 `$XDG_CACHE_HOME/wpk`，未设置该变量时用 `~/.cache/wpk`。缓存文件名沿用 Scoop 的 `<pkg>#<ver>#<文件名>`。

当前版本记录在 `opt/<pkg>/current` 这个文本文件里，不使用 junction。和 `.shim` 一样，这个文件在任何时候都能改写。

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

## shim

`wpk-shim` 是自己写的 Rust 程序，编出 console 和 gui 两个变体。安装 bin 时读取目标 exe 的 PE 子系统，复制对应的变体；bat 和 cmd 用 console 变体。

shim 启动时读取同目录下同名的 `.shim` 文件，格式为：

```
path = C:\Users\x\wpk\opt\fd\10.5.2\fd.exe
args = --hidden
```

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

任务栏按 AppUserModelID 分组，程序没有设 ID 时按 exe 路径匹配。快捷方式指向 shim，实际窗口属于另一个 exe，不处理的话会显示成两个图标。处理方式：

- 每个快捷方式都写入 `System.AppUserModel.ID`。manifest 的 `shortcut_aumid` 扩展字段给出了 ID 就用它，用于自己设了 ID 的程序（如 wezterm）；没有则生成 `wpk.<pkg>.<name>`。
- gui shim 从自身的 STARTUPINFO 读到 Shell 传入的 ID，再用 `STARTF_TITLEISAPPID` 传给子进程。

这个机制需要先做原型验证。

## persist

`persist` 的数据存放在 `var/<pkg>`。安装新版本时，目录在 `opt/<pkg>/<ver>` 中建 junction 指向 `var/<pkg>` 下的对应位置，文件用硬链接。`var/<pkg>` 中还没有的条目，先从安装包里复制一份过去；安装包里也没有的，目录建为空目录，文件建为空文件。

## 解包

zip、7z、tar.*、gz 内置，msi 调用系统的 `msiexec /a`。其余格式调用 wpk 已安装的包：innosetup 用 `innounp`，rar、cab、iso、NSIS 等用 `7zip`。这两个包由 `wpk init` 作为普通包安装，解包时按它们的收据找到可执行文件；未安装时报错并提示安装。

main 中的 `7zip` 带有脚本字段，会被拒绝，需要在 `wpk` bucket 中提供一份不含脚本的 manifest。`innounp` 可以直接用 main 的。

## 安装与升级流程

1. 获取全局锁。
2. 解析包和依赖，检查字段。有任何被拒绝的字段时，在下载之前报错退出。
3. 并行下载到缓存，校验 hash。
4. 解包到 `opt/<pkg>/<ver>.tmp`，完成后改名为 `<ver>`。
5. 建立 persist：`var/<pkg>` 中缺少的条目，从安装包复制，安装包里也没有的建为空目录或空文件；然后建立 junction 和硬链接。写入 `.wpk/manifest.json` 和 `receipt.json`。
6. 写入 bin 和 launcher 的 `.shim` 及 exe，删除新版本中已不存在的 shim。
7. 写入 `current`。
8. 打印 notes、`env_set` 提示和 `suggest`。
9. 如果距上次 GC 超过 `gc_interval_days`（默认 7 天），执行 GC。

任何一步失败，删除 `.tmp` 目录；`current` 和 `.shim` 只在最后几步才改动，所以失败时旧版本保持可用。

## GC

GC 删除 `opt/<pkg>/` 下除 `current` 所指版本以外的所有版本目录，以及被改名移开的旧 shim。目录中有 exe 或 dll 正被进程占用时跳过，留到下一次 GC。`wpk gc` 可以手动触发。

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

bucket 是 git 仓库，只支持 https 远程，用 gix 操作，HTTP 传输走 curl。

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

提供 `install.ps1` 和 `install.elv`，两者做同样的事：下载 wpk.exe 到临时目录，执行 `wpk init`，然后提示用户把 `~/wpk/bin` 加入 PATH。

## 发布

wpk.exe 发布在 MinIO：alias `prod`，bucket `scoop-bucket`，对象路径 `wpk/<ver>/wpk.zip`，公开地址 `https://s3.orkward.dev/scoop-bucket/wpk/<ver>/wpk.zip`。同一目录下的 `wpk/latest.json` 记录最新版本的 url 和 sha256，供安装脚本使用。wpk 的自我升级不读 `latest.json`，只看 wpk-bucket 中的 manifest。

## 工程结构

仓库位于 `~/Repos/winpkgs`，cargo workspace：

| crate | 内容 |
|---|---|
| `wpk` | CLI，参数解析和终端输出 |
| `libwpk` | 核心逻辑：manifest、bucket、下载、解包、安装、GC |
| `wpk-shim` | shim 程序，console 变体 `wpk-shim.exe` 和 gui 变体 `wpk-shimw.exe` |

根目录可以用环境变量 `WPK_ROOT` 覆盖，主要用于测试。

主要依赖：

- gix：bucket 的 git 操作
- curl：静态链接，TLS 用 schannel
- RustCrypto 的 md5、sha1、sha2：hash 校验
- serde_json、toml：manifest 和配置
- clap：命令行

并发用线程池，不引入 async 运行时。

许可证为 BSD-3-Clause。从 libscoop 搬来的代码按其 MIT 许可使用，保留原版权声明。

## 其他规则

- 两个包提供同名的 bin 或 launcher 时，拒绝安装后来的那个，并报出冲突的包名。
- bucket 中的版本字符串与已安装版本不同就视为可升级，不比较大小。
- 不提供从 Scoop 或 hok 迁移的功能。

从 hok 的 libscoop 搬过来的部分：manifest 解析（`Vectorized` 等反序列化）、bucket 查询、依赖 DAG、下载与缓存校验、zip/7z/tar 解包和 msi 解包。sync、shim、script、persist 等模块重写。
