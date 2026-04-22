use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    std::env::set_var("PROTOC", protoc_bin_vendored::protoc_bin_path()?);

    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    let descriptor_path = out_dir.join("quote_ledger_descriptor.bin");

    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .file_descriptor_set_path(&descriptor_path)
        .compile_protos(&["proto/quote_ledger/v1/quote_ledger.proto"], &["proto"])?;
    Ok(())
}
