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
    pub photoshop_path: Option<PathBuf>,
    pub timeout_sec: u64,
}

#[derive(Debug, Clone)]
struct PreparedRuntime {
    runtime_dir: PathBuf,
    response_path: PathBuf,
    jsx_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostPlatform {
    Windows,
    MacOs,
    Unsupported,
}

pub fn run_photoshop_export(
    request: &PhotoshopExportRequest,
    options: &PhotoshopExportOptions,
    workspace_root: &Path,
) -> Result<PhotoshopExportResponse> {
    let runtime = prepare_runtime(request, workspace_root)?;

    match detect_host_platform() {
        HostPlatform::Windows => run_windows_driver(&runtime, options)?,
        HostPlatform::MacOs => run_macos_driver(&runtime, options)?,
        HostPlatform::Unsupported => {
            return Err(AppError::Photoshop(
                "Photoshop export is only supported on Windows and macOS hosts.".to_string(),
            ));
        }
    }

    let response_json =
        fs::read_to_string(&runtime.response_path).map_err(|error| AppError::io(&runtime.response_path, error))?;
    serde_json::from_str(&response_json).map_err(|error| {
        AppError::Photoshop(format!(
            "failed to parse Photoshop export response {}: {error}",
            runtime.response_path.display()
        ))
    })
}

fn prepare_runtime(request: &PhotoshopExportRequest, workspace_root: &Path) -> Result<PreparedRuntime> {
    let runtime_dir = workspace_root.join(".photoshop-runtime");
    if runtime_dir.exists() {
        fs::remove_dir_all(&runtime_dir).map_err(|error| AppError::io(&runtime_dir, error))?;
    }
    fs::create_dir_all(&runtime_dir).map_err(|error| AppError::io(&runtime_dir, error))?;

    let request_path = runtime_dir.join("request.json");
    let response_path = runtime_dir.join("response.json");
    let jsx_path = runtime_dir.join("export_layers.jsx");

    let request_json = serde_json::to_vec_pretty(request)?;
    fs::write(&request_path, request_json).map_err(|error| AppError::io(&request_path, error))?;

    let abs_request_path = fs::canonicalize(&request_path).map_err(|error| AppError::io(&request_path, error))?;
    let abs_runtime_dir = fs::canonicalize(&runtime_dir).map_err(|error| AppError::io(&runtime_dir, error))?;
    let abs_response_path = abs_runtime_dir.join("response.json");

    let staging_path = Path::new(&request.staging_dir);
    let abs_staging_dir = if staging_path.exists() {
        fs::canonicalize(staging_path).map_err(|error| AppError::io(staging_path, error))?
    } else {
        fs::create_dir_all(staging_path).map_err(|error| AppError::io(staging_path, error))?;
        fs::canonicalize(staging_path).map_err(|error| AppError::io(staging_path, error))?
    };

    fs::write(
        &jsx_path,
        build_jsx_script(&abs_request_path, &abs_response_path, &abs_staging_dir, request),
    )
    .map_err(|error| AppError::io(&jsx_path, error))?;

    Ok(PreparedRuntime {
        runtime_dir,
        response_path,
        jsx_path,
    })
}

fn run_windows_driver(runtime: &PreparedRuntime, options: &PhotoshopExportOptions) -> Result<()> {
    let driver_path = runtime.runtime_dir.join("run_photoshop_export.js");
    fs::write(&driver_path, windows_script_driver()).map_err(|error| AppError::io(&driver_path, error))?;

    let abs_jsx_path = fs::canonicalize(&runtime.jsx_path).map_err(|error| AppError::io(&runtime.jsx_path, error))?;
    let abs_driver_path = fs::canonicalize(&driver_path).map_err(|error| AppError::io(&driver_path, error))?;

    let mut command = Command::new("cscript.exe");
    command
        .arg("//NoLogo")
        .arg(&abs_driver_path)
        .arg(&abs_jsx_path)
        .arg(options.timeout_sec.to_string());

    if let Some(photoshop_path) = options.photoshop_path.as_ref().filter(|path| !path.as_os_str().is_empty()) {
        let canonical_path =
            validate_windows_photoshop_path(photoshop_path).and_then(|path| canonicalize_existing_path(&path))?;
        command.arg(canonical_path);
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

    Ok(())
}

fn run_macos_driver(runtime: &PreparedRuntime, options: &PhotoshopExportOptions) -> Result<()> {
    let driver_path = runtime.runtime_dir.join("run_photoshop_export.applescript");
    fs::write(&driver_path, macos_script_driver()).map_err(|error| AppError::io(&driver_path, error))?;

    let abs_jsx_path = fs::canonicalize(&runtime.jsx_path).map_err(|error| AppError::io(&runtime.jsx_path, error))?;
    let mut command = Command::new("osascript");
    command
        .arg(&driver_path)
        .arg(&abs_jsx_path)
        .arg(options.timeout_sec.to_string());

    if let Some(photoshop_path) = options.photoshop_path.as_ref().filter(|path| !path.as_os_str().is_empty()) {
        let canonical_path =
            validate_macos_photoshop_path(photoshop_path).and_then(|path| canonicalize_existing_path(&path))?;
        command.arg(canonical_path);
    }

    let output = command.output().map_err(|error| {
        AppError::Photoshop(format!("failed to launch macOS Photoshop automation driver: {error}"))
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(AppError::Photoshop(map_macos_driver_failure(
            stdout.trim(),
            stderr.trim(),
        )));
    }

    Ok(())
}

fn detect_host_platform() -> HostPlatform {
    host_platform_from(std::env::consts::OS)
}

fn host_platform_from(host_os: &str) -> HostPlatform {
    match host_os {
        "windows" => HostPlatform::Windows,
        "macos" => HostPlatform::MacOs,
        _ => HostPlatform::Unsupported,
    }
}

fn validate_windows_photoshop_path(path: &Path) -> Result<PathBuf> {
    if !path.exists() {
        return Err(AppError::Photoshop(format!(
            "Photoshop executable does not exist: {}",
            path.display()
        )));
    }

    Ok(path.to_path_buf())
}

fn validate_macos_photoshop_path(path: &Path) -> Result<PathBuf> {
    if !path.exists() {
        return Err(AppError::Photoshop(format!(
            "Photoshop application does not exist: {}",
            path.display()
        )));
    }

    let extension = path.extension().and_then(|value| value.to_str()).unwrap_or_default();
    if !extension.eq_ignore_ascii_case("app") {
        return Err(AppError::Photoshop(format!(
            "Photoshop application must be a .app bundle on macOS: {}",
            path.display()
        )));
    }

    Ok(path.to_path_buf())
}

fn canonicalize_existing_path(path: &Path) -> Result<PathBuf> {
    fs::canonicalize(path).map_err(|error| AppError::io(path, error))
}

fn windows_script_driver() -> &'static str {
    r#"
function fail(message) {
    WScript.StdErr.WriteLine(message);
    WScript.Quit(1);
}

var jsxPath = WScript.Arguments.length > 0 ? WScript.Arguments.Item(0) : "";
var timeoutSec = WScript.Arguments.length > 1 ? parseInt(WScript.Arguments.Item(1), 10) : 120;
var photoshopPath = WScript.Arguments.length > 2 ? WScript.Arguments.Item(2) : "";

if (!jsxPath) {
    fail("Photoshop JSX script path was not provided.");
}

var filesystem = new ActiveXObject("Scripting.FileSystemObject");
if (!filesystem.FileExists(jsxPath)) {
    fail("Photoshop JSX script not found: " + jsxPath);
}

if (photoshopPath) {
    if (!filesystem.FileExists(photoshopPath)) {
        fail("Photoshop executable not found: " + photoshopPath);
    }
    var shell = new ActiveXObject("WScript.Shell");
    shell.Run('"' + photoshopPath + '"', 0, false);
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

fn macos_script_driver() -> &'static str {
    r#"
on fail(messageText)
    error messageText number 1
end fail

on launchPhotoshop(photoshopPath)
    if photoshopPath is not "" then
        do shell script "/usr/bin/open -a " & quoted form of photoshopPath
    else
        tell application id "com.adobe.Photoshop" to activate
    end if
end launchPhotoshop

on waitForPhotoshop(timeoutSeconds)
    set deadlineDate to (current date) + timeoutSeconds
    set lastErrorMessage to ""
    repeat while (current date) is less than deadlineDate
        try
            tell application id "com.adobe.Photoshop"
                activate
                do javascript "1;"
            end tell
            return
        on error errMsg number errNum
            set lastErrorMessage to errMsg & " (" & errNum & ")"
            delay 0.5
        end try
    end repeat
    fail("Unable to connect to Photoshop via Apple events within timeout. Last error: " & lastErrorMessage)
end waitForPhotoshop

on run argv
    if (count of argv) < 2 then
        fail("Photoshop JSX script path and timeout must be provided.")
    end if

    set jsxPath to item 1 of argv
    set timeoutSec to (item 2 of argv) as integer
    set photoshopPath to ""
    if (count of argv) ≥ 3 then
        set photoshopPath to item 3 of argv
    end if

    if jsxPath is "" then
        fail("Photoshop JSX script path was not provided.")
    end if

    set jsxFile to POSIX file jsxPath

    if photoshopPath is not "" then
        set photoshopBundle to POSIX file photoshopPath
        tell application "System Events"
            if not (exists disk item photoshopBundle) then
                fail("Photoshop application does not exist: " & photoshopPath)
            end if
        end tell
    end if

    launchPhotoshop(photoshopPath)
    waitForPhotoshop(timeoutSec)

    try
        tell application id "com.adobe.Photoshop"
            activate
            do javascript of file jsxFile
        end tell
    on error errMsg number errNum
        fail("Photoshop script execution failed: " & errMsg & " (" & errNum & ")")
    end try
end run
"#
}

fn map_macos_driver_failure(stdout: &str, stderr: &str) -> String {
    let combined = if stdout.is_empty() {
        stderr.to_string()
    } else if stderr.is_empty() {
        stdout.to_string()
    } else {
        format!("{stdout} {stderr}")
    };

    if combined.contains("(-1743)") || combined.contains("Not authorized to send Apple events") {
        return "Photoshop Automation permission was denied. Allow the calling app to control Photoshop in System Settings > Privacy & Security > Automation, then retry the export.".to_string();
    }

    if combined.contains("Unable to connect to Photoshop via Apple events within timeout") {
        return combined;
    }

    if combined.contains("Photoshop application does not exist:")
        || combined.contains("Photoshop application must be a .app bundle")
    {
        return combined;
    }

    if combined.contains("application id \"com.adobe.Photoshop\"")
        || combined.contains("Can’t get application")
        || combined.contains("Application isn’t running")
    {
        return format!(
            "Unable to connect to Adobe Photoshop on macOS. Verify Photoshop is installed, launchable, and that Automation access is allowed. Driver output: {combined}"
        );
    }

    format!("macOS Photoshop automation driver failed: {combined}")
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
    let clean = raw.strip_prefix(r"\\?\").unwrap_or(&raw);
    clean.replace('\\', "/").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::{HostPlatform, host_platform_from, map_macos_driver_failure, validate_macos_photoshop_path};

    #[test]
    fn host_platform_selection_supports_windows_and_macos() {
        assert_eq!(host_platform_from("windows"), HostPlatform::Windows);
        assert_eq!(host_platform_from("macos"), HostPlatform::MacOs);
        assert_eq!(host_platform_from("linux"), HostPlatform::Unsupported);
    }

    #[test]
    fn macos_permission_denial_is_mapped_to_actionable_message() {
        let message = map_macos_driver_failure("", "execution error: Not authorized to send Apple events to Adobe Photoshop. (-1743)");
        assert!(message.contains("Automation permission was denied"));
    }

    #[test]
    fn macos_photoshop_path_requires_app_bundle() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let bundle_path = temp_dir.path().join("Adobe Photoshop 2025");
        std::fs::create_dir_all(&bundle_path).expect("fake photoshop dir");
        let error = validate_macos_photoshop_path(&bundle_path)
            .expect_err("non-.app path should be rejected");
        assert!(error.to_string().contains(".app bundle"));
    }
}
