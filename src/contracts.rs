//! Versioned, explicit evidence for edges that cross repository boundaries.
use serde::Deserialize;

#[derive(Debug, Deserialize, serde::Serialize)]
pub struct Manifest {
    pub schema: String,
    pub package: Package,
    #[serde(default)]
    pub exports: Vec<Export>,
    #[serde(default)]
    pub imports: Vec<Import>,
}
#[derive(Debug, Deserialize, serde::Serialize)]
pub struct Package {
    pub name: String,
    pub version: String,
}
#[derive(Debug, Deserialize, serde::Serialize)]
pub struct Export {
    pub symbol: String,
    pub contract: String,
}
#[derive(Debug, Deserialize, serde::Serialize)]
pub struct Import {
    pub package: String,
    pub version: String,
    pub contract: String,
}

pub fn load(path: &std::path::Path) -> Result<Manifest, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let m: Manifest = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if m.schema != "glasir.contracts.v1"
        || m.package.name.is_empty()
        || m.package.version.is_empty()
    {
        return Err("invalid glasir contract manifest".into());
    }
    if m.exports
        .iter()
        .any(|e| e.symbol.is_empty() || e.contract.is_empty())
        || m.imports
            .iter()
            .any(|i| i.package.is_empty() || i.version.is_empty() || i.contract.is_empty())
    {
        return Err("empty contract field".into());
    }
    Ok(m)
}

#[derive(Debug, PartialEq, Eq)]
pub struct CrossEdge {
    pub source_package: String,
    pub target_package: String,
    pub contract: String,
    pub target_symbol: String,
}

pub fn report(manifests: &[Manifest]) -> serde_json::Value {
    let (edges, unresolved) = resolve(manifests);
    serde_json::json!({
        "schema": "glasir.cross-repo-report.v1",
        "edges": edges.iter().map(|edge| serde_json::json!({
            "source": edge.source_package, "target": edge.target_package,
            "contract": edge.contract, "symbol": edge.target_symbol,
            "evidence": "contract"
        })).collect::<Vec<_>>(),
        "unresolved": unresolved
    })
}

/// Resolves only exact package/version/contract matches. Version ranges are
/// intentionally not guessed here; an enterprise impact answer must name the
/// exact dependency snapshot it relied on.
pub fn resolve(manifests: &[Manifest]) -> (Vec<CrossEdge>, Vec<String>) {
    let mut edges = Vec::new();
    let mut unresolved = Vec::new();
    for source in manifests {
        for import in &source.imports {
            let target = manifests
                .iter()
                .find(|m| m.package.name == import.package && m.package.version == import.version);
            match target.and_then(|m| {
                m.exports
                    .iter()
                    .find(|e| e.contract == import.contract)
                    .map(|e| (m, e))
            }) {
                Some((target, export)) => edges.push(CrossEdge {
                    source_package: format!("{}@{}", source.package.name, source.package.version),
                    target_package: format!("{}@{}", target.package.name, target.package.version),
                    contract: import.contract.clone(),
                    target_symbol: export.symbol.clone(),
                }),
                None => unresolved.push(format!(
                    "{}@{} -> {}@{} ({})",
                    source.package.name,
                    source.package.version,
                    import.package,
                    import.version,
                    import.contract
                )),
            }
        }
    }
    edges.sort_by(|a, b| {
        (
            a.source_package.as_str(),
            a.target_package.as_str(),
            a.contract.as_str(),
        )
            .cmp(&(
                b.source_package.as_str(),
                b.target_package.as_str(),
                b.contract.as_str(),
            ))
    });
    unresolved.sort();
    (edges, unresolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest(
        name: &str,
        version: &str,
        exports: &[(&str, &str)],
        imports: &[(&str, &str, &str)],
    ) -> Manifest {
        Manifest {
            schema: "glasir.contracts.v1".into(),
            package: Package {
                name: name.into(),
                version: version.into(),
            },
            exports: exports
                .iter()
                .map(|(symbol, contract)| Export {
                    symbol: (*symbol).into(),
                    contract: (*contract).into(),
                })
                .collect(),
            imports: imports
                .iter()
                .map(|(package, version, contract)| Import {
                    package: (*package).into(),
                    version: (*version).into(),
                    contract: (*contract).into(),
                })
                .collect(),
        }
    }
    #[test]
    fn resolves_only_exact_versioned_contracts() {
        let api = manifest(
            "ledger",
            "4.0.0",
            &[("src/api.rs#post", "openapi:ledger.v1")],
            &[],
        );
        let app = manifest(
            "payments",
            "1.0.0",
            &[],
            &[("ledger", "4.0.0", "openapi:ledger.v1")],
        );
        let (edges, missing) = resolve(&[app, api]);
        assert_eq!(edges.len(), 1);
        assert!(missing.is_empty());
        assert_eq!(edges[0].target_symbol, "src/api.rs#post");
    }
    #[test]
    fn never_guesses_a_version_or_contract() {
        let api = manifest(
            "ledger",
            "4.0.0",
            &[("src/api.rs#post", "openapi:ledger.v1")],
            &[],
        );
        let app = manifest(
            "payments",
            "1.0.0",
            &[],
            &[("ledger", "4.1.0", "openapi:ledger.v2")],
        );
        let (edges, missing) = resolve(&[app, api]);
        assert!(edges.is_empty());
        assert_eq!(missing.len(), 1);
    }
    #[test]
    fn report_keeps_evidence_and_unresolved_imports() {
        let api = manifest("ledger", "4", &[("api#post", "openapi:v1")], &[]);
        let app = manifest(
            "payments",
            "1",
            &[],
            &[("ledger", "4", "openapi:v1"), ("missing", "1", "event:x")],
        );
        let value = report(&[app, api]);
        assert_eq!(value["schema"], "glasir.cross-repo-report.v1");
        assert_eq!(value["edges"][0]["evidence"], "contract");
        assert_eq!(value["unresolved"].as_array().unwrap().len(), 1);
    }
}
