# psd-export

第一阶段 Rust CLI，用于把单个 PSD 导出为：

- `manifest.json`
- `images/*.png`
- `masks/*.png`
- `preview/document.png`

当前版本为 Photoshop-only 导出链路：

- PSD 结构、文本、mask、clip 元数据由 `rawpsd` 解析
- 叶子层 PNG 与整张预览图由 Photoshop 导出
- 没有配置或无法自动化 Photoshop 时，导出会直接失败

## 用法

Windows 示例：

```powershell
cargo run -- export .\your.psd --out .\out\demo --photoshop-path "C:\Program Files\Adobe\Adobe Photoshop 2025\Photoshop.exe"
```

macOS Apple Silicon 示例：

```bash
cargo run -- export ./your.psd --out ./out/demo --photoshop-path "/Applications/Adobe Photoshop 2025/Adobe Photoshop 2025.app"
```

兼容旧脚本时，也可以继续使用：

```powershell
cargo run -- export .\your.psd --out .\out\demo --photoshop-exe "C:\Program Files\Adobe\Adobe Photoshop 2025\Photoshop.exe"
```

## 参数

- `--include-hidden`：隐藏叶子层也导出图片
- `--strict`：任何 warning 都直接失败
- `--with-preview`：兼容参数，当前阶段默认始终导出 `preview/document.png`
- `--photoshop-path <path>`：Photoshop 应用路径。Windows 传 `.exe`，macOS 传 `.app`
- `--photoshop-exe <path>`：`--photoshop-path` 的兼容别名
- `--photoshop-timeout-sec <n>`：等待 Photoshop 自动化可用的超时时间，默认 `120`

## 平台行为

### Windows

- 使用 `cscript + COM` 驱动 Photoshop
- `--photoshop-path` 应指向 Photoshop 可执行文件

### macOS Apple Silicon

- 使用 `osascript + Apple events` 驱动 Photoshop
- `--photoshop-path` 应指向 Photoshop `.app`
- 首次运行时，系统可能要求允许终端或 Unity 控制 Photoshop

### 不支持的平台

- Intel Mac 不在当前支持范围内
- 其它桌面平台不会尝试 best-effort 回退

## 当前能力边界

- 支持：8-bit RGB PSD、层级结构、叶子层位图导出、图层蒙版导出、merged preview 导出
- 尝试提取：文本内容、字体、字号、颜色、对齐；基础描边、阴影；基础 shape fill/stroke 语义
- Photoshop 后端当前重点提升的是像素保真度，不改变下游 `manifest.json` 的消费方式
- 暂不完整支持：调整图层、复杂 shape/vector 数据的语义化恢复

## 测试

运行全部测试：

```powershell
cargo test
```

运行解析层测试：

```powershell
cargo test psd::tests -- --nocapture
```
