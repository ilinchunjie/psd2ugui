use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::manifest::Bounds;

#[derive(Debug, Clone, Serialize)]
pub struct PhotoshopExportRequest {
    pub source_psd_path: String,
    pub staging_dir: String,
    pub preview_relpath: String,
    pub layers: Vec<PhotoshopLayerExportRequest>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PhotoshopLayerExportRequest {
    pub id: String,
    pub name: String,
    pub raw_index: usize,
    pub path_indices: Vec<usize>,
    pub expected_visible: bool,
    pub output_png_relpath: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PhotoshopExportResponse {
    #[serde(default)]
    pub preview_path: Option<String>,
    #[serde(default)]
    pub preview_width: Option<u32>,
    #[serde(default)]
    pub preview_height: Option<u32>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub layers: Vec<PhotoshopLayerExportResult>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PhotoshopLayerExportResult {
    pub id: String,
    pub exported: bool,
    #[serde(default)]
    pub bounds: Option<Bounds>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PhotoshopExportOptions {
    pub photoshop_exe: Option<PathBuf>,
    pub timeout_sec: u64,
}

pub fn run_photoshop_export(
    request: &PhotoshopExportRequest,
    options: &PhotoshopExportOptions,
    workspace_root: &Path,
) -> Result<PhotoshopExportResponse> {
    let runtime_dir = workspace_root.join(".photoshop-runtime");
    if runtime_dir.exists() {
        fs::remove_dir_all(&runtime_dir).map_err(|error| AppError::io(&runtime_dir, error))?;
    }
    fs::create_dir_all(&runtime_dir).map_err(|error| AppError::io(&runtime_dir, error))?;

    let request_path = runtime_dir.join("request.json");
    let response_path = runtime_dir.join("response.json");
    let jsx_path = runtime_dir.join("export_layers.jsx");
    let js_driver_path = runtime_dir.join("run_photoshop_export.js");

    let request_json = serde_json::to_vec_pretty(request)?;
    fs::write(&request_path, request_json).map_err(|error| AppError::io(&request_path, error))?;

    // Photoshop's ExtendScript resolves File() paths relative to its own
    // working directory, which is NOT the cwd of cscript.exe.  Canonicalize
    // all paths embedded in the JSX so they are always absolute.
    let abs_request_path = fs::canonicalize(&request_path)
        .map_err(|error| AppError::io(&request_path, error))?;
    let abs_response_path = runtime_dir
        .canonicalize()
        .map_err(|error| AppError::io(&runtime_dir, error))?
        .join("response.json");
    let staging_path = Path::new(&request.staging_dir);
    let abs_staging_dir = if staging_path.exists() {
        fs::canonicalize(staging_path)
            .map_err(|error| AppError::io(staging_path, error))?
    } else {
        fs::create_dir_all(staging_path)
            .map_err(|error| AppError::io(staging_path, error))?;
        fs::canonicalize(staging_path)
            .map_err(|error| AppError::io(staging_path, error))?
    };

    fs::write(
        &jsx_path,
        build_jsx_script(&abs_request_path, &abs_response_path, &abs_staging_dir, request),
    )
    .map_err(|error| AppError::io(&jsx_path, error))?;
    fs::write(&js_driver_path, windows_script_driver())
        .map_err(|error| AppError::io(&js_driver_path, error))?;

    // Canonicalize the driver and JSX paths so that Photoshop's
    // DoJavaScriptFile receives an absolute path via the cscript argument.
    let abs_jsx_path = fs::canonicalize(&jsx_path)
        .map_err(|error| AppError::io(&jsx_path, error))?;
    let abs_driver_path = fs::canonicalize(&js_driver_path)
        .map_err(|error| AppError::io(&js_driver_path, error))?;

    let mut command = Command::new("cscript.exe");
    command
        .arg("//NoLogo")
        .arg(&abs_driver_path)
        .arg(&abs_jsx_path)
        .arg(options.timeout_sec.to_string());

    if let Some(photoshop_exe) = &options.photoshop_exe {
        command.arg(photoshop_exe);
    }

    let output = command.output().map_err(|error| {
        AppError::Photoshop(format!(
            "failed to launch Windows Script Host Photoshop driver: {error}"
        ))
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(AppError::Photoshop(format!(
            "Windows Script Host Photoshop driver exited with status {}. stdout: {} stderr: {}",
            output.status,
            stdout.trim(),
            stderr.trim()
        )));
    }

    let response_json =
        fs::read_to_string(&response_path).map_err(|error| AppError::io(&response_path, error))?;
    serde_json::from_str(&response_json).map_err(|error| {
        AppError::Photoshop(format!(
            "failed to parse Photoshop export response {}: {error}",
            response_path.display()
        ))
    })
}

fn windows_script_driver() -> &'static str {
    r#"
function fail(message) {
    WScript.StdErr.WriteLine(message);
    WScript.Quit(1);
}

var jsxPath = WScript.Arguments.length > 0 ? WScript.Arguments.Item(0) : "";
var timeoutSec = WScript.Arguments.length > 1 ? parseInt(WScript.Arguments.Item(1), 10) : 120;
var photoshopExe = WScript.Arguments.length > 2 ? WScript.Arguments.Item(2) : "";

if (!jsxPath) {
    fail("Photoshop JSX script path was not provided.");
}

var filesystem = new ActiveXObject("Scripting.FileSystemObject");
if (!filesystem.FileExists(jsxPath)) {
    fail("Photoshop JSX script not found: " + jsxPath);
}

if (photoshopExe) {
    if (!filesystem.FileExists(photoshopExe)) {
        fail("Photoshop executable not found: " + photoshopExe);
    }
    var shell = new ActiveXObject("WScript.Shell");
    shell.Run('"' + photoshopExe + '"', 0, false);
}

var deadline = new Date().getTime() + Math.max(isNaN(timeoutSec) ? 120 : timeoutSec, 1) * 1000;
var app = null;
while (new Date().getTime() < deadline) {
    try {
        app = new ActiveXObject("Photoshop.Application");
        break;
    } catch (error) {
        WScript.Sleep(500);
    }
}

if (!app) {
    fail("Unable to connect to Photoshop COM automation within timeout.");
}

try {
    app.DoJavaScriptFile(jsxPath);
} catch (error) {
    fail("Photoshop script execution failed: " + error.message);
}
"#
}

fn build_jsx_script(
    request_path: &Path,
    response_path: &Path,
    staging_dir: &Path,
    request: &PhotoshopExportRequest,
) -> String {
    format!(
        r#"
var REQUEST_PATH = "{request_path}";
var RESPONSE_PATH = "{response_path}";
var STAGING_DIR = "{staging_dir}";
"#,
        request_path = js_path(request_path),
        response_path = js_path(response_path),
        staging_dir = js_path(staging_dir),
    ) + &format!(
        r#"

function readText(path) {{
    var file = new File(path);
    file.encoding = "UTF8";
    if (!file.open("r")) {{
        throw new Error("Unable to open file for read: " + path);
    }}
    var text = file.read();
    file.close();
    return text;
}}

function writeText(path, text) {{
    var file = new File(path);
    if (file.parent && !file.parent.exists) {{
        file.parent.create();
    }}
    file.encoding = "UTF8";
    if (!file.open("w")) {{
        throw new Error("Unable to open file for write: " + path);
    }}
    file.write(text);
    file.close();
}}

function ensureParentFolder(file) {{
    var current = file.parent;
    if (!current) {{
        return;
    }}
    var stack = [];
    while (current && !current.exists) {{
        stack.push(current);
        current = current.parent;
    }}
    for (var index = stack.length - 1; index >= 0; index--) {{
        stack[index].create();
    }}
}}

function jsonQuote(value) {{
    if (value === null || value === undefined) {{
        return "null";
    }}
    var text = String(value);
    text = text.replace(/\\/g, "\\\\");
    text = text.replace(/"/g, '\\"');
    text = text.replace(/\r/g, "\\r");
    text = text.replace(/\n/g, "\\n");
    text = text.replace(/\t/g, "\\t");
    return '"' + text + '"';
}}

function toJson(value) {{
    if (value === null || value === undefined) {{
        return "null";
    }}
    if (typeof value === "string") {{
        return jsonQuote(value);
    }}
    if (typeof value === "number" || typeof value === "boolean") {{
        return String(value);
    }}
    if (value instanceof Array) {{
        var items = [];
        for (var index = 0; index < value.length; index++) {{
            items.push(toJson(value[index]));
        }}
        return "[" + items.join(",") + "]";
    }}
    var pairs = [];
    for (var key in value) {{
        if (!value.hasOwnProperty(key)) {{
            continue;
        }}
        pairs.push(jsonQuote(key) + ":" + toJson(value[key]));
    }}
    return "{{" + pairs.join(",") + "}}";
}}

function parseJson(text) {{
    return eval("(" + text + ")");
}}

function asPixels(value) {{
    if (value === null || value === undefined) {{
        return 0;
    }}
    if (typeof value === "number") {{
        return Math.round(value);
    }}
    if (value.as) {{
        return Math.round(value.as("px"));
    }}
    return Math.round(Number(value));
}}

function layerCollection(container) {{
    if (container.layers !== undefined) {{
        return container.layers;
    }}
    return [];
}}

function resolveLayerByPath(documentRef, pathIndices) {{
    var current = documentRef;
    for (var index = 0; index < pathIndices.length; index++) {{
        var layers = layerCollection(current);
        current = layers[pathIndices[index]];
        if (!current) {{
            throw new Error("Unable to resolve layer path index " + pathIndices[index] + " at depth " + index);
        }}
    }}
    return current;
}}

function flattenLayersBottomUp(container) {{
    var result = [];
    var layers = layerCollection(container);
    for (var i = layers.length - 1; i >= 0; i--) {{
        var layer = layers[i];
        if (layer.typename === "LayerSet") {{
            result.push(null);
            var children = flattenLayersBottomUp(layer);
            for (var j = 0; j < children.length; j++) {{
                result.push(children[j]);
            }}
            result.push(layer);
        }} else {{
            result.push(layer);
        }}
    }}
    return result;
}}

var flatLayerMap = null;

function buildFlatLayerMap(documentRef) {{
    flatLayerMap = flattenLayersBottomUp(documentRef);
}}

function resolveLayer(documentRef, pathIndices, rawIndex) {{
    try {{
        return resolveLayerByPath(documentRef, pathIndices);
    }} catch (pathError) {{
        if (flatLayerMap && rawIndex !== undefined && rawIndex !== null && rawIndex < flatLayerMap.length) {{
            var layer = flatLayerMap[rawIndex];
            if (layer) {{
                return layer;
            }}
        }}
        throw pathError;
    }}
}}

function hideAllLayers(container) {{
    var layers = layerCollection(container);
    for (var index = 0; index < layers.length; index++) {{
        hideAllLayers(layers[index]);
        layers[index].visible = false;
    }}
}}

function revealLayerAncestry(layer) {{
    var current = layer;
    while (current && current.typename !== "Document") {{
        current.visible = true;
        current = current.parent;
    }}
}}

function collectBounds(layer) {{
    var bounds = layer.bounds;
    return {{
        x: asPixels(bounds[0]),
        y: asPixels(bounds[1]),
        width: Math.max(0, asPixels(bounds[2]) - asPixels(bounds[0])),
        height: Math.max(0, asPixels(bounds[3]) - asPixels(bounds[1]))
    }};
}}

function saveDocumentAsPng(documentRef, relativePath) {{
    var outputFile = new File(STAGING_DIR + "/" + relativePath.replace(/\\/g, "/"));
    ensureParentFolder(outputFile);
    var options = new PNGSaveOptions();
    options.interlaced = false;
    documentRef.saveAs(outputFile, options, true, Extension.LOWERCASE);
}}

function createLayerExportDocument(sourceDoc, layerBounds, documentName) {{
    return app.documents.add(
        UnitValue(Math.max(layerBounds.width, 1), "px"),
        UnitValue(Math.max(layerBounds.height, 1), "px"),
        sourceDoc.resolution,
        documentName,
        NewDocumentMode.RGB,
        DocumentFill.TRANSPARENT
    );
}}

function duplicateLayerIntoDocument(sourceDoc, sourceLayer, exportDoc) {{
    app.activeDocument = sourceDoc;
    sourceDoc.activeLayer = sourceLayer;
    var placeholderLayer = exportDoc.activeLayer;
    sourceLayer.duplicate(exportDoc, ElementPlacement.PLACEATBEGINNING);
    app.activeDocument = exportDoc;
    var duplicatedLayer = null;
    var layers = layerCollection(exportDoc);
    for (var index = 0; index < layers.length; index++) {{
        if (layers[index] != placeholderLayer) {{
            duplicatedLayer = layers[index];
            break;
        }}
    }}
    if (!duplicatedLayer) {{
        duplicatedLayer = exportDoc.activeLayer;
    }}
    if (placeholderLayer && exportDoc.layers.length > 1) {{
        placeholderLayer.remove();
    }}
    return duplicatedLayer;
}}

function unlockLayerForTransform(layer) {{
    try {{
        if (layer.isBackgroundLayer) {{
            layer.isBackgroundLayer = false;
        }}
    }} catch (error) {{
    }}
    try {{
        layer.allLocked = false;
    }} catch (error) {{
    }}
    try {{
        layer.positionLocked = false;
    }} catch (error) {{
    }}
}}

function normalizeLayerPosition(layer) {{
    var layerBounds = collectBounds(layer);
    if (layerBounds.x !== 0 || layerBounds.y !== 0) {{
        unlockLayerForTransform(layer);
        layer.translate(-layerBounds.x, -layerBounds.y);
    }}
}}

function cropDocumentToBounds(documentRef, layerBounds) {{
    documentRef.crop([
        UnitValue(layerBounds.x, "px"),
        UnitValue(layerBounds.y, "px"),
        UnitValue(layerBounds.x + layerBounds.width, "px"),
        UnitValue(layerBounds.y + layerBounds.height, "px")
    ]);
}}

function exportLayerViaIsolatedDocument(sourceDoc, layerRequest, layerBounds) {{
    var workingDoc = sourceDoc.duplicate();
    try {{
        var targetLayer = resolveLayerByPath(workingDoc, layerRequest.path_indices);
        hideAllLayers(workingDoc);
        revealLayerAncestry(targetLayer);
        cropDocumentToBounds(workingDoc, layerBounds);
        saveDocumentAsPng(workingDoc, layerRequest.output_png_relpath);
    }} finally {{
        workingDoc.close(SaveOptions.DONOTSAVECHANGES);
    }}
}}

function exportPreview(sourceDoc, response) {{
    var previewDoc = sourceDoc.duplicate();
    try {{
        saveDocumentAsPng(previewDoc, "{preview_relpath}");
        response.preview_path = "{preview_relpath}";
        response.preview_width = asPixels(previewDoc.width);
        response.preview_height = asPixels(previewDoc.height);
    }} finally {{
        previewDoc.close(SaveOptions.DONOTSAVECHANGES);
    }}
}}

function exportLayer(sourceDoc, layerRequest) {{
    var result = {{
        id: layerRequest.id,
        exported: false,
        bounds: null,
        width: null,
        height: null,
        warnings: []
    }};

    var layerBounds = null;
    var exportDoc = null;

    try {{
        var targetLayer = resolveLayer(sourceDoc, layerRequest.path_indices, layerRequest.raw_index);
        layerBounds = collectBounds(targetLayer);
        if (layerBounds.width <= 0 || layerBounds.height <= 0) {{
            result.warnings.push("layer bounds were empty");
        }} else {{
            try {{
                exportDoc = createLayerExportDocument(sourceDoc, layerBounds, layerRequest.id);
                var duplicatedLayer = duplicateLayerIntoDocument(sourceDoc, targetLayer, exportDoc);
                normalizeLayerPosition(duplicatedLayer);
                saveDocumentAsPng(exportDoc, layerRequest.output_png_relpath);
            }} catch (primaryError) {{
                if (exportDoc) {{
                    exportDoc.close(SaveOptions.DONOTSAVECHANGES);
                    exportDoc = null;
                }}
                exportLayerViaIsolatedDocument(sourceDoc, layerRequest, layerBounds);
                result.warnings.push("used isolated-document export fallback because " + primaryError);
            }}
            result.exported = true;
            result.bounds = layerBounds;
            result.width = layerBounds.width;
            result.height = layerBounds.height;
        }}
    }} catch (error) {{
        result.warnings.push(String(error));
    }} finally {{
        if (exportDoc) {{
            exportDoc.close(SaveOptions.DONOTSAVECHANGES);
        }}
    }}

    return result;
}}

var response = {{
    preview_path: null,
    preview_width: null,
    preview_height: null,
    warnings: [],
    layers: []
}};

var request = parseJson(readText(REQUEST_PATH));
var sourceDoc = null;

try {{
    app.displayDialogs = DialogModes.NO;
    sourceDoc = app.open(new File(request.source_psd_path));
    buildFlatLayerMap(sourceDoc);
    exportPreview(sourceDoc, response);
    for (var layerIndex = 0; layerIndex < request.layers.length; layerIndex++) {{
        response.layers.push(exportLayer(sourceDoc, request.layers[layerIndex]));
    }}
}} catch (fatalError) {{
    response.warnings.push(String(fatalError));
}} finally {{
    if (sourceDoc) {{
        sourceDoc.close(SaveOptions.DONOTSAVECHANGES);
    }}
    writeText(RESPONSE_PATH, toJson(response));
}}
"#,
        preview_relpath = request.preview_relpath.replace('\\', "/"),
    )
}

fn js_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    // fs::canonicalize on Windows produces \\?\ extended-length paths.
    // Photoshop ExtendScript's File() cannot parse that prefix, so strip it.
    let clean = raw.strip_prefix(r"\\?\").unwrap_or(&raw);
    // ExtendScript File() expects forward slashes.
    clean.replace('\\', "/")
        .replace('"', "\\\"")
}
