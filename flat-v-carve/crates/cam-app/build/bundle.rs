use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, io, path::Path};

const MANIFEST: &str = ".bundle-manifest.json";
const ROOT_INPUTS: &[&str] = &[
    "index.html",
    "package.json",
    "pnpm-lock.yaml",
    "tsconfig.json",
    "vite.config.ts",
];

fn files(root: &Path, relative: &str, result: &mut Vec<String>) -> io::Result<()> {
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.is_symlink() {
        return Err(io::Error::other(format!(
            "Bundle paths must not be symlinks: {}",
            path.display()
        )));
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(&path)? {
            let name = entry?
                .file_name()
                .into_string()
                .map_err(|_| io::Error::other("Bundle names must be UTF-8"))?;
            files(
                root,
                &if relative.is_empty() {
                    name
                } else {
                    format!("{relative}/{name}")
                },
                result,
            )?;
        }
    } else if metadata.is_file() {
        result.push(relative.to_owned());
    } else {
        return Err(io::Error::other("Bundle paths must be regular files"));
    }
    Ok(())
}
fn hashes(root: &Path, paths: &[String]) -> io::Result<BTreeMap<String, String>> {
    paths
        .iter()
        .map(|path| {
            Ok((
                path.clone(),
                format!("{:x}", Sha256::digest(fs::read(root.join(path))?)),
            ))
        })
        .collect()
}

/// A source or asset change requires rebuilding the frontend, never a stale UI.
pub fn generate(web: &Path, engine_version: &str) -> Result<String, Box<dyn std::error::Error>> {
    let dist = web.join("dist");
    let manifest: serde_json::Value = serde_json::from_slice(&fs::read(dist.join(MANIFEST))?)?;
    if manifest["schemaVersion"] != 1 || manifest["engineVersion"] != engine_version {
        return Err("UI manifest schema or engine version differs from this build".into());
    }
    let mut sources: Vec<String> = ROOT_INPUTS.iter().map(|p| (*p).into()).collect();
    for directory in ["src", "scripts", "public"] {
        if web.join(directory).exists() {
            files(web, directory, &mut sources)?;
        }
    }
    let expected: BTreeMap<String, String> = serde_json::from_value(manifest["sources"].clone())?;
    if hashes(web, &sources)? != expected {
        return Err("UI sources changed since pnpm build; rebuild the frontend".into());
    }
    let mut assets = Vec::new();
    files(&dist, "", &mut assets)?;
    assets.retain(|path| path != MANIFEST);
    assets.sort();
    let expected: BTreeMap<String, String> = serde_json::from_value(manifest["assets"].clone())?;
    if !assets.iter().any(|path| path == "index.html") || hashes(&dist, &assets)? != expected {
        return Err("UI assets are missing, changed, or inconsistent with their manifest; rebuild the frontend".into());
    }
    let mut total = 0;
    let mut generated = String::from("const BUNDLED_ASSETS: &[(&str, &[u8])] = &[\n");
    for asset in assets {
        let path = dist.join(&asset);
        total += fs::metadata(&path)?.len();
        if total > 32_000_000 {
            return Err("Bundled UI exceeds the service's 32 MB asset limit".into());
        }
        generated.push_str(&format!(
            "    ({asset:?}, include_bytes!({:?})),\n",
            path.to_str().ok_or("UI path must be UTF-8")?
        ));
    }
    generated.push_str("];\n");
    Ok(generated)
}
