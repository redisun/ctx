//! Cargo workspace metadata parsing and edge extraction.
//!
//! This module provides functionality to:
//! - Run `cargo metadata` to extract workspace structure
//! - Parse the JSON output into deterministically serializable types
//! - Generate relationship edges between packages, targets, and files
//!
//! # Example
//!
//! ```no_run
//! use ctx_core::CtxRepo;
//!
//! let mut repo = CtxRepo::open(".").unwrap();
//! let report = repo.analyze_cargo().unwrap();
//! println!("Analyzed {} packages", report.packages_found);
//! ```

use crate::error::{CtxError, Result};
use crate::types::{Confidence, Edge, EdgeLabel, Evidence, EvidenceTool, NodeId, NodeKind};
use crate::ObjectId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

/// Full Cargo workspace snapshot (deterministically serializable).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CargoMetadataSnapshot {
    /// Workspace root path.
    pub workspace_root: String,
    /// Packages in workspace (sorted by name for determinism).
    pub packages: Vec<Package>,
    /// Resolved dependency graph (optional).
    pub resolve: Option<Resolve>,
    /// Metadata format version.
    pub metadata_version: u32,
}

/// A Cargo package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Package {
    /// Package name.
    pub name: String,
    /// Package version.
    pub version: String,
    /// Package ID (unique identifier).
    pub id: String,
    /// Path to manifest.
    pub manifest_path: String,
    /// Rust edition.
    pub edition: String,
    /// Build targets (sorted by name).
    pub targets: Vec<Target>,
    /// Features (BTreeMap for determinism).
    pub features: BTreeMap<String, Vec<String>>,
    /// Dependencies (sorted by name).
    pub dependencies: Vec<PackageDep>,
    /// Default features.
    pub default_features: Vec<String>,
}

/// A build target within a package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    /// Target name.
    pub name: String,
    /// Target kind (lib, bin, test, bench, example, proc-macro).
    pub kind: TargetKind,
    /// Path to source file.
    pub src_path: String,
    /// Crate types (lib, rlib, dylib, etc.).
    pub crate_types: Vec<String>,
    /// Required features.
    pub required_features: Vec<String>,
}

/// Kind of build target.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TargetKind {
    /// Library crate (lib.rs).
    Lib = 1,
    /// Binary executable.
    Bin = 2,
    /// Test target.
    Test = 3,
    /// Benchmark target.
    Bench = 4,
    /// Example program.
    Example = 5,
    /// Procedural macro crate.
    ProcMacro = 6,
    /// Custom build script (build.rs).
    CustomBuild = 7,
}

impl TargetKind {
    /// Parse from cargo metadata "kind" array.
    pub fn from_cargo_kinds(kinds: &[String]) -> Self {
        for kind in kinds {
            match kind.as_str() {
                "lib" => return TargetKind::Lib,
                "bin" => return TargetKind::Bin,
                "test" => return TargetKind::Test,
                "bench" => return TargetKind::Bench,
                "example" => return TargetKind::Example,
                "proc-macro" => return TargetKind::ProcMacro,
                "custom-build" => return TargetKind::CustomBuild,
                _ => continue,
            }
        }
        TargetKind::Lib // Default fallback
    }
}

/// A package dependency declaration.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PackageDep {
    /// Dependency name (as used in code).
    pub name: String,
    /// Package name (may differ from name).
    pub package: Option<String>,
    /// Version requirement.
    pub req: String,
    /// Dependency kind.
    pub kind: DepKind,
    /// Optional dependency.
    pub optional: bool,
    /// Target platform filter.
    pub target: Option<String>,
    /// Features enabled on this dependency.
    pub features: Vec<String>,
    /// Whether default features are enabled.
    pub default_features: bool,
}

/// Kind of dependency.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DepKind {
    /// Normal dependency (used at runtime).
    Normal = 1,
    /// Development dependency (tests, examples).
    Dev = 2,
    /// Build dependency (build.rs scripts).
    Build = 3,
}

impl DepKind {
    /// Parse from cargo metadata "kind" field.
    pub fn from_cargo_kind(kind: Option<&str>) -> Self {
        match kind {
            Some("dev") => DepKind::Dev,
            Some("build") => DepKind::Build,
            _ => DepKind::Normal,
        }
    }
}

/// Resolved dependency graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolve {
    /// Root package ID (if single package workspace).
    pub root: Option<String>,
    /// Resolved nodes (sorted by id).
    pub nodes: Vec<ResolveNode>,
}

/// A node in the resolved dependency graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveNode {
    /// Package ID.
    pub id: String,
    /// Resolved dependencies (sorted).
    pub deps: Vec<ResolvedDep>,
    /// Enabled features (sorted).
    pub features: Vec<String>,
}

/// A resolved dependency reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedDep {
    /// Package ID of dependency.
    pub pkg: String,
    /// Name used to reference this dependency.
    pub name: String,
    /// Dependency kinds.
    pub dep_kinds: Vec<DepKindInfo>,
}

/// Dependency kind info.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepKindInfo {
    /// Dependency kind.
    pub kind: DepKind,
    /// Target platform filter.
    pub target: Option<String>,
}

/// Report from cargo analysis.
#[derive(Debug, Clone)]
pub struct CargoAnalysisReport {
    /// Number of packages found.
    pub packages_found: usize,
    /// Number of targets found.
    pub targets_found: usize,
    /// Number of dependencies found.
    pub dependencies_found: usize,
    /// Number of edges generated.
    pub edges_generated: usize,
    /// ObjectId of the stored snapshot.
    pub snapshot_id: ObjectId,
    /// ObjectId of the stored EdgeBatch.
    pub edge_batch_id: ObjectId,
    /// ObjectId of the created commit.
    pub commit_id: ObjectId,
}

/// Check if cargo is available.
pub fn is_available() -> bool {
    Command::new("cargo")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Run `cargo metadata` and return raw JSON.
///
/// # Errors
///
/// Returns an error if:
/// - Cargo.toml doesn't exist in the path
/// - cargo is not installed
/// - cargo metadata command fails
pub fn run_cargo_metadata(path: &Path) -> Result<String> {
    // Check for Cargo.toml
    let manifest = path.join("Cargo.toml");
    if !manifest.exists() {
        return Err(CtxError::NoCargoManifest(path.display().to_string()));
    }

    let output = Command::new("cargo")
        .arg("metadata")
        .arg("--format-version")
        .arg("1")
        .arg("--no-deps") // Faster, workspace only
        .current_dir(path)
        .output()
        .map_err(|e| CtxError::CargoMetadataFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CtxError::CargoMetadataFailed(stderr.to_string()));
    }

    String::from_utf8(output.stdout)
        .map_err(|e| CtxError::CargoMetadataFailed(format!("Invalid UTF-8: {}", e)))
}

/// Parse cargo metadata JSON into our types.
///
/// # Errors
///
/// Returns an error if the JSON is malformed or doesn't match expected schema.
pub fn parse_cargo_metadata(json: &str) -> Result<CargoMetadataSnapshot> {
    let value: Value =
        serde_json::from_str(json).map_err(|e| CtxError::CargoMetadataParseFailed(e.to_string()))?;

    let workspace_root = value["workspace_root"]
        .as_str()
        .ok_or_else(|| CtxError::CargoMetadataParseFailed("Missing workspace_root".to_string()))?
        .to_string();

    let metadata_version = value["version"]
        .as_u64()
        .ok_or_else(|| CtxError::CargoMetadataParseFailed("Missing version".to_string()))?
        as u32;

    let packages_array = value["packages"]
        .as_array()
        .ok_or_else(|| CtxError::CargoMetadataParseFailed("Missing packages array".to_string()))?;

    let mut packages = Vec::new();
    for pkg_val in packages_array {
        packages.push(parse_package(pkg_val)?);
    }

    // Sort packages by name for determinism
    packages.sort_by(|a, b| a.name.cmp(&b.name));

    let resolve = if let Some(resolve_val) = value.get("resolve") {
        if !resolve_val.is_null() {
            Some(parse_resolve(resolve_val)?)
        } else {
            None
        }
    } else {
        None
    };

    Ok(CargoMetadataSnapshot {
        workspace_root,
        packages,
        resolve,
        metadata_version,
    })
}

fn parse_package(val: &Value) -> Result<Package> {
    let name = val["name"]
        .as_str()
        .ok_or_else(|| CtxError::CargoMetadataParseFailed("Missing package name".to_string()))?
        .to_string();

    let version = val["version"]
        .as_str()
        .ok_or_else(|| CtxError::CargoMetadataParseFailed("Missing package version".to_string()))?
        .to_string();

    let id = val["id"]
        .as_str()
        .ok_or_else(|| CtxError::CargoMetadataParseFailed("Missing package id".to_string()))?
        .to_string();

    let manifest_path = val["manifest_path"]
        .as_str()
        .ok_or_else(|| CtxError::CargoMetadataParseFailed("Missing manifest_path".to_string()))?
        .to_string();

    let edition = val["edition"]
        .as_str()
        .ok_or_else(|| CtxError::CargoMetadataParseFailed("Missing edition".to_string()))?
        .to_string();

    // Parse targets
    let targets_array = val["targets"]
        .as_array()
        .ok_or_else(|| CtxError::CargoMetadataParseFailed("Missing targets array".to_string()))?;

    let mut targets = Vec::new();
    for target_val in targets_array {
        targets.push(parse_target(target_val)?);
    }
    targets.sort_by(|a, b| a.name.cmp(&b.name));

    // Parse features (BTreeMap for determinism)
    let mut features = BTreeMap::new();
    if let Some(features_obj) = val["features"].as_object() {
        for (key, val_arr) in features_obj {
            if let Some(arr) = val_arr.as_array() {
                let mut feature_list = Vec::new();
                for item in arr {
                    if let Some(s) = item.as_str() {
                        feature_list.push(s.to_string());
                    }
                }
                feature_list.sort();
                features.insert(key.clone(), feature_list);
            }
        }
    }

    // Parse dependencies
    let deps_array = val["dependencies"]
        .as_array()
        .ok_or_else(|| CtxError::CargoMetadataParseFailed("Missing dependencies".to_string()))?;

    let mut dependencies = Vec::new();
    for dep_val in deps_array {
        dependencies.push(parse_dependency(dep_val)?);
    }
    dependencies.sort_by(|a, b| a.name.cmp(&b.name));

    // Parse default features
    let mut default_features = Vec::new();
    if let Some(default_arr) = features.get("default") {
        default_features = default_arr.clone();
    }

    Ok(Package {
        name,
        version,
        id,
        manifest_path,
        edition,
        targets,
        features,
        dependencies,
        default_features,
    })
}

fn parse_target(val: &Value) -> Result<Target> {
    let name = val["name"]
        .as_str()
        .ok_or_else(|| CtxError::CargoMetadataParseFailed("Missing target name".to_string()))?
        .to_string();

    let kind_array = val["kind"]
        .as_array()
        .ok_or_else(|| CtxError::CargoMetadataParseFailed("Missing target kind".to_string()))?;

    let kind_strings: Vec<String> = kind_array
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();

    let kind = TargetKind::from_cargo_kinds(&kind_strings);

    let src_path = val["src_path"]
        .as_str()
        .ok_or_else(|| CtxError::CargoMetadataParseFailed("Missing src_path".to_string()))?
        .to_string();

    let empty_vec = vec![];
    let crate_types_array = val["crate_types"].as_array().unwrap_or(&empty_vec);
    let mut crate_types: Vec<String> = crate_types_array
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    crate_types.sort();

    let empty_vec2 = vec![];
    let required_features_array = val["required-features"].as_array().unwrap_or(&empty_vec2);
    let mut required_features: Vec<String> = required_features_array
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    required_features.sort();

    Ok(Target {
        name,
        kind,
        src_path,
        crate_types,
        required_features,
    })
}

fn parse_dependency(val: &Value) -> Result<PackageDep> {
    let name = val["name"]
        .as_str()
        .ok_or_else(|| CtxError::CargoMetadataParseFailed("Missing dependency name".to_string()))?
        .to_string();

    let package = val["rename"].as_str().map(|s| s.to_string());

    let req = val["req"]
        .as_str()
        .ok_or_else(|| CtxError::CargoMetadataParseFailed("Missing req".to_string()))?
        .to_string();

    let kind = DepKind::from_cargo_kind(val["kind"].as_str());

    let optional = val["optional"].as_bool().unwrap_or(false);

    let target = val["target"].as_str().map(|s| s.to_string());

    let empty_vec_feat = vec![];
    let features_array = val["features"].as_array().unwrap_or(&empty_vec_feat);
    let mut features: Vec<String> = features_array
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    features.sort();

    let default_features = val["uses_default_features"].as_bool().unwrap_or(true);

    Ok(PackageDep {
        name,
        package,
        req,
        kind,
        optional,
        target,
        features,
        default_features,
    })
}

fn parse_resolve(val: &Value) -> Result<Resolve> {
    let root = val["root"].as_str().map(|s| s.to_string());

    let nodes_array = val["nodes"]
        .as_array()
        .ok_or_else(|| CtxError::CargoMetadataParseFailed("Missing nodes array".to_string()))?;

    let mut nodes = Vec::new();
    for node_val in nodes_array {
        nodes.push(parse_resolve_node(node_val)?);
    }
    nodes.sort_by(|a, b| a.id.cmp(&b.id));

    Ok(Resolve { root, nodes })
}

fn parse_resolve_node(val: &Value) -> Result<ResolveNode> {
    let id = val["id"]
        .as_str()
        .ok_or_else(|| CtxError::CargoMetadataParseFailed("Missing node id".to_string()))?
        .to_string();

    let empty_vec_deps = vec![];
    let deps_array = val["deps"].as_array().unwrap_or(&empty_vec_deps);
    let mut deps = Vec::new();
    for dep_val in deps_array {
        deps.push(parse_resolved_dep(dep_val)?);
    }
    deps.sort_by(|a, b| a.name.cmp(&b.name));

    let empty_vec_feat = vec![];
    let features_array = val["features"].as_array().unwrap_or(&empty_vec_feat);
    let mut features: Vec<String> = features_array
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    features.sort();

    Ok(ResolveNode { id, deps, features })
}

fn parse_resolved_dep(val: &Value) -> Result<ResolvedDep> {
    let pkg = val["pkg"]
        .as_str()
        .ok_or_else(|| CtxError::CargoMetadataParseFailed("Missing dep pkg".to_string()))?
        .to_string();

    let name = val["name"]
        .as_str()
        .ok_or_else(|| CtxError::CargoMetadataParseFailed("Missing dep name".to_string()))?
        .to_string();

    let empty_vec_dk = vec![];
    let dep_kinds_array = val["dep_kinds"].as_array().unwrap_or(&empty_vec_dk);
    let mut dep_kinds = Vec::new();
    for dk_val in dep_kinds_array {
        let kind = DepKind::from_cargo_kind(dk_val["kind"].as_str());
        let target = dk_val["target"].as_str().map(|s| s.to_string());
        dep_kinds.push(DepKindInfo { kind, target });
    }

    Ok(ResolvedDep {
        pkg,
        name,
        dep_kinds,
    })
}

/// Extract edges from cargo metadata snapshot.
///
/// Generates the following edge types:
/// - Package → DependsOn → Package (for dependencies)
/// - Target → TargetOf → Package (target membership)
/// - Crate → CrateFromTarget → Target (for lib/proc-macro targets)
/// - File → Contains → Target (source file entry points)
pub fn extract_cargo_edges(snapshot: &CargoMetadataSnapshot, commit_id: ObjectId) -> Vec<Edge> {
    let mut edges = Vec::new();

    for package in &snapshot.packages {
        let pkg_id = format!("{}@{}", package.name, package.version);

        // Package → DependsOn → Package
        for dep in &package.dependencies {
            let dep_pkg = dep.package.as_ref().unwrap_or(&dep.name);
            edges.push(Edge {
                from: NodeId {
                    kind: NodeKind::Package,
                    id: pkg_id.clone(),
                },
                to: NodeId {
                    kind: NodeKind::Package,
                    id: dep_pkg.clone(),
                },
                label: EdgeLabel::DependsOn,
                weight: Some(match dep.kind {
                    DepKind::Normal => 1000, // Strong coupling
                    DepKind::Build => 500,   // Build-time only
                    DepKind::Dev => 200,     // Test/dev only
                }),
                evidence: Evidence {
                    commit_id,
                    tool: EvidenceTool::Cargo,
                    confidence: Confidence::High,
                    span: None,
                    blob_id: None,
                },
            });
        }

        // Target edges
        for target in &package.targets {
            let target_id = format!("{}::{}", package.name, target.name);

            // Target → TargetOf → Package
            edges.push(Edge {
                from: NodeId {
                    kind: NodeKind::Target,
                    id: target_id.clone(),
                },
                to: NodeId {
                    kind: NodeKind::Package,
                    id: pkg_id.clone(),
                },
                label: EdgeLabel::TargetOf,
                weight: None,
                evidence: Evidence {
                    commit_id,
                    tool: EvidenceTool::Cargo,
                    confidence: Confidence::High,
                    span: None,
                    blob_id: None,
                },
            });

            // Crate → CrateFromTarget → Target (for lib targets)
            if target.kind == TargetKind::Lib || target.kind == TargetKind::ProcMacro {
                let crate_name = package.name.replace("-", "_");
                edges.push(Edge {
                    from: NodeId {
                        kind: NodeKind::Crate,
                        id: crate_name,
                    },
                    to: NodeId {
                        kind: NodeKind::Target,
                        id: target_id.clone(),
                    },
                    label: EdgeLabel::CrateFromTarget,
                    weight: None,
                    evidence: Evidence {
                        commit_id,
                        tool: EvidenceTool::Cargo,
                        confidence: Confidence::High,
                        span: None,
                        blob_id: None,
                    },
                });
            }

            // File → Contains → Target
            edges.push(Edge {
                from: NodeId {
                    kind: NodeKind::File,
                    id: target.src_path.clone(),
                },
                to: NodeId {
                    kind: NodeKind::Target,
                    id: target_id,
                },
                label: EdgeLabel::Contains,
                weight: None,
                evidence: Evidence {
                    commit_id,
                    tool: EvidenceTool::Cargo,
                    confidence: Confidence::High,
                    span: None,
                    blob_id: None,
                },
            });
        }
    }

    // Sort for determinism
    edges.sort_by(|a, b| (&a.from, &a.to, &a.label).cmp(&(&b.from, &b.to, &b.label)));

    edges
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ObjectStore;
    use tempfile::TempDir;

    #[test]
    fn test_target_kind_from_cargo_kinds() {
        assert_eq!(
            TargetKind::from_cargo_kinds(&["lib".to_string()]),
            TargetKind::Lib
        );
        assert_eq!(
            TargetKind::from_cargo_kinds(&["bin".to_string()]),
            TargetKind::Bin
        );
        assert_eq!(
            TargetKind::from_cargo_kinds(&["proc-macro".to_string()]),
            TargetKind::ProcMacro
        );
    }

    #[test]
    fn test_dep_kind_from_cargo_kind() {
        assert_eq!(DepKind::from_cargo_kind(None), DepKind::Normal);
        assert_eq!(DepKind::from_cargo_kind(Some("dev")), DepKind::Dev);
        assert_eq!(DepKind::from_cargo_kind(Some("build")), DepKind::Build);
    }

    #[test]
    fn test_snapshot_determinism() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        let snapshot = CargoMetadataSnapshot {
            workspace_root: "/test".to_string(),
            packages: vec![],
            resolve: None,
            metadata_version: 1,
        };

        // Same content = same ID
        let id1 = store.put_typed(&snapshot).unwrap();
        let id2 = store.put_typed(&snapshot).unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_edge_extraction() {
        let commit_id = ObjectId::from_bytes([1; 32]);

        let snapshot = CargoMetadataSnapshot {
            workspace_root: "/test".to_string(),
            packages: vec![Package {
                name: "my_pkg".to_string(),
                version: "0.1.0".to_string(),
                id: "my_pkg 0.1.0".to_string(),
                manifest_path: "/test/Cargo.toml".to_string(),
                edition: "2021".to_string(),
                targets: vec![Target {
                    name: "my_pkg".to_string(),
                    kind: TargetKind::Lib,
                    src_path: "/test/src/lib.rs".to_string(),
                    crate_types: vec!["lib".to_string()],
                    required_features: vec![],
                }],
                features: BTreeMap::new(),
                dependencies: vec![PackageDep {
                    name: "serde".to_string(),
                    package: None,
                    req: "^1.0".to_string(),
                    kind: DepKind::Normal,
                    optional: false,
                    target: None,
                    features: vec![],
                    default_features: true,
                }],
                default_features: vec![],
            }],
            resolve: None,
            metadata_version: 1,
        };

        let edges = extract_cargo_edges(&snapshot, commit_id);

        // Should have: DependsOn, TargetOf, CrateFromTarget, Contains
        assert!(edges.len() >= 4);

        // Check DependsOn edge
        let depends_on = edges
            .iter()
            .find(|e| e.label == EdgeLabel::DependsOn)
            .unwrap();
        assert_eq!(depends_on.from.kind, NodeKind::Package);
        assert_eq!(depends_on.to.id, "serde");

        // Check all edges have Cargo tool
        for edge in &edges {
            assert_eq!(edge.evidence.tool, EvidenceTool::Cargo);
            assert_eq!(edge.evidence.confidence, Confidence::High);
        }
    }
}
