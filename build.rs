fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    std::env::set_var("PROTOC", protoc_bin_vendored::protoc_bin_path()?);

    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&["proto/quote_ledger/v1/quote_ledger.proto"], &["proto"])?;
    Ok(())
}
