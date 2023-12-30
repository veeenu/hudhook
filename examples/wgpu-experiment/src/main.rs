use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Result};
use dll_syringe::process::OwnedProcess;
use dll_syringe::Syringe;

fn main() -> Result<()> {
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let status = Command::new(cargo)
        .current_dir(project_root())
        .args(["build", "--lib", "--release"])
        .status()
        .map_err(|e| anyhow!("cargo: {}", e))?;

    if !status.success() {
        bail!("cargo build failed");
    }

    println!("{:?}", project_root());
    let dll_path =
        project_root().join("target").join("release").join("wgpu_experiment.dll").canonicalize()?;

    let process = OwnedProcess::find_first_by_name("DarkSoulsIII.exe")
        .ok_or_else(|| anyhow!("Could not find process"))?;
    let syringe = Syringe::for_process(process);
    syringe.inject(dll_path)?;

    Ok(())
}

fn project_root() -> PathBuf {
    Path::new(&env!("CARGO_MANIFEST_DIR")).to_path_buf()
}
