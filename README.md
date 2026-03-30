# PSD2UGUI

PSD2UGUI 是一个将 Photoshop PSD 转换为 Unity UGUI 预制体的工具链，包含：

- Rust CLI：负责 PSD 导出、规划与校验
- Unity Package：负责在 Unity Editor 内触发流程，并把结果导入为 UGUI Prefab

当前仓库是一个 monorepo，其中 Unity Package 位于 `com.miloooo.game.psd2ugui` 子目录。

## 功能概览

- 从 Unity 菜单直接导入 `.psd`
- 调用内置 `psd2ugui.exe` 执行导出与编排流程
- 将导出 bundle 导入为 Unity 资源与 Prefab
- 支持 TextMeshPro 字体映射
- 支持将中间产物缓存到项目目录下，便于重复导入

## 仓库结构

```text
.
|- com.miloooo.game.psd2ugui   # Unity UPM package
|- psd-export                  # PSD 导出 CLI
|- ui-orchestrator             # UI 编排逻辑
|- psd2ugui-cli                # 总入口 CLI
|- PSD2UGUI                    # 示例/调试用 Unity 工程
```

## 环境要求

### Unity Package 使用要求

- Unity `2022.3` 或更高版本
- Windows Unity Editor
- 已安装 Adobe Photoshop
- 可正常使用 TextMeshPro

说明：

- `Import PSD...` 当前仅支持 Windows Unity Editor
- PSD 导入流程依赖 Photoshop 可执行文件路径
- 直接导入 bundle 时，不依赖 Photoshop，但仍需要先准备好 bundle 目录

## 在 Unity 中通过 Git URL 导入 Package

由于本仓库的 Unity Package 不在仓库根目录，而是在 `com.miloooo.game.psd2ugui` 子目录，所以需要使用 `?path=` 指定包路径。

### 方式一：Package Manager 界面导入

1. 打开 Unity
2. 进入 `Window > Package Manager`
3. 点击左上角 `+`
4. 选择 `Add package from git URL...`
5. 输入：

```text
https://github.com/ilinchunjie/psd2ugui.git?path=/com.miloooo.game.psd2ugui
```

如果你希望锁定到某个 tag、branch 或 commit，可以使用：

```text
https://github.com/ilinchunjie/psd2ugui.git?path=/com.miloooo.game.psd2ugui#main
```

或：

```text
https://github.com/ilinchunjie/psd2ugui.git?path=/com.miloooo.game.psd2ugui#v0.3.1
```

### 方式二：直接编辑 `manifest.json`

打开 Unity 项目的 `Packages/manifest.json`，在 `dependencies` 中加入：

```json
{
  "dependencies": {
    "com.miloooo.game.psd2ugui": "https://github.com/ilinchunjie/psd2ugui.git?path=/com.miloooo.game.psd2ugui"
  }
}
```

如果项目里已经有其它依赖，只需要补充这一项即可。

## 安装后配置

导入 package 后，在 Unity 中打开：

```text
Project Settings > PSD2UGUI
```

需要配置的关键项：

- `Export Cache Directory`
  用于存放导出中间文件，默认会落在项目根目录下的 `.psd2ugui`
- `Unity Import Root`
  导入后的 Unity 资源根目录，必须是 `Assets/` 下的子目录，默认是 `Assets/Generated/PsdUi`
- `Photoshop Executable`
  Photoshop 的可执行文件路径
- `TextMeshPro Font`
  默认使用的 TMP 字体；如果这里不填，也可以依赖 TMP 全局默认字体

## Unity 内使用方式

安装并完成配置后，可以通过以下菜单使用：

```text
美术/PSD2UGUI/Import PSD...
美术/PSD2UGUI/Import Bundle...
美术/PSD2UGUI/Open Settings
```

### 导入 PSD

`Import PSD...` 的流程大致如下：

1. 选择一个 `.psd` 文件
2. 工具调用内置 `psd2ugui.exe`
3. CLI 执行 export / plan / validate 等流程
4. 生成 bundle 目录
5. Unity 将 bundle 中的图片、描述数据和层级信息导入为资源与 Prefab

成功后会在 Console 输出导入后的 prefab 路径。

### 导入 Bundle

如果你已经提前生成了 bundle，也可以直接使用：

```text
美术/PSD2UGUI/Import Bundle...
```

选择 bundle 目录后，工具会直接导入 Unity 资源并生成 Prefab。

## CLI 开发说明

仓库根目录是 Rust workspace，包含以下成员：

- `psd-export`
- `ui-orchestrator`
- `psd2ugui-cli`

### 常用命令

运行 CLI：

```powershell
cargo run -p psd2ugui -- --help
```

运行测试：

```powershell
cargo test
```

`psd-export` 也可以单独执行，例如：

```powershell
cargo run -p psd-export -- export .\your.psd --out .\out\demo
```

使用 Photoshop 作为栅格后端：

```powershell
cargo run -p psd-export -- export .\your.psd --out .\out\demo --raster-backend photoshop --photoshop-exe "C:\Program Files\Adobe\Adobe Photoshop 2025\Photoshop.exe"
```

## 注意事项

- Unity UPM 安装时，这个仓库必须使用带 `?path=/com.miloooo.game.psd2ugui` 的 git URL
- `Unity Import Root` 不能直接设为 `Assets`，必须是 `Assets` 下的子目录
- `Import PSD...` 依赖 Windows + Photoshop
- 如果没有配置 TMP 字体，至少需要保证项目的 TMP 默认字体可用

## License

[MIT](LICENSE)
