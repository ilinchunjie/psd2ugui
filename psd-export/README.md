# psd-export

第一阶段 Rust CLI，用于把单个 PSD 导出为：

- `manifest.json`
- `images/*.png`
- `masks/*.png`
- `preview/document.png`

## 用法

默认仍使用 `rawpsd` 进行结构解析和 PNG 导出：

```powershell
cargo run -- export .\your.psd --out .\out\demo
```

使用 Photoshop 作为高保真栅格后端：

```powershell
cargo run -- export .\your.psd --out .\out\demo --raster-backend photoshop --photoshop-exe "C:\Program Files\Adobe\Adobe Photoshop 2025\Photoshop.exe"
```

自动尝试 Photoshop，失败时回退到 `rawpsd`：

```powershell
cargo run -- export .\your.psd --out .\out\demo --raster-backend auto --photoshop-exe "C:\Program Files\Adobe\Adobe Photoshop 2025\Photoshop.exe"
```

## 参数

- `--include-hidden`：隐藏叶子层也导出图片
- `--strict`：任何 warning 都直接失败
- `--with-preview`：兼容参数，当前阶段默认始终导出 `preview/document.png`
- `--raster-backend rawpsd|photoshop|auto`：选择 PNG 栅格后端
- `--photoshop-exe <path>`：Photoshop 可执行文件路径
- `--photoshop-timeout-sec <n>`：等待 Photoshop COM 自动化的超时时间，默认 `120`

## 栅格后端

### `rawpsd`

- 不依赖 Photoshop
- 输出最稳定
- 图层效果、smart object、复杂 shape/vector 仍是近似或部分保留

### `photoshop`

- 结构、文本、mask、clip 元数据仍由 `rawpsd` 解析
- 叶子层 PNG 与整张预览图由 Photoshop 导出
- 成功由 Photoshop 栅格化的节点会在 `effects.baked` 中记录 `"photoshop_raster"`
- 任一请求层导出失败会直接报错

### `auto`

- 先尝试 Photoshop 栅格
- Photoshop 不可用、脚本执行失败、或个别图层导出失败时，自动回退到 `rawpsd`
- 回退行为会写入 `manifest.warnings`

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
