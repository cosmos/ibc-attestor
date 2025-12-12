use std::{env, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("env var should be valid"));

    let attestor_descriptor_path = out_dir.join("ibc_attestor_descriptor.bin");
    tonic_build::configure()
        .file_descriptor_set_path(&attestor_descriptor_path)
        .build_server(true)
        .compile_protos(
            &["../../proto/ibc_attestor/ibc_attestor.proto"],
            &["../../proto"],
        )?;

    let signer_descriptor_path = out_dir.join("signer_descriptor.bin");
    tonic_build::configure()
        .file_descriptor_set_path(&signer_descriptor_path)
        .build_client(true)
        .build_server(false)
        .compile_protos(
            &["../../proto/signer/signerservice.proto"],
            &["../../proto"],
        )?;

    Ok(())
}
