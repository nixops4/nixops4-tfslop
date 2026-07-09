fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Generate gRPC stubs from the vendored tfplugin protocol definitions.
    // Only v6 is wired up initially — v5 support arrives when a real target
    // needs it.
    tonic_prost_build::configure().compile_protos(
        &["vendor/proto/tfplugin6.proto"],
        &["vendor/proto"],
    )?;

    println!("cargo:rerun-if-changed=vendor/proto/tfplugin6.proto");

    Ok(())
}
