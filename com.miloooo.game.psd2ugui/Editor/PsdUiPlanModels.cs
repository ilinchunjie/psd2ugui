using System.Collections.Generic;
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;

namespace PsdUi.Editor
{
    internal sealed class UiPlanDocument
    {
        [JsonProperty("plan_version")] public string PlanVersion { get; set; } = string.Empty;
        [JsonProperty("source_bundle")] public SourceBundleInfo SourceBundle { get; set; } = new();
        [JsonProperty("document")] public PlanDocumentInfo Document { get; set; } = new();
        [JsonProperty("nodes")] public List<PlanNodeData> Nodes { get; set; } = new();
        [JsonProperty("review_items")] public List<ReviewItemData> ReviewItems { get; set; } = new();
        [JsonProperty("warnings")] public List<PlanWarningData> Warnings { get; set; } = new();
    }

    internal sealed class SourceBundleInfo
    {
        [JsonProperty("bundle_dir")] public string BundleDir { get; set; } = string.Empty;
        [JsonProperty("manifest_path")] public string ManifestPath { get; set; } = string.Empty;
        [JsonProperty("preview_path")] public string PreviewPath { get; set; } = string.Empty;
        [JsonProperty("document_id")] public string DocumentId { get; set; } = string.Empty;
        [JsonProperty("generated_at")] public string GeneratedAt { get; set; } = string.Empty;
    }

    internal sealed class PlanDocumentInfo
    {
        [JsonProperty("width")] public int Width { get; set; }
        [JsonProperty("height")] public int Height { get; set; }
        [JsonProperty("preview_path")] public string PreviewPath { get; set; } = string.Empty;
    }

    internal sealed class PlanNodeData
    {
        [JsonProperty("node_id")] public string NodeId { get; set; } = string.Empty;
        [JsonProperty("name")] public string Name { get; set; } = string.Empty;
        [JsonProperty("source_layer_ids")] public List<string> SourceLayerIds { get; set; } = new();
        [JsonProperty("component_type")] public string ComponentType { get; set; } = string.Empty;
        [JsonProperty("rect")] public PlanRectData Rect { get; set; } = new();
        [JsonProperty("render_order")] public int RenderOrder { get; set; }
        [JsonProperty("children")] public List<PlanNodeData> Children { get; set; } = new();
        [JsonProperty("confidence")] public float Confidence { get; set; }
        [JsonProperty("needs_review")] public bool NeedsReview { get; set; }
        [JsonProperty("metadata")] public JObject Metadata { get; set; } = new();
        [JsonProperty("text")] public PlanTextData Text { get; set; }
        [JsonProperty("interaction")] public PlanInteractionData Interaction { get; set; }
    }

    internal sealed class PlanRectData
    {
        [JsonProperty("x")] public int X { get; set; }
        [JsonProperty("y")] public int Y { get; set; }
        [JsonProperty("width")] public int Width { get; set; }
        [JsonProperty("height")] public int Height { get; set; }
        [JsonProperty("local_x")] public int LocalX { get; set; }
        [JsonProperty("local_y")] public int LocalY { get; set; }
    }

    internal sealed class PlanTextData
    {
        [JsonProperty("content")] public string Content { get; set; } = string.Empty;
        [JsonProperty("source")] public string Source { get; set; } = string.Empty;
        [JsonProperty("confidence")] public float Confidence { get; set; }
        [JsonProperty("character_runs")] public List<PlanTextCharacterRunData> CharacterRuns { get; set; } = new();
        [JsonProperty("paragraph_runs")] public List<PlanTextParagraphRunData> ParagraphRuns { get; set; } = new();
        [JsonProperty("font_size")] public float? FontSize { get; set; }
        [JsonProperty("alignment")] public string Alignment { get; set; } = string.Empty;
        [JsonProperty("color")] public string Color { get; set; } = string.Empty;
    }

    internal sealed class PlanTextCharacterRunData
    {
        [JsonProperty("start")] public int Start { get; set; }
        [JsonProperty("length")] public int Length { get; set; }
        [JsonProperty("font_family")] public string FontFamily { get; set; } = string.Empty;
        [JsonProperty("font_style")] public string FontStyle { get; set; } = string.Empty;
        [JsonProperty("font_size")] public float? FontSize { get; set; }
        [JsonProperty("color")] public string Color { get; set; } = string.Empty;
    }

    internal sealed class PlanTextParagraphRunData
    {
        [JsonProperty("start")] public int Start { get; set; }
        [JsonProperty("length")] public int Length { get; set; }
        [JsonProperty("alignment")] public string Alignment { get; set; } = string.Empty;
    }

    internal sealed class PlanInteractionData
    {
        [JsonProperty("kind")] public string Kind { get; set; } = string.Empty;
        [JsonProperty("horizontal")] public bool? Horizontal { get; set; }
        [JsonProperty("vertical")] public bool? Vertical { get; set; }
        [JsonProperty("content_rect")] public PlanRectData ContentRect { get; set; }
    }

    internal sealed class ReviewItemData
    {
        [JsonProperty("kind")] public string Kind { get; set; } = string.Empty;
        [JsonProperty("severity")] public string Severity { get; set; } = string.Empty;
        [JsonProperty("node_id")] public string NodeId { get; set; } = string.Empty;
        [JsonProperty("message")] public string Message { get; set; } = string.Empty;
    }

    internal sealed class PlanWarningData
    {
        [JsonProperty("code")] public string Code { get; set; } = string.Empty;
        [JsonProperty("message")] public string Message { get; set; } = string.Empty;
    }
}
