use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{OrchestratorError, Result};
use crate::models::{COMPONENT_CONTAINER, COMPONENT_IMAGE, COMPONENT_TEXT, PlanNode, UiPlan};

pub fn load_plan(plan_path: &Path) -> Result<UiPlan> {
    let path = resolve_plan_path(plan_path);
    if !path.exists() {
        return Err(OrchestratorError::MissingFile(path));
    }

    let contents = fs::read_to_string(&path).map_err(|source| OrchestratorError::ReadFile {
        path: path.clone(),
        source,
    })?;

    serde_json::from_str(&contents).map_err(|source| OrchestratorError::ParseJson { path, source })
}

pub fn resolve_plan_path(input: &Path) -> PathBuf {
    if input.is_dir() {
        input.join("ui_plan.json")
    } else {
        input.to_path_buf()
    }
}

pub fn validate_plan(plan: &UiPlan) -> Result<()> {
    let mut errors = Vec::new();
    let mut node_ids = HashSet::new();

    if plan.plan_version.trim().is_empty() {
        errors.push("plan_version must not be empty".to_string());
    }
    if plan.source_bundle.document_id.trim().is_empty() {
        errors.push("source_bundle.document_id must not be empty".to_string());
    }

    for node in &plan.nodes {
        validate_node(node, &mut node_ids, &mut errors);
    }

    if !errors.is_empty() {
        return Err(OrchestratorError::PlanValidation(errors.join("\n")));
    }

    Ok(())
}

fn validate_node(node: &PlanNode, node_ids: &mut HashSet<String>, errors: &mut Vec<String>) {
    if node.node_id.trim().is_empty() {
        errors.push("node_id must not be empty".to_string());
    } else if !node_ids.insert(node.node_id.clone()) {
        errors.push(format!("duplicate node_id detected: {}", node.node_id));
    }

    if node.source_layer_ids.is_empty() {
        errors.push(format!("node {} is missing source_layer_ids", node.node_id));
    }

    if !(0.0..=1.0).contains(&node.confidence) {
        errors.push(format!(
            "node {} has invalid confidence {}",
            node.node_id, node.confidence
        ));
    }

    let supported = [
        COMPONENT_CONTAINER,
        COMPONENT_IMAGE,
        COMPONENT_TEXT,
    ];

    if !supported.contains(&node.component_type.as_str()) {
        errors.push(format!(
            "node {} has unsupported component_type {}",
            node.node_id, node.component_type
        ));
    }

    if node.component_type == COMPONENT_TEXT && node.text.is_none() {
        errors.push(format!(
            "text node {} is missing text payload",
            node.node_id
        ));
    }

    for child in &node.children {
        validate_node(child, node_ids, errors);
    }
}
