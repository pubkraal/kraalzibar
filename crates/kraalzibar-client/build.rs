use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let proto_root = manifest_dir.join("../../proto");

    tonic_build::configure().compile_protos(
        &[
            proto_root.join("kraalzibar/v1/core.proto"),
            proto_root.join("kraalzibar/v1/permission.proto"),
            proto_root.join("kraalzibar/v1/relationship.proto"),
            proto_root.join("kraalzibar/v1/schema.proto"),
        ],
        &[&proto_root],
    )?;

    Ok(())
}
