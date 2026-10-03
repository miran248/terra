//! Launch the reproducible Blender production pipeline through the running MCP add-on.
use std::{
    path::Path,
    process::{Command, ExitCode},
};

fn main() -> ExitCode {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("blender/generate.py");
    match Command::new("python3")
        .arg(script)
        .arg("--production")
        .args(std::env::args_os().skip(1))
        .status()
    {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("Could not start Blender asset pipeline: {error}");
            ExitCode::FAILURE
        }
    }
}
