//! Regenerates `schemas/termflow.schema.json`.

fn main() -> std::io::Result<()> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/termflow.schema.json");
    std::fs::write(&path, termflow_config::json_schema())?;
    println!("wrote {}", path.display());
    Ok(())
}
