# ui-orchestrator

Second-stage offline planner for converting `psd-export` bundles into:

- `ui_plan.json`
- `validation_report.json`

The planner is intentionally focused on structure and stable metadata:

- it reads `manifest.json` and `preview/document.png`
- it passes image and mask paths through `ui_plan.json` so the Unity importer can bind real sprites
- it creates `Image` nodes for raster content, which Unity later materializes as `Image + Sprite`
- it attempts text recovery from manifest text blocks exposed by stage one

## Usage

```powershell
cargo run -- generate ..\psd-export\out\demo --out ..\psd-export\out\demo\plan
```

Validate an existing plan:

```powershell
cargo run -- validate ..\psd-export\out\demo\plan
```

## Output

`ui_plan.json` contains:

- `plan_version`
- `source_bundle`
- `document`
- `nodes`
- `review_items`
- `warnings`

`validation_report.json` contains:

- `plan_status`
- `coverage`
- `text_recovery`
- `component_summary`
- `review_count`
- `unity_apply_status`
- `warnings`

## Unity import behavior

The Unity-side importer lives under `D:\ui-auto-gen\unity-project`.

- It copies bundle `images/` and `masks/` into `Assets/Generated/PsdUi/<doc>/Imported/`
- It imports copied PNGs as Sprite assets
- It binds sprites to generated `Image`, `Button`, and mask-related nodes when `metadata.asset_path` or `metadata.mask_path` is present

## Current text recovery behavior

The planner does not ship OCR in this crate yet. It uses two stages:

1. `manifest.text.content` when stage one exposes text semantics

Layers without usable manifest text payload are emitted as `Image` nodes and surfaced via review items when needed.
